//! A small MQTT broker for tests and measurements only (never shipped in the application).
//!
//! Enough of MQTT 3.1.1 to stand in for a real broker on the loopback address: CONNECT
//! with optional credentials, retained messages (an empty retained payload erases),
//! wildcard subscriptions with the matching retained messages sent in filter order, last
//! will on an unclean loss (including a keep-alive timeout), QoS 0 delivery, ping. It can
//! also speak TLS with a given identity, and it records every message published to it so
//! tests can check what a client sent. Each connection has a thread that reads with a short
//! timeout: simple, and acceptable for a tool that only runs during tests.

use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use schannel::cert_context::{CertContext, KeySpec};
use schannel::cert_store::{CertAdd, Memory};
use schannel::crypt_prov::{AcquireOptions, ProviderType};
use schannel::schannel_cred::{Direction, SchannelCred};
use schannel::tls_stream;

use crate::codec::{matches, Packet, Publish, Will};

/// How long a connection thread waits for bytes before looking at its stop flag.
const TICK: Duration = Duration::from_millis(20);

/// Broker settings.
#[derive(Default)]
pub struct BrokerConfig {
    /// Required user name and password; `None` = anyone may connect.
    pub credentials: Option<(String, String)>,
    /// Encrypt every connection with this identity.
    pub tls: Option<Identity>,
    /// Port to listen on (0 = any free port).
    pub port: u16,
}

/// Name of the key container holding the test key.
///
/// The Windows TLS server does its private-key work in another process, which only finds
/// keys saved in a named container of the user's profile (an in-memory key is refused).
/// A fixed name means one small container, replaced at each run, instead of a new key file
/// left behind by every test run (what a usual PKCS #12 import does).
const KEY_CONTAINER: &str = "BetterVolume-test-broker";

/// The certificate and private key the broker encrypts with.
pub struct Identity(CertContext);

impl Identity {
    /// A certificate (PEM) and its PKCS #8 private key (PEM).
    pub fn from_pem(certificate: &str, key: &[u8]) -> std::io::Result<Identity> {
        let certificate = CertContext::from_pem(certificate)?;
        let rsa = ProviderType::rsa_full();
        let mut container = match AcquireOptions::new().container(KEY_CONTAINER).acquire(rsa) {
            Ok(container) => container,
            Err(_) => {
                AcquireOptions::new().container(KEY_CONTAINER).new_keyset(true).acquire(rsa)?
            }
        };
        container.import().import_pkcs8_pem(key)?;
        certificate
            .set_key_prov_info()
            .container(KEY_CONTAINER)
            .type_(rsa)
            .keep_open(true)
            .key_spec(KeySpec::key_exchange())
            .set()?;
        let mut store = Memory::new()?.into_store();
        Ok(Identity(store.add_cert(&certificate, CertAdd::Always)?))
    }
}

/// A message the broker received from a client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    /// Identifier of the client that sent it.
    pub client: String,
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// Retained flag.
    pub retain: bool,
}

enum Stream {
    Plain(TcpStream),
    Tls(Box<schannel::tls_stream::TlsStream<TcpStream>>),
}

impl Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.write_all(bytes).and_then(|()| s.flush()),
            Stream::Tls(s) => s.write_all(bytes).and_then(|()| s.flush()),
        }
    }
}

struct Session {
    id: u64,
    stream: Arc<Mutex<Stream>>,
    raw: TcpStream,
    filters: Vec<String>,
}

#[derive(Default)]
struct State {
    sessions: Vec<Session>,
    retained: BTreeMap<String, Vec<u8>>,
    received: Vec<Received>,
    connections: u64,
}

