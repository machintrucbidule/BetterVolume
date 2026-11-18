//! Home Assistant entities, topics and payloads (FR-110 to FR-117).
//!
//! Pure description of what BetterVolume publishes over MQTT and what it accepts: the
//! discovery configurations (one Home Assistant device per PC, FR-110), the state payloads,
//! the parsing of commands (FR-117) and a [`Publisher`] that sends only what changed. The
//! network side lives in `bv-mqtt`, the wiring to the audio engine in the executable.
//!
//! Topics: discovery under `<prefix>/<component>/<node>/<object>/config`; everything else
//! under `bettervolume/<node>/`: `availability`, `<object>/state`, `<object>/set`. The node
//! identifier is generated once per installation and never changes, so renaming the PC or
//! a device changes labels, never identifiers (FR-116).
//!
//! Availability (FR-114): the microphone entities and the media buttons follow the
//! `availability` topic (with the connection's last will); the output select, volume
//! and mute have none, so Home Assistant keeps sending their commands while the PC is away.
//! Those commands are published retained (`retain` in their configuration), kept by the
//! broker, applied at the next connection, then cleared ([`Identity::clear_command`]).

use crate::audio::Flow;
use crate::i18n::{Language, Msg};

/// Keep-alive interval of the MQTT connection, in seconds: an unclean loss is
/// noticed by the broker after about one and a half intervals.
pub const KEEP_ALIVE_SECS: u16 = 30;
/// Availability payloads.
pub const ONLINE: &str = "online";
/// Availability payload of a PC that is away (also the last will).
pub const OFFLINE: &str = "offline";
/// Root of the state and command topics.
const ROOT: &str = "bettervolume";
/// The state payload meaning "no value" (Home Assistant shows the entity as unknown).
const NONE: &str = "None";
const ON: &str = "ON";
const OFF: &str = "OFF";
const PRESS: &str = "PRESS";

/// One Home Assistant entity of the PC (FR-111).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entity {
    /// Output device (select).
    Output,
    /// Output volume (number, 0 to 100).
    OutputVolume,
    /// Output mute (switch).
    OutputMute,
    /// Microphone device (select).
    Microphone,
    /// Microphone level (number).
    MicrophoneVolume,
    /// Microphone mute (switch).
    MicrophoneMute,
    /// Play/pause media key (button).
    PlayPause,
    /// Next track media key (button).
    NextTrack,
    /// Previous track media key (button).
    PreviousTrack,
}

/// What kind of Home Assistant entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Component {
    Select,
    Number,
    Switch,
    Button,
}

impl Component {
    fn name(self) -> &'static str {
        match self {
            Component::Select => "select",
            Component::Number => "number",
            Component::Switch => "switch",
            Component::Button => "button",
        }
    }
}

impl Entity {
    /// Every entity, in subscription order: the output first, so that orders kept while
    /// the PC was away are applied output, then volume, then mute.
    pub const ALL: [Entity; 9] = [
        Entity::Output,
        Entity::OutputVolume,
        Entity::OutputMute,
        Entity::Microphone,
        Entity::MicrophoneVolume,
        Entity::MicrophoneMute,
        Entity::PlayPause,
        Entity::NextTrack,
        Entity::PreviousTrack,
    ];

