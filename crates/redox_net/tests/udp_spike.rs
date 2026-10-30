//! NET-01 spike measurements over real UDP on 127.0.0.1. Ignored in normal runs because it takes
//! seconds and prints figures:
//! `cargo test -p redox_net --test udp_spike -- --ignored --nocapture`
//!
//! Not measured here: loss. The UDP backend does not inject loss; the 10 % loss case is covered by
//! `LoopbackWire` in `tests/loopback.rs`. Reliable payloads are 1100 bytes because v1 has no
//! fragmentation (the spec's 2 KB case is out of scope, see ADR 0008).

use std::net::SocketAddr;
use std::thread::sleep;
use std::time::{Duration, Instant};

use redox_net::{Channel, ClientId, ConnectionState, NetClient, NetServer, UdpTransport};

const UNRELIABLE_COUNT: u32 = 1000;
const UNRELIABLE_BYTES: usize = 100;
const RELIABLE_COUNT: u32 = 100;
const RELIABLE_BYTES: usize = 1100;
const WAIT: Duration = Duration::from_secs(10);

#[test]
#[ignore = "NET-01 spike measurement: run with --ignored --nocapture"]
fn udp_spike_prints_rtt_and_delivery() {
    let transport =
        UdpTransport::server(SocketAddr::from(([127, 0, 0, 1], 0))).expect("bind a free port");
    let addr = transport.local_addr().expect("bound socket has an address");
    let mut server = NetServer::new(transport);
    let mut client = NetClient::new(UdpTransport::client(addr).expect("client socket"));
    let mut last = Instant::now();
    let deadline = Instant::now() + WAIT;

    let mut step = |server: &mut NetServer<UdpTransport>, client: &mut NetClient<UdpTransport>| {
        let now = Instant::now();
        let dt = now.duration_since(last);
        last = now;
        server.poll(dt);
        client.poll(dt);
        sleep(Duration::from_millis(1));
    };
    while client.state() != ConnectionState::Connected && Instant::now() < deadline {
        step(&mut server, &mut client);
    }
    assert_eq!(client.state(), ConnectionState::Connected, "no connection");

    // Unreliable burst: count what arrives within the window.
    let payload = vec![7u8; UNRELIABLE_BYTES];
    for _ in 0..UNRELIABLE_COUNT {
        server
            .send(ClientId(1), Channel::Unreliable, &payload)
            .expect("connected");
    }
    let mut delivered_unreliable = 0u32;
    let unreliable_until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < unreliable_until {
        step(&mut server, &mut client);
        while client.recv(Channel::Unreliable).is_some() {
            delivered_unreliable += 1;
        }
    }

    // Reliable stream: time until every message arrived in order.
    let reliable = vec![3u8; RELIABLE_BYTES];
    let started = Instant::now();
    let mut delivered_reliable = 0u32;
    let mut sent = 0u32;
    while delivered_reliable < RELIABLE_COUNT && Instant::now() < deadline {
        if sent < RELIABLE_COUNT {
            server
                .send(ClientId(1), Channel::Reliable, &reliable)
                .expect("connected");
            sent += 1;
        }
        step(&mut server, &mut client);
        while client.recv(Channel::Reliable).is_some() {
            delivered_reliable += 1;
        }
    }
    let reliable_time = started.elapsed();

    // Wait for the first ping answer, then read the round-trip time.
    while server.rtt(ClientId(1)).is_none() && Instant::now() < deadline {
        step(&mut server, &mut client);
    }

    println!("udp spike (127.0.0.1, no injected loss):");
    println!("  rtt: {:?}", server.rtt(ClientId(1)));
    println!("  unreliable {UNRELIABLE_BYTES} B x {UNRELIABLE_COUNT}: delivered {delivered_unreliable}");
    println!(
        "  reliable {RELIABLE_BYTES} B x {RELIABLE_COUNT}: delivered {delivered_reliable} in {reliable_time:?}"
    );
}
