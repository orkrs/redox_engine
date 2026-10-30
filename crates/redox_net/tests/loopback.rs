//! Behaviour of server and client over the deterministic loopback wire. No sockets are used.

use std::collections::BTreeSet;
use std::time::Duration;

use redox_net::{
    Channel, ClientId, ConnectionState, DisconnectReason, LoopbackConditions, LoopbackWire,
    MAX_PLAYERS, NetClient, NetEvent, NetServer, SERVER_PEER,
};

/// One frame at 60 Hz.
const FRAME: Duration = Duration::from_micros(16_667);

#[test]
fn a_client_connects_and_the_server_sees_it() {
    let wire = LoopbackWire::new(LoopbackConditions::default());
    let mut server = NetServer::new(wire.server());
    let mut client = NetClient::new(wire.client(ClientId(1)));

    let server_events = server.poll(FRAME);
    let client_events = client.poll(FRAME);

    assert_eq!(server_events, vec![NetEvent::Connected(ClientId(1))]);
    assert_eq!(client_events, vec![NetEvent::Connected(SERVER_PEER)]);
    assert_eq!(client.state(), ConnectionState::Connected);
    assert_eq!(server.client_ids(), vec![ClientId(1)]);
}

#[test]
fn the_player_limit_is_enforced() {
    let wire = LoopbackWire::new(LoopbackConditions::default());
    let mut server = NetServer::new(wire.server());
    let mut clients: Vec<NetClient<_>> = (1..=MAX_PLAYERS as u32 + 1)
        .map(|id| NetClient::new(wire.client(ClientId(id))))
        .collect();

    server.poll(FRAME);
    for client in &mut clients {
        client.poll(FRAME);
    }

    assert_eq!(server.client_ids().len(), MAX_PLAYERS);
    let refused = clients.last().expect("seventeen clients were created");
    assert_eq!(
        refused.state(),
        ConnectionState::Disconnected(DisconnectReason::Full)
    );
}

#[test]
fn reliable_messages_arrive_in_order_despite_losses() {
    let conditions = LoopbackConditions {
        latency: Duration::from_millis(20),
        jitter: Duration::from_millis(15),
        loss: 0.3,
        duplicate: 0.1,
        seed: 9,
    };
    let wire = LoopbackWire::new(conditions);
    let mut server = NetServer::new(wire.server());
    let mut client = NetClient::new(wire.client(ClientId(1)));
    server.poll(FRAME);
    client.poll(FRAME);

    for index in 0..1000u32 {
        server
            .send(ClientId(1), Channel::Reliable, &index.to_le_bytes())
            .expect("the client is connected");
    }

    let mut received = Vec::new();
    // Two minutes of simulated time at 60 Hz is far more than the retransmissions need.
    for _ in 0..(60 * 120) {
        server.poll(FRAME);
        client.poll(FRAME);
        while let Some(bytes) = client.recv(Channel::Reliable) {
            let raw: [u8; 4] = bytes.try_into().expect("every payload is four bytes");
            received.push(u32::from_le_bytes(raw));
        }
        if received.len() == 1000 {
            break;
        }
    }

    assert_eq!(received, (0..1000).collect::<Vec<u32>>());
}

#[test]
fn unreliable_messages_may_be_lost_but_never_duplicated_to_the_game() {
    let conditions = LoopbackConditions {
        latency: Duration::from_millis(20),
        jitter: Duration::from_millis(15),
        loss: 0.3,
        duplicate: 0.5,
        seed: 3,
    };
    let wire = LoopbackWire::new(conditions);
    let mut server = NetServer::new(wire.server());
    let mut client = NetClient::new(wire.client(ClientId(1)));
    server.poll(FRAME);
    client.poll(FRAME);

    for index in 0..1000u32 {
        server
            .send(ClientId(1), Channel::Unreliable, &index.to_le_bytes())
            .expect("the client is connected");
    }

    let mut received = Vec::new();
    for _ in 0..120 {
        server.poll(FRAME);
        client.poll(FRAME);
        while let Some(bytes) = client.recv(Channel::Unreliable) {
            let raw: [u8; 4] = bytes.try_into().expect("every payload is four bytes");
            received.push(u32::from_le_bytes(raw));
        }
    }

    let unique: BTreeSet<u32> = received.iter().copied().collect();
    assert_eq!(
        unique.len(),
        received.len(),
        "no payload reaches the game twice"
    );
    assert!(
        received.len() < 1000,
        "some datagrams are lost at 30 % loss"
    );
    assert!(!received.is_empty());
}

#[test]
fn a_disconnect_is_reported_when_packets_stop() {
    let wire = LoopbackWire::new(LoopbackConditions::default());
    let mut server = NetServer::new(wire.server());
    let mut client = NetClient::new(wire.client(ClientId(1)));
    server.poll(FRAME);
    client.poll(FRAME);

    // The client is never polled again, so the server hears nothing for six seconds.
    let mut events = Vec::new();
    for _ in 0..(6 * 60) {
        events.extend(server.poll(FRAME));
    }

    assert!(events.contains(&NetEvent::Disconnected(
        ClientId(1),
        DisconnectReason::Timeout
    )));
    assert!(server.client_ids().is_empty());
}
