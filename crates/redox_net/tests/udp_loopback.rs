//! Real sockets on 127.0.0.1 with an OS-chosen port: connect, exchange reliable messages, measure
//! the round-trip time and disconnect. Wall-clock timeouts keep a broken socket from hanging the run.

use std::net::SocketAddr;
use std::thread::sleep;
use std::time::{Duration, Instant};

use redox_net::{
    Channel, ClientId, ConnectionState, DisconnectReason, NetClient, NetEvent, NetServer,
    UdpTransport,
};

const TIMEOUT: Duration = Duration::from_secs(10);

fn localhost_any_port() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// One step of both endpoints, with the real time since the previous step as `dt`.
fn pump(
    server: &mut NetServer<UdpTransport>,
    client: &mut NetClient<UdpTransport>,
    last: &mut Instant,
    server_events: &mut Vec<NetEvent>,
) {
    let now = Instant::now();
    let dt = now.duration_since(*last);
    *last = now;
    server_events.extend(server.poll(dt));
    client.poll(dt);
    sleep(Duration::from_millis(1));
}

fn decode_index(bytes: &[u8]) -> u32 {
    let raw: [u8; 4] = bytes.try_into().expect("every payload is four bytes");
    u32::from_le_bytes(raw)
}

#[test]
fn a_client_connects_exchanges_messages_and_disconnects_over_udp() {
    let transport = UdpTransport::server(localhost_any_port()).expect("bind a free port");
    let addr = transport.local_addr().expect("bound socket has an address");
    let mut server = NetServer::new(transport);
    let mut client = NetClient::new(UdpTransport::client(addr).expect("client socket"));
    let mut last = Instant::now();
    let mut server_events = Vec::new();
    let deadline = Instant::now() + TIMEOUT;

    while client.state() != ConnectionState::Connected {
        assert!(Instant::now() < deadline, "the client did not connect");
        pump(&mut server, &mut client, &mut last, &mut server_events);
    }
    assert_eq!(server.client_ids(), vec![ClientId(1)]);
    assert!(server_events.contains(&NetEvent::Connected(ClientId(1))));

    for index in 0..100u32 {
        server
            .send(ClientId(1), Channel::Reliable, &index.to_le_bytes())
            .expect("the client is connected");
        client
            .send(Channel::Reliable, &index.to_le_bytes())
            .expect("the connection is open");
    }
    let mut from_server = Vec::new();
    let mut from_client = Vec::new();
    while from_server.len() < 100 || from_client.len() < 100 {
        assert!(Instant::now() < deadline, "reliable messages did not arrive");
        pump(&mut server, &mut client, &mut last, &mut server_events);
        while let Some(bytes) = client.recv(Channel::Reliable) {
            from_server.push(decode_index(&bytes));
        }
        while let Some(bytes) = server.recv(ClientId(1), Channel::Reliable) {
            from_client.push(decode_index(&bytes));
        }
    }
    assert_eq!(from_server, (0..100).collect::<Vec<u32>>());
    assert_eq!(from_client, (0..100).collect::<Vec<u32>>());

    // A round-trip time appears once the first ping has been answered.
    while server.rtt(ClientId(1)).is_none() {
        assert!(Instant::now() < deadline, "no round-trip time was measured");
        pump(&mut server, &mut client, &mut last, &mut server_events);
    }

    client.disconnect();
    assert_eq!(
        client.state(),
        ConnectionState::Disconnected(DisconnectReason::Closed)
    );
    while !server_events.contains(&NetEvent::Disconnected(
        ClientId(1),
        DisconnectReason::Closed,
    )) {
        assert!(Instant::now() < deadline, "the server did not see the disconnect");
        pump(&mut server, &mut client, &mut last, &mut server_events);
    }
    assert!(server.client_ids().is_empty());
}
