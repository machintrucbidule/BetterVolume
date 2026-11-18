//! The Home Assistant link (FR-110 to FR-119): starting and stopping the
//! MQTT client, publishing the PC's state when it changes, applying the commands.
//!
//! The network itself is behind [`Link`] (the real client in the executable, a fake in
//! tests). The client is created only while the integration is on and configured
//! (FR-118); it is stopped with an `offline` farewell before sleep and on quit, and with
//! the removal of every entity when the integration is turned off (FR-116).

use std::sync::mpsc::Receiver;
use std::time::Duration;

use bv_audio::{AudioEngine, Flow, Role};
use bv_core::devices;
use bv_core::ha::{self, Command, Entity, Incoming, Publisher};
use bv_core::settings::HomeAssistant;
use bv_mqtt::{Event, Options, Problem, Tls, Will};

use super::App;

/// How long the farewell may take before sleep (Windows gives about two seconds).
pub const FAREWELL_TIMEOUT: Duration = Duration::from_millis(1500);

/// The MQTT connection as the application sees it.
pub trait Link {
    /// Publishes a message (dropped while not connected).
    fn publish(&mut self, message: ha::Message);
    /// What happened since the last call.
    fn events(&mut self) -> Vec<Event>;
    /// Publishes `farewell`, disconnects and stops.
    fn stop(self: Box<Self>, farewell: Vec<ha::Message>);
}

/// Starts a connection with these options.
pub type Connect = Box<dyn FnMut(Options) -> Box<dyn Link>>;

/// Turns a text into another, or fails (password encryption and decryption).
pub type Transform = Box<dyn Fn(&str) -> Option<String>>;

/// Starts a connection test; its outcome arrives on the channel.
pub type Tester = Box<dyn FnMut(Options) -> Receiver<Result<(), Problem>>>;

/// What the application needs from the platform to run the link.
pub struct Platform {
    /// Starts a connection.
    pub connect: Connect,
    /// Decrypts the stored password (FR-092).
    pub unprotect: Transform,
    /// Encrypts a password for storage (FR-092).
    pub protect: Transform,
    /// Runs a connection test in the background ("Test connection", FR-091); the result
    /// arrives on the channel (and the UI thread is woken).
    pub test: Tester,
    /// The computer name (default PC name, FR-090).
    pub computer_name: String,
}

/// Where the link stands (shown on the Home Assistant page, FR-091).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The integration is off.
    Off,
    /// On, but no broker address is set.
    NotConfigured,
    /// Connecting.
    Connecting,
    /// Connected.
    Connected,
    /// Not connected; tries again after a while.
    Failed(Problem),
}

/// How the link says goodbye.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Farewell {
    /// The PC goes away for a while (sleep, quit): its entities become unavailable.
    Offline,
    /// The integration is turned off: its entities are removed (FR-116).
    Remove,
}

/// What the caller must do after Home Assistant news.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HaEffects {
    /// Something shown may have changed (tray, flyout, settings window).
    pub refresh: bool,
    /// Show the volume bubble (a volume or mute change ordered from Home
    /// Assistant).
    pub volume_bubble: bool,
    /// Press these media keys.
    pub media: Vec<ha::MediaKey>,
}

/// The link's state kept by the application.
#[derive(Default)]
pub(super) struct HaSession {
    pub(super) platform: Option<Platform>,
    link: Option<Box<dyn Link>>,
    /// The identity the link was started with (topics, last will).
    identity: Option<ha::Identity>,
    publisher: Publisher,
    pub(super) status: Option<Status>,
}

impl HaSession {
    /// The computer name (empty before the platform is known).
    pub(super) fn computer_name(&self) -> String {
        self.platform.as_ref().map(|p| p.computer_name.clone()).unwrap_or_default()
    }

    /// Encrypts a password for storage.
    pub(super) fn protect(&self, password: &str) -> Option<String> {
        self.platform.as_ref().and_then(|p| (p.protect)(password))
    }

    /// Decrypts a stored password.
    pub(super) fn unprotect(&self, stored: &str) -> Option<String> {
        self.platform.as_ref().and_then(|p| (p.unprotect)(stored))
    }

    /// Starts a connection test in the background.
    pub(super) fn run_test(&mut self, options: Options) -> Option<Receiver<Result<(), Problem>>> {
        self.platform.as_mut().map(|p| (p.test)(options))
    }
}

impl<E: AudioEngine> App<E> {
    /// Gives the application the means to run the link, and starts it if it is on.
    pub fn set_ha_platform(&mut self, platform: Platform) {
        self.ha.platform = Some(platform);
        self.ha_start();
    }

