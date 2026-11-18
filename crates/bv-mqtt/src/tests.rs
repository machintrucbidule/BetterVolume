//! The client against the test broker, over the loopback address (no other network).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::broker::{Broker, BrokerConfig};
use crate::client::{self, Client, Event, Message, Options, Problem, Refusal, Tls};
use crate::codec::Will;

const WAIT: Duration = Duration::from_secs(10);

fn options(port: u16) -> Options {
    Options {
        host: "127.0.0.1".into(),
        port,
        tls: None,
        client_id: "test-client".into(),
        username: None,
        password: None,
        keep_alive: Duration::from_secs(30),
        will: Some(Will {
            topic: "pc/availability".into(),
            payload: b"offline".to_vec(),
            retain: true,
        }),
        subscriptions: vec!["pc/first/set".into(), "pc/+/set".into()],
    }
}

/// Events received by a test, and how far earlier waits already looked.
#[derive(Debug, Default)]
struct Seen {
    all: Vec<Event>,
    next: usize,
}

impl Seen {
    fn iter(&self) -> std::slice::Iter<'_, Event> {
        self.all.iter()
    }
}

/// Waits (at most `WAIT`) for an event after the last one an earlier wait matched.
fn until(client: &Client, seen: &mut Seen, wanted: impl Fn(&Event) -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(i) = seen.all[seen.next..].iter().position(&wanted) {
            seen.next += i + 1;
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        seen.all.extend(client.events());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn connected(event: &Event) -> bool {
    *event == Event::Connected
}

fn notify_counter() -> (Arc<AtomicUsize>, client::Notify) {
    let count = Arc::new(AtomicUsize::new(0));
    let inner = Arc::clone(&count);
    (
        count,
        Arc::new(move || {
            inner.fetch_add(1, Ordering::SeqCst);
        }),
    )
}

fn message(topic: &str, payload: &str, retain: bool) -> Message {
    Message { topic: topic.into(), payload: payload.as_bytes().to_vec(), retain }
}

#[test]
fn publishes_and_receives_retained_then_live_messages() {
    let broker = Broker::start(BrokerConfig::default()).unwrap();
    broker.publish("pc/first/set", b"kept", true);
    broker.publish("pc/second/set", b"also kept", true);
    let (notified, notify) = notify_counter();
    let client = Client::start(options(broker.port()), notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected), "{seen:?}");
    // Retained messages come in subscription order, flagged as kept.
    assert!(until(
        &client,
        &mut seen,
        |e| matches!(e, Event::Message { topic, .. } if topic == "pc/second/set")
    ));
    let messages: Vec<(String, bool)> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Message { topic, retained, .. } => Some((topic.clone(), *retained)),
            _ => None,
        })
        .collect();
    assert_eq!(messages, [("pc/first/set".into(), true), ("pc/second/set".into(), true)]);
    assert!(notified.load(Ordering::SeqCst) > 0, "the application is woken");

    broker.publish("pc/third/set", b"live", false);
    assert!(until(&client, &mut seen, |e| matches!(
        e,
        Event::Message { topic, payload, retained: false } if topic == "pc/third/set" && payload == b"live"
    )));

    client.publish(message("pc/state", "40", true));
    assert!(broker.wait_for(WAIT, |b| b.retained("pc/state").as_deref() == Some(&b"40"[..])));
    client.stop(vec![message("pc/availability", "offline", true)], WAIT);
    assert!(broker.wait_for(WAIT, |b| b.connected() == 0));
    assert_eq!(broker.retained("pc/availability").as_deref(), Some(&b"offline"[..]));
}

#[test]
fn a_clean_stop_does_not_publish_the_will_but_a_loss_does_and_reconnects() {
    let broker = Broker::start(BrokerConfig::default()).unwrap();
    let (_, notify) = notify_counter();
    let client = Client::start(options(broker.port()), notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected));
    client.publish(message("pc/availability", "online", true));
    assert!(broker.wait_for(WAIT, |b| b.retained("pc/availability").is_some()));

    // A network loss: the broker publishes the will, the client comes back by itself.
    broker.drop_connections();
    assert!(until(&client, &mut seen, |e| matches!(
        e,
        Event::Disconnected { problem: Problem::Lost, .. }
    )));
    assert!(broker
        .wait_for(WAIT, |b| b.retained("pc/availability").as_deref() == Some(&b"offline"[..])));
    assert!(until(&client, &mut seen, connected), "reconnected: {seen:?}");
    assert_eq!(broker.connections(), 2);

    client.publish(message("pc/availability", "online", true));
    assert!(
        broker.wait_for(WAIT, |b| b.retained("pc/availability").as_deref() == Some(&b"online"[..]))
    );
    client.stop(Vec::new(), WAIT);
    assert!(broker.wait_for(WAIT, |b| b.connected() == 0));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        broker.retained("pc/availability").as_deref(),
        Some(&b"online"[..]),
        "a clean stop sends no will"
    );
}

#[test]
fn keep_alive_holds_an_idle_connection() {
    let broker = Broker::start(BrokerConfig::default()).unwrap();
    let (_, notify) = notify_counter();
    let mut opts = options(broker.port());
    opts.keep_alive = Duration::from_secs(1);
    let client = Client::start(opts, notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected));
    // The broker drops a client silent for 1.5 intervals: pings keep it.
    std::thread::sleep(Duration::from_millis(3500));
    assert_eq!(broker.connected(), 1);
    assert_eq!(broker.connections(), 1, "never reconnected");
    assert!(client.events().iter().all(|e| !matches!(e, Event::Disconnected { .. })));
    client.stop(Vec::new(), WAIT);
}