    /// The fixed, language-independent object identifier.
    pub fn object_id(self) -> &'static str {
        match self {
            Entity::Output => "output",
            Entity::OutputVolume => "output_volume",
            Entity::OutputMute => "output_mute",
            Entity::Microphone => "microphone",
            Entity::MicrophoneVolume => "microphone_volume",
            Entity::MicrophoneMute => "microphone_mute",
            Entity::PlayPause => "play_pause",
            Entity::NextTrack => "next_track",
            Entity::PreviousTrack => "previous_track",
        }
    }

    fn component(self) -> Component {
        match self {
            Entity::Output | Entity::Microphone => Component::Select,
            Entity::OutputVolume | Entity::MicrophoneVolume => Component::Number,
            Entity::OutputMute | Entity::MicrophoneMute => Component::Switch,
            Entity::PlayPause | Entity::NextTrack | Entity::PreviousTrack => Component::Button,
        }
    }

    /// The audio flow an entity controls (none for the media buttons).
    pub fn flow(self) -> Option<Flow> {
        match self {
            Entity::Output | Entity::OutputVolume | Entity::OutputMute => Some(Flow::Output),
            Entity::Microphone | Entity::MicrophoneVolume | Entity::MicrophoneMute => {
                Some(Flow::Input)
            }
            Entity::PlayPause | Entity::NextTrack | Entity::PreviousTrack => None,
        }
    }

    /// Usable while the PC is away; its last order is kept by the broker.
    pub fn kept_while_away(self) -> bool {
        matches!(self, Entity::Output | Entity::OutputVolume | Entity::OutputMute)
    }

    fn name(self) -> Msg {
        match self {
            Entity::Output => Msg::HaOutput,
            Entity::OutputVolume => Msg::HaOutputVolume,
            Entity::OutputMute => Msg::HaOutputMute,
            Entity::Microphone => Msg::HaMicrophone,
            Entity::MicrophoneVolume => Msg::HaMicrophoneVolume,
            Entity::MicrophoneMute => Msg::HaMicrophoneMute,
            Entity::PlayPause => Msg::HaPlayPause,
            Entity::NextTrack => Msg::HaNextTrack,
            Entity::PreviousTrack => Msg::HaPreviousTrack,
        }
    }

    /// Material Design icon shown by Home Assistant.
    fn icon(self) -> &'static str {
        match self {
            Entity::Output => "mdi:speaker",
            Entity::OutputVolume => "mdi:volume-high",
            Entity::OutputMute => "mdi:volume-off",
            Entity::Microphone => "mdi:microphone",
            Entity::MicrophoneVolume => "mdi:microphone-settings",
            Entity::MicrophoneMute => "mdi:microphone-off",
            Entity::PlayPause => "mdi:play-pause",
            Entity::NextTrack => "mdi:skip-next",
            Entity::PreviousTrack => "mdi:skip-previous",
        }
    }

    fn has_state(self) -> bool {
        self.component() != Component::Button
    }
}

/// Who this PC is in Home Assistant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Stable node identifier (lower-case letters and digits).
    pub node_id: String,
    /// Display name of the PC (FR-090).
    pub pc_name: String,
    /// Discovery prefix.
    pub prefix: String,
    /// Language of the entity names.
    pub language: Language,
    /// Application version (shown on the Home Assistant device page).
    pub version: String,
}

impl Identity {
    /// Root of this PC's state and command topics.
    pub fn base(&self) -> String {
        format!("{ROOT}/{}", self.node_id)
    }

    /// Availability topic (`online` / `offline`, retained; the last will says `offline`).
    pub fn availability_topic(&self) -> String {
        format!("{}/availability", self.base())
    }

    /// Discovery configuration topic of an entity.
    pub fn config_topic(&self, entity: Entity) -> String {
        format!(
            "{}/{}/{}/{}/config",
            self.prefix,
            entity.component().name(),
            self.node_id,
            entity.object_id()
        )
    }

    /// State topic of an entity (buttons have none).
    pub fn state_topic(&self, entity: Entity) -> Option<String> {
        entity.has_state().then(|| format!("{}/{}/state", self.base(), entity.object_id()))
    }

    /// Command topic of an entity.
    pub fn command_topic(&self, entity: Entity) -> String {
        format!("{}/{}/set", self.base(), entity.object_id())
    }

    /// Home Assistant's own status topic (`online` when it starts).
    pub fn status_topic(&self) -> String {
        format!("{}/status", self.prefix)
    }

    /// Topics to subscribe to, in order: every command topic (outputs first, so that the
    /// orders kept while away come output, volume, mute), then Home Assistant's
    /// status. Microphone commands arrive even while microphone management is off (the
    /// subscription lasts as long as the connection); the application ignores them then.
    pub fn subscriptions(&self) -> Vec<String> {
        let mut topics: Vec<String> = Entity::ALL.iter().map(|e| self.command_topic(*e)).collect();
        topics.push(self.status_topic());
        topics
    }

