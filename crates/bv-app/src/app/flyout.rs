//! What the flyout shows and what its actions do (FR-030 to FR-065).
//!
//! The application builds a [`FlyoutContent`] from the engine and the settings (all texts
//! translated), and applies the [`FlyoutAction`]s the flyout reports. Devices and
//! applications travel as opaque [`ItemKey`]s: a hash of their identity, looked up again
//! when an action comes back.

use std::hash::{Hash, Hasher};

use bv_audio::{AppKey, AudioEngine, DeviceInfo, Flow, Role, SwitchScope};
use bv_core::apps;
use bv_core::audio::{Bus, FormFactor};
use bv_core::devices;
use bv_core::i18n::Msg;
use bv_core::settings::FlyoutHeight;
use bv_ui_kit::controls::device_icon::DeviceIcon;
use bv_ui_kit::controls::flyout::{
    AppsSection, DeviceTile, Devices, FlyoutAction, FlyoutContent, FlyoutTab, Footer, ItemKey,
    PanelHeight, SliderTarget, Tabs, VolumeControl, VolumeIcon,
};

use super::{applied, volume_level, App};

/// What the flyout is showing, as far as the application is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FlyoutSession {
    /// The flyout is open.
    pub open: bool,
    /// The view shown (always Output at opening).
    pub tab: FlyoutTab,
    /// The application sessions are being watched.
    pub watching: bool,
}

impl Default for FlyoutSession {
    fn default() -> Self {
        Self { open: false, tab: FlyoutTab::Output, watching: false }
    }
}

/// What the caller must do after a flyout action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutEffect {
    /// Show the new content (and the new tray state).
    Refresh,
    /// Open the Windows sound settings.
    OpenSoundSettings,
    /// Open the BetterVolume settings.
    OpenSettings,
}

/// The key of a device or an application.
fn key_of(value: impl Hash) -> ItemKey {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    ItemKey(hasher.finish())
}

fn device_key(id: &str) -> ItemKey {
    key_of(("device", id))
}

fn app_key(app: &AppKey) -> ItemKey {
    key_of(("app", app))
}

/// The icon the user chose for a device, if any (an unknown identifier counts as none).
pub(crate) fn chosen_icon(known: &[bv_core::devices::KnownDevice], id: &str) -> Option<DeviceIcon> {
    known.iter().find(|k| k.id == id).and_then(|k| k.icon.as_deref()).and_then(DeviceIcon::from_id)
}

/// The icon chosen automatically for a device (FR-067): from the type Windows declares
/// and, for headphones, the connection.
pub(crate) fn device_icon(flow: Flow, info: &DeviceInfo) -> DeviceIcon {
    let bluetooth = info.bus == Bus::Bluetooth;
    match (flow, info.form_factor) {
        // A Bluetooth speaker is almost always a portable one.
        (Flow::Output, FormFactor::Speakers) if bluetooth => DeviceIcon::PortableSpeaker,
        (_, FormFactor::Speakers) => DeviceIcon::DesktopSpeakers,
        (Flow::Output, FormFactor::Headphones | FormFactor::Headset) if bluetooth => {
            DeviceIcon::BluetoothHeadphones
        }
        (_, FormFactor::Headphones) => DeviceIcon::WiredHeadphones,
        (_, FormFactor::Headset | FormFactor::Handset) => DeviceIcon::Headset,
        (_, FormFactor::Display) => DeviceIcon::Monitor,
        (Flow::Output, FormFactor::Spdif | FormFactor::LineLevel) => DeviceIcon::AvReceiver,
        (Flow::Input, FormFactor::Spdif | FormFactor::LineLevel) => DeviceIcon::AudioInterface,
        // A USB microphone is most often a desk microphone; others (built-in, jack)
        // keep the generic one.
        (Flow::Input, FormFactor::Microphone) if info.bus == Bus::Usb => DeviceIcon::DeskMicrophone,
        (_, FormFactor::Remote) => DeviceIcon::VirtualDevice,
        (Flow::Output, _) => DeviceIcon::GenericOutput,
        (Flow::Input, _) => DeviceIcon::GenericInput,
    }
}

impl<E: AudioEngine> App<E> {
    fn flow(&self) -> Flow {
        match self.flyout.tab {
            FlyoutTab::Output => Flow::Output,
            FlyoutTab::Input => Flow::Input,
        }
    }

    /// The flyout opens: always on the output view; the application list is
    /// watched if it shows.
    pub fn flyout_opening(&mut self) {
        self.flyout.open = true;
        self.flyout.tab = FlyoutTab::Output;
        self.update_session_watch();
    }

