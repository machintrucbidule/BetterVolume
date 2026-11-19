//! Settings schema and its file format.
//!
//! This is the in-memory, versioned shape of the user's settings with the defaults required
//! by the functional specification (section 5.1), the device lists (section 5.2: order,
//! custom names, visibility, icon overrides), the remembered applications (section 5.6:
//! order, visibility) and remembered interface state. [`file`]
//! turns them into the text of the settings file and back (no I/O here: the executable
//! reads and writes the file, FR-125/FR-126), and the Home Assistant link (section 5.3).
//! A key added after version 1 (the bubble, the flyout height, Home Assistant) is simply
//! missing from older files: it keeps its default, no version change is needed.
//!
//! Versioning rule: [`SCHEMA_VERSION`] is bumped whenever a stored field changes meaning or
//! shape, and a migration from the previous version is added with it. A file written by a
//! newer application is never interpreted (see [`classify_version`]). Version 1 is the
//! first one ever written to disk.

pub mod file;

use crate::apps::AppList;
use crate::audio::Flow;
use crate::devices::KnownDevice;

/// Version of the schema described by this module.
pub const SCHEMA_VERSION: u32 = 1;

/// Smallest accepted wheel step (FR-073).
pub const WHEEL_STEP_MIN: u8 = 1;
/// Largest accepted wheel step (FR-073).
pub const WHEEL_STEP_MAX: u8 = 10;
/// Largest coordinate or size of a remembered window place, in pixels.
const PLACE_LIMIT: i32 = 100_000;

/// Default wheel step (FR-073).
pub const WHEEL_STEP_DEFAULT: u8 = 2;

/// What must be done with a stored settings file, given the schema version it declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionStatus {
    /// Same schema: use as is.
    Current,
    /// Older schema: migrate step by step from this version up to [`SCHEMA_VERSION`].
    NeedsMigration {
        /// Version declared by the file.
        from: u32,
    },
    /// Written by a newer application, or not a valid version: do not interpret. The
    /// caller starts from defaults and keeps the file aside (FR-126).
    Unsupported,
}

/// Classifies the schema version found in a stored settings file.
pub fn classify_version(found: u32) -> VersionStatus {
    if found == SCHEMA_VERSION {
        VersionStatus::Current
    } else if (1..SCHEMA_VERSION).contains(&found) {
        VersionStatus::NeedsMigration { from: found }
    } else {
        VersionStatus::Unsupported
    }
}

/// Which Windows default role a device switch changes (FR-072).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SwitchScope {
    /// Media and calls.
    #[default]
    MediaAndCalls,
    /// Media only.
    MediaOnly,
    /// Calls (communications) only.
    CallsOnly,
}

/// Tray icon picture (FR-011): what stands left of the waves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayIconStyle {
    /// The speaker.
    #[default]
    SameAsWindows,
    /// A drawing of the type of the current output.
    WithDeviceBadge,
}

/// Tray icon look (FR-084).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayIconLook {
    /// The Windows volume icon, in the taskbar's text colour.
    #[default]
    Windows,
    /// The speaker of the application icon, in the Windows accent colour.
    BetterVolume,
}

/// Tray icon size (FR-083).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrayIconSize {
    /// The size of the Windows volume icon.
    #[default]
    SameAsWindows,
    /// Taller, filling the height of the notification-area square.
    Large,
}

/// How tall the flyout is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlyoutHeight {
    /// The whole height of the screen: the panel never changes size.
    Full,
    /// As tall as its content (the default).
    #[default]
    Content,
}

