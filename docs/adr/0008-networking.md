# ADR 0008: Networking (transport choice, client-server model, own reliability layer)

Status: proposed (2026-10-30, written by Claude as the NET-01 executor; needs owner confirmation of the
assumptions below before NET-02 starts).

Spec: `docs/engineering/specs/NET-01.md`. Spike notes: `docs/engineering/notes/net-spike.md`.
Crate facts were read from docs.rs pages on 2026-10-30. No crate was downloaded, built or run in this
task: the spike measurements (RTT, loss, MTU) are not done and are marked `ASSUMPTION`.

## Context

The engine has no networking subsystem yet. NET-02 (ECS replication) and NET-03 (prediction) need:

- a server that is authoritative and runs a fixed tick,
- reliable ordered messages, unreliable messages, and snapshots where only the newest counts,
- a transport that can be replaced without touching game code,
- tests that run without sockets.

## Decision

1. **Client-server, authoritative server.** A listen server is the server in the same process as the
   client (ASSUMPTION 1 from the spec).
2. **UDP datagrams** through a `Transport` trait (`crates/redox_net/src/transport.rs`). The trait is
   the only seam; game code and the NET-02/03 layers never see a backend type.
3. **Reliability and ordering are implemented in `redox_net`**, on top of datagrams. ACK frames carry
   the next expected sequence number, unacknowledged packets are resent every 100 ms, the receiver
   buffers up to 1024 packets. Unreliable packets carry a sequence number and a 64-packet duplicate
   window, so nothing reaches the game twice. Snapshots are newest-wins.
4. **Serialization:** `postcard` with a `u16` message id header (`Message` trait). Already in the tree
   through `redox_scene`.
5. **Fixed tick 30 Hz** on the server (`SERVER_TICK_RATE_HZ`), at most 8 ticks per poll. The client
   renders every frame and estimates the server tick from snapshot tick + RTT/2, smoothed with an EWMA.
6. **Only Windows, up to 16 players**, `CLIENT_TIMEOUT` 5 s.
7. **No encryption in v1.** Local network and friends only (ASSUMPTION 6).

## Transport candidates (checked on docs.rs, 2026-10-30)

| Candidate | Version | Licence | Async runtime | Reliability / channels | Fit |
|---|---|---|---|---|---|
| `renet` | 2.0.0 | MIT OR Apache-2.0 | none in deps (`bytes`, `log`, `octets`) | unreliable, reliable ordered, reliable unordered | Good. Fragmentation, MTU and MSRV not shown on docs.rs (unverified). |
| `renet_netcode` | 2.0.0 | MIT OR Apache-2.0 | none (`log`, `renet`, `renetcode`) | connection tokens, key-based secure mode (scheme not spelled out on docs.rs) | Good for a later encryption step. Socket type not shown (unverified). |
| `quinn` | 0.11.12 | MIT OR Apache-2.0 | **requires tokio** (`^1.28.1`) | QUIC streams and datagrams | Rejected: forces an async runtime into the game loop and the dependency tree. |
| `laminar` | 0.5.0 (released 2026-09-27 per docs.rs) | MIT OR Apache-2.0 | none (synchronous) | unreliable and reliable, sequenced, multi-stream | Possible, but the docs do not show release history, so maintenance is unconfirmed. |
| own layer over `std::net::UdpSocket` | this crate | workspace | none | ACK, resend, window, dedupe, snapshot gate | Chosen. About 600 lines, all tested over loopback. |

## Why our own layer (and not renet now)

- The layer is small (framing, ARQ, dedupe, snapshot gate) and already implemented and tested over
  `LoopbackWire`, so the NET-02/03 tests do not depend on a socket.
- `renet` brings its own channel model, connection state and packet limits. Its fragmentation and
  MTU behaviour could not be verified from documentation, and the spike that would measure it was not
  run in this task.
- No new dependency is needed for NET-01: `postcard`, `serde`, `thiserror` and `log` are already in
  the workspace. `cargo deny` stays unchanged.
- `renet_netcode` stays the candidate for the encryption and connection-token step. It fits behind the
  same `Transport` trait later, without changing the layer above it.

Cost of the choice: we own the ARQ code and its bugs. If it proves too slow in the spike measurements
or in NET-02, `renet` replaces the `Transport` and its reliable channel can replace our ARQ.

## Limits (v1)

- Payload per datagram at most 1200 bytes; reliable payloads at most 1195 bytes. No fragmentation, so
  the 2 KB reliable messages in the spike plan are out of scope for v1 (ASSUMPTION 7).
- Channels are carried over the transport's unreliable path. The `Channel::Reliable` label is honoured
  by our layer, not by the transport.
- Client ids are non-zero; `SERVER_PEER` is `ClientId(0)`.
- Not in scope: NAT traversal, matchmaking, voice, dedicated orchestration, encryption beyond what a
  later `renet_netcode` step gives.

## Consequences

- `crates/redox_net` has no socket code yet. The UDP adapter (`transport_udp.rs`, `std::net::UdpSocket`)
  is the next step and needs a measured spike before it is trusted.
- NET-02/03 build on the `Transport` trait and `LoopbackWire`.

## Open questions for the owner

1. Confirm the own-layer choice, or ask for a `renet` spike before the UDP adapter is written?
2. Is 16 players and 30 Hz still right for the first game?
3. Is a 1200-byte datagram limit (no fragmentation) acceptable for v1 reliable messages?