    /// Microphone management was turned on or off (FR-076): without it the flyout shows
    /// the output view only.
    pub(super) fn microphones_changed(&mut self) {
        if !self.settings.general.microphone_enabled {
            self.flyout.tab = FlyoutTab::Output;
        }
        self.update_session_watch();
    }

    /// The flyout closed: nothing is watched any more.
    pub fn flyout_closed(&mut self) {
        self.flyout.open = false;
        self.update_session_watch();
    }

    /// Whether the application believes the flyout is open.
    pub fn flyout_open(&self) -> bool {
        self.flyout.open
    }

    /// Sessions are watched only while the flyout shows the application list.
    fn update_session_watch(&mut self) {
        let wanted = self.flyout.open
            && self.flyout.tab == FlyoutTab::Output
            && self.settings.general.apps_section_expanded;
        if wanted != self.flyout.watching {
            self.engine.watch_sessions(wanted);
            self.flyout.watching = wanted;
        }
    }

    /// Everything the flyout shows now.
    pub fn flyout_content(&self) -> FlyoutContent {
        let text = |msg| self.language.text(msg).to_owned();
        let flow = self.flow();
        let tabs = self.settings.general.microphone_enabled.then(|| Tabs {
            name: text(Msg::FlyoutTabs),
            selected: self.flyout.tab,
            output: text(Msg::FlyoutTabOutput),
            input: text(Msg::FlyoutTabInput),
        });
        let (devices_name, empty) = match flow {
            Flow::Output => (Msg::FlyoutOutputDevices, Msg::FlyoutNoOutput),
            Flow::Input => (Msg::FlyoutInputDevices, Msg::FlyoutNoInput),
        };
        let media = self.engine.default_device(flow, Role::Media);
        let calls = self.engine.default_device(flow, Role::Calls);
        let known = self.settings.devices.list(flow);
        let live = self.engine.devices(flow);
        let tiles = devices::visible(known, live, media)
            .into_iter()
            .map(|device| DeviceTile {
                key: device_key(&device.info.id),
                name: device.display_name,
                icon: chosen_icon(known, &device.info.id)
                    .unwrap_or_else(|| device_icon(flow, device.info)),
                current: device.current,
                // FR-046: the device used for calls when it is not the media one.
                calls: (calls == Some(device.info.id.as_str()) && calls != media)
                    .then(|| text(Msg::FlyoutCalls)),
            })
            .collect();
        let apps = (flow == Flow::Output).then(|| self.apps_section());
        FlyoutContent {
            name: text(Msg::AppName),
            tabs,
            devices: Devices { name: text(devices_name), tiles, empty: text(empty) },
            apps,
            main: self.main_control(flow),
            footer: Footer {
                sound_settings: text(Msg::FlyoutSoundSettings),
                settings: text(Msg::MenuSettings),
                settings_enabled: true,
            },
            update: self.update_line(),
            wheel_step: self.settings.general.wheel_step,
            height: match self.settings.general.flyout_height {
                FlyoutHeight::Full => PanelHeight::Full,
                FlyoutHeight::Content => PanelHeight::Content,
            },
        }
    }

    fn named_control(
        &self,
        target: SliderTarget,
        label: String,
        icon: VolumeIcon,
        volume: u8,
        muted: bool,
    ) -> VolumeControl {
        let mute = if muted { Msg::FlyoutUnmute } else { Msg::FlyoutMute };
        VolumeControl {
            target,
            mute_name: self.language.format(Msg::FlyoutMuteName, &[("name", &label)]),
            slider_name: self.language.format(Msg::FlyoutVolumeName, &[("name", &label)]),
            mute_tooltip: self.language.text(mute).to_owned(),
            label,
            icon,
            icon_source: None,
            volume,
            muted,
            enabled: true,
        }
    }

    /// The row at the bottom: master volume or microphone level (FR-050, FR-047).
    fn main_control(&self, flow: Flow) -> VolumeControl {
        let current = self.engine.default_device(flow, Role::Media);
        let info = current.and_then(|id| self.engine.devices(flow).iter().find(|d| d.id == id));
        let endpoint = self.engine.endpoint(flow);
        let (volume, muted) = endpoint.map_or((0, false), |e| (e.percent(), e.muted));
        let label = match info {
            Some(info) => devices::display_name(
                self.settings.devices.list(flow),
                self.engine.devices(flow),
                info,
            ),
            None => self
                .language
                .text(match flow {
                    Flow::Output => Msg::FlyoutNoOutput,
                    Flow::Input => Msg::FlyoutNoInput,
                })
                .to_owned(),
        };
        let icon = match flow {
            Flow::Output => VolumeIcon::Level(volume_level(volume, muted)),
            Flow::Input => VolumeIcon::Microphone,
        };
        let mut control = self.named_control(SliderTarget::Main, label, icon, volume, muted);
        // No device, or a device without volume control: nothing to adjust.
        control.enabled = endpoint.is_some();
        control
    }

