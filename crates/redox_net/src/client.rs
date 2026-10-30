//! Client handle: one connection to the server, its state and the estimate of the server tick.

use std::time::Duration;

use crate::error::NetError;
use crate::frame::Frame;
use crate::peer::Peer;
use crate::server::{CLIENT_TIMEOUT, RESEND_INTERVAL};
use crate::tick::TickEstimator;
use crate::transport::{Channel, DisconnectReason, NetEvent, SERVER_PEER, Transport};

/// Connection state as seen by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Waiting for the server to accept the connection.
    Connecting,
    /// Connected to the server.
    Connected,
    /// Closed, with the reason.
    Disconnected(DisconnectReason),
}

/// Client handle over any [`Transport`].
pub struct NetClient<T: Transport> {
    transport: T,
    peer: Peer,
    state: ConnectionState,
    now: Duration,
    estimator: TickEstimator,
}

impl<T: Transport> NetClient<T> {
    /// A client on `transport`, not yet accepted by the server.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            peer: Peer::new(Duration::ZERO),
            state: ConnectionState::Connecting,
            now: Duration::ZERO,
            estimator: TickEstimator::default(),
        }
    }

    /// Advances networking by `dt` and returns the connection events of this step.
    pub fn poll(&mut self, dt: Duration) -> Vec<NetEvent> {
        self.now += dt;
        self.transport.poll(dt);
        self.estimator.advance(dt);
        let mut events = Vec::new();
        for event in self.transport.events() {
            self.apply_event(event, &mut events);
        }
        if matches!(self.state, ConnectionState::Disconnected(_)) {
            return events;
        }
        self.receive();
        if self.peer.is_silent(self.now, CLIENT_TIMEOUT) {
            self.lose_connection(DisconnectReason::Timeout, &mut events);
            return events;
        }
        for bytes in self.peer.flush(self.now, RESEND_INTERVAL) {
            self.transport
                .send(SERVER_PEER, Channel::Unreliable, &bytes);
        }
        events
    }

    /// Queues a payload for the server on `channel`.
    pub fn send(&mut self, channel: Channel, data: &[u8]) -> Result<(), NetError> {
        if matches!(self.state, ConnectionState::Disconnected(_)) {
            return Err(NetError::NotConnected);
        }
        self.peer.send(channel, data, 0)
    }

    /// Takes the next payload received from the server on `channel`.
    pub fn recv(&mut self, channel: Channel) -> Option<Vec<u8>> {
        self.peer.pop(channel)
    }

    /// Current connection state.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Estimated current server tick, or `None` before the first snapshot.
    pub fn server_tick_estimate(&self) -> Option<u64> {
        self.estimator.estimate()
    }

    /// Round-trip time to the server, if connected.
    pub fn rtt(&self) -> Option<Duration> {
        self.transport.rtt(SERVER_PEER)
    }

    /// Closes the connection locally.
    pub fn disconnect(&mut self) {
        if !matches!(self.state, ConnectionState::Disconnected(_)) {
            self.lose_connection(DisconnectReason::Closed, &mut Vec::new());
        }
    }

    fn apply_event(&mut self, event: NetEvent, events: &mut Vec<NetEvent>) {
        match event {
            NetEvent::Connected(_) => self.state = ConnectionState::Connected,
            NetEvent::Disconnected(_, reason) => {
                self.state = ConnectionState::Disconnected(reason);
            }
        }
        events.push(event);
    }

    fn receive(&mut self) {
        while let Some(bytes) = self.transport.recv(SERVER_PEER, Channel::Unreliable) {
            let frame = match Frame::decode(&bytes) {
                Ok(frame) => frame,
                Err(error) => {
                    log::warn!("dropping packet from the server: {error}");
                    continue;
                }
            };
            if let Some(tick) = self.peer.receive(frame, self.now) {
                let rtt = self.transport.rtt(SERVER_PEER).unwrap_or(Duration::ZERO);
                self.estimator.observe(tick, rtt);
            }
        }
    }

    fn lose_connection(&mut self, reason: DisconnectReason, events: &mut Vec<NetEvent>) {
        self.transport.disconnect(SERVER_PEER, reason);
        self.state = ConnectionState::Disconnected(reason);
        events.push(NetEvent::Disconnected(SERVER_PEER, reason));
    }
}