struct Shared {
    state: Mutex<State>,
    stop: AtomicBool,
    next_id: AtomicU64,
    credentials: Option<(String, String)>,
}

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    /// Stores (if retained) and delivers a message to every matching subscription.
    fn route(&self, topic: &str, payload: &[u8], retain: bool) {
        let targets: Vec<Arc<Mutex<Stream>>> = {
            let mut state = guard(&self.state);
            if retain {
                if payload.is_empty() {
                    state.retained.remove(topic);
                } else {
                    state.retained.insert(topic.to_owned(), payload.to_vec());
                }
            }
            state
                .sessions
                .iter()
                .filter(|s| s.filters.iter().any(|f| matches(f, topic)))
                .map(|s| Arc::clone(&s.stream))
                .collect()
        };
        let packet = Packet::Publish(Publish {
            topic: topic.to_owned(),
            payload: payload.to_vec(),
            qos: 0,
            retain: false,
            id: 0,
        })
        .encode();
        for stream in targets {
            let _ = guard(&stream).write_all(&packet);
        }
    }
}

/// A running test broker on the loopback address.
pub struct Broker {
    shared: Arc<Shared>,
    port: u16,
}

impl Broker {
    /// Starts listening.
    pub fn start(config: BrokerConfig) -> std::io::Result<Broker> {
        let listener = TcpListener::bind(("127.0.0.1", config.port))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            stop: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            credentials: config.credentials,
        });
        let acceptor = config.tls.map(Arc::new);
        let accept_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            while !accept_shared.stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((tcp, _)) => {
                        let shared = Arc::clone(&accept_shared);
                        let acceptor = acceptor.clone();
                        std::thread::spawn(move || serve(&shared, tcp, acceptor.as_deref()));
                    }
                    Err(_) => std::thread::sleep(TICK),
                }
            }
        });
        Ok(Broker { shared, port })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every message clients published, in order.
    pub fn received(&self) -> Vec<Received> {
        guard(&self.shared.state).received.clone()
    }

    /// Forgets the recorded messages.
    pub fn clear_received(&self) {
        guard(&self.shared.state).received.clear();
    }

    /// The retained message of `topic`, if any.
    pub fn retained(&self, topic: &str) -> Option<Vec<u8>> {
        guard(&self.shared.state).retained.get(topic).cloned()
    }

    /// Every retained topic.
    pub fn retained_topics(&self) -> Vec<String> {
        guard(&self.shared.state).retained.keys().cloned().collect()
    }

    /// Publishes as if another client (Home Assistant) did.
    pub fn publish(&self, topic: &str, payload: &[u8], retain: bool) {
        self.shared.route(topic, payload, retain);
    }

    /// Number of clients connected now.
    pub fn connected(&self) -> usize {
        guard(&self.shared.state).sessions.len()
    }

    /// Number of connections accepted so far.
    pub fn connections(&self) -> u64 {
        guard(&self.shared.state).connections
    }

    /// Cuts every connection as a network loss would (their wills are published).
    pub fn drop_connections(&self) {
        for session in &guard(&self.shared.state).sessions {
            let _ = session.raw.shutdown(Shutdown::Both);
        }
    }

    /// Waits until `condition` holds (at most `timeout`); returns whether it did.
    pub fn wait_for(&self, timeout: Duration, condition: impl Fn(&Broker) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if condition(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        condition(self)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.drop_connections();
    }
}