/// The "General" settings (section 5.1) and remembered interface state.
///
/// "Start with Windows" (FR-074) is deliberately absent: the Windows autostart entry
/// itself is the source of truth (the installer and the user can change it outside the
/// application), so the settings window reads and writes that entry directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct General {
    /// FR-072.
    pub switch_scope: SwitchScope,
    /// FR-073. Always within [`WHEEL_STEP_MIN`]..=[`WHEEL_STEP_MAX`] after
    /// [`Settings::normalise`].
    pub wheel_step: u8,
    /// FR-075. `None` = automatic (OS display language); otherwise a language tag.
    pub language: Option<String>,
    /// FR-076.
    pub microphone_enabled: bool,
    /// FR-077.
    pub switch_notification: bool,
    /// FR-081: the volume bubble of the wheel and middle click.
    pub volume_bubble: bool,
    /// FR-078.
    pub tray_icon_style: TrayIconStyle,
    /// FR-083.
    pub tray_icon_size: TrayIconSize,
    /// FR-084.
    pub tray_icon_look: TrayIconLook,
    /// FR-082.
    pub flyout_height: FlyoutHeight,
    /// FR-079. Off by default.
    pub hide_windows_volume_icon: bool,
    /// FR-080.
    pub check_updates: bool,
    /// FR-060: whether the application-volumes section of the flyout is expanded.
    pub apps_section_expanded: bool,
    /// FR-071: becomes true once the settings window has been shown on first launch.
    pub first_run_done: bool,
    /// Becomes true once the first-run question about hiding the Windows volume
    /// icon was answered (files from before that question have it false: asked once).
    pub volume_icon_question_done: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            switch_scope: SwitchScope::default(),
            wheel_step: WHEEL_STEP_DEFAULT,
            language: None,
            microphone_enabled: true,
            switch_notification: true,
            volume_bubble: true,
            tray_icon_style: TrayIconStyle::default(),
            tray_icon_size: TrayIconSize::default(),
            tray_icon_look: TrayIconLook::default(),
            flyout_height: FlyoutHeight::default(),
            hide_windows_volume_icon: false,
            check_updates: true,
            apps_section_expanded: true,
            first_run_done: false,
            volume_icon_question_done: false,
        }
    }
}

/// The device lists (section 5.2): every device seen so far, per flow, in the user's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceLists {
    /// Output devices.
    pub outputs: Vec<KnownDevice>,
    /// Input devices.
    pub inputs: Vec<KnownDevice>,
}

impl DeviceLists {
    /// The list of one flow.
    pub fn list(&self, flow: Flow) -> &[KnownDevice] {
        match flow {
            Flow::Output => &self.outputs,
            Flow::Input => &self.inputs,
        }
    }

    /// The list of one flow, for changes.
    pub fn list_mut(&mut self, flow: Flow) -> &mut Vec<KnownDevice> {
        match flow {
            Flow::Output => &mut self.outputs,
            Flow::Input => &mut self.inputs,
        }
    }
}

/// Default MQTT port (FR-090).
pub const MQTT_PORT_DEFAULT: u16 = 1883;
/// Default discovery prefix: Home Assistant's own default.
pub const DISCOVERY_PREFIX_DEFAULT: &str = "homeassistant";
/// Longest accepted text of a Home Assistant field (host, user, PC name, prefix).
pub const HA_TEXT_MAX: usize = 255;

/// The Home Assistant link (section 5.3, FR-090 to FR-092).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeAssistant {
    /// The integration is on (off by default: no network at all, FR-118).
    pub enabled: bool,
    /// Broker address (host name or IP address).
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// Encrypted connection (TLS).
    pub tls: bool,
    /// User name; empty = none.
    pub username: String,
    /// The password, encrypted for the current Windows user (FR-092): the Base64
    /// text of the protected blob, opaque here. `None` = no password.
    pub password: Option<String>,
    /// Name of this PC in Home Assistant; `None` = the computer name (FR-090).
    pub pc_name: Option<String>,
    /// Discovery prefix.
    pub discovery_prefix: String,
    /// Stable identifier of this installation in Home Assistant (FR-116): generated once,
    /// lower-case letters and digits. `None` until the link first starts.
    pub node_id: Option<String>,
    /// SHA-256 fingerprint (lower-case hex) of the certificate the user trusted for this
    /// server. `None` = only certificates Windows trusts.
    pub trusted_certificate: Option<String>,
}

impl Default for HomeAssistant {
    fn default() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: MQTT_PORT_DEFAULT,
            tls: false,
            username: String::new(),
            password: None,
            pc_name: None,
            discovery_prefix: DISCOVERY_PREFIX_DEFAULT.to_owned(),
            node_id: None,
            trusted_certificate: None,
        }
    }
}

