//! The Home Assistant link of the application: with a fake connection, then end to end with
//! the real client and the test broker (loopback only).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bv_audio::fake::FakeEngine;
use bv_audio::{DeviceInfo, Flow, Role};
use bv_core::devices::KnownDevice;
use bv_core::ha::{Entity, MediaKey, Message};
use bv_core::i18n::Language;
use bv_core::settings::Settings;
use bv_mqtt::{Event, Options, Problem};

use super::*;

/// What the fake connection saw.
#[derive(Default)]
struct Wire {
    options: Vec<Options>,
    published: Vec<Message>,
    pending: Vec<Event>,
    farewell: Option<Vec<Message>>,
}

struct FakeLink(Rc<RefCell<Wire>>);

impl Link for FakeLink {
    fn publish(&mut self, message: Message) {
        self.0.borrow_mut().published.push(message);
    }

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0.borrow_mut().pending)
    }

    fn stop(self: Box<Self>, farewell: Vec<Message>) {
        self.0.borrow_mut().farewell = Some(farewell);
    }
}

fn device(id: &str, name: &str) -> DeviceInfo {
    DeviceInfo { id: id.into(), windows_name: name.into(), ..Default::default() }
}

fn engine() -> FakeEngine {
    let mut engine = FakeEngine::new()
        .with_device(Flow::Output, device("spk", "Speakers"))
        .with_device(Flow::Output, device("hs", "Headset"))
        .with_device(Flow::Output, device("tv", "TV"))
        .with_device(Flow::Input, device("mic", "Mic"));
    engine.levels.insert("spk".into(), (0.4, false));
    engine.sync();
    engine
}

fn enabled() -> Settings {
    let mut settings = Settings::default();
    let ha = &mut settings.home_assistant;
    ha.enabled = true;
    ha.host = "broker.lan".into();
    ha.username = "user".into();
    ha.password = Some("protected".into());
    settings.devices.outputs = vec![
        KnownDevice { windows_name: "Speakers".into(), ..KnownDevice::new("spk") },
        KnownDevice { windows_name: "Headset".into(), ..KnownDevice::new("hs") },
        KnownDevice { windows_name: "TV".into(), hidden: true, ..KnownDevice::new("tv") },
    ];
    settings
}

fn platform(wire: &Rc<RefCell<Wire>>) -> Platform {
    let wire = Rc::clone(wire);
    Platform {
        connect: Box::new(move |options| {
            wire.borrow_mut().options.push(options);
            Box::new(FakeLink(Rc::clone(&wire)))
        }),
        unprotect: Box::new(|stored| (stored == "protected").then(|| "secret".to_owned())),
        protect: Box::new(|password| Some(format!("protected:{password}"))),
        test: Box::new(|_| std::sync::mpsc::channel().1),
        computer_name: "DESKTOP-1".into(),
    }
}

fn app(settings: Settings) -> (App<FakeEngine>, Rc<RefCell<Wire>>) {
    let wire = Rc::new(RefCell::new(Wire::default()));
    let mut app = App::new(engine(), settings, Language::ENGLISH);
    app.set_ha_platform(platform(&wire));
    (app, wire)
}

fn connect(app: &mut App<FakeEngine>, wire: &Rc<RefCell<Wire>>) {
    wire.borrow_mut().pending.push(Event::Connected);
    app.on_ha_events();
}

fn receive(
    app: &mut App<FakeEngine>,
    wire: &Rc<RefCell<Wire>>,
    entity: Entity,
    payload: &str,
) -> HaEffects {
    let identity = app.ha_identity().expect("identity");
    wire.borrow_mut().pending.push(Event::Message {
        topic: identity.command_topic(entity),
        payload: payload.as_bytes().to_vec(),
        retained: false,
    });
    app.on_ha_events()
}

fn published(wire: &Rc<RefCell<Wire>>, topic_end: &str) -> Vec<String> {
    wire.borrow()
        .published
        .iter()
        .filter(|m| m.topic.ends_with(topic_end))
        .map(|m| m.payload.clone())
        .collect()
}

#[test]
fn nothing_starts_while_the_integration_is_off() {
    let (app, wire) = app(Settings::default());
    assert!(wire.borrow().options.is_empty(), "FR-118: no connection");
    assert_eq!(app.ha_status(), Status::Off);
    let mut settings = Settings::default();
    settings.home_assistant.enabled = true;
    let (app, wire) = self::app(settings);
    assert!(wire.borrow().options.is_empty(), "no address: nothing to connect to");
    assert_eq!(app.ha_status(), Status::NotConfigured);
}