    /// Where the link stands.
    pub fn ha_status(&self) -> Status {
        let ha = &self.settings.home_assistant;
        if !ha.enabled {
            Status::Off
        } else if !ha.configured() {
            Status::NotConfigured
        } else {
            self.ha.status.clone().unwrap_or(Status::Connecting)
        }
    }

    /// Who this PC is in Home Assistant now (`None` before a node identifier exists).
    fn ha_identity(&self) -> Option<ha::Identity> {
        let ha = &self.settings.home_assistant;
        let computer = self.ha.platform.as_ref().map_or("", |p| p.computer_name.as_str());
        let pc_name = ha.pc_name.clone().unwrap_or_else(|| computer.to_owned());
        Some(ha::Identity {
            node_id: ha.node_id.clone()?,
            pc_name: if pc_name.trim().is_empty() {
                self.language.text(bv_core::i18n::Msg::AppName).to_owned()
            } else {
                pc_name
            },
            prefix: ha.discovery_prefix.clone(),
            language: self.language,
            version: crate::VERSION.to_owned(),
        })
    }

    /// Starts the link if it is on, configured and not running (FR-118: nothing otherwise).
    pub fn ha_start(&mut self) {
        if self.ha.link.is_some() || self.ha.platform.is_none() {
            return;
        }
        let ha = &self.settings.home_assistant;
        if !ha.enabled || !ha.configured() {
            return;
        }
        if self.settings.home_assistant.node_id.is_none() {
            self.settings.home_assistant.node_id = Some(new_node_id());
            self.dirty = true;
        }
        let Some(identity) = self.ha_identity() else { return };
        let Some(platform) = self.ha.platform.as_ref() else { return };
        let ha = &self.settings.home_assistant;
        let password = ha.password.as_deref().and_then(|stored| {
            let password = (platform.unprotect)(stored);
            if password.is_none() {
                bv_shell::log::error("home assistant: the stored password cannot be read");
            }
            password
        });
        let mut options = connection_options(ha, password);
        options.client_id = format!("bettervolume-{}", identity.node_id);
        options.will = Some(Will {
            topic: identity.availability_topic(),
            payload: ha::OFFLINE.as_bytes().to_vec(),
            retain: true,
        });
        options.subscriptions = identity.subscriptions();
        let Some(platform) = self.ha.platform.as_mut() else { return };
        bv_shell::log::info("home assistant: connecting");
        self.ha.link = Some((platform.connect)(options));
        self.ha.identity = Some(identity);
        self.ha.publisher.reset();
        self.ha.status = Some(Status::Connecting);
    }

    /// Stops the link, saying goodbye (`offline`, or removing every entity).
    pub fn ha_stop(&mut self, farewell: Farewell) {
        let Some(link) = self.ha.link.take() else { return };
        let identity = self.ha.identity.take();
        // Without a connection there is nobody to tell: no wait at all (a worker busy
        // connecting gives up by itself).
        let connected = self.ha.status == Some(Status::Connected);
        let messages = match (farewell, identity) {
            (_, Some(_)) if !connected => Vec::new(),
            (Farewell::Offline, Some(identity)) => vec![identity.offline()],
            (Farewell::Remove, Some(identity)) => ha::removal(&identity),
            (_, None) => Vec::new(),
        };
        link.stop(messages);
        self.ha.status = None;
        bv_shell::log::info("home assistant: stopped");
    }

    /// Applies what the link reported (called when the UI thread is woken).
    pub fn on_ha_events(&mut self) -> HaEffects {
        // A connection test of the settings window may have finished (FR-091).
        let mut effects = HaEffects { refresh: self.ha_test_done(), ..HaEffects::default() };
        let events = match self.ha.link.as_mut() {
            Some(link) => link.events(),
            None => return effects,
        };
        for event in events {
            match event {
                Event::Connected => {
                    bv_shell::log::info("home assistant: connected");
                    self.ha.status = Some(Status::Connected);
                    self.ha.publisher.reset();
                    effects.refresh = true;
                }
                Event::Disconnected { problem, retry_in } => {
                    if self.ha.status.as_ref() != Some(&Status::Failed(problem.clone())) {
                        bv_shell::log::info(&format!(
                            "home assistant: not connected ({}), next try in {} s",
                            problem_name(&problem),
                            retry_in.as_secs()
                        ));
                    }
                    self.ha.status = Some(Status::Failed(problem));
                    effects.refresh = true;
                }
                Event::Message { topic, payload, .. } => {
                    self.ha_message(&topic, &payload, &mut effects);
                }
            }
        }
        // States changed by the commands go out with the rest (`ha_sync`).
        self.ha_sync();
        effects
    }

