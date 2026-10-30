# NET-01 spike notes

Date: 2026-10-30. Method: docs.rs pages only. Nothing was downloaded, built or run; the cargo
measurements requested by the spec (RTT, 10 % loss, 1000 x 100 B unreliable + 100 x 2 KB reliable)
are NOT done. ASSUMPTION: the choice below holds until those measurements exist.

## Candidates

| Candidate | Version | Licence | Async | Notes from docs.rs |
|---|---|---|---|---|
| renet | 2.0.0 | MIT OR Apache-2.0 | none | deps: bytes, log, octets. Channels: unreliable, reliable ordered, reliable unordered. Fragmentation, MTU, socket type not stated. |
| renet_netcode | 2.0.0 | MIT OR Apache-2.0 | none | deps: log, renet, renetcode. Connection tokens and secure/unsecure auth. Socket type not stated. |
| quinn | 0.11.12 | MIT OR Apache-2.0 | tokio required | rustls optional. Rejected. |
| laminar | 0.5.0 | MIT OR Apache-2.0 | none | docs show release 2026-09-27; no release history, maintenance unconfirmed. |
| own layer (std UdpSocket) | - | workspace | none | implemented in `crates/redox_net`, tested over loopback. |

## Result

Chosen: own layer over `std::net::UdpSocket` behind the `Transport` trait (see ADR 0008). renet_netcode
kept as the candidate for encryption later. Reasons: no socket needed for the NET-02/03 tests, no new
dependency, MTU/fragmentation behaviour of renet unverified.

## Not measured (to do before the UDP adapter is trusted)

- RTT and jitter of the UDP adapter on 127.0.0.1.
- Delivery of 1000 unreliable 100 B and 100 reliable messages under 10 % loss (loopback covers the
  logic; real sockets are unmeasured).
- Whether a 2 KB reliable message is needed: v1 limits payloads to 1195 bytes.