#[test]
fn the_connection_uses_the_settings() {
    let (mut app, wire) = app(enabled());
    let node = app.settings().home_assistant.node_id.clone().expect("generated once");
    assert!(bv_core::settings::valid_node_id(&node), "{node}");
    assert!(app.take_dirty(), "the new identifier is saved");
    assert_eq!(app.ha_status(), Status::Connecting);
    let wire_ref = wire.borrow();
    let options = &wire_ref.options[0];
    assert_eq!((options.host.as_str(), options.port), ("broker.lan", 1883));
    assert_eq!(options.username.as_deref(), Some("user"));
    assert_eq!(options.password.as_deref(), Some("secret"), "decrypted for the connection only");
    assert_eq!(options.tls, None);
    assert_eq!(options.client_id, format!("bettervolume-{node}"));
    assert_eq!(options.keep_alive, Duration::from_secs(30), "keep-alive period");
    let will = options.will.as_ref().expect("last will");
    assert_eq!(will.topic, format!("bettervolume/{node}/availability"));
    assert_eq!((will.payload.as_slice(), will.retain), (&b"offline"[..], true));
    assert_eq!(options.subscriptions[0], format!("bettervolume/{node}/output/set"));
}

#[test]
fn a_trusted_certificate_is_passed_to_the_connection() {
    let mut settings = enabled();
    settings.home_assistant.tls = true;
    settings.home_assistant.trusted_certificate = Some("ab".repeat(32));
    let (_, wire) = app(settings);
    let tls = wire.borrow().options[0].tls.clone().expect("encrypted");
    assert_eq!(tls.trusted, Some([0xAB; 32]));
}

#[test]
fn everything_is_published_on_connection_mirroring_the_flyout() {
    let (mut app, wire) = app(enabled());
    assert!(wire.borrow().published.is_empty(), "nothing before the connection");
    connect(&mut app, &wire);
    assert_eq!(app.ha_status(), Status::Connected);
    assert_eq!(published(&wire, "/availability"), ["online"]);
    // The hidden TV is not offered (FR-112); the computer name names the PC (FR-090).
    let select = published(&wire, "/select/").pop();
    let config = wire
        .borrow()
        .published
        .iter()
        .find(|m| m.topic.contains("/select/") && m.topic.ends_with("/output/config"))
        .map(|m| m.payload.clone())
        .expect("output select");
    assert!(select.is_none());
    assert!(config.contains("\"options\":[\"Speakers\",\"Headset\"]"), "{config}");
    assert!(config.contains("\"name\":\"DESKTOP-1\""), "{config}");
    assert_eq!(published(&wire, "/output/state"), ["Speakers"]);
    assert_eq!(published(&wire, "/output_volume/state"), ["40"]);
    assert_eq!(published(&wire, "/microphone/state"), ["Mic"]);

    // Afterwards only changes go out.
    wire.borrow_mut().published.clear();
    app.wheel(1);
    app.ha_sync();
    let sent: Vec<String> = wire.borrow().published.iter().map(|m| m.topic.clone()).collect();
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(published(&wire, "/output_volume/state"), ["42"]);
}

#[test]
fn a_hidden_device_made_current_elsewhere_is_offered_while_current() {
    let (mut app, wire) = app(enabled());
    app.engine
        .set_default_device(Flow::Output, "tv", bv_audio::SwitchScope::MediaAndCalls)
        .unwrap();
    connect(&mut app, &wire);
    let state = app.ha_state();
    assert_eq!(state.output.options, ["Speakers", "Headset", "TV"], "FR-045 at the end");
    assert_eq!(state.output.current.as_deref(), Some("TV"));
}

#[test]
fn commands_are_applied_and_kept_orders_cleared() {
    let (mut app, wire) = app(enabled());
    connect(&mut app, &wire);
    let node = app.settings().home_assistant.node_id.clone().unwrap();

    let effects = receive(&mut app, &wire, Entity::Output, "Headset");
    assert!(effects.refresh);
    assert_eq!(app.engine.default_device(Flow::Output, Role::Media), Some("hs"));
    assert_eq!(app.engine.default_device(Flow::Output, Role::Calls), Some("hs"), "scope");
    let cleared = wire.borrow().published.iter().any(|m| {
        m.topic == format!("bettervolume/{node}/output/set") && m.payload.is_empty() && m.retain
    });
    assert!(cleared, "the kept order is erased");
    assert_eq!(published(&wire, "/output/state").last().map(String::as_str), Some("Headset"));

    let effects = receive(&mut app, &wire, Entity::OutputVolume, "15");
    assert!(effects.volume_bubble, "a volume order from Home Assistant shows the bubble");
    assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 15);
    let effects = receive(&mut app, &wire, Entity::OutputMute, "ON");
    assert!(effects.volume_bubble);
    assert!(app.engine.endpoint(Flow::Output).unwrap().muted);

    // The microphone level: applied, no bubble (the bubble shows the output).
    let effects = receive(&mut app, &wire, Entity::MicrophoneVolume, "70");
    assert!(!effects.volume_bubble);
    assert_eq!(app.engine.endpoint(Flow::Input).unwrap().percent(), 70);

    let effects = receive(&mut app, &wire, Entity::NextTrack, "PRESS");
    assert_eq!(effects.media, [MediaKey::Next]);
}