    /// The application list (FR-060 to FR-065).
    fn apps_section(&self) -> AppsSection {
        let expanded = self.settings.general.apps_section_expanded;
        let rows = if expanded {
            apps::rows(self.engine.sessions())
                .into_iter()
                .map(|row| {
                    let system = row.is_system();
                    let label = if system {
                        self.language.text(Msg::FlyoutSystemSounds).to_owned()
                    } else {
                        row.name.to_owned()
                    };
                    let icon =
                        if system { VolumeIcon::SystemSounds } else { VolumeIcon::App(None) };
                    let target = SliderTarget::App(app_key(row.app));
                    let mut control =
                        self.named_control(target, label, icon, row.volume, row.muted);
                    control.icon_source = row.icon.map(str::to_owned);
                    control
                })
                .collect()
        } else {
            Vec::new()
        };
        AppsSection { title: self.language.text(Msg::FlyoutApps).to_owned(), expanded, rows }
    }

    /// Applies what the user asked for in the flyout.
    pub fn flyout_action(&mut self, action: FlyoutAction) -> FlyoutEffect {
        let flow = self.flow();
        match action {
            FlyoutAction::SelectTab(tab) => {
                self.flyout.tab = tab;
                self.update_session_watch();
            }
            FlyoutAction::SelectDevice(key) => self.select_device(flow, key),
            FlyoutAction::SetVolume(SliderTarget::Main, percent) => {
                // Unmutes, like Windows (FR-052).
                applied(self.engine.set_volume(flow, percent), "flyout volume");
            }
            FlyoutAction::SetVolume(SliderTarget::App(key), percent) => {
                if let Some(app) = self.app_by_key(key) {
                    applied(self.engine.set_app_volume(&app, percent), "application volume");
                }
            }
            FlyoutAction::ToggleMute(SliderTarget::Main) => {
                if let Some(muted) = self.engine.endpoint(flow).map(|e| e.muted) {
                    applied(self.engine.set_mute(flow, !muted), "flyout mute");
                }
            }
            FlyoutAction::ToggleMute(SliderTarget::App(key)) => {
                let row = apps::rows(self.engine.sessions())
                    .into_iter()
                    .find(|r| app_key(r.app) == key)
                    .map(|r| (r.app.clone(), r.muted));
                if let Some((app, muted)) = row {
                    applied(self.engine.set_app_mute(&app, !muted), "application mute");
                }
            }
            FlyoutAction::SetAppsExpanded(expanded) => {
                self.settings.general.apps_section_expanded = expanded;
                self.dirty = true;
                self.update_session_watch();
            }
            FlyoutAction::OpenSoundSettings => return FlyoutEffect::OpenSoundSettings,
            FlyoutAction::OpenSettings => return FlyoutEffect::OpenSettings,
            FlyoutAction::InstallUpdate => self.update_install(),
            FlyoutAction::DismissUpdate => self.update_dismiss(),
        }
        FlyoutEffect::Refresh
    }

    fn app_by_key(&self, key: ItemKey) -> Option<AppKey> {
        self.engine.sessions().iter().map(|s| &s.app).find(|a| app_key(a) == key).cloned()
    }

    /// A tile was clicked (FR-041).
    fn select_device(&mut self, flow: Flow, key: ItemKey) {
        let Some(id) =
            self.engine.devices(flow).iter().map(|d| d.id.clone()).find(|id| device_key(id) == key)
        else {
            return;
        };
        self.switch_device(flow, &id);
    }

    /// Makes `id` the current device of `flow` with the configured scope (FR-072); on the
    /// current device, brings calls back to it under "media and calls". Used by
    /// the tiles and by Home Assistant.
    pub(super) fn switch_device(&mut self, flow: Flow, id: &str) {
        let id = id.to_owned();
        let scope = self.settings.general.switch_scope;
        let media = self.engine.default_device(flow, Role::Media).map(str::to_owned);
        let calls = self.engine.default_device(flow, Role::Calls).map(str::to_owned);
        if media.as_deref() == Some(id.as_str()) {
            // The highlighted tile: with "media and calls", brings calls back to it.
            if scope == SwitchScope::MediaAndCalls && calls.as_deref() != Some(id.as_str()) {
                applied(
                    self.engine.set_default_device(flow, &id, SwitchScope::CallsOnly),
                    "calls device",
                );
            }
            return;
        }
        applied(self.engine.set_default_device(flow, &id, scope), "device switch");
    }
}