impl HomeAssistant {
    /// Whether the connection can be attempted (an address is set).
    pub fn configured(&self) -> bool {
        !self.host.trim().is_empty()
    }
}

/// Whether `prefix` can be an MQTT topic prefix: not empty, no wildcard, no empty level.
pub fn valid_discovery_prefix(prefix: &str) -> bool {
    !prefix.is_empty()
        && !prefix.contains(['+', '#', '\0'])
        && prefix.split('/').all(|level| !level.is_empty())
}

/// Whether `id` is a valid node identifier (lower-case ASCII letters and digits).
pub fn valid_node_id(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Where the settings window was last left, in physical screen pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowPlacement {
    /// Left edge of the restored (not maximised) window.
    pub x: i32,
    /// Top edge of the restored window.
    pub y: i32,
    /// Width of the restored window.
    pub width: i32,
    /// Height of the restored window.
    pub height: i32,
    /// The window was maximised.
    pub maximized: bool,
}

/// Updates (FR-132). The automatic check itself (FR-080) is
/// [`General::check_updates`], kept in the `[general]` table of the file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Updates {
    /// Previews are offered too (off by default).
    pub previews: bool,
    /// When the release service last answered (Unix time, seconds): automatic checks are
    /// a day apart (FR-132).
    pub last_check: Option<u64>,
    /// The version whose flyout line was closed.
    pub dismissed: Option<String>,
    /// The version that ran last: a newer one starting says it was updated.
    pub last_version: Option<String>,
}

/// All user settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Schema version these settings conform to.
    pub version: u32,
    /// Section 5.1.
    pub general: General,
    /// Section 5.2.
    pub devices: DeviceLists,
    /// Section 5.6: every application remembered, in the user's order.
    pub apps: AppList,
    /// Section 5.3.
    pub home_assistant: HomeAssistant,
    /// Last place of the settings window; `None` = centred, default size.
    pub settings_window: Option<WindowPlacement>,
    /// Section 11 (updates).
    pub updates: Updates,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            general: General::default(),
            devices: DeviceLists::default(),
            apps: AppList::default(),
            home_assistant: HomeAssistant::default(),
            settings_window: None,
            updates: Updates::default(),
        }
    }
}