    fn ha_message(&mut self, topic: &str, payload: &[u8], effects: &mut HaEffects) {
        let Some(identity) = self.ha.identity.clone() else { return };
        match ha::parse(&identity, topic, payload) {
            Incoming::Nothing => {}
            Incoming::Invalid { entity } => {
                // FR-117: ignored and logged, without the value.
                bv_shell::log::info(&format!(
                    "home assistant: invalid command for {} ignored",
                    entity.object_id()
                ));
                self.ha_clear(&identity, Some(entity));
            }
            Incoming::Command { entity, command } => {
                self.ha_command(command, effects);
                self.ha_clear(&identity, entity);
            }
        }
    }

    /// Erases an order kept by the broker once handled.
    fn ha_clear(&mut self, identity: &ha::Identity, entity: Option<Entity>) {
        if let (Some(entity), Some(link)) = (entity, self.ha.link.as_mut()) {
            if entity.kept_while_away() {
                link.publish(identity.clear_command(entity));
            }
        }
    }

    fn ha_command(&mut self, command: Command, effects: &mut HaEffects) {
        let microphones = self.settings.general.microphone_enabled;
        match command {
            Command::Device(Flow::Input, _)
            | Command::Volume(Flow::Input, _)
            | Command::Mute(Flow::Input, _)
                if !microphones => {}
            Command::Device(flow, name) => {
                let known = self.settings.devices.list(flow);
                let live = self.engine.devices(flow);
                let current = self.engine.default_device(flow, Role::Media);
                let id = devices::visible(known, live, current)
                    .into_iter()
                    .find(|v| v.display_name == name)
                    .map(|v| v.info.id.clone());
                match id {
                    Some(id) => {
                        self.switch_device(flow, &id);
                        effects.refresh = true;
                    }
                    None => bv_shell::log::info("home assistant: unknown device name ignored"),
                }
            }
            Command::Volume(flow, percent) => {
                if self.engine.endpoint(flow).is_some()
                    && super::applied(self.engine.set_volume(flow, percent), "remote volume")
                {
                    effects.refresh = true;
                    effects.volume_bubble |= flow == Flow::Output;
                }
            }
            Command::Mute(flow, muted) => {
                if self.engine.endpoint(flow).is_some()
                    && super::applied(self.engine.set_mute(flow, muted), "remote mute")
                {
                    effects.refresh = true;
                    effects.volume_bubble |= flow == Flow::Output;
                }
            }
            Command::Media(key) => effects.media.push(key),
            Command::HomeAssistantOnline => self.ha.publisher.reset(),
        }
    }

    /// What Home Assistant must show of one flow (FR-112: the flyout's list).
    fn ha_flow(&self, flow: Flow) -> ha::FlowState {
        let known = self.settings.devices.list(flow);
        let live = self.engine.devices(flow);
        let current = self.engine.default_device(flow, Role::Media);
        let visible = devices::visible(known, live, current);
        let endpoint = self.engine.endpoint(flow);
        ha::FlowState {
            options: visible.iter().map(|v| v.display_name.clone()).collect(),
            current: visible.iter().find(|v| v.current).map(|v| v.display_name.clone()),
            volume: endpoint.map(|e| e.percent()),
            muted: endpoint.map(|e| e.muted),
        }
    }

    /// Everything Home Assistant shows of the PC.
    pub fn ha_state(&self) -> ha::State {
        ha::State {
            output: self.ha_flow(Flow::Output),
            microphone: self.settings.general.microphone_enabled.then(|| self.ha_flow(Flow::Input)),
        }
    }

    /// Publishes what changed since the last call (after every event; nothing while not
    /// connected: everything goes out on connection).
    pub fn ha_sync(&mut self) {
        if self.ha.status != Some(Status::Connected) || self.ha.link.is_none() {
            return;
        }
        // The PC name or the language may have changed: the identity follows them.
        let Some(identity) = self.ha_identity() else { return };
        let state = self.ha_state();
        let messages = self.ha.publisher.update(&identity, &state);
        if let Some(link) = self.ha.link.as_mut() {
            for message in messages {
                link.publish(message);
            }
        }
    }
}

/// The connection part of the options (address, encryption, credentials): what a test
/// and the link share. A password is only sent with a user name (MQTT requires it).
pub(super) fn connection_options(ha: &HomeAssistant, password: Option<String>) -> Options {
    let trusted = ha.trusted_certificate.as_deref().and_then(bv_core::encoding::from_hex);
    let username = (!ha.username.is_empty()).then(|| ha.username.clone());
    Options {
        host: ha.host.clone(),
        port: ha.port,
        tls: ha
            .tls
            .then(|| Tls { trusted: trusted.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok()) }),
        client_id: String::new(),
        password: password.filter(|_| username.is_some()),
        username,
        keep_alive: Duration::from_secs(ha::KEEP_ALIVE_SECS.into()),
        will: None,
        subscriptions: Vec::new(),
    }
}