#[cfg(test)]
mod tests {
    use bv_audio::fake::FakeEngine;
    use bv_audio::{DeviceState, Stream};
    use bv_core::i18n::Language;
    use bv_core::settings::Settings;

    use super::*;

    fn device(id: &str, name: &str, form_factor: FormFactor) -> DeviceInfo {
        DeviceInfo { id: id.into(), windows_name: name.into(), form_factor, ..Default::default() }
    }

    fn stream(app: AppKey, name: &str, volume: f32, muted: bool) -> Stream {
        Stream {
            app,
            process: 1,
            name: name.into(),
            icon: Some(r"C:\a.exe".into()),
            volume,
            muted,
            heard_here: true,
        }
    }

    fn engine() -> FakeEngine {
        let mut engine = FakeEngine::new()
            .with_device(Flow::Output, device("spk", "Speakers", FormFactor::Speakers))
            .with_device(Flow::Output, device("hp", "Headphones", FormFactor::Headphones))
            .with_device(Flow::Input, device("mic", "Microphone", FormFactor::Microphone));
        engine.levels.insert("spk".into(), (0.4, false));
        engine.streams = vec![
            stream(AppKey::executable(r"C:\Player.exe"), "Player", 0.5, false),
            stream(AppKey::System, "", 1.0, false),
        ];
        engine.sync();
        engine
    }

    fn app(engine: FakeEngine) -> App<FakeEngine> {
        App::new(engine, Settings::default(), Language::ENGLISH)
    }