impl Settings {
    /// Brings every value back into its valid range, so the rest of the application never
    /// has to defend against a hand-edited or damaged file. Returns `true` if anything
    /// changed.
    pub fn normalise(&mut self) -> bool {
        let before = self.clone();
        self.general.wheel_step = self.general.wheel_step.clamp(WHEEL_STEP_MIN, WHEEL_STEP_MAX);
        if let Some(tag) = &self.general.language {
            let trimmed = tag.trim();
            self.general.language =
                if trimmed.is_empty() { None } else { Some(trimmed.to_ascii_lowercase()) };
        }
        // A place no screen can hold (damaged or hand-edited file) is forgotten; the
        // bounds also keep every edge computation far from overflowing.
        let sane = |w: &WindowPlacement| {
            (1..=PLACE_LIMIT).contains(&w.width)
                && (1..=PLACE_LIMIT).contains(&w.height)
                && (-PLACE_LIMIT..=PLACE_LIMIT).contains(&w.x)
                && (-PLACE_LIMIT..=PLACE_LIMIT).contains(&w.y)
        };
        if self.settings_window.is_some_and(|w| !sane(&w)) {
            self.settings_window = None;
        }
        let ha = &mut self.home_assistant;
        let clean = |text: &mut String| {
            let trimmed: String = text.trim().chars().take(HA_TEXT_MAX).collect();
            if trimmed != *text {
                *text = trimmed;
            }
        };
        clean(&mut ha.host);
        clean(&mut ha.username);
        clean(&mut ha.discovery_prefix);
        if ha.port == 0 {
            ha.port = MQTT_PORT_DEFAULT;
        }
        if !valid_discovery_prefix(&ha.discovery_prefix) {
            ha.discovery_prefix = DISCOVERY_PREFIX_DEFAULT.to_owned();
        }
        if let Some(name) = ha.pc_name.as_mut() {
            clean(name);
        }
        if ha.pc_name.as_deref().is_some_and(str::is_empty) {
            ha.pc_name = None;
        }
        if ha.password.as_deref().is_some_and(str::is_empty) {
            ha.password = None;
        }
        if ha.node_id.as_deref().is_some_and(|id| !valid_node_id(id)) {
            ha.node_id = None;
        }
        let fingerprint = |f: &str| f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit());
        if ha.trusted_certificate.as_deref().is_some_and(|f| !fingerprint(f)) {
            ha.trusted_certificate = None;
        }
        if let Some(f) = ha.trusted_certificate.as_mut() {
            f.make_ascii_lowercase();
        }
        let updates = &mut self.updates;
        for version in [&mut updates.dismissed, &mut updates.last_version] {
            if version.as_deref().is_some_and(|v| crate::package::Version::parse(v).is_none()) {
                *version = None;
            }
        }
        for list in [&mut self.devices.outputs, &mut self.devices.inputs] {
            // A damaged file may repeat a device: its first entry wins (as when shown).
            let mut seen: Vec<String> = Vec::with_capacity(list.len());
            list.retain(|k| {
                let first = !k.id.is_empty() && !seen.contains(&k.id);
                seen.push(k.id.clone());
                first
            });
            for entry in list.iter_mut() {
                if entry.custom_name.as_deref().is_some_and(|n| n.trim().is_empty()) {
                    entry.custom_name = None;
                }
                if entry.icon.as_deref().is_some_and(|i| i.trim().is_empty()) {
                    entry.icon = None;
                }
            }
        }
        self.apps.normalise();
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_specification() {
        let settings = Settings::default();
        assert_eq!(settings.version, SCHEMA_VERSION);
        let general = settings.general;
        assert_eq!(general.switch_scope, SwitchScope::MediaAndCalls);
        assert_eq!(general.wheel_step, 2);
        assert_eq!(general.language, None);
        assert!(general.microphone_enabled);
        assert!(general.switch_notification);
        assert!(general.volume_bubble);
        assert_eq!(general.tray_icon_style, TrayIconStyle::SameAsWindows);
        assert_eq!(general.tray_icon_size, TrayIconSize::SameAsWindows);
        assert_eq!(general.tray_icon_look, TrayIconLook::Windows);
        assert_eq!(general.flyout_height, FlyoutHeight::Content);
        assert!(!general.hide_windows_volume_icon);
        assert!(general.check_updates);
        assert!(general.apps_section_expanded);
        assert!(!general.first_run_done);
        assert!(!general.volume_icon_question_done);
        assert!(settings.devices.outputs.is_empty() && settings.devices.inputs.is_empty());
    }

    #[test]
    fn device_lists_are_reached_by_flow() {
        let mut lists = DeviceLists::default();
        lists.list_mut(Flow::Output).push(KnownDevice::new("out"));
        lists.list_mut(Flow::Input).push(KnownDevice::new("in"));
        assert_eq!(lists.list(Flow::Output), [KnownDevice::new("out")]);
        assert_eq!(lists.list(Flow::Input), [KnownDevice::new("in")]);
        assert_eq!(lists.outputs.len() + lists.inputs.len(), 2);
    }

