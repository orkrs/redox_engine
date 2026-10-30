//! Authoritative server: accepts clients up to [`MAX_PLAYERS`], runs the fixed tick and keeps one
//! peer per client.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::error::NetError;
use crate::frame::Frame;
use crate::peer::Peer;
use crate::tick::drain_fixed_ticks;
use crate::transport::{Channel, ClientId, DisconnectReason, NetEvent, Transport};

/// Maximum number of connected clients. The next client is refused with `Full`.
pub const MAX_PLAYERS: usize = 16;
/// A peer that sent nothing for this long is disconnected.
pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);
/// Unacknowledged reliable packets are resent after this interval.
pub const RESEND_INTERVAL: Duration = Duration::from_millis(100);

/// Server handle over any [`Transport`].
pub struct NetServer<T: Transport> {
    transport: T,
    peers: BTreeMap<ClientId, Peer>,
    now: Duration,
    tick: u64,
    tick_accumulator: Duration,
}

impl<T: Transport> NetServer<T> {
    /// A server on `transport` with no clients.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            peers: BTreeMap::new(),
            now: Duration::ZERO,
            tick: 0,
            tick_accumulator: Duration::ZERO,
        }
    }

    /// Advances networking by `dt` and returns the connection events of this step.
    pub fn poll(&mut self, dt: Duration) -> Vec<NetEvent> {
        self.now += dt;
        self.tick_accumulator += dt;
        self.transport.poll(dt);
        let mut events = Vec::new();
        for event in self.transport.events() {
            match event {
                NetEvent::Connected(id) => self.accept(id, &mut events),
                NetEvent::Disconnected(id, reason) => {
                    if self.peers.remove(&id).is_some() {
                        events.push(NetEvent::Disconnected(id, reason));
                    }
                }
            }
        }
        self.receive_all();
        self.drop_silent(&mut events);
        self.flush_all();
        events
    }

    /// Number of fixed ticks due since the previous call. Advances [`NetServer::tick_number`].
    pub fn take_due_ticks(&mut self) -> u32 {
        let ticks = drain_fixed_ticks(&mut self.tick_accumulator);
        self.tick += u64::from(ticks);
        ticks
    }

    /// Current server tick.
    pub fn tick_number(&self) -> u64 {
        self.tick
    }

    /// Ids of the connected clients, in ascending order.
    pub fn client_ids(&self) -> Vec<ClientId> {
        self.peers.keys().copied().collect()
    }

    /// Sends `data` to one client on `channel`.
    pub fn send(
        &mut self,
        client: ClientId,
        channel: Channel,
        data: &[u8],
    ) -> Result<(), NetError> {
        let tick = self.tick;
        let peer = self
            .peers
            .get_mut(&client)
            .ok_or(NetError::UnknownClient(client))?;
        peer.send(channel, data, tick)
    }

    /// Sends `data` to every connected client on `channel`.
    pub fn broadcast(&mut self, channel: Channel, data: &[u8]) -> Result<(), NetError> {
        for id in self.client_ids() {
            self.send(id, channel, data)?;
        }
        Ok(())
    }

    /// Takes the next payload received from `client` on `channel`.
    pub fn recv(&mut self, client: ClientId, channel: Channel) -> Option<Vec<u8>> {
        self.peers.get_mut(&client)?.pop(channel)
    }

    /// Round-trip time to `client`, if connected.
    pub fn rtt(&self, client: ClientId) -> Option<Duration> {
        self.transport.rtt(client)
    }

    fn accept(&mut self, id: ClientId, events: &mut Vec<NetEvent>) {
        if self.peers.contains_key(&id) {
            return;
        }
        if self.peers.len() >= MAX_PLAYERS {
            self.transport.disconnect(id, DisconnectReason::Full);
            return;
        }
        self.peers.insert(id, Peer::new(self.now));
        events.push(NetEvent::Connected(id));
    }

    fn receive_all(&mut self) {
        for id in self.client_ids() {
            while let Some(bytes) = self.transport.recv(id, Channel::Unreliable) {
                let frame = match Frame::decode(&bytes) {
                    Ok(frame) => frame,
                    Err(error) => {
                        log::warn!("dropping packet from {id:?}: {error}");
                        continue;
                    }
                };
                if let Some(peer) = self.peers.get_mut(&id) {
                    peer.receive(frame, self.now);
                }
            }
        }
    }

    fn drop_silent(&mut self, events: &mut Vec<NetEvent>) {
        let now = self.now;
        let silent: Vec<ClientId> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.is_silent(now, CLIENT_TIMEOUT))
            .map(|(id, _)| *id)
            .collect();
        for id in silent {
            self.peers.remove(&id);
            self.transport.disconnect(id, DisconnectReason::Timeout);
            events.push(NetEvent::Disconnected(id, DisconnectReason::Timeout));
        }
    }

    fn flush_all(&mut self) {
        let now = self.now;
        for (id, peer) in self.peers.iter_mut() {
            for bytes in peer.flush(now, RESEND_INTERVAL) {
                self.transport.send(*id, Channel::Unreliable, &bytes);
            }
        }
    }
}