    /// The message erasing an order kept by the broker once it was applied.
    pub fn clear_command(&self, entity: Entity) -> Message {
        Message::retained(self.command_topic(entity), "")
    }

    /// The `online` availability message.
    pub fn online(&self) -> Message {
        Message::retained(self.availability_topic(), ONLINE)
    }

    /// The `offline` availability message (also the connection's last will).
    pub fn offline(&self) -> Message {
        Message::retained(self.availability_topic(), OFFLINE)
    }

    /// A lower-case ASCII form of the PC name for the suggested entity identifiers.
    fn slug(&self) -> String {
        let mut slug = String::new();
        for c in self.pc_name.chars().flat_map(char::to_lowercase) {
            if c.is_ascii_alphanumeric() {
                slug.push(c);
            } else if !slug.ends_with('_') && !slug.is_empty() {
                slug.push('_');
            }
        }
        let slug = slug.trim_end_matches('_');
        if slug.is_empty() {
            format!("{ROOT}_{}", self.node_id)
        } else {
            slug.to_owned()
        }
    }

    /// The discovery configuration of an entity (JSON). `options`: the choices of a
    /// select (FR-112).
    pub fn config(&self, entity: Entity, options: &[String]) -> String {
        let mut json = Json::object();
        json.text("name", self.language.text(entity.name()));
        json.text("unique_id", &format!("{ROOT}_{}_{}", self.node_id, entity.object_id()));
        let component = entity.component().name();
        json.text(
            "default_entity_id",
            &format!("{component}.{}_{}", self.slug(), entity.object_id()),
        );
        json.text("icon", entity.icon());
        if let Some(state) = self.state_topic(entity) {
            json.text("state_topic", &state);
        }
        json.text("command_topic", &self.command_topic(entity));
        match entity.component() {
            Component::Select => {
                json.raw("options", &Json::array(options));
            }
            Component::Number => {
                json.raw("min", "0");
                json.raw("max", "100");
                json.raw("step", "1");
                json.text("mode", "slider");
                json.text("unit_of_measurement", "%");
            }
            Component::Switch => {
                json.text("payload_on", ON);
                json.text("payload_off", OFF);
            }
            Component::Button => {
                json.text("payload_press", PRESS);
            }
        }
        if entity.kept_while_away() {
            json.raw("retain", "true");
        } else {
            json.text("availability_topic", &self.availability_topic());
        }
        let app = self.language.text(Msg::AppName);
        let mut device = Json::object();
        device.raw("identifiers", &Json::array(&[format!("{ROOT}_{}", self.node_id)]));
        device.text("name", &self.pc_name);
        device.text("manufacturer", app);
        device.text("model", app);
        device.text("sw_version", &self.version);
        json.raw("device", &device.finish());
        let mut origin = Json::object();
        origin.text("name", app);
        origin.text("sw_version", &self.version);
        json.raw("origin", &origin.finish());
        json.finish()
    }
}

/// One MQTT message to publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Topic.
    pub topic: String,
    /// Payload (UTF-8 text; empty with `retain` erases a retained message).
    pub payload: String,
    /// Kept by the broker for later subscribers.
    pub retain: bool,
}

impl Message {
    /// A retained message.
    pub fn retained(topic: String, payload: &str) -> Message {
        Message { topic, payload: payload.to_owned(), retain: true }
    }
}

/// The state of one flow as Home Assistant shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlowState {
    /// Choices of the select: display names in the flyout's order (FR-112).
    pub options: Vec<String>,
    /// The current device's display name, if it is listed.
    pub current: Option<String>,
    /// Volume in whole percent, if the current device has a volume.
    pub volume: Option<u8>,
    /// Mute state, if the current device has a volume.
    pub muted: Option<bool>,
}

/// Everything Home Assistant shows of the PC.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The output.
    pub output: FlowState,
    /// The microphone; `None` while microphone management is off (FR-076).
    pub microphone: Option<FlowState>,
}