#[test]
fn refused_credentials_and_unreachable_brokers_are_told_apart() {
    let config = BrokerConfig {
        credentials: Some(("user".into(), "right".into())),
        ..BrokerConfig::default()
    };
    let broker = Broker::start(config).unwrap();
    let mut opts = options(broker.port());
    opts.username = Some("user".into());
    opts.password = Some("wrong".into());
    assert_eq!(client::test(&opts), Err(Problem::Refused(Refusal::Credentials)));
    opts.password = Some("right".into());
    assert_eq!(client::test(&opts), Ok(()));

    let (_, notify) = notify_counter();
    opts.password = Some("wrong".into());
    let client = Client::start(opts, notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, |e| matches!(
        e,
        Event::Disconnected { problem: Problem::Refused(Refusal::Credentials), retry_in }
            if *retry_in == Duration::from_secs(1)
    )));
    // The next delay is longer (FR-115).
    assert!(until(&client, &mut seen, |e| matches!(
        e,
        Event::Disconnected { retry_in, .. } if *retry_in == Duration::from_secs(2)
    )));
    client.stop(Vec::new(), WAIT);

    // Nothing listens on that port any more.
    let port = broker.port();
    drop(broker);
    std::thread::sleep(Duration::from_millis(100));
    let mut closed = options(port);
    closed.host = "127.0.0.1".into();
    assert_eq!(client::test(&closed), Err(Problem::Unreachable));
    closed.host = "name.invalid".into();
    assert_eq!(client::test(&closed), Err(Problem::Unreachable));
}

fn tls_broker() -> Broker {
    let identity = crate::broker::Identity::from_pem(
        include_str!("../tests/data/cert.pem"),
        include_bytes!("../tests/data/key.pem"),
    )
    .expect("test identity");
    Broker::start(BrokerConfig { tls: Some(identity), ..BrokerConfig::default() }).unwrap()
}

fn test_certificate() -> [u8; 32] {
    bv_core::encoding::sha256(include_bytes!("../tests/data/cert.der"))
}

#[test]
fn an_untrusted_certificate_is_reported_then_trusted_by_fingerprint() {
    let broker = tls_broker();
    let mut opts = options(broker.port());
    opts.tls = Some(Tls { trusted: None });
    // Windows does not trust the self-made test certificate.
    assert_eq!(client::test(&opts), Err(Problem::CertificateUnknown(test_certificate())));

    opts.tls = Some(Tls { trusted: Some(test_certificate()) });
    assert_eq!(client::test(&opts), Ok(()));
    let (_, notify) = notify_counter();
    let client = Client::start(opts.clone(), notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected), "{seen:?}");
    broker.publish("pc/first/set", b"over tls", false);
    assert!(until(&client, &mut seen, |e| matches!(
        e,
        Event::Message { payload, .. } if payload == b"over tls"
    )));
    client.publish(message("pc/state", "secret", true));
    assert!(broker.wait_for(WAIT, |b| b.retained("pc/state").is_some()));
    client.stop(Vec::new(), WAIT);

    // Another certificate than the trusted one is refused.
    opts.tls = Some(Tls { trusted: Some([7; 32]) });
    assert_eq!(client::test(&opts), Err(Problem::CertificateChanged(test_certificate())));

    // Encryption asked from a broker that has none.
    let plain = Broker::start(BrokerConfig::default()).unwrap();
    let mut opts = options(plain.port());
    opts.tls = Some(Tls { trusted: None });
    assert_eq!(client::test(&opts), Err(Problem::Encryption));
}

#[test]
fn many_messages_arrive_complete_and_in_order() {
    let broker = Broker::start(BrokerConfig::default()).unwrap();
    let (_, notify) = notify_counter();
    let client = Client::start(options(broker.port()), notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected));
    let big = "x".repeat(40_000);
    for i in 0..200 {
        let payload = if i == 100 { big.clone() } else { i.to_string() };
        broker.publish("pc/many/set", payload.as_bytes(), false);
    }
    let mut payloads = Vec::new();
    let deadline = Instant::now() + WAIT;
    while payloads.len() < 200 && Instant::now() < deadline {
        for event in client.events() {
            if let Event::Message { payload, .. } = event {
                payloads.push(String::from_utf8(payload).unwrap());
            }
        }
    }
    assert_eq!(payloads.len(), 200);
    assert_eq!(payloads[100], big);
    assert_eq!(payloads[199], "199");
    client.stop(Vec::new(), WAIT);
}

#[test]
fn a_dropped_client_lets_the_broker_publish_its_will() {
    let broker = Broker::start(BrokerConfig::default()).unwrap();
    let (_, notify) = notify_counter();
    let client = Client::start(options(broker.port()), notify);
    let mut seen = Seen::default();
    assert!(until(&client, &mut seen, connected));
    client.publish(message("pc/availability", "online", true));
    assert!(
        broker.wait_for(WAIT, |b| b.retained("pc/availability").as_deref() == Some(&b"online"[..]))
    );
    // The application went away without saying goodbye (closed by Windows).
    drop(client);
    assert!(broker
        .wait_for(WAIT, |b| b.retained("pc/availability").as_deref() == Some(&b"offline"[..])));
}
