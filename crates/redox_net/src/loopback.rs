//! In-memory transport with configurable latency, jitter, loss and duplication. Every test that needs
//! a network without sockets uses it. Virtual time advances only when the server endpoint is polled.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::rng::SplitMix64;
use crate::transport::{Channel, ClientId, DisconnectReason, NetEvent, SERVER_PEER, Transport};

/// Network conditions applied to every datagram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoopbackConditions {
    /// One-way delay.
    pub latency: Duration,
    /// Extra random delay in `[0, jitter)` per datagram, which also reorders them.
    pub jitter: Duration,
    /// Probability in `[0, 1]` that a datagram is dropped.
    pub loss: f64,
    /// Probability in `[0, 1]` that a datagram is delivered twice.
    pub duplicate: f64,
    /// Seed of the generator that decides loss, duplication and jitter.
    pub seed: u64,
}

impl Default for LoopbackConditions {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            loss: 0.0,
            duplicate: 0.0,
            seed: 1,
        }
    }
}

#[derive(Debug)]
struct Packet {
    deliver_at: Duration,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    /// Client to server, from this client.
    Up(ClientId),
    /// Server to this client.
    Down(ClientId),
}

struct WireState {
    now: Duration,
    conditions: LoopbackConditions,
    rng: SplitMix64,
    pending_connects: Vec<ClientId>,
    connected: BTreeSet<ClientId>,
    /// Packets in flight, kept sorted by delivery time.
    upstream: BTreeMap<ClientId, VecDeque<Packet>>,
    downstream: BTreeMap<ClientId, VecDeque<Packet>>,
    server_events: Vec<NetEvent>,
    client_events: BTreeMap<ClientId, Vec<NetEvent>>,
}

impl WireState {
    fn queue_for(&mut self, direction: Direction) -> &mut VecDeque<Packet> {
        match direction {
            Direction::Up(id) => self.upstream.entry(id).or_default(),
            Direction::Down(id) => self.downstream.entry(id).or_default(),
        }
    }

    /// Schedules `data` for delivery, applying loss, duplication and jitter.
    fn transmit(&mut self, direction: Direction, data: &[u8]) {
        if self.rng.next_unit() < self.conditions.loss {
            return;
        }
        let copies = if self.rng.next_unit() < self.conditions.duplicate {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let jitter = random_duration(&mut self.rng, self.conditions.jitter);
            let deliver_at = self.now + self.conditions.latency + jitter;
            let queue = self.queue_for(direction);
            let index = queue.partition_point(|packet| packet.deliver_at <= deliver_at);
            queue.insert(
                index,
                Packet {
                    deliver_at,
                    data: data.to_vec(),
                },
            );
        }
    }
}

fn random_duration(rng: &mut SplitMix64, max: Duration) -> Duration {
    let nanos = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
    if nanos == 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(rng.next_u64() % nanos)
    }
}

/// Shared state of one simulated network. Clone it to hand out endpoints.
#[derive(Clone)]
pub struct LoopbackWire {
    state: Arc<Mutex<WireState>>,
}

impl LoopbackWire {
    /// A new wire with the given conditions.
    pub fn new(conditions: LoopbackConditions) -> Self {
        let state = WireState {
            now: Duration::ZERO,
            conditions,
            rng: SplitMix64::new(conditions.seed),
            pending_connects: Vec::new(),
            connected: BTreeSet::new(),
            upstream: BTreeMap::new(),
            downstream: BTreeMap::new(),
            server_events: Vec::new(),
            client_events: BTreeMap::new(),
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// The server endpoint.
    pub fn server(&self) -> LoopbackTransport {
        LoopbackTransport {
            wire: self.clone(),
            side: Side::Server,
        }
    }

    /// The endpoint for client `id`. The server sees the connection on its next poll.
    pub fn client(&self, id: ClientId) -> LoopbackTransport {
        self.lock().pending_connects.push(id);
        LoopbackTransport {
            wire: self.clone(),
            side: Side::Client(id),
        }
    }

    fn lock(&self) -> MutexGuard<'_, WireState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Server,
    Client(ClientId),
}

/// One end of a [`LoopbackWire`].
pub struct LoopbackTransport {
    wire: LoopbackWire,
    side: Side,
}

impl Transport for LoopbackTransport {
    fn poll(&mut self, dt: Duration) {
        if self.side == Side::Server {
            let mut state = self.wire.lock();
            state.now += dt;
        }
    }

    fn send(&mut self, client: ClientId, _channel: Channel, data: &[u8]) {
        let mut state = self.wire.lock();
        match self.side {
            Side::Server => {
                if state.connected.contains(&client) {
                    state.transmit(Direction::Down(client), data);
                }
            }
            Side::Client(id) => state.transmit(Direction::Up(id), data),
        }
    }

    fn broadcast(&mut self, _channel: Channel, data: &[u8]) {
        let mut state = self.wire.lock();
        match self.side {
            Side::Server => {
                let clients: Vec<ClientId> = state.connected.iter().copied().collect();
                for id in clients {
                    state.transmit(Direction::Down(id), data);
                }
            }
            Side::Client(id) => state.transmit(Direction::Up(id), data),
        }
    }

    fn recv(&mut self, client: ClientId, _channel: Channel) -> Option<Vec<u8>> {
        let mut state = self.wire.lock();
        let now = state.now;
        let queue = match self.side {
            Side::Server => {
                if !state.connected.contains(&client) {
                    return None;
                }
                state.upstream.get_mut(&client)?
            }
            Side::Client(id) => state.downstream.get_mut(&id)?,
        };
        let ready = queue.front().is_some_and(|packet| packet.deliver_at <= now);
        if ready {
            queue.pop_front().map(|packet| packet.data)
        } else {
            None
        }
    }

    fn events(&mut self) -> Vec<NetEvent> {
        let mut state = self.wire.lock();
        match self.side {
            Side::Server => {
                let pending = std::mem::take(&mut state.pending_connects);
                for id in pending {
                    state.connected.insert(id);
                    state
                        .client_events
                        .entry(id)
                        .or_default()
                        .push(NetEvent::Connected(SERVER_PEER));
                    state.server_events.push(NetEvent::Connected(id));
                }
                std::mem::take(&mut state.server_events)
            }
            Side::Client(id) => state.client_events.remove(&id).unwrap_or_default(),
        }
    }

    fn rtt(&self, client: ClientId) -> Option<Duration> {
        let state = self.wire.lock();
        let id = match self.side {
            Side::Server => client,
            Side::Client(id) => id,
        };
        if state.connected.contains(&id) {
            Some(state.conditions.latency * 2)
        } else {
            None
        }
    }

    fn disconnect(&mut self, client: ClientId, reason: DisconnectReason) {
        let mut state = self.wire.lock();
        match self.side {
            Side::Server => {
                if state.connected.remove(&client) {
                    state.upstream.remove(&client);
                    state.downstream.remove(&client);
                    state
                        .client_events
                        .entry(client)
                        .or_default()
                        .push(NetEvent::Disconnected(SERVER_PEER, reason));
                }
            }
            Side::Client(id) => {
                if state.connected.remove(&id) {
                    state.upstream.remove(&id);
                    state.downstream.remove(&id);
                    state
                        .server_events
                        .push(NetEvent::Disconnected(id, reason));
                }
            }
        }
    }
}