impl State {
    fn flow(&self, flow: Flow) -> Option<&FlowState> {
        match flow {
            Flow::Output => Some(&self.output),
            Flow::Input => self.microphone.as_ref(),
        }
    }

    /// The configuration of every entity that exists now (`None`: removed).
    fn configs(&self, identity: &Identity) -> Vec<(Entity, Option<String>)> {
        Entity::ALL
            .iter()
            .map(|entity| {
                let flow = entity.flow().map_or(Some(&self.output), |f| self.flow(f));
                // A select needs at least one choice: with no device, it is not offered.
                let empty_select = |f: &&FlowState| {
                    entity.component() == Component::Select && f.options.is_empty()
                };
                let config =
                    flow.filter(|f| !empty_select(f)).map(|f| identity.config(*entity, &f.options));
                (*entity, config)
            })
            .collect()
    }

    /// The state payload of an entity (`None`: the entity does not exist now).
    fn payload(&self, entity: Entity) -> Option<String> {
        let flow = self.flow(entity.flow()?)?;
        Some(match entity {
            Entity::Output | Entity::Microphone => flow.current.clone().unwrap_or(NONE.into()),
            Entity::OutputVolume | Entity::MicrophoneVolume => {
                flow.volume.map_or(NONE.into(), |v| v.to_string())
            }
            Entity::OutputMute | Entity::MicrophoneMute => {
                flow.muted.map_or(NONE, |m| if m { ON } else { OFF }).to_owned()
            }
            Entity::PlayPause | Entity::NextTrack | Entity::PreviousTrack => return None,
        })
    }
}

/// Publishes the PC's state, sending only what changed since the last call.
#[derive(Clone, Debug, Default)]
pub struct Publisher {
    /// What was published since the connection started; `None` = nothing yet.
    sent: Option<Sent>,
}

#[derive(Clone, Debug)]
struct Sent {
    configs: Vec<(Entity, Option<String>)>,
    states: Vec<(Entity, Option<String>)>,
}

impl Publisher {
    /// Forgets what was published: the next [`Publisher::update`] sends everything (a new
    /// connection, or Home Assistant restarted).
    pub fn reset(&mut self) {
        self.sent = None;
    }

    /// The messages bringing Home Assistant to `state`. After a reset: availability,
    /// every configuration (an empty one for entities that do not exist, so that entities
    /// removed while the PC was away disappear, FR-116) and every state.
    pub fn update(&mut self, identity: &Identity, state: &State) -> Vec<Message> {
        let configs = state.configs(identity);
        let states: Vec<(Entity, Option<String>)> =
            Entity::ALL.iter().filter(|e| e.has_state()).map(|e| (*e, state.payload(*e))).collect();
        let mut out = Vec::new();
        let first = self.sent.is_none();
        if first {
            out.push(identity.online());
        }
        let before = self.sent.take();
        for (entity, config) in &configs {
            let old = before.as_ref().and_then(|s| s.configs.iter().find(|(e, _)| e == entity));
            if first || old.is_none_or(|(_, c)| c != config) {
                let payload = config.as_deref().unwrap_or("");
                out.push(Message::retained(identity.config_topic(*entity), payload));
            }
        }
        for (entity, payload) in &states {
            let old = before.as_ref().and_then(|s| s.states.iter().find(|(e, _)| e == entity));
            if first || old.is_none_or(|(_, p)| p != payload) {
                let Some(topic) = identity.state_topic(*entity) else { continue };
                // A removed entity's state is erased, so it does not linger in the broker.
                out.push(Message::retained(topic, payload.as_deref().unwrap_or("")));
            }
        }
        self.sent = Some(Sent { configs, states });
        out
    }
}

