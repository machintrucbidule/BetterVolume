//! The MQTT connection: one worker thread that connects, writes and keeps the connection
//! alive, one reader thread per connection (ARCHITECTURE §3).
//!
//! Nothing here polls: the worker sleeps on its command channel until something must be
//! written or the next keep-alive is due; the reader sleeps in the socket until
//! bytes arrive. Both report to the application through a channel and the `notify`
//! function (which wakes the UI thread); neither touches application state.
//!
//! Encryption (FR-090) goes through the Windows TLS implementation (`native-tls`
//! uses Schannel). A certificate Windows does not trust is reported with its fingerprint
//! so the user can trust it; a trusted fingerprint is then the only one accepted.
//!
//! The socket is shared between the two threads through a mutex: the reader waits for
//! bytes with `peek` *outside* the lock (so the worker can write meanwhile), then drains
//! everything readable inside it, switching the socket to non-blocking for that moment.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::codec::{Connect, Packet, Publish, Will};

/// How long a connection attempt (TCP, encryption, answer of the broker) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one write may block (a dead network with a full send buffer).
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// First delay before trying again (FR-115); doubled at each failure.
const FIRST_RETRY: Duration = Duration::from_secs(1);
/// How long a connection must hold before the next loss starts the delays over.
const STABLE: Duration = Duration::from_secs(60);
/// Longest delay between two attempts.
const LAST_RETRY: Duration = Duration::from_secs(60);

/// A SHA-256 certificate fingerprint.
pub type Fingerprint = [u8; 32];

/// Function waking the application (called from the client's threads).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Encryption settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tls {
    /// The only certificate accepted, by fingerprint; `None` = the certificates
    /// Windows trusts, for the broker's name.
    pub trusted: Option<Fingerprint>,
}

/// What to connect to, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Broker address.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Encryption, if wanted.
    pub tls: Option<Tls>,
    /// Client identifier.
    pub client_id: String,
    /// User name.
    pub username: Option<String>,
    /// Password.
    pub password: Option<String>,
    /// Keep-alive interval.
    pub keep_alive: Duration,
    /// Last will.
    pub will: Option<Will>,
    /// Topic filters subscribed after each connection, in this order.
    pub subscriptions: Vec<String>,
}

/// Why the broker refused the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Wrong user name or password.
    Credentials,
    /// Not allowed to connect.
    NotAuthorized,
    /// Another reason (code of the standard).
    Other(u8),
}

/// Why the client is not connected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The address cannot be resolved or the broker cannot be reached.
    Unreachable,
    /// The broker answered but refused.
    Refused(Refusal),
    /// Encryption is on and the broker's certificate is not trusted by Windows.
    CertificateUnknown(Fingerprint),
    /// The broker's certificate is not the one the user trusted.
    CertificateChanged(Fingerprint),
    /// Encryption failed otherwise (no TLS on that port, protocol mismatch).
    Encryption,
    /// The broker did not answer like an MQTT broker.
    Protocol,
    /// An established connection was lost.
    Lost,
}

/// What happened, for the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Connected and subscribed.
    Connected,
    /// A message arrived.
    Message {
        /// Topic.
        topic: String,
        /// Payload.
        payload: Vec<u8>,
        /// Delivered from the broker's store (kept while away).
        retained: bool,
    },
    /// Not connected; the client tries again after `retry_in`.
    Disconnected {
        /// Why.
        problem: Problem,
        /// When the next attempt is made.
        retry_in: Duration,
    },
}

/// A message to publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// Retained by the broker.
    pub retain: bool,
}

enum Command {
    Publish(Message),
    /// QoS 1 message received: acknowledge it.
    Ack(u16),
    /// The reader of connection `generation` stopped.
    ReaderEnded(u64),
    /// Publish these, disconnect cleanly, and stop; then answer.
    Stop(Vec<Message>, Sender<()>),
    /// Stop at once without a clean disconnection: the broker publishes the last will.
    Abandon,
}

/// A running MQTT client. Dropping it abandons the connection without a clean
/// disconnection, so the broker publishes the last will (the application went away
/// without saying goodbye).
pub struct Client {
    commands: Sender<Command>,
    events: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    /// Set when the client is given up (dropped, or a stop that did not complete in
    /// time): a worker still busy connecting must then never send CONNECT, or it would
    /// take over the identifier of a newer client.
    abandoned: Arc<AtomicBool>,
}