    #[test]
    fn defaults_are_already_normalised() {
        let mut settings = Settings::default();
        assert!(!settings.normalise());
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn wheel_step_is_clamped() {
        for (input, expected) in [(0, 1), (1, 1), (2, 2), (10, 10), (11, 10), (255, 10)] {
            let mut settings = Settings::default();
            settings.general.wheel_step = input;
            assert_eq!(settings.normalise(), input != expected, "input {input}");
            assert_eq!(settings.general.wheel_step, expected, "input {input}");
        }
    }

    #[test]
    fn language_is_cleaned() {
        let mut settings = Settings::default();
        settings.general.language = Some("  ".into());
        assert!(settings.normalise());
        assert_eq!(settings.general.language, None);

        settings.general.language = Some(" FR ".into());
        assert!(settings.normalise());
        assert_eq!(settings.general.language.as_deref(), Some("fr"));
        assert!(!settings.normalise());
    }

    #[test]
    fn damaged_device_lists_and_placement_are_repaired() {
        let mut settings = Settings::default();
        settings.devices.outputs = vec![
            KnownDevice {
                custom_name: Some(" ".into()),
                icon: Some("".into()),
                ..KnownDevice::new("a")
            },
            KnownDevice { hidden: true, ..KnownDevice::new("a") },
            KnownDevice::new(""),
            KnownDevice::new("b"),
        ];
        settings.settings_window =
            Some(WindowPlacement { width: 0, height: 10, ..Default::default() });
        assert!(settings.normalise());
        assert_eq!(settings.devices.outputs, [KnownDevice::new("a"), KnownDevice::new("b")]);
        assert_eq!(settings.settings_window, None);
        assert!(!settings.normalise());
        // Edges far beyond any screen (hand-edited file) are forgotten too.
        let far =
            WindowPlacement { x: i32::MAX - 5, y: 0, width: 800, height: 600, maximized: false };
        let fine = WindowPlacement { x: -3000, y: 40, width: 800, height: 600, maximized: true };
        let lowest = WindowPlacement { x: i32::MIN, y: i32::MIN, ..fine };
        for place in [far, lowest] {
            settings.settings_window = Some(place);
            assert!(settings.normalise(), "{place:?}");
            assert_eq!(settings.settings_window, None);
        }
        settings.settings_window = Some(fine);
        assert!(!settings.normalise(), "a place on a monitor at the left is kept");
    }

    #[test]
    fn home_assistant_is_off_by_default_and_repaired() {
        let ha = HomeAssistant::default();
        assert!(!ha.enabled, "FR-118: off by default");
        assert_eq!((ha.port, ha.discovery_prefix.as_str()), (1883, "homeassistant"));
        assert!(!ha.configured());

        let mut settings = Settings::default();
        let ha = &mut settings.home_assistant;
        ha.host = "  broker.lan ".into();
        ha.port = 0;
        ha.discovery_prefix = "home/#".into();
        ha.pc_name = Some("   ".into());
        ha.password = Some(String::new());
        ha.node_id = Some("Not-Valid".into());
        ha.trusted_certificate = Some("AB".repeat(32));
        assert!(settings.normalise());
        let ha = &settings.home_assistant;
        assert_eq!(ha.host, "broker.lan");
        assert_eq!(ha.port, 1883);
        assert_eq!(ha.discovery_prefix, "homeassistant");
        assert_eq!((ha.pc_name.as_deref(), ha.password.as_deref()), (None, None));
        assert_eq!(ha.node_id, None);
        assert_eq!(ha.trusted_certificate.as_deref(), Some("ab".repeat(32).as_str()));
        assert!(!settings.normalise());
        settings.home_assistant.trusted_certificate = Some("xyz".into());
        assert!(settings.normalise());
        assert_eq!(settings.home_assistant.trusted_certificate, None);
    }

    #[test]
    fn discovery_prefixes_and_node_ids_are_checked() {
        for good in ["homeassistant", "ha/discovery", "a"] {
            assert!(valid_discovery_prefix(good), "{good}");
        }
        for bad in ["", "/ha", "ha/", "a//b", "ha/+", "ha/#"] {
            assert!(!valid_discovery_prefix(bad), "{bad}");
        }
        assert!(valid_node_id("a1b2c3d4e5f60718"));
        for bad in ["", "ABC", "a_b", &"a".repeat(33)] {
            assert!(!valid_node_id(bad), "{bad}");
        }
    }

    #[test]
    fn versions_are_classified() {
        assert_eq!(classify_version(SCHEMA_VERSION), VersionStatus::Current);
        assert_eq!(classify_version(0), VersionStatus::Unsupported);
        assert_eq!(classify_version(SCHEMA_VERSION + 1), VersionStatus::Unsupported);
        assert_eq!(classify_version(u32::MAX), VersionStatus::Unsupported);
        // Every older version (none exists while the schema is at version 1) is migrated.
        let older_versions = 1..SCHEMA_VERSION;
        for older in older_versions {
            assert_eq!(classify_version(older), VersionStatus::NeedsMigration { from: older });
        }
    }
}