/// Every message removing the PC from Home Assistant (integration turned off, FR-116):
/// configurations, states, kept orders and availability are erased.
pub fn removal(identity: &Identity) -> Vec<Message> {
    let mut out = Vec::new();
    for entity in Entity::ALL {
        out.push(Message::retained(identity.config_topic(entity), ""));
        if let Some(topic) = identity.state_topic(entity) {
            out.push(Message::retained(topic, ""));
        }
        if entity.kept_while_away() {
            out.push(identity.clear_command(entity));
        }
    }
    out.push(Message::retained(identity.availability_topic(), ""));
    out
}

/// A media key (FR-111), pressed like the keyboard's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKey {
    /// Play/pause.
    PlayPause,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
}

/// What Home Assistant asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Make the device with this display name the current one.
    Device(Flow, String),
    /// Set the volume (whole percent).
    Volume(Flow, u8),
    /// Mute or unmute.
    Mute(Flow, bool),
    /// Press a media key.
    Media(MediaKey),
    /// Home Assistant (re)started: publish everything again.
    HomeAssistantOnline,
}

/// A received message, understood.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// A valid command; `entity` is set for entity commands (so a kept order can be
    /// cleared).
    Command {
        /// The entity commanded, if any.
        entity: Option<Entity>,
        /// The command.
        command: Command,
    },
    /// A command with an invalid payload (FR-117: ignored and logged, value not quoted).
    Invalid {
        /// The entity commanded.
        entity: Entity,
    },
    /// Nothing to do (an erased order, another topic, Home Assistant going offline).
    Nothing,
}

/// Understands a message received on `topic`.
pub fn parse(identity: &Identity, topic: &str, payload: &[u8]) -> Incoming {
    let Ok(payload) = std::str::from_utf8(payload) else {
        return match Entity::ALL.iter().find(|e| identity.command_topic(**e) == topic) {
            Some(entity) => Incoming::Invalid { entity: *entity },
            None => Incoming::Nothing,
        };
    };
    if topic == identity.status_topic() {
        return if payload == ONLINE {
            Incoming::Command { entity: None, command: Command::HomeAssistantOnline }
        } else {
            Incoming::Nothing
        };
    }
    let Some(entity) = Entity::ALL.iter().copied().find(|e| identity.command_topic(*e) == topic)
    else {
        return Incoming::Nothing;
    };
    // An empty payload is an erased kept order (our own).
    if payload.is_empty() {
        return Incoming::Nothing;
    }
    let command = match entity {
        Entity::Output | Entity::Microphone => entity
            .flow()
            .filter(|_| !payload.trim().is_empty())
            .map(|flow| Command::Device(flow, payload.to_owned())),
        Entity::OutputVolume | Entity::MicrophoneVolume => {
            percent(payload).zip(entity.flow()).map(|(v, flow)| Command::Volume(flow, v))
        }
        Entity::OutputMute | Entity::MicrophoneMute => {
            let on = match payload {
                ON => Some(true),
                OFF => Some(false),
                _ => None,
            };
            on.zip(entity.flow()).map(|(m, flow)| Command::Mute(flow, m))
        }
        Entity::PlayPause | Entity::NextTrack | Entity::PreviousTrack => (payload == PRESS)
            .then_some({
                Command::Media(match entity {
                    Entity::PlayPause => MediaKey::PlayPause,
                    Entity::NextTrack => MediaKey::Next,
                    _ => MediaKey::Previous,
                })
            }),
    };
    match command {
        Some(command) => Incoming::Command { entity: Some(entity), command },
        None => Incoming::Invalid { entity },
    }
}

/// A volume payload ("37", "37.0", "37.4") as whole percent; `None` if not a number
/// between 0 and 100.
fn percent(payload: &str) -> Option<u8> {
    let text = payload.trim();
    let valid = !text.is_empty()
        && text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && text.bytes().filter(|b| *b == b'.').count() <= 1
        && !text.starts_with('.')
        && !text.ends_with('.');
    if !valid {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    (0.0..=100.0).contains(&value).then(|| value.round() as u8)
}

/// A minimal JSON writer (objects of text, raw values and arrays of text).
struct Json(String);

impl Json {
    fn object() -> Json {
        Json(String::from("{"))
    }

    fn key(&mut self, key: &str) {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        quote(&mut self.0, key);
        self.0.push(':');
    }

    fn text(&mut self, key: &str, value: &str) {
        self.key(key);
        quote(&mut self.0, value);
    }

    fn raw(&mut self, key: &str, value: &str) {
        self.key(key);
        self.0.push_str(value);
    }

    fn finish(mut self) -> String {
        self.0.push('}');
        self.0
    }

    fn array(items: &[String]) -> String {
        let mut out = String::from("[");
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            quote(&mut out, item);
        }
        out.push(']');
        out
    }
}

