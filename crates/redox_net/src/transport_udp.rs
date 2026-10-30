//! UDP backend for [`Transport`] over `std::net::UdpSocket`, with no async runtime.
//!
//! The socket is non-blocking and is drained once per [`Transport::poll`]. Every datagram starts with a
//! class byte: `DATA` carries one layer frame from the layer above, the other classes are connection
//! control (`HELLO`, `ACCEPT`, `BYE`) and clock probes (`PING`, `PONG`) that give the round-trip time.
//!
//! ASSUMPTION: no encryption and no source-address validation in v1 (ADR 0008). A spoofed `HELLO` can
//! take a player slot until the timeout. The server limit still applies.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, ErrorKind};
use std::net::{Ipv6Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use crate::transport::{Channel, ClientId, DisconnectReason, NetEvent, SERVER_PEER, Transport};

const CLASS_DATA: u8 = 0x10;
const CLASS_HELLO: u8 = 0x20;
const CLASS_ACCEPT: u8 = 0x21;
const CLASS_BYE: u8 = 0x22;
const CLASS_PING: u8 = 0x23;
const CLASS_PONG: u8 = 0x24;

/// Larger than any datagram the layer produces (frames are at most 1200 bytes plus the class byte).
const RECV_BUFFER_BYTES: usize = 2048;
/// How often a client repeats `HELLO` until the server accepts it.
const HELLO_INTERVAL: Duration = Duration::from_millis(250);
/// How often each connected peer is pinged for the round-trip time.
const PING_INTERVAL: Duration = Duration::from_millis(500);
/// Datagrams beyond this backlog are dropped, oldest first.
const INBOX_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Server,
    Client,
}

struct RemotePeer {
    addr: SocketAddr,
    /// True once the peer accepted the connection (server side: once `HELLO` was seen).
    connected: bool,
    /// Client side only: keep sending `HELLO` until accepted or refused.
    wants_connection: bool,
    inbox: VecDeque<Vec<u8>>,
    /// Last measured round-trip time.
    rtt: Option<Duration>,
}

impl RemotePeer {
    fn new(addr: SocketAddr, connected: bool, wants_connection: bool) -> Self {
        Self {
            addr,
            connected,
            wants_connection,
            inbox: VecDeque::new(),
            rtt: None,
        }
    }
}

/// [`Transport`] over one UDP socket. Use [`UdpTransport::server`] on the host and
/// [`UdpTransport::client`] on the other end.
pub struct UdpTransport {
    socket: UdpSocket,
    role: Role,
    now: Duration,
    next_hello: Duration,
    next_ping: Duration,
    next_id: u32,
    peers: BTreeMap<ClientId, RemotePeer>,
    by_addr: BTreeMap<SocketAddr, ClientId>,
    events: Vec<NetEvent>,
}

impl UdpTransport {
    /// A server socket bound to `bind`. Use port 0 to let the OS pick one (see [`Self::local_addr`]).
    pub fn server(bind: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        Self::with_socket(socket, Role::Server)
    }

    /// A client socket on an ephemeral local port that connects to `server`.
    pub fn client(server: SocketAddr) -> io::Result<Self> {
        let local = if server.is_ipv4() {
            SocketAddr::from(([0, 0, 0, 0], 0))
        } else {
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
        };
        let socket = UdpSocket::bind(local)?;
        let mut transport = Self::with_socket(socket, Role::Client)?;
        transport
            .peers
            .insert(SERVER_PEER, RemotePeer::new(server, false, true));
        transport.by_addr.insert(server, SERVER_PEER);
        Ok(transport)
    }

    /// The local address of the socket.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    fn with_socket(socket: UdpSocket, role: Role) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            role,
            now: Duration::ZERO,
            next_hello: Duration::ZERO,
            next_ping: Duration::ZERO,
            next_id: 1,
            peers: BTreeMap::new(),
            by_addr: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    fn is_server(&self) -> bool {
        self.role == Role::Server
    }

    fn peer_id(&self, from: SocketAddr) -> Option<ClientId> {
        self.by_addr.get(&from).copied()
    }

    fn connected_addr(&self, client: ClientId) -> Option<SocketAddr> {
        self.peers
            .get(&client)
            .filter(|peer| peer.connected)
            .map(|peer| peer.addr)
    }