    fn tile<'a>(content: &'a FlyoutContent, name: &str) -> &'a DeviceTile {
        content.devices.tiles.iter().find(|t| t.name == name).expect("tile")
    }

    #[test]
    fn the_output_view_shows_tiles_apps_and_master() {
        let mut app = app(engine());
        app.flyout_opening();
        assert!(app.engine.watching_sessions, "the list shows: sessions are watched");
        let content = app.flyout_content();
        assert_eq!(content.tabs.as_ref().map(|t| t.selected), Some(FlyoutTab::Output));
        assert!(tile(&content, "Speakers").current);
        assert!(!tile(&content, "Headphones").current);
        assert_eq!(tile(&content, "Speakers").icon, DeviceIcon::DesktopSpeakers);
        let rows = &content.apps.as_ref().unwrap().rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "System sounds", "System sounds first (FR-064)");
        assert_eq!(rows[1].label, "Player");
        assert_eq!(rows[1].icon_source.as_deref(), Some(r"C:\a.exe"));
        assert_eq!(content.main.label, "Speakers");
        assert_eq!(content.main.volume, 40);
        assert_eq!(content.main.slider_name, "Volume: Speakers");
        app.flyout_closed();
        assert!(!app.engine.watching_sessions, "closed: nothing watched");
    }

    #[test]
    fn a_tile_click_switches_the_device_and_the_flyout_follows() {
        let mut app = app(engine());
        app.flyout_opening();
        let key = tile(&app.flyout_content(), "Headphones").key;
        assert_eq!(app.flyout_action(FlyoutAction::SelectDevice(key)), FlyoutEffect::Refresh);
        assert_eq!(app.engine.default_device(Flow::Output, Role::Media), Some("hp"));
        assert_eq!(app.engine.default_device(Flow::Output, Role::Calls), Some("hp"), "scope: both");
        let content = app.flyout_content();
        assert!(tile(&content, "Headphones").current);
        assert_eq!(content.main.label, "Headphones", "FR-053");
    }

    #[test]
    fn the_calls_device_is_marked_and_a_click_on_the_highlight_reunites_them() {
        let mut engine = engine();
        engine.defaults[Flow::Output.index()][Role::Calls.index()] = Some("hp".into());
        let mut app = app(engine);
        app.flyout_opening();
        let content = app.flyout_content();
        assert_eq!(tile(&content, "Headphones").calls.as_deref(), Some("Used for calls"));
        assert_eq!(tile(&content, "Speakers").calls, None);
        let key = tile(&content, "Speakers").key;
        app.flyout_action(FlyoutAction::SelectDevice(key));
        assert_eq!(
            app.engine.default_device(Flow::Output, Role::Calls),
            Some("spk"),
            "the calls default follows a tile click"
        );
        assert_eq!(tile(&app.flyout_content(), "Headphones").calls, None);
    }

    #[test]
    fn with_another_scope_the_highlighted_tile_does_nothing() {
        let mut engine = engine();
        engine.defaults[Flow::Output.index()][Role::Calls.index()] = Some("hp".into());
        let mut settings = Settings::default();
        settings.general.switch_scope = SwitchScope::MediaOnly;
        let mut app = App::new(engine, settings, Language::ENGLISH);
        let key = tile(&app.flyout_content(), "Speakers").key;
        app.flyout_action(FlyoutAction::SelectDevice(key));
        assert_eq!(app.engine.default_device(Flow::Output, Role::Calls), Some("hp"));
    }

    #[test]
    fn volumes_and_mutes_reach_the_engine() {
        let mut app = app(engine());
        app.flyout_opening();
        app.flyout_action(FlyoutAction::SetVolume(SliderTarget::Main, 70));
        assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 70);
        app.flyout_action(FlyoutAction::ToggleMute(SliderTarget::Main));
        assert!(app.engine.endpoint(Flow::Output).unwrap().muted);
        let player = app.flyout_content().apps.unwrap().rows[1].target;
        app.flyout_action(FlyoutAction::SetVolume(player, 20));
        app.flyout_action(FlyoutAction::ToggleMute(player));
        let row = &app.flyout_content().apps.unwrap().rows[1];
        assert_eq!((row.volume, row.muted), (20, true));
        assert_eq!(row.mute_tooltip, "Unmute");
    }

    #[test]
    fn collapsing_the_list_stops_watching_sessions() {
        let mut app = app(engine());
        app.flyout_opening();
        app.flyout_action(FlyoutAction::SetAppsExpanded(false));
        assert!(!app.engine.watching_sessions);
        assert!(app.flyout_content().apps.unwrap().rows.is_empty());
        app.flyout_action(FlyoutAction::SetAppsExpanded(true));
        assert!(app.engine.watching_sessions);
        // The microphone view has no list either.
        app.flyout_action(FlyoutAction::SelectTab(FlyoutTab::Input));
        assert!(!app.engine.watching_sessions);
        let content = app.flyout_content();
        assert!(content.apps.is_none());
        assert_eq!(content.main.label, "Microphone");
        assert_eq!(content.main.icon, VolumeIcon::Microphone);
        // Reopening starts on the output view.
        app.flyout_closed();
        app.flyout_opening();
        assert_eq!(app.flyout_content().tabs.unwrap().selected, FlyoutTab::Output);
    }

    #[test]
    fn without_microphone_management_there_are_no_tabs() {
        let mut settings = Settings::default();
        settings.general.microphone_enabled = false;
        let app = App::new(engine(), settings, Language::ENGLISH);
        assert!(app.flyout_content().tabs.is_none());
    }

    #[test]
    fn without_device_the_controls_are_disabled() {
        let app = app(FakeEngine::new());
        let content = app.flyout_content();
        assert!(content.devices.tiles.is_empty());
        assert_eq!(content.devices.empty, "No output device");
        assert!(!content.main.enabled);
    }

    #[test]
    fn unavailable_devices_have_no_tile() {
        let mut engine = engine();
        engine.devices[0][1].state = DeviceState::Unplugged;
        let app = app(engine);
        assert_eq!(app.flyout_content().devices.tiles.len(), 1);
    }

    #[test]
    fn automatic_icons_follow_the_device_type() {
        let mut info = device("x", "x", FormFactor::Headphones);
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::WiredHeadphones);
        info.bus = Bus::Bluetooth;
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::BluetoothHeadphones);
        info.form_factor = FormFactor::Headset;
        assert_eq!(device_icon(Flow::Input, &info), DeviceIcon::Headset);
        info.form_factor = FormFactor::Unknown;
        assert_eq!(device_icon(Flow::Input, &info), DeviceIcon::GenericInput);
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::GenericOutput);
        info.form_factor = FormFactor::Microphone;
        assert_eq!(device_icon(Flow::Input, &info), DeviceIcon::GenericInput);
        info.bus = Bus::Usb;
        assert_eq!(device_icon(Flow::Input, &info), DeviceIcon::DeskMicrophone);
        info.form_factor = FormFactor::Display;
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::Monitor);
        // Speakers: portable over Bluetooth, desktop otherwise.
        info.form_factor = FormFactor::Speakers;
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::DesktopSpeakers);
        info.bus = Bus::Bluetooth;
        assert_eq!(device_icon(Flow::Output, &info), DeviceIcon::PortableSpeaker);
    }
}
