//! Networking on the app loop (NET-01 step 3).
//!
//! The plugin owns the UDP server and/or client as resources and polls them once per frame on
//! `Stage::Input`. A poll also flushes what the game queued in the previous frame, so the plugin
//! has no separate flush system.
//!
//! ASSUMPTION: a message queued during a frame leaves on the next frame's poll. A one-frame delay
//! is accepted for v1. A separate flush at the end of the frame would need a second poll call.
//!
//! Game code reads and writes messages through the resources directly:
//! `NetServer<UdpTransport>` or `NetClient<UdpTransport>`. Connection events arrive as
//! `Events<NetEvent>`.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use redox_ecs::{Events, World};
use redox_net::{NetClient, NetEvent, NetServer, UdpTransport};

use crate::app::AppBuilder;
use crate::dispatcher::Stage;
use crate::plugin::Plugin;
use crate::time::{MAX_DELTA, Time};

/// What this process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMode {
    /// No networking. Only the `Events<NetEvent>` resource exists.
    Off,
    /// Dedicated server listening on `bind`.
    Server {
        /// Address to bind, for example `0.0.0.0:7777`.
        bind: SocketAddr,
    },
    /// Client connected to `server`.
    Client {
        /// Server address.
        server: SocketAddr,
    },
    /// A server on `bind` and a local client connected to it over 127.0.0.1.
    ListenServer {
        /// Address to bind for the server.
        bind: SocketAddr,
    },
}

/// Installs the networking resources and polls them each frame. Errors while binding sockets are
/// logged and leave networking off, so the game still starts.
pub struct NetPlugin {
    /// The role of this process.
    pub mode: NetMode,
}

impl Plugin for NetPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let mode = self.mode;
        app.on_setup(move |world| install(world, mode));
        app.register_events::<NetEvent>();
        app.add_system_mut(Stage::Input, |world, _| net_poll_system(world));
    }
}

fn install(world: &mut World, mode: NetMode) {
    world.insert_resource(Events::<NetEvent>::new());
    let result = match mode {
        NetMode::Off => Ok(()),
        NetMode::Server { bind } => start_server(world, bind),
        NetMode::Client { server } => start_client(world, server),
        NetMode::ListenServer { bind } => start_listen_server(world, bind),
    };
    if let Err(error) = result {
        log::error!("networking is off: {error}");
    }
}

fn start_server(world: &mut World, bind: SocketAddr) -> io::Result<()> {
    let transport = UdpTransport::server(bind)?;
    log::info!("server listening on {}", transport.local_addr()?);
    world.insert_resource(NetServer::new(transport));
    Ok(())
}

fn start_client(world: &mut World, server: SocketAddr) -> io::Result<()> {
    let transport = UdpTransport::client(server)?;
    world.insert_resource(NetClient::new(transport));
    Ok(())
}

fn start_listen_server(world: &mut World, bind: SocketAddr) -> io::Result<()> {
    let transport = UdpTransport::server(bind)?;
    let port = transport.local_addr()?.port();
    world.insert_resource(NetServer::new(transport));
    let local = SocketAddr::from(([127, 0, 0, 1], port));
    world.insert_resource(NetClient::new(UdpTransport::client(local)?));
    Ok(())
}

fn net_poll_system(world: &mut World) {
    let dt = frame_delta(world);
    let mut events = Vec::new();
    if let Some(server) = world.get_resource_mut::<NetServer<UdpTransport>>() {
        events.extend(server.poll(dt));
    }
    if let Some(client) = world.get_resource_mut::<NetClient<UdpTransport>>() {
        events.extend(client.poll(dt));
    }
    if let Some(queue) = world.get_resource_mut::<Events<NetEvent>>() {
        for event in events {
            queue.send(event);
        }
    }
}

/// Frame time for the network clock, clamped like the game clock and never negative.
fn frame_delta(world: &World) -> Duration {
    let seconds = world
        .get_resource::<Time>()
        .map_or(1.0 / 60.0, |time| time.delta_time);
    if seconds.is_finite() && seconds > 0.0 {
        Duration::from_secs_f32(seconds.min(MAX_DELTA))
    } else {
        Duration::ZERO
    }
}