    fn receive_all(&mut self) {
        let mut buffer = [0u8; RECV_BUFFER_BYTES];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((len, from)) => self.handle(from, &buffer[..len]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                // Windows reports an earlier ICMP port-unreachable as ConnectionReset on the next receive.
                Err(error) if error.kind() == ErrorKind::ConnectionReset => continue,
                Err(error) => {
                    log::warn!("udp receive failed: {error}");
                    break;
                }
            }
        }
    }

    fn handle(&mut self, from: SocketAddr, datagram: &[u8]) {
        let Some((&class, body)) = datagram.split_first() else {
            return;
        };
        match class {
            CLASS_HELLO if self.is_server() => self.on_hello(from),
            CLASS_ACCEPT if !self.is_server() => self.on_accept(from),
            CLASS_BYE => self.on_bye(from, body),
            CLASS_DATA => self.on_data(from, body),
            CLASS_PING => self.on_ping(from, body),
            CLASS_PONG => self.on_pong(from, body),
            _ => {}
        }
    }

    fn on_hello(&mut self, from: SocketAddr) {
        if self.peer_id(from).is_some() {
            // The client repeated HELLO because it did not see our ACCEPT yet.
            self.send_control(from, CLASS_ACCEPT, &[]);
            return;
        }
        let id = ClientId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.peers.insert(id, RemotePeer::new(from, true, false));
        self.by_addr.insert(from, id);
        self.events.push(NetEvent::Connected(id));
        self.send_control(from, CLASS_ACCEPT, &[]);
    }

    fn on_accept(&mut self, from: SocketAddr) {
        if self.peer_id(from) != Some(SERVER_PEER) {
            return;
        }
        if let Some(peer) = self
            .peers
            .get_mut(&SERVER_PEER)
            .filter(|peer| !peer.connected)
        {
            peer.connected = true;
            peer.wants_connection = false;
            self.events.push(NetEvent::Connected(SERVER_PEER));
        }
    }

    fn on_bye(&mut self, from: SocketAddr, body: &[u8]) {
        let Some(id) = self.peer_id(from) else {
            return;
        };
        let reason = reason_from_code(body.first().copied());
        if self.is_server() {
            self.remove_peer(id);
            self.events.push(NetEvent::Disconnected(id, reason));
        } else {
            if let Some(peer) = self.peers.get_mut(&SERVER_PEER) {
                peer.connected = false;
                peer.wants_connection = false;
            }
            self.events
                .push(NetEvent::Disconnected(SERVER_PEER, reason));
        }
    }

    fn on_data(&mut self, from: SocketAddr, body: &[u8]) {
        let Some(id) = self.peer_id(from) else {
            return;
        };
        let Some(peer) = self.peers.get_mut(&id) else {
            return;
        };
        if !peer.connected {
            return;
        }
        if peer.inbox.len() >= INBOX_CAPACITY {
            peer.inbox.pop_front();
        }
        peer.inbox.push_back(body.to_vec());
    }

    fn on_ping(&mut self, from: SocketAddr, body: &[u8]) {
        if self.peer_id(from).is_some() {
            self.send_control(from, CLASS_PONG, body);
        }
    }

    fn on_pong(&mut self, from: SocketAddr, body: &[u8]) {
        let Some(id) = self.peer_id(from) else {
            return;
        };
        let Some(sent) = read_micros(body) else {
            return;
        };
        let sent = Duration::from_micros(sent);
        if self.now < sent {
            return;
        }
        if let Some(peer) = self.peers.get_mut(&id) {
            peer.rtt = Some(self.now - sent);
        }
    }

    fn send_timers(&mut self) {
        let hello_due = self.now >= self.next_hello;
        if let Some(server) = self.client_hello_target().filter(|_| hello_due) {
            self.send_control(server, CLASS_HELLO, &[]);
            self.next_hello = self.now + HELLO_INTERVAL;
        }
        if self.now >= self.next_ping {
            let stamp = micros(self.now).to_le_bytes();
            let targets: Vec<SocketAddr> = self
                .peers
                .values()
                .filter(|peer| peer.connected)
                .map(|peer| peer.addr)
                .collect();
            for addr in targets {
                self.send_control(addr, CLASS_PING, &stamp);
            }
            self.next_ping = self.now + PING_INTERVAL;
        }
    }

    /// Server address while a client still wants to connect.
    fn client_hello_target(&self) -> Option<SocketAddr> {
        if self.is_server() {
            return None;
        }
        self.peers
            .get(&SERVER_PEER)
            .filter(|peer| peer.wants_connection && !peer.connected)
            .map(|peer| peer.addr)
    }

    fn remove_peer(&mut self, id: ClientId) -> Option<RemotePeer> {
        let peer = self.peers.remove(&id)?;
        self.by_addr.remove(&peer.addr);
        Some(peer)
    }

    fn send_data(&self, addr: SocketAddr, data: &[u8]) {
        let mut datagram = Vec::with_capacity(data.len() + 1);
        datagram.push(CLASS_DATA);
        datagram.extend_from_slice(data);
        self.send_datagram(addr, &datagram);
    }

    fn send_control(&self, addr: SocketAddr, class: u8, body: &[u8]) {
        let mut datagram = Vec::with_capacity(body.len() + 1);
        datagram.push(class);
        datagram.extend_from_slice(body);
        self.send_datagram(addr, &datagram);
    }

    fn send_datagram(&self, addr: SocketAddr, datagram: &[u8]) {
        // Unreliable by design: a full socket buffer drops the datagram, and the layer above resends
        // reliable frames. Only real errors are logged.
        if let Err(error) = self.socket.send_to(datagram, addr)
            && error.kind() != ErrorKind::WouldBlock
        {
            log::warn!("udp send to {addr} failed: {error}");
        }
    }
}