/// Writes `text` as a JSON string.
fn quote(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity {
            node_id: "abc123".into(),
            pc_name: "PC Bureau".into(),
            prefix: "homeassistant".into(),
            language: Language::ENGLISH,
            version: "0.1.0".into(),
        }
    }

    fn state() -> State {
        State {
            output: FlowState {
                options: vec!["Speakers".into(), "Headset \"Pro\"".into()],
                current: Some("Speakers".into()),
                volume: Some(40),
                muted: Some(false),
            },
            microphone: Some(FlowState {
                options: vec!["Mic".into()],
                current: Some("Mic".into()),
                volume: Some(80),
                muted: Some(true),
            }),
        }
    }

    #[test]
    fn topics_follow_the_documented_layout() {
        let id = identity();
        assert_eq!(id.availability_topic(), "bettervolume/abc123/availability");
        assert_eq!(id.config_topic(Entity::Output), "homeassistant/select/abc123/output/config");
        assert_eq!(
            id.config_topic(Entity::OutputVolume),
            "homeassistant/number/abc123/output_volume/config"
        );
        assert_eq!(
            id.config_topic(Entity::MicrophoneMute),
            "homeassistant/switch/abc123/microphone_mute/config"
        );
        assert_eq!(
            id.config_topic(Entity::PlayPause),
            "homeassistant/button/abc123/play_pause/config"
        );
        assert_eq!(
            id.state_topic(Entity::Output).as_deref(),
            Some("bettervolume/abc123/output/state")
        );
        assert_eq!(id.state_topic(Entity::NextTrack), None);
        assert_eq!(id.command_topic(Entity::OutputMute), "bettervolume/abc123/output_mute/set");
        assert_eq!(id.status_topic(), "homeassistant/status");
    }

    #[test]
    fn subscriptions_put_the_output_first() {
        let id = identity();
        let all = id.subscriptions();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0], "bettervolume/abc123/output/set");
        assert_eq!(all[1], "bettervolume/abc123/output_volume/set");
        assert_eq!(all[2], "bettervolume/abc123/output_mute/set");
        assert_eq!(all[9], "homeassistant/status");
    }

    #[test]
    fn configurations_describe_each_entity() {
        let id = identity();
        let select = id.config(Entity::Output, &["Speakers".into(), "Headset \"Pro\"".into()]);
        assert!(select.starts_with("{\"name\":\"Audio output\","), "{select}");
        for part in [
            "\"unique_id\":\"bettervolume_abc123_output\"",
            "\"default_entity_id\":\"select.pc_bureau_output\"",
            "\"state_topic\":\"bettervolume/abc123/output/state\"",
            "\"command_topic\":\"bettervolume/abc123/output/set\"",
            "\"options\":[\"Speakers\",\"Headset \\\"Pro\\\"\"]",
            "\"retain\":true",
            "\"identifiers\":[\"bettervolume_abc123\"]",
            "\"name\":\"PC Bureau\"",
            "\"origin\":{\"name\":\"BetterVolume\",\"sw_version\":\"0.1.0\"}",
        ] {
            assert!(select.contains(part), "missing {part} in {select}");
        }
        assert!(!select.contains("availability"), "the output stays usable while away");

        let volume = id.config(Entity::OutputVolume, &[]);
        for part in ["\"min\":0", "\"max\":100", "\"step\":1", "\"mode\":\"slider\"", "\"%\""] {
            assert!(volume.contains(part), "missing {part} in {volume}");
        }
        let mic = id.config(Entity::MicrophoneMute, &[]);
        assert!(mic.contains("\"availability_topic\":\"bettervolume/abc123/availability\""));
        assert!(mic.contains("\"payload_on\":\"ON\""));
        assert!(!mic.contains("\"retain\""), "orders to the microphone are not kept");
        let button = id.config(Entity::PlayPause, &[]);
        assert!(button.contains("\"payload_press\":\"PRESS\""));
        assert!(!button.contains("state_topic"));
    }

    #[test]
    fn entity_names_follow_the_language_but_identifiers_do_not() {
        let mut id = identity();
        id.language = Language::from_tag("fr").expect("French ships");
        let select = id.config(Entity::Output, &[]);
        assert!(select.starts_with("{\"name\":\"Sortie audio\""), "{select}");
        assert!(select.contains("\"unique_id\":\"bettervolume_abc123_output\""));
        assert!(select.contains("\"default_entity_id\":\"select.pc_bureau_output\""));
    }

    #[test]
    fn suggested_identifiers_are_plain_ascii() {
        let mut id = identity();
        id.pc_name = "Été  -- PC".into();
        assert_eq!(id.slug(), "t_pc");
        id.pc_name = "ÉÉÉ".into();
        assert_eq!(id.slug(), "bettervolume_abc123");
    }

    #[test]
    fn the_publisher_sends_everything_first_then_only_changes() {
        let id = identity();
        let mut publisher = Publisher::default();
        let first = publisher.update(&id, &state());
        assert_eq!(first[0], id.online());
        // availability + 9 configurations + 6 states.
        assert_eq!(first.len(), 16);
        assert!(first.iter().all(|m| m.retain));
        assert!(first
            .contains(&Message::retained("bettervolume/abc123/output_volume/state".into(), "40")));
        assert!(first.contains(&Message::retained(
            "bettervolume/abc123/microphone_mute/state".into(),
            "ON"
        )));
        assert!(publisher.update(&id, &state()).is_empty(), "nothing changed");

        let mut changed = state();
        changed.output.volume = Some(41);
        assert_eq!(
            publisher.update(&id, &changed),
            [Message::retained("bettervolume/abc123/output_volume/state".into(), "41")]
        );
        // A new device changes the options of the select: its configuration again.
        changed.output.options.push("TV".into());
        let sent = publisher.update(&id, &changed);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].topic, "homeassistant/select/abc123/output/config");
        // Renaming the PC changes every configuration, not the identifiers (FR-116).
        let mut renamed = id.clone();
        renamed.pc_name = "Salon".into();
        let sent = publisher.update(&renamed, &changed);
        assert_eq!(sent.len(), 9);
        assert!(sent.iter().all(|m| m.payload.contains("\"unique_id\":\"bettervolume_abc123_")));

        publisher.reset();
        assert_eq!(publisher.update(&renamed, &changed).len(), 16, "everything after a reset");
    }

    #[test]
    fn microphone_entities_go_away_with_microphone_management() {
        let id = identity();
        let mut publisher = Publisher::default();
        publisher.update(&id, &state());
        let mut off = state();
        off.microphone = None;
        let sent = publisher.update(&id, &off);
        assert_eq!(sent.len(), 6, "3 configurations and 3 states erased: {sent:?}");
        assert!(sent.iter().all(|m| m.payload.is_empty() && m.topic.contains("microphone")));
        // After a reset, absent entities are erased too (removed while away).
        publisher.reset();
        let sent = publisher.update(&id, &off);
        assert!(sent.contains(&Message::retained(id.config_topic(Entity::Microphone), "")));
    }

    #[test]
    fn missing_values_are_published_as_unknown() {
        let id = identity();
        let mut publisher = Publisher::default();
        let empty = State::default();
        let sent = publisher.update(&id, &empty);
        let output = sent.iter().find(|m| m.topic == "bettervolume/abc123/output/state").unwrap();
        assert_eq!(output.payload, "None");
        let mute =
            sent.iter().find(|m| m.topic == "bettervolume/abc123/output_mute/state").unwrap();
        assert_eq!(mute.payload, "None");
        // No device at all: the select is not offered (it needs a choice), the rest is.
        let select = sent.iter().find(|m| m.topic == id.config_topic(Entity::Output)).unwrap();
        assert!(select.payload.is_empty());
        let volume =
            sent.iter().find(|m| m.topic == id.config_topic(Entity::OutputVolume)).unwrap();
        assert!(!volume.payload.is_empty());
    }

    #[test]
    fn removal_erases_everything() {
        let id = identity();
        let messages = removal(&id);
        assert!(messages.iter().all(|m| m.retain && m.payload.is_empty()));
        assert!(messages.iter().any(|m| m.topic == id.availability_topic()));
        assert!(messages.iter().any(|m| m.topic == id.command_topic(Entity::Output)));
        assert!(!messages.iter().any(|m| m.topic == id.command_topic(Entity::Microphone)));
        assert_eq!(messages.len(), 9 + 6 + 3 + 1);
    }

    #[test]
    fn commands_are_parsed_and_invalid_ones_reported() {
        let id = identity();
        let cmd = |entity: Entity, payload: &str| {
            parse(&id, &id.command_topic(entity), payload.as_bytes())
        };
        assert_eq!(
            cmd(Entity::Output, "Headset"),
            Incoming::Command {
                entity: Some(Entity::Output),
                command: Command::Device(Flow::Output, "Headset".into())
            }
        );
        assert_eq!(
            cmd(Entity::MicrophoneVolume, "37.0"),
            Incoming::Command {
                entity: Some(Entity::MicrophoneVolume),
                command: Command::Volume(Flow::Input, 37)
            }
        );
        assert!(matches!(
            cmd(Entity::OutputVolume, "100"),
            Incoming::Command { command: Command::Volume(Flow::Output, 100), .. }
        ));
        assert!(matches!(
            cmd(Entity::OutputMute, "ON"),
            Incoming::Command { command: Command::Mute(Flow::Output, true), .. }
        ));
        assert!(matches!(
            cmd(Entity::NextTrack, "PRESS"),
            Incoming::Command { command: Command::Media(MediaKey::Next), .. }
        ));
        for (entity, bad) in [
            (Entity::OutputVolume, "101"),
            (Entity::OutputVolume, "-1"),
            (Entity::OutputVolume, "loud"),
            (Entity::OutputVolume, "1e2"),
            (Entity::OutputVolume, "NaN"),
            (Entity::OutputMute, "on"),
            (Entity::PlayPause, "GO"),
            (Entity::Output, "   "),
        ] {
            assert_eq!(cmd(entity, bad), Incoming::Invalid { entity }, "{bad}");
        }
        assert_eq!(
            parse(&id, &id.command_topic(Entity::Output), &[0xFF]),
            Incoming::Invalid { entity: Entity::Output }
        );
        // Our own erasing of a kept order is not a command.
        assert_eq!(cmd(Entity::Output, ""), Incoming::Nothing);
        assert_eq!(parse(&id, "other/topic", b"x"), Incoming::Nothing);
        assert_eq!(
            parse(&id, "homeassistant/status", b"online"),
            Incoming::Command { entity: None, command: Command::HomeAssistantOnline }
        );
        assert_eq!(parse(&id, "homeassistant/status", b"offline"), Incoming::Nothing);
    }

    #[test]
    fn only_output_orders_are_kept_while_away() {
        let kept: Vec<Entity> = Entity::ALL.into_iter().filter(|e| e.kept_while_away()).collect();
        assert_eq!(kept, [Entity::Output, Entity::OutputVolume, Entity::OutputMute]);
        assert_eq!(
            identity().clear_command(Entity::OutputVolume),
            Message::retained("bettervolume/abc123/output_volume/set".into(), "")
        );
    }

    #[test]
    fn json_strings_are_escaped() {
        let mut out = String::new();
        quote(&mut out, "a\"b\\c\nd\u{1}é");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001é\"");
    }
}