impl Client {
    /// Starts connecting in the background.
    pub fn start(options: Options, notify: Notify) -> Client {
        let (commands, inbox) = mpsc::channel();
        let (outbox, events) = mpsc::channel();
        let loopback = commands.clone();
        let abandoned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&abandoned);
        let worker = std::thread::Builder::new()
            .name("bv-mqtt".into())
            .spawn(move || {
                Worker { options, inbox, loopback, outbox, notify, generation: 0, abandoned: flag }
                    .run()
            })
            .ok();
        Client { commands, events, worker, abandoned }
    }

    /// Publishes `message` if connected (otherwise it is dropped: everything is published
    /// again on connection).
    pub fn publish(&self, message: Message) {
        let _ = self.commands.send(Command::Publish(message));
    }

    /// What happened since the last call.
    pub fn events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Publishes `farewell` (if connected), disconnects cleanly and stops, waiting at most
    /// `timeout` (before sleep, the system gives little time).
    pub fn stop(mut self, farewell: Vec<Message>, timeout: Duration) {
        let (done, wait) = mpsc::channel();
        let stopped = self.commands.send(Command::Stop(farewell, done)).is_ok()
            && wait.recv_timeout(timeout).is_ok();
        match self.worker.take() {
            Some(worker) if stopped => {
                let _ = worker.join();
            }
            // Busy connecting: it gives up as soon as it looks (see `abandoned`).
            _ => self.abandoned.store(true, Ordering::SeqCst),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // The worker holds a sender of its own channel (for its readers), so it would not
        // notice the client going away by itself.
        if self.worker.take().is_some() {
            self.abandoned.store(true, Ordering::SeqCst);
            let _ = self.commands.send(Command::Abandon);
        }
    }
}

/// The socket, plain or encrypted.
enum Stream {
    Plain(TcpStream),
    Tls(Box<native_tls::TlsStream<TcpStream>>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            Stream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            Stream::Tls(s) => s.flush(),
        }
    }
}

fn lock(stream: &Mutex<Stream>) -> MutexGuard<'_, Stream> {
    stream.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An established connection.
struct Link {
    stream: Arc<Mutex<Stream>>,
    /// The raw socket, to wake the reader by shutting it down.
    raw: TcpStream,
    reader: Option<JoinHandle<()>>,
}

impl Link {
    fn write(&self, packet: &Packet) -> std::io::Result<()> {
        let mut stream = lock(&self.stream);
        stream.write_all(&packet.encode())?;
        stream.flush()
    }