/// Removes this PC from Home Assistant when BetterVolume is uninstalled with its settings
/// deleted: connects with the saved settings, erases every entity and disconnects.
/// Returns `true` when done or when there was nothing to remove (the link never
/// configured, or never connected: no node identifier), `false` when the server could not
/// be reached within `timeout`.
pub fn remove_at_uninstall(
    settings: &HomeAssistant,
    unprotect: &dyn Fn(&str) -> Option<String>,
    timeout: Duration,
) -> bool {
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Instant;

    if !settings.enabled || !settings.configured() {
        return true;
    }
    let Some(node_id) = settings.node_id.clone() else { return true };
    let identity = ha::Identity {
        node_id,
        pc_name: String::new(),
        prefix: settings.discovery_prefix.clone(),
        language: bv_core::i18n::Language::ENGLISH,
        version: String::new(),
    };
    let password = settings.password.as_deref().and_then(unprotect);
    let mut options = connection_options(settings, password);
    options.client_id = format!("bettervolume-{}", identity.node_id);
    let (woken, wake) = mpsc::channel();
    let client = bv_mqtt::Client::start(
        options,
        Arc::new(move || {
            let _ = woken.send(());
        }),
    );
    let deadline = Instant::now() + timeout;
    let connected = loop {
        let events = client.events();
        if events.iter().any(|e| matches!(e, Event::Connected)) {
            break true;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if events.iter().any(|e| matches!(e, Event::Disconnected { .. })) || left.is_zero() {
            break false;
        }
        let _ = wake.recv_timeout(left);
    };
    if !connected {
        bv_shell::log::error("uninstall: Home Assistant could not be reached, entities kept");
        client.stop(Vec::new(), Duration::ZERO);
        return false;
    }
    let farewell = ha::removal(&identity)
        .into_iter()
        .map(|m| bv_mqtt::Message {
            topic: m.topic,
            payload: m.payload.into_bytes(),
            retain: m.retain,
        })
        .collect();
    client.stop(farewell, Duration::from_secs(5));
    bv_shell::log::info("uninstall: this PC was removed from Home Assistant");
    true
}

/// A new node identifier: 16 lower-case hexadecimal digits from the process's random
/// hashing keys, the time and the process identifier.
fn new_node_id() -> String {
    use std::hash::BuildHasher;
    let seed = (std::time::SystemTime::now(), std::process::id());
    let value = std::collections::hash_map::RandomState::new().hash_one(seed);
    format!("{value:016x}")
}

/// A problem's name for the log (English, no personal data).
pub fn problem_name(problem: &Problem) -> &'static str {
    match problem {
        Problem::Unreachable => "unreachable",
        Problem::Refused(bv_mqtt::Refusal::Credentials) => "credentials refused",
        Problem::Refused(bv_mqtt::Refusal::NotAuthorized) => "not authorized",
        Problem::Refused(bv_mqtt::Refusal::Other(_)) => "refused",
        Problem::CertificateUnknown(_) => "unknown certificate",
        Problem::CertificateChanged(_) => "certificate changed",
        Problem::Encryption => "encryption failed",
        Problem::Protocol => "not an MQTT broker",
        Problem::Lost => "connection lost",
    }
}

/// The real MQTT client as a [`Link`].
pub struct MqttLink(pub Option<bv_mqtt::Client>);

impl Link for MqttLink {
    fn publish(&mut self, message: ha::Message) {
        if let Some(client) = &self.0 {
            client.publish(bv_mqtt::Message {
                topic: message.topic,
                payload: message.payload.into_bytes(),
                retain: message.retain,
            });
        }
    }

    fn events(&mut self) -> Vec<Event> {
        self.0.as_ref().map(bv_mqtt::Client::events).unwrap_or_default()
    }

    fn stop(mut self: Box<Self>, farewell: Vec<ha::Message>) {
        if let Some(client) = self.0.take() {
            let farewell: Vec<bv_mqtt::Message> = farewell
                .into_iter()
                .map(|m| bv_mqtt::Message {
                    topic: m.topic,
                    payload: m.payload.into_bytes(),
                    retain: m.retain,
                })
                .collect();
            let timeout = if farewell.is_empty() { Duration::ZERO } else { FAREWELL_TIMEOUT };
            client.stop(farewell, timeout);
        }
    }
}

#[cfg(test)]
mod tests;