#[test]
fn invalid_and_unknown_commands_change_nothing() {
    let (mut app, wire) = app(enabled());
    connect(&mut app, &wire);
    for (entity, payload) in [
        (Entity::Output, "Nonexistent"),
        (Entity::Output, "TV"), // hidden, not offered (FR-112)
        (Entity::OutputVolume, "250"),
        (Entity::OutputMute, "maybe"),
    ] {
        let effects = receive(&mut app, &wire, entity, payload);
        assert!(!effects.volume_bubble, "{payload}");
    }
    assert_eq!(app.engine.default_device(Flow::Output, Role::Media), Some("spk"));
    assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 40);
    assert!(!app.engine.endpoint(Flow::Output).unwrap().muted);
}

#[test]
fn microphone_entities_follow_microphone_management() {
    let mut settings = enabled();
    settings.general.microphone_enabled = false;
    let (mut app, wire) = app(settings);
    connect(&mut app, &wire);
    assert!(app.ha_state().microphone.is_none());
    let mic_config = wire
        .borrow()
        .published
        .iter()
        .find(|m| m.topic.ends_with("/microphone/config"))
        .map(|m| m.payload.clone());
    assert_eq!(mic_config.as_deref(), Some(""), "removed (FR-076)");
    receive(&mut app, &wire, Entity::MicrophoneVolume, "10");
    assert_eq!(app.engine.endpoint(Flow::Input).unwrap().percent(), 50, "ignored while off");
}

#[test]
fn home_assistant_restarting_gets_everything_again() {
    let (mut app, wire) = app(enabled());
    connect(&mut app, &wire);
    let first = wire.borrow().published.len();
    wire.borrow_mut().published.clear();
    let identity = app.ha_identity().unwrap();
    wire.borrow_mut().pending.push(Event::Message {
        topic: identity.status_topic(),
        payload: b"online".to_vec(),
        retained: false,
    });
    app.on_ha_events();
    assert_eq!(wire.borrow().published.len(), first);
}

#[test]
fn stopping_says_goodbye_or_removes_everything() {
    let (mut app, wire) = app(enabled());
    connect(&mut app, &wire);
    app.ha_stop(Farewell::Offline);
    let farewell = wire.borrow_mut().farewell.take().expect("stopped");
    assert_eq!(farewell.len(), 1);
    assert_eq!(farewell[0].payload, "offline");
    assert_eq!(app.ha_status(), Status::Connecting, "on but not running");

    app.ha_start();
    assert_eq!(wire.borrow().options.len(), 2, "started again (resume)");
    // Not connected yet: nobody to tell, so nothing to wait for.
    app.ha_stop(Farewell::Offline);
    assert_eq!(wire.borrow_mut().farewell.take(), Some(Vec::new()));

    app.ha_start();
    connect(&mut app, &wire);
    app.ha_stop(Farewell::Remove);
    let farewell = wire.borrow_mut().farewell.take().expect("stopped");
    assert!(farewell.len() > 10 && farewell.iter().all(|m| m.payload.is_empty() && m.retain));
}

#[test]
fn failures_are_reported_in_the_status() {
    let (mut app, wire) = app(enabled());
    wire.borrow_mut().pending.push(Event::Disconnected {
        problem: Problem::Unreachable,
        retry_in: Duration::from_secs(4),
    });
    let effects = app.on_ha_events();
    assert!(effects.refresh);
    assert_eq!(app.ha_status(), Status::Failed(Problem::Unreachable));
    assert!(wire.borrow().published.is_empty());
}

