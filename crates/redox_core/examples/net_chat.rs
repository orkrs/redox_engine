//! Console chat over UDP (NET-01 step 3).
//!
//! `net_chat --selftest` runs a server and two clients in one process on 127.0.0.1. Each client sends
//! ten reliable messages, the server relays every message to all clients, and the run passes when
//! every client has received all twenty. Exit code 0 means pass, 1 means fail.
//!
//! `net_chat --host <addr>` runs a server. `net_chat --connect <addr> [name]` runs a client. Type a
//! line to send it; the server relays client messages to everyone, and lines typed on the server
//! go out as `host`. Stop with Ctrl-C.
//!
//! ASSUMPTION: the chat has no encryption and no login (ADR 0008 v1). Use it on a trusted network.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use redox_net::{
    Channel, ConnectionState, Message, NetClient, NetServer, UdpTransport, decode, encode,
};
use serde::{Deserialize, Serialize};

const MESSAGES_PER_CLIENT: usize = 10;
const CLIENT_COUNT: usize = 2;
const WAIT: Duration = Duration::from_secs(5);
const STEP: Duration = Duration::from_millis(5);

#[derive(Debug, Serialize, Deserialize)]
struct Chat {
    from: String,
    text: String,
}

impl Message for Chat {
    const ID: u16 = 1;
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--selftest"] => selftest(),
        ["--host", addr] => parse_addr(addr).and_then(host),
        ["--connect", addr] => parse_addr(addr).and_then(|server| connect(server, "client")),
        ["--connect", addr, name] => {
            parse_addr(addr).and_then(|server| connect(server, (*name).to_string()))
        }
        _ => Err(usage()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn usage() -> String {
    "usage: net_chat --selftest | --host <addr> | --connect <addr> [name]".to_string()
}

fn to_text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn parse_addr(text: &str) -> Result<SocketAddr, String> {
    text.parse()
        .map_err(|error| format!("bad address {text}: {error}"))
}

fn localhost_any_port() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn check_deadline(deadline: Instant, what: &str) -> Result<(), String> {
    if Instant::now() >= deadline {
        Err(format!("timed out waiting for {what}"))
    } else {
        Ok(())
    }
}

fn spawn_stdin_reader() -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

/// One frame of wall-clock time for both endpoints.
fn step(
    server: &mut NetServer<UdpTransport>,
    clients: &mut [NetClient<UdpTransport>],
    last: &mut Instant,
) {
    let now = Instant::now();
    let dt = now.duration_since(*last);
    *last = now;
    server.poll(dt);
    for client in clients.iter_mut() {
        client.poll(dt);
    }
    thread::sleep(STEP);
}

fn connect_client(server: SocketAddr) -> Result<NetClient<UdpTransport>, String> {
    let transport = UdpTransport::client(server).map_err(to_text)?;
    Ok(NetClient::new(transport))
}

fn selftest() -> Result<(), String> {
    let transport = UdpTransport::server(localhost_any_port()).map_err(to_text)?;
    let addr = transport.local_addr().map_err(to_text)?;
    let mut server = NetServer::new(transport);
    let mut clients: Vec<NetClient<UdpTransport>> = (0..CLIENT_COUNT)
        .map(|_| connect_client(addr))
        .collect::<Result<_, _>>()?;
    let mut last = Instant::now();
    let deadline = Instant::now() + WAIT;

    while server.client_ids().len() != CLIENT_COUNT
        || clients
            .iter()
            .any(|client| client.state() != ConnectionState::Connected)
    {
        check_deadline(deadline, "the clients to connect")?;
        step(&mut server, &mut clients, &mut last);
    }

    for (index, client) in clients.iter_mut().enumerate() {
        for number in 0..MESSAGES_PER_CLIENT {
            let chat = Chat {
                from: format!("client{index}"),
                text: format!("message {number}"),
            };
            client
                .send(Channel::Reliable, &encode(&chat).map_err(to_text)?)
                .map_err(to_text)?;
        }
    }

    // The server relays every message it receives to all clients.
    let total = CLIENT_COUNT * MESSAGES_PER_CLIENT;
    let mut relayed = 0usize;
    while relayed < total {
        check_deadline(deadline, "messages at the server")?;
        step(&mut server, &mut clients, &mut last);
        for id in server.client_ids() {
            while let Some(bytes) = server.recv(id, Channel::Reliable) {
                server
                    .broadcast(Channel::Reliable, &bytes)
                    .map_err(to_text)?;
                relayed += 1;
            }
        }
    }

    let mut received = vec![0usize; CLIENT_COUNT];
    while received.iter().any(|&count| count < total) {
        check_deadline(deadline, "relayed messages at the clients")?;
        step(&mut server, &mut clients, &mut last);
        for (index, client) in clients.iter_mut().enumerate() {
            while let Some(bytes) = client.recv(Channel::Reliable) {
                decode::<Chat>(&bytes).map_err(to_text)?;
                received[index] += 1;
            }
        }
    }
    println!("selftest passed: {total} messages relayed to {CLIENT_COUNT} clients");
    Ok(())
}

fn host(bind: SocketAddr) -> Result<(), String> {
    let transport = UdpTransport::server(bind).map_err(to_text)?;
    println!(
        "hosting on {}",
        transport.local_addr().map_err(to_text)?
    );
    let mut server = NetServer::new(transport);
    let lines = spawn_stdin_reader();
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = now.duration_since(last);
        last = now;
        for event in server.poll(dt) {
            println!("{event:?}");
        }
        for id in server.client_ids() {
            while let Some(bytes) = server.recv(id, Channel::Reliable) {
                if let Ok(chat) = decode::<Chat>(&bytes) {
                    println!("{}: {}", chat.from, chat.text);
                }
                server
                    .broadcast(Channel::Reliable, &bytes)
                    .map_err(to_text)?;
            }
        }
        while let Ok(text) = lines.try_recv() {
            let chat = Chat {
                from: "host".to_string(),
                text,
            };
            server
                .broadcast(Channel::Reliable, &encode(&chat).map_err(to_text)?)
                .map_err(to_text)?;
        }
        thread::sleep(STEP);
    }
}

fn connect(server: SocketAddr, name: impl Into<String>) -> Result<(), String> {
    let name = name.into();
    let mut client = connect_client(server)?;
    let lines = spawn_stdin_reader();
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = now.duration_since(last);
        last = now;
        for event in client.poll(dt) {
            println!("{event:?}");
        }
        while let Some(bytes) = client.recv(Channel::Reliable) {
            if let Ok(chat) = decode::<Chat>(&bytes) {
                println!("{}: {}", chat.from, chat.text);
            }
        }
        if let ConnectionState::Disconnected(reason) = client.state() {
            println!("disconnected: {reason:?}");
            return Ok(());
        }
        while let Ok(text) = lines.try_recv() {
            let chat = Chat {
                from: name.clone(),
                text,
            };
            client
                .send(Channel::Reliable, &encode(&chat).map_err(to_text)?)
                .map_err(to_text)?;
        }
        thread::sleep(STEP);
    }
}