/// One client connection, from CONNECT to the end.
fn serve(shared: &Arc<Shared>, tcp: TcpStream, acceptor: Option<&Identity>) {
    let _ = tcp.set_nonblocking(false);
    let Ok(raw) = tcp.try_clone() else { return };
    let stream = match acceptor {
        None => Stream::Plain(tcp),
        Some(Identity(certificate)) => {
            let credentials =
                SchannelCred::builder().cert(certificate.clone()).acquire(Direction::Inbound);
            let Ok(credentials) = credentials else { return };
            match tls_stream::Builder::new().accept(credentials, tcp) {
                Ok(s) => Stream::Tls(Box::new(s)),
                Err(_) => return,
            }
        }
    };
    if raw.set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let stream = Arc::new(Mutex::new(stream));
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    let mut input = Vec::new();
    let mut buf = vec![0_u8; 16 * 1024];
    let mut will: Option<Will> = None;
    let mut keep_alive: Option<Duration> = None;
    let mut client_id = String::new();
    let mut last_heard = Instant::now();
    let mut clean_end = false;
    let mut registered = false;
    'connection: while !shared.stop.load(Ordering::SeqCst) {
        // Wait for bytes outside the lock (an encrypted read does not give up on a
        // timeout), then drain everything readable, like the client.
        match raw.peek(&mut buf[..1]) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if keep_alive.is_some_and(|k| last_heard.elapsed() > k * 3 / 2) {
                    break;
                }
                continue;
            }
            Err(_) => break,
        }
        {
            let mut locked = guard(&stream);
            if raw.set_nonblocking(true).is_err() {
                break;
            }
            let mut closed = false;
            loop {
                match locked.read(&mut buf) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => input.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }
            let _ = raw.set_nonblocking(false);
            last_heard = Instant::now();
            if closed && input.is_empty() {
                break;
            }
        }
        loop {
            let packet = match Packet::decode(&input) {
                Ok(Some((packet, used))) => {
                    input.drain(..used);
                    packet
                }
                Ok(None) => break,
                Err(_) => break 'connection,
            };
            let reply = |p: Packet| guard(&stream).write_all(&p.encode());
            match packet {
                Packet::Connect(c) if !registered => {
                    let accepted = shared.credentials.as_ref().is_none_or(|(user, pass)| {
                        c.username.as_deref() == Some(user.as_str())
                            && c.password.as_deref() == Some(pass.as_bytes())
                    });
                    let code = if accepted { 0 } else { 4 };
                    let _ = reply(Packet::ConnAck { session_present: false, code });
                    if !accepted {
                        clean_end = true;
                        break 'connection;
                    }
                    will = c.will;
                    keep_alive =
                        (c.keep_alive > 0).then(|| Duration::from_secs(c.keep_alive.into()));
                    client_id = c.client_id;
                    let mut state = guard(&shared.state);
                    state.connections += 1;
                    let Ok(raw) = raw.try_clone() else { break 'connection };
                    state.sessions.push(Session {
                        id,
                        stream: Arc::clone(&stream),
                        raw,
                        filters: Vec::new(),
                    });
                    registered = true;
                }
                Packet::Publish(p) if registered => {
                    guard(&shared.state).received.push(Received {
                        client: client_id.clone(),
                        topic: p.topic.clone(),
                        payload: p.payload.clone(),
                        retain: p.retain,
                    });
                    if p.qos == 1 {
                        let _ = reply(Packet::PubAck(p.id));
                    }
                    shared.route(&p.topic, &p.payload, p.retain);
                }
                Packet::Subscribe(packet_id, filters) if registered => {
                    let codes = vec![0; filters.len()];
                    let retained: Vec<(String, Vec<u8>)> = {
                        let mut state = guard(&shared.state);
                        if let Some(session) = state.sessions.iter_mut().find(|s| s.id == id) {
                            session.filters.extend(filters.iter().map(|(f, _)| f.clone()));
                        }
                        let mut out = Vec::new();
                        for (filter, _) in &filters {
                            for (topic, payload) in &state.retained {
                                if matches(filter, topic) && !out.iter().any(|(t, _)| t == topic) {
                                    out.push((topic.clone(), payload.clone()));
                                }
                            }
                        }
                        out
                    };
                    let _ = reply(Packet::SubAck(packet_id, codes));
                    for (topic, payload) in retained {
                        let _ = reply(Packet::Publish(Publish {
                            topic,
                            payload,
                            qos: 0,
                            retain: true,
                            id: 0,
                        }));
                    }
                }
                Packet::PingReq if registered => {
                    let _ = reply(Packet::PingResp);
                }
                Packet::PubAck(_) => {}
                Packet::Disconnect if registered => {
                    clean_end = true;
                    break 'connection;
                }
                _ => break 'connection,
            }
        }
    }
    guard(&shared.state).sessions.retain(|s| s.id != id);
    let _ = raw.shutdown(Shutdown::Both);
    if registered && !clean_end {
        if let Some(will) = will {
            shared.route(&will.topic, &will.payload, will.retain);
        }
    }
}