#[test]
fn the_pc_name_and_language_reach_home_assistant_without_new_identifiers() {
    let mut settings = enabled();
    settings.home_assistant.pc_name = Some("Bureau".into());
    let (mut app, wire) = app(settings);
    connect(&mut app, &wire);
    wire.borrow_mut().published.clear();
    app.settings.general.language = Some("fr".into());
    app.set_os_languages(Vec::new());
    app.ha_sync();
    let configs: Vec<String> = wire
        .borrow()
        .published
        .iter()
        .filter(|m| m.topic.ends_with("/config"))
        .map(|m| m.payload.clone())
        .collect();
    // Every name changes but "Volume", the same in both languages.
    assert_eq!(configs.len(), 8);
    assert!(configs.iter().any(|c| c.contains("\"name\":\"Sortie audio\"")));
    assert!(configs.iter().all(|c| c.contains("\"name\":\"Bureau\"")));
}

// ----- End to end: the real client and the test broker -----

#[test]
fn end_to_end_with_the_test_broker() {
    use bv_mqtt::broker::{Broker, BrokerConfig};

    let broker = Broker::start(BrokerConfig::default()).unwrap();
    let mut settings = enabled();
    settings.home_assistant.host = "127.0.0.1".into();
    settings.home_assistant.port = broker.port();
    settings.home_assistant.username = String::new();
    settings.home_assistant.password = None;
    settings.home_assistant.node_id = Some("e2e".into());
    // An order given while the PC was away is waiting.
    broker.publish("bettervolume/e2e/output/set", b"Headset", true);

    let mut app = App::new(engine(), settings, Language::ENGLISH);
    app.set_ha_platform(Platform {
        connect: Box::new(|options| {
            let client = bv_mqtt::Client::start(options, std::sync::Arc::new(|| {}));
            Box::new(MqttLink(Some(client)))
        }),
        unprotect: Box::new(|_| None),
        protect: Box::new(|_| None),
        test: Box::new(|_| std::sync::mpsc::channel().1),
        computer_name: "PC".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let wait = |app: &mut App<FakeEngine>, done: &dyn Fn(&App<FakeEngine>) -> bool| {
        while Instant::now() < deadline && !done(app) {
            app.on_ha_events();
            std::thread::sleep(Duration::from_millis(10));
        }
        done(app)
    };
    assert!(wait(&mut app, &|a| a.engine.default_device(Flow::Output, Role::Media) == Some("hs")));
    assert!(broker.wait_for(Duration::from_secs(5), |b| {
        b.retained("bettervolume/e2e/output/state").as_deref() == Some(&b"Headset"[..])
            && b.retained("bettervolume/e2e/output/set").is_none()
    }));
    assert_eq!(broker.retained("bettervolume/e2e/availability").as_deref(), Some(&b"online"[..]));
    assert!(broker.retained("homeassistant/select/e2e/output/config").is_some());

    // A live command from Home Assistant.
    broker.publish("bettervolume/e2e/output_volume/set", b"25", true);
    assert!(wait(&mut app, &|a| a
        .engine
        .endpoint(Flow::Output)
        .is_some_and(|e| e.percent() == 25)));

    app.ha_stop(Farewell::Offline);
    assert!(broker.wait_for(Duration::from_secs(5), |b| b.connected() == 0));
    assert_eq!(broker.retained("bettervolume/e2e/availability").as_deref(), Some(&b"offline"[..]));
}

#[test]
fn uninstalling_removes_the_pc_from_home_assistant() {
    use bv_mqtt::broker::{Broker, BrokerConfig};

    let broker = Broker::start(BrokerConfig::default()).unwrap();
    // What a previous run left in the broker.
    broker.publish("homeassistant/select/gone/output/config", b"{}", true);
    broker.publish("bettervolume/gone/output/state", b"Headset", true);
    broker.publish("bettervolume/gone/availability", b"offline", true);
    let mut settings = enabled().home_assistant;
    settings.host = "127.0.0.1".into();
    settings.port = broker.port();
    settings.username = String::new();
    settings.password = None;
    settings.node_id = Some("gone".into());

    assert!(remove_at_uninstall(&settings, &|_| None, Duration::from_secs(5)));
    assert!(broker.wait_for(Duration::from_secs(5), |b| {
        !b.retained_topics().iter().any(|t| t.contains("/gone/"))
    }));

    // Never configured or never connected: nothing to do, nothing tried.
    let mut never = settings.clone();
    never.node_id = None;
    assert!(remove_at_uninstall(&never, &|_| None, Duration::ZERO));
    let mut off = settings.clone();
    off.enabled = false;
    assert!(remove_at_uninstall(&off, &|_| None, Duration::ZERO));

    // An unreachable server: reported, within the time given.
    drop(broker);
    let started = Instant::now();
    assert!(!remove_at_uninstall(&settings, &|_| None, Duration::from_secs(2)));
    assert!(started.elapsed() < Duration::from_secs(4));
}