    fn close(mut self) {
        let _ = self.raw.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

struct Worker {
    options: Options,
    inbox: Receiver<Command>,
    loopback: Sender<Command>,
    outbox: Sender<Event>,
    notify: Notify,
    generation: u64,
    abandoned: Arc<AtomicBool>,
}

impl Worker {
    fn emit(&self, event: Event) {
        if self.outbox.send(event).is_ok() {
            (self.notify)();
        }
    }

    fn run(mut self) {
        let mut retry = FIRST_RETRY;
        loop {
            if self.abandoned.load(Ordering::SeqCst) {
                return;
            }
            let problem = match connect(&self.options, &self.abandoned) {
                Ok((stream, raw)) => {
                    let since = Instant::now();
                    let ended = self.serve(stream, raw);
                    // Back to the first delay only after a connection that held: a
                    // broker dropping every client right away is retried less and less
                    // often (FR-115).
                    if since.elapsed() >= STABLE {
                        retry = FIRST_RETRY;
                    }
                    match ended {
                        Some(problem) => problem,
                        None => return,
                    }
                }
                Err(problem) => problem,
            };
            self.emit(Event::Disconnected { problem, retry_in: retry });
            // Wait before the next attempt; a stop request ends it at once.
            let deadline = Instant::now() + retry;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.inbox.recv_timeout(left) {
                    Ok(Command::Stop(_, done)) => {
                        let _ = done.send(());
                        return;
                    }
                    Ok(Command::Abandon) => return,
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            retry = (retry * 2).min(LAST_RETRY);
        }
    }

    /// Serves one connection until it is lost (returns why) or the client stops (`None`).
    fn serve(&mut self, stream: Stream, raw: TcpStream) -> Option<Problem> {
        self.generation += 1;
        let generation = self.generation;
        let stream = Arc::new(Mutex::new(stream));
        let reader = {
            let reader_raw = raw.try_clone().ok()?;
            let stream = Arc::clone(&stream);
            let commands = self.loopback.clone();
            let outbox = self.outbox.clone();
            let notify = Arc::clone(&self.notify);
            let silence = self.options.keep_alive * 3 / 2;
            std::thread::Builder::new()
                .name("bv-mqtt-reader".into())
                .spawn(move || {
                    read_loop(&stream, &reader_raw, silence, &commands, &outbox, &notify);
                    let _ = commands.send(Command::ReaderEnded(generation));
                })
                .ok()
        };
        let link = Link { stream, raw, reader };
        if link.reader.is_none() {
            link.close();
            return Some(Problem::Lost);
        }
        let filters: Vec<(String, u8)> =
            self.options.subscriptions.iter().map(|t| (t.clone(), 0)).collect();
        if !filters.is_empty() && link.write(&Packet::Subscribe(1, filters)).is_err() {
            link.close();
            return Some(Problem::Lost);
        }
        self.emit(Event::Connected);
        let mut last_ping = Instant::now();
        loop {
            let wait = self.options.keep_alive.saturating_sub(last_ping.elapsed());
            let written = match self.inbox.recv_timeout(wait) {
                Ok(Command::Publish(m)) => link.write(&Packet::Publish(Publish {
                    topic: m.topic,
                    payload: m.payload,
                    qos: 0,
                    retain: m.retain,
                    id: 0,
                })),
                Ok(Command::Ack(id)) => link.write(&Packet::PubAck(id)),
                Ok(Command::ReaderEnded(g)) if g == generation => {
                    link.close();
                    return Some(Problem::Lost);
                }
                Ok(Command::ReaderEnded(_)) => Ok(()),
                // Gone without a goodbye (or a stop that came too late): no clean
                // disconnection, so the broker publishes the last will.
                Ok(Command::Abandon) => {
                    link.close();
                    return None;
                }
                Ok(Command::Stop(_, _)) if self.abandoned.load(Ordering::SeqCst) => {
                    link.close();
                    return None;
                }
                Ok(Command::Stop(farewell, done)) => {
                    for m in farewell {
                        let packet = Packet::Publish(Publish {
                            topic: m.topic,
                            payload: m.payload,
                            qos: 0,
                            retain: m.retain,
                            id: 0,
                        });
                        if link.write(&packet).is_err() {
                            break;
                        }
                    }
                    let _ = link.write(&Packet::Disconnect);
                    link.close();
                    let _ = done.send(());
                    return None;
                }
                Err(RecvTimeoutError::Timeout) => {
                    // A ping every interval whatever else was sent, so the broker always
                    // answers something and the reader can tell a dead connection.
                    last_ping = Instant::now();
                    link.write(&Packet::PingReq)
                }
                Err(RecvTimeoutError::Disconnected) => {
                    link.close();
                    return None;
                }
            };
            if written.is_err() {
                link.close();
                return Some(Problem::Lost);
            }
        }
    }
}

/// Reads packets until the connection ends (or stays silent longer than `silence`).
fn read_loop(
    stream: &Mutex<Stream>,
    raw: &TcpStream,
    silence: Duration,
    commands: &Sender<Command>,
    outbox: &Sender<Event>,
    notify: &Notify,
) {
    if raw.set_read_timeout(Some(silence)).is_err() {
        return;
    }
    let mut input: Vec<u8> = Vec::new();
    let mut buf = vec![0_u8; 16 * 1024];
    loop {
        // Wait for bytes without holding the lock.
        match raw.peek(&mut buf[..1]) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        {
            let mut guard = lock(stream);
            if raw.set_nonblocking(true).is_err() {
                return;
            }
            let mut closed = false;
            loop {
                match guard.read(&mut buf) {
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
            if raw.set_nonblocking(false).is_err() {
                closed = true;
            }
            drop(guard);
            let mut delivered = false;
            loop {
                match Packet::decode(&input) {
                    Ok(Some((packet, used))) => {
                        input.drain(..used);
                        if let Packet::Publish(p) = packet {
                            if p.qos == 1 {
                                let _ = commands.send(Command::Ack(p.id));
                            }
                            delivered |= outbox
                                .send(Event::Message {
                                    topic: p.topic,
                                    payload: p.payload,
                                    retained: p.retain,
                                })
                                .is_ok();
                        }
                    }
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
            if delivered {
                notify();
            }
            if closed {
                return;
            }
        }
    }
}

/// Resolves, connects (encrypting if asked), sends CONNECT and reads CONNACK.
fn connect(options: &Options, abandoned: &AtomicBool) -> Result<(Stream, TcpStream), Problem> {
    let tcp = tcp(options)?;
    let raw = tcp.try_clone().map_err(|_| Problem::Unreachable)?;
    let mut stream = match &options.tls {
        None => Stream::Plain(tcp),
        Some(tls) => Stream::Tls(Box::new(encrypt(options, tls, tcp)?)),
    };
    let connect = Packet::Connect(Connect {
        client_id: options.client_id.clone(),
        keep_alive: u16::try_from(options.keep_alive.as_secs()).unwrap_or(u16::MAX),
        clean_session: true,
        will: options.will.clone(),
        username: options.username.clone(),
        password: options.password.as_ref().map(|p| p.as_bytes().to_vec()),
    });
    // Given up meanwhile: never take over the identifier of a newer client.
    if abandoned.load(Ordering::SeqCst) {
        return Err(Problem::Lost);
    }
    stream.write_all(&connect.encode()).map_err(|_| Problem::Lost)?;
    stream.flush().map_err(|_| Problem::Lost)?;
    let mut input = Vec::new();
    let mut buf = [0_u8; 64];
    let answer = loop {
        match Packet::decode(&input) {
            Ok(Some((packet, _))) => break packet,
            Ok(None) => {}
            Err(_) => return Err(Problem::Protocol),
        }
        match stream.read(&mut buf) {
            Ok(0) => return Err(Problem::Protocol),
            Ok(n) => input.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            // No answer in time: nobody is listening there (FR-091 says "unreachable").
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(Problem::Unreachable)
            }
            Err(_) => return Err(Problem::Protocol),
        }
    };
    match answer {
        Packet::ConnAck { code: 0, .. } => {}
        Packet::ConnAck { code: 4, .. } => return Err(Problem::Refused(Refusal::Credentials)),
        Packet::ConnAck { code: 5, .. } => return Err(Problem::Refused(Refusal::NotAuthorized)),
        Packet::ConnAck { code, .. } => return Err(Problem::Refused(Refusal::Other(code))),
        _ => return Err(Problem::Protocol),
    }
    // From here on the reader decides when the connection is dead.
    raw.set_read_timeout(None).map_err(|_| Problem::Lost)?;
    Ok((stream, raw))
}

fn tcp(options: &Options) -> Result<TcpStream, Problem> {
    let addresses: Vec<SocketAddr> = (options.host.as_str(), options.port)
        .to_socket_addrs()
        .map_err(|_| Problem::Unreachable)?
        .collect();
    let tcp = addresses
        .iter()
        .find_map(|a| TcpStream::connect_timeout(a, CONNECT_TIMEOUT).ok())
        .ok_or(Problem::Unreachable)?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT)).map_err(|_| Problem::Unreachable)?;
    tcp.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(|_| Problem::Unreachable)?;
    Ok(tcp)
}

/// The fingerprint of the certificate a TLS stream received.
fn fingerprint(stream: &native_tls::TlsStream<TcpStream>) -> Option<Fingerprint> {
    let certificate = stream.peer_certificate().ok()??;
    let der = certificate.to_der().ok()?;
    Some(bv_core::encoding::sha256(&der))
}

fn encrypt(
    options: &Options,
    tls: &Tls,
    tcp: TcpStream,
) -> Result<native_tls::TlsStream<TcpStream>, Problem> {
    let connector = |any_certificate: bool| {
        native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(any_certificate)
            .danger_accept_invalid_hostnames(any_certificate)
            .build()
            .map_err(|_| Problem::Encryption)
    };
    match tls.trusted {
        // Only the certificate the user trusted, whatever Windows thinks of it.
        Some(trusted) => {
            let stream =
                connector(true)?.connect(&options.host, tcp).map_err(|_| Problem::Encryption)?;
            match fingerprint(&stream) {
                Some(seen) if seen == trusted => Ok(stream),
                Some(seen) => Err(Problem::CertificateChanged(seen)),
                None => Err(Problem::Encryption),
            }
        }
        None => match connector(false)?.connect(&options.host, tcp) {
            Ok(stream) => Ok(stream),
            // Was it the certificate? Look at it over a second connection that accepts
            // any certificate (nothing is sent over it).
            Err(_) => {
                let probe = connector(true)?
                    .connect(&options.host, self::tcp(options)?)
                    .map_err(|_| Problem::Encryption)?;
                Err(fingerprint(&probe).map_or(Problem::Encryption, Problem::CertificateUnknown))
            }
        },
    }
}

/// Tries one connection with `options` and reports the outcome ("Test connection",
/// FR-091): `Ok` when the broker accepted it.
pub fn test(options: &Options) -> Result<(), Problem> {
    let (mut stream, raw) = connect(options, &AtomicBool::new(false))?;
    let _ = stream.write_all(&Packet::Disconnect.encode());
    let _ = stream.flush();
    let _ = raw.shutdown(Shutdown::Both);
    Ok(())
}