impl Transport for UdpTransport {
    fn poll(&mut self, dt: Duration) {
        self.now += dt;
        self.receive_all();
        self.send_timers();
    }

    fn send(&mut self, client: ClientId, _channel: Channel, data: &[u8]) {
        if let Some(addr) = self.connected_addr(client) {
            self.send_data(addr, data);
        }
    }

    fn broadcast(&mut self, _channel: Channel, data: &[u8]) {
        let targets: Vec<SocketAddr> = self
            .peers
            .values()
            .filter(|peer| peer.connected)
            .map(|peer| peer.addr)
            .collect();
        for addr in targets {
            self.send_data(addr, data);
        }
    }

    fn recv(&mut self, client: ClientId, _channel: Channel) -> Option<Vec<u8>> {
        self.peers.get_mut(&client)?.inbox.pop_front()
    }

    fn events(&mut self) -> Vec<NetEvent> {
        std::mem::take(&mut self.events)
    }

    fn rtt(&self, client: ClientId) -> Option<Duration> {
        self.peers
            .get(&client)
            .filter(|peer| peer.connected)
            .and_then(|peer| peer.rtt)
    }

    fn disconnect(&mut self, client: ClientId, reason: DisconnectReason) {
        let code = reason_code(reason);
        if self.is_server() {
            if let Some(peer) = self.remove_peer(client) {
                self.send_control(peer.addr, CLASS_BYE, &[code]);
            }
            return;
        }
        let Some(peer) = self.peers.get_mut(&SERVER_PEER) else {
            return;
        };
        let addr = peer.addr;
        let was_connected = peer.connected;
        peer.connected = false;
        peer.wants_connection = false;
        if was_connected {
            self.send_control(addr, CLASS_BYE, &[code]);
        }
    }
}

fn reason_code(reason: DisconnectReason) -> u8 {
    match reason {
        DisconnectReason::Full => 1,
        DisconnectReason::Timeout => 2,
        DisconnectReason::Closed => 3,
    }
}

/// Unknown codes map to `Closed`, so a newer peer cannot make an older one fail.
fn reason_from_code(code: Option<u8>) -> DisconnectReason {
    match code {
        Some(1) => DisconnectReason::Full,
        Some(2) => DisconnectReason::Timeout,
        _ => DisconnectReason::Closed,
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn read_micros(body: &[u8]) -> Option<u64> {
    let raw: [u8; 8] = body.get(..8)?.try_into().ok()?;
    Some(u64::from_le_bytes(raw))
}
