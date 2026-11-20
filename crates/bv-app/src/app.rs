//! Application state and behaviour, independent of the Windows shell so it can be tested
//! with the in-memory audio engine.
//!
//! The tray icon and tooltip follow the current output live (FR-010 to FR-012), a middle
//! click toggles its mute state (FR-016), the wheel over the icon changes its volume
//! (FR-015); the flyout (`flyout.rs`) and the settings window (`settings_window.rs`) are
//! described and driven from here. Every change to the settings marks them for saving
//! (`take_dirty`, FR-125). The bubble above the taskbar (`bubble.rs`) announces device
//! switches and shows the volume after a wheel turn or a middle click.

use bv_audio::{AudioEngine, AudioError, Changes, Endpoint, Flow, Role};
use bv_core::audio::FormFactor;
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{Settings, TrayIconLook, TrayIconSize, TrayIconStyle, WindowPlacement};
use bv_core::text::ellipsize;
use bv_core::{devices, volume};
use bv_ui_kit::controls::tray::{DeviceType, TrayIcon, TrayLook};
use bv_ui_kit::icons::VolumeLevel;

use bv_shell::TOOLTIP_MAX_UNITS;

/// Room kept for the volume part of the tooltip (": 100 %", ": muted", in any language),
/// so that a long device name is shortened rather than the volume cut off.
const TOOLTIP_VOLUME_RESERVE: usize = 24;

mod apps_page;
mod bubble;
mod flyout;
pub mod ha;
mod ha_page;
mod settings_window;
mod update;
mod volume_icon;
// Language sheets (images for a visual check of the translations, run on demand).
#[cfg(test)]
mod sheets;
// Documentation images (rendered layers for the README, run on demand).
#[cfg(test)]
mod readme;
// Microsoft Store images (rendered layers for the listing, run on demand).
#[cfg(test)]
mod store;

pub use apps_page::ProcessList;
pub use flyout::FlyoutEffect;
pub use settings_window::SettingsEffect;
pub use update::{Download, UpdateEffect, UpdatePlatform};

/// The running application.
pub struct App<E: AudioEngine> {
    engine: E,
    settings: Settings,
    language: Language,
    /// Windows display languages in order of preference (automatic language, FR-121).
    os_languages: Vec<String>,
    flyout: flyout::FlyoutSession,
    settings_window: settings_window::SettingsSession,
    /// The Home Assistant link (section 8).
    ha: ha::HaSession,
    /// The default devices last seen (device-switch bubble, FR-105).
    defaults: bubble::Defaults,
    /// Hiding the Windows volume icon (FR-018).
    volume_icon: volume_icon::VolumeIconSession,
    /// Updates (FR-132).
    update: update::UpdateSession,
    /// The settings changed since they were last saved.
    dirty: bool,
}

impl<E: AudioEngine> App<E> {
    /// Starts from the engine's current state.
    pub fn new(engine: E, settings: Settings, language: Language) -> Self {
        let mut app = Self {
            engine,
            settings,
            language,
            os_languages: Vec::new(),
            flyout: flyout::FlyoutSession::default(),
            settings_window: settings_window::SettingsSession::default(),
            ha: ha::HaSession::default(),
            defaults: Default::default(),
            volume_icon: Default::default(),
            update: Default::default(),
            dirty: false,
        };
        app.learn_devices();
        app.learn_apps();
        // The devices current at start are not announced.
        app.defaults = bubble::read_defaults(&app.engine);
        app
    }

    /// The settings as they are now.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Whether the settings changed since the last call (they must then be saved).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// The language of the interface.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The Windows display languages, in order of preference; the language is resolved
    /// again from them.
    pub fn set_os_languages(&mut self, languages: Vec<String>) {
        self.os_languages = languages;
        self.language =
            Language::resolve(self.settings.general.language.as_deref(), &self.os_languages);
    }

    /// Whether the settings window must open by itself (very first launch, FR-071).
    pub fn first_run_pending(&self) -> bool {
        !self.settings.general.first_run_done
    }

    /// The settings window was shown on first launch: never again by itself (FR-071).
    pub fn first_run_done(&mut self) {
        if !self.settings.general.first_run_done {
            self.settings.general.first_run_done = true;
            self.dirty = true;
        }
    }

    /// Remembers where the settings window was left.
    pub fn set_settings_place(&mut self, place: Option<WindowPlacement>) {
        if place.is_some() && self.settings.settings_window != place {
            self.settings.settings_window = place;
            self.dirty = true;
        }
    }

    /// The engine has news (its notify function fired): apply it.
    pub fn on_audio_signals(&mut self) -> Changes {
        let mut changes = self.engine.process_signals();
        if changes.devices {
            self.learn_devices();
        }
        // A newly remembered application shows on the Applications page.
        changes.sessions |= self.learn_apps();
        changes
    }

    /// The computer resumed from sleep: re-read everything (NFR-05).
    pub fn on_resumed(&mut self) -> Changes {
        let changes = self.engine.refresh();
        self.learn_devices();
        self.learn_apps();
        changes
    }

    /// Middle click on the tray icon: toggles the mute state of the current output
    /// (FR-016). Returns `true` if it changed. Nothing happens without an output.
    pub fn toggle_output_mute(&mut self) -> bool {
        let Some(muted) = self.output_endpoint().map(|e| e.muted) else {
            return false;
        };
        // A failure (device just unplugged) leaves things as they are; the engine
        // re-reads the devices on its next notification.
        applied(self.engine.set_mute(Flow::Output, !muted), "middle click mute")
    }

    /// Mouse wheel over the tray icon (FR-015): changes the output volume by the
    /// configured step per notch, and unmutes, like Windows. Returns `true` if anything
    /// changed. Nothing happens without an output.
    pub fn wheel(&mut self, notches: i32) -> bool {
        let Some(endpoint) = self.output_endpoint() else { return false };
        let (percent, muted) = (endpoint.percent(), endpoint.muted);
        let target = volume::step(percent, notches, self.settings.general.wheel_step);
        let mut changed = false;
        if target != percent {
            changed |= applied(self.engine.set_volume(Flow::Output, target), "wheel volume");
        }
        if muted {
            changed |= applied(self.engine.set_mute(Flow::Output, false), "wheel unmute");
        }
        changed
    }

    /// The volume of the current output, re-reading everything first if Windows has a
    /// current output but the engine holds no volume for it: binding failed while the
    /// device was settling (after a resume, a service restart or a device change). A user
    /// action is the occasion to recover (NFR-05).
    fn output_endpoint(&mut self) -> Option<Endpoint> {
        let current = self.engine.default_device(Flow::Output, Role::Media).is_some();
        if current && self.engine.endpoint(Flow::Output).is_none() {
            self.on_resumed();
        }
        self.engine.endpoint(Flow::Output).cloned()
    }

    /// What the tray icon shows (FR-010, FR-011, FR-083, FR-084): the volume step of the
    /// current output, with the picture "device" its type, the chosen size and look.
    pub fn tray_icon(&self) -> TrayIcon {
        let g = &self.settings.general;
        self.tray_icon_with(g.tray_icon_style, g.tray_icon_size, g.tray_icon_look)
    }

    /// What the tray icon would show with these choices (the pictures of the settings
    /// window show each choice with the two others).
    pub(crate) fn tray_icon_with(
        &self,
        picture: TrayIconStyle,
        size: TrayIconSize,
        look: TrayIconLook,
    ) -> TrayIcon {
        let large = size == TrayIconSize::Large;
        let look = match look {
            TrayIconLook::Windows => TrayLook::Windows,
            TrayIconLook::Colourful => TrayLook::Colourful,
        };
        let Some(current) = self.engine.default_device(Flow::Output, Role::Media) else {
            return TrayIcon { level: None, badge: None, large, look };
        };
        // A device without volume control shows the full speaker, like Windows.
        let level = match self.engine.endpoint(Flow::Output) {
            Some(e) => volume_level(e.percent(), e.muted),
            None => VolumeLevel::High,
        };
        let badge = match picture {
            TrayIconStyle::SameAsWindows => None,
            TrayIconStyle::WithDeviceBadge => {
                self.engine.devices(Flow::Output).iter().find(|d| d.id == current).and_then(|d| {
                    // A chosen icon gives its family's drawing (FR-088); otherwise the
                    // type Windows declares.
                    match flyout::chosen_icon(self.settings.devices.list(Flow::Output), &d.id) {
                        Some(icon) => badge_of(icon),
                        None => device_type(d.form_factor),
                    }
                })
            }
        };
        TrayIcon { level: Some(level), badge, large, look }
    }

    /// The tray tooltip (FR-012): `<display name>: <volume>%`, `<display name>: muted`, or
    /// "no audio device".
    pub fn tooltip(&self) -> String {
        let Some(current) = self.engine.default_device(Flow::Output, Role::Media) else {
            return self.language.text(Msg::TrayNoDevice).to_owned();
        };
        let Some(info) = self.engine.devices(Flow::Output).iter().find(|d| d.id == current) else {
            return self.language.text(Msg::TrayNoDevice).to_owned();
        };
        let known = self.settings.devices.list(Flow::Output);
        let name = devices::display_name(known, self.engine.devices(Flow::Output), info);
        let name = ellipsize(&name, TOOLTIP_MAX_UNITS - TOOLTIP_VOLUME_RESERVE);
        let tooltip = match self.engine.endpoint(Flow::Output) {
            Some(endpoint) if endpoint.muted => {
                self.language.format(Msg::TrayTooltipMuted, &[("device", &name)])
            }
            Some(endpoint) => self.language.format(
                Msg::TrayTooltip,
                &[("device", &name), ("volume", &endpoint.percent().to_string())],
            ),
            // The device has no volume control: its name alone.
            None => name.into_owned(),
        };
        ellipsize(&tooltip, TOOLTIP_MAX_UNITS).into_owned()
    }

    /// Adds never-seen available devices to the settings lists (FR-044).
    fn learn_devices(&mut self) {
        for flow in Flow::ALL {
            let live = self.engine.devices(flow);
            self.dirty |= devices::learn(self.settings.devices.list_mut(flow), live);
        }
    }
}

/// Whether a user action reached Windows; a failure goes to the log (FR-005): it is not
/// shown, the next notification re-reads the devices.
fn applied(result: Result<(), AudioError>, action: &str) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            bv_shell::log::error(&format!("{action} failed: {error}"));
            false
        }
    }
}

/// The step of the Windows 11 volume icon for a volume in whole percent (FR-010): muted,
/// 0 %, 1 to 33 %, 34 to 66 %, 67 to 100 %.
fn volume_level(percent: u8, muted: bool) -> VolumeLevel {
    match (muted, percent) {
        (true, _) => VolumeLevel::Muted,
        (false, 0) => VolumeLevel::Zero,
        (false, 1..=33) => VolumeLevel::Low,
        (false, 34..=66) => VolumeLevel::Medium,
        (false, _) => VolumeLevel::High,
    }
}

/// The tray badge drawing of a catalogue icon's family (speakers, headphones,
/// headset, display; others keep the Windows speaker).
fn badge_of(icon: bv_ui_kit::controls::device_icon::DeviceIcon) -> Option<DeviceType> {
    use bv_ui_kit::controls::device_icon::DeviceIcon as I;
    match icon {
        I::DesktopSpeakers
        | I::BookshelfSpeakers
        | I::Soundbar
        | I::AvReceiver
        | I::Subwoofer
        | I::PortableSpeaker => Some(DeviceType::Speakers),
        I::WiredHeadphones
        | I::BluetoothHeadphones
        | I::WiredEarbuds
        | I::WirelessEarbuds
        | I::BoneConduction => Some(DeviceType::Headphones),
        I::GamingHeadset | I::Headset => Some(DeviceType::Headset),
        I::Monitor | I::Tv | I::Projector | I::Laptop => Some(DeviceType::Display),
        I::Phone
        | I::VrHeadset
        | I::DeskMicrophone
        | I::StudioMicrophone
        | I::LavalierMicrophone
        | I::Webcam
        | I::AudioInterface
        | I::VirtualDevice
        | I::GenericOutput
        | I::GenericInput => None,
    }
}

/// The device type drawn by the "with device badge" tray icon style. Types the
/// drawing has no picture for keep the Windows speaker.
fn device_type(form_factor: FormFactor) -> Option<DeviceType> {
    match form_factor {
        FormFactor::Speakers => Some(DeviceType::Speakers),
        FormFactor::Headphones => Some(DeviceType::Headphones),
        FormFactor::Headset | FormFactor::Handset => Some(DeviceType::Headset),
        FormFactor::Display => Some(DeviceType::Display),
        FormFactor::Microphone
        | FormFactor::LineLevel
        | FormFactor::Spdif
        | FormFactor::Remote
        | FormFactor::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use bv_audio::fake::FakeEngine;
    use bv_audio::{DeviceInfo, DeviceState, SwitchScope};
    use bv_core::devices::KnownDevice;

    use super::*;

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo { id: id.into(), windows_name: name.into(), ..Default::default() }
    }

    /// French typography: `{a0}` marks a non-breaking space (U+00A0).
    fn nbsp(text: &str) -> String {
        text.replace("{a0}", &char::from(0xA0_u8).to_string())
    }

    fn known(id: &str, windows_name: &str) -> KnownDevice {
        KnownDevice { windows_name: windows_name.into(), ..KnownDevice::new(id) }
    }

    fn english(engine: FakeEngine) -> App<FakeEngine> {
        App::new(engine, Settings::default(), Language::ENGLISH)
    }

    fn french(engine: FakeEngine) -> App<FakeEngine> {
        App::new(engine, Settings::default(), Language::from_tag("fr").unwrap())
    }

    fn speakers() -> FakeEngine {
        let mut engine = FakeEngine::new().with_device(Flow::Output, device("spk", "Speakers"));
        engine.levels.insert("spk".into(), (0.4, false));
        engine.sync();
        engine
    }

    #[test]
    fn tooltip_shows_device_and_volume() {
        assert_eq!(english(speakers()).tooltip(), "Speakers: 40%");
        assert_eq!(french(speakers()).tooltip(), nbsp("Speakers{a0}: 40{a0}%"));
    }

    #[test]
    fn tooltip_shows_muted() {
        let mut engine = speakers();
        engine.levels.insert("spk".into(), (0.4, true));
        engine.sync();
        assert_eq!(english(engine).tooltip(), "Speakers: muted");
        let mut engine = speakers();
        engine.levels.insert("spk".into(), (0.4, true));
        engine.sync();
        assert_eq!(french(engine).tooltip(), nbsp("Speakers{a0}: muet"));
    }

    #[test]
    fn tooltip_without_device() {
        assert_eq!(english(FakeEngine::new()).tooltip(), "No audio device");
        assert_eq!(french(FakeEngine::new()).tooltip(), "Aucun périphérique audio");
    }

    #[test]
    fn tooltip_uses_the_custom_name() {
        let mut settings = Settings::default();
        settings
            .devices
            .outputs
            .push(KnownDevice { custom_name: Some("Desk".into()), ..KnownDevice::new("spk") });
        let app = App::new(speakers(), settings, Language::ENGLISH);
        assert_eq!(app.tooltip(), "Desk: 40%");
    }

    #[test]
    fn long_device_names_are_shortened_but_the_volume_stays() {
        let long = "Very long device name ".repeat(10);
        let engine = FakeEngine::new().with_device(Flow::Output, device("x", &long));
        let tooltip = english(engine).tooltip();
        assert!(tooltip.encode_utf16().count() <= TOOLTIP_MAX_UNITS, "{tooltip}");
        assert!(tooltip.ends_with("…: 50%"), "{tooltip}");
    }

    #[test]
    fn tooltip_follows_live_changes() {
        let mut app = english(speakers().with_device(Flow::Output, device("hp", "Headphones")));
        assert_eq!(app.tooltip(), "Speakers: 40%");

        // Another program switches the default device (FR-003, FR-053).
        app.engine.defaults[Flow::Output.index()] = [Some("hp".into()), Some("hp".into())];
        app.engine.levels.insert("hp".into(), (0.75, false));
        app.engine.pending = Changes { defaults: true, output: true, ..Changes::default() };
        let changes = app.on_audio_signals();
        assert!(changes.output);
        assert_eq!(app.tooltip(), "Headphones: 75%");

        // The device is unplugged: Windows has no output left.
        app.engine.devices[0].iter_mut().for_each(|d| d.state = DeviceState::Unplugged);
        app.engine.defaults[0] = [None, None];
        app.engine.pending = Changes::ALL;
        app.on_audio_signals();
        assert_eq!(app.tooltip(), "No audio device");
    }

    #[test]
    fn middle_click_toggles_mute() {
        let mut app = english(speakers());
        assert!(app.toggle_output_mute());
        assert!(app.engine.endpoint(Flow::Output).unwrap().muted);
        assert_eq!(app.tooltip(), "Speakers: muted");
        assert!(app.toggle_output_mute());
        assert!(!app.engine.endpoint(Flow::Output).unwrap().muted);
        assert_eq!(app.tooltip(), "Speakers: 40%");
    }

    #[test]
    fn middle_click_recovers_a_failed_volume_binding() {
        let mut app = english(speakers());
        app.engine.binding_failed[Flow::Output.index()] = true;
        app.engine.sync();
        assert_eq!(app.tooltip(), "Speakers", "no volume known: the name alone");
        assert!(app.toggle_output_mute(), "the click reads everything again, then mutes");
        assert_eq!(app.engine.refreshes, 1);
        assert_eq!(app.tooltip(), "Speakers: muted");
        // With a working binding, a click does not re-read anything.
        assert!(app.toggle_output_mute());
        assert_eq!(app.engine.refreshes, 1);
    }

    #[test]
    fn middle_click_without_output_or_on_failure_does_nothing() {
        let mut app = english(FakeEngine::new());
        assert!(!app.toggle_output_mute());

        let mut app = english(speakers());
        app.engine.fail_next = Some(bv_audio::AudioError::DeviceGone);
        assert!(!app.toggle_output_mute());
        assert!(!app.engine.endpoint(Flow::Output).unwrap().muted);
    }

    #[test]
    fn new_devices_are_learnt_at_start_and_when_they_appear() {
        let engine = speakers().with_device(
            Flow::Input,
            DeviceInfo { state: DeviceState::Unplugged, ..device("mic", "Mic") },
        );
        let mut app = english(engine);
        assert_eq!(app.settings.devices.outputs, [known("spk", "Speakers")]);
        assert!(app.take_dirty(), "learnt devices are saved");
        assert!(app.settings.devices.inputs.is_empty(), "unavailable devices are not learnt");

        app.engine.devices[Flow::Input.index()][0].state = DeviceState::Active;
        app.engine.pending = Changes { devices: true, ..Changes::default() };
        app.on_audio_signals();
        assert_eq!(app.settings.devices.inputs, [known("mic", "Mic")]);
    }

    #[test]
    fn resume_re_reads_everything() {
        let mut app = english(speakers());
        assert_eq!(app.on_resumed(), Changes::ALL);
        assert_eq!(app.engine.refreshes, 1);
        // A device that appeared during sleep is learnt on resume.
        app.engine.devices[Flow::Output.index()].push(device("new", "New"));
        app.on_resumed();
        assert_eq!(app.settings.devices.outputs, [known("spk", "Speakers"), known("new", "New")]);
    }

    #[test]
    fn wheel_changes_the_volume_by_the_configured_step() {
        let mut app = english(speakers());
        assert!(app.wheel(1));
        assert_eq!(
            app.engine.endpoint(Flow::Output).unwrap().percent(),
            42,
            "default wheel increment of 2"
        );
        assert!(app.wheel(-3));
        assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 36);
        app.settings.general.wheel_step = 10;
        app.wheel(10);
        assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 100, "capped");
        assert!(!app.wheel(1), "already at the top: nothing changes");
        app.wheel(-30);
        assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 0, "floored");
    }

    #[test]
    fn wheel_unmutes_like_windows() {
        let mut engine = speakers();
        engine.levels.insert("spk".into(), (1.0, true));
        engine.sync();
        let mut app = english(engine);
        // Even at the top, where the volume cannot rise, the turn unmutes.
        assert!(app.wheel(1));
        let endpoint = app.engine.endpoint(Flow::Output).unwrap();
        assert!(!endpoint.muted);
        assert_eq!(endpoint.percent(), 100);
    }

    #[test]
    fn wheel_without_output_does_nothing_and_recovers_a_failed_binding() {
        let mut app = english(FakeEngine::new());
        assert!(!app.wheel(1));

        let mut app = english(speakers());
        app.engine.binding_failed[Flow::Output.index()] = true;
        app.engine.sync();
        assert!(app.wheel(1), "the turn reads everything again, then applies");
        assert_eq!(app.engine.refreshes, 1);
        assert_eq!(app.engine.endpoint(Flow::Output).unwrap().percent(), 42);
    }

    #[test]
    fn tray_icon_follows_the_level_and_mute() {
        let mut app = english(speakers());
        assert_eq!(
            app.tray_icon(),
            TrayIcon {
                level: Some(VolumeLevel::Medium),
                badge: None,
                large: false,
                look: TrayLook::Windows
            }
        );
        app.toggle_output_mute();
        assert_eq!(app.tray_icon().level, Some(VolumeLevel::Muted));
        assert_eq!(
            english(FakeEngine::new()).tray_icon(),
            TrayIcon { level: None, badge: None, large: false, look: TrayLook::Windows }
        );
    }

    #[test]
    fn tray_icon_badge_shows_the_device_type_in_the_badge_style() {
        let headphones =
            DeviceInfo { form_factor: FormFactor::Headphones, ..device("hp", "Headphones") };
        let mut engine = FakeEngine::new().with_device(Flow::Output, headphones);
        engine.levels.insert("hp".into(), (0.8, false));
        engine.sync();
        let mut settings = Settings::default();
        settings.general.tray_icon_style = TrayIconStyle::WithDeviceBadge;
        let app = App::new(engine, settings, Language::ENGLISH);
        assert_eq!(
            app.tray_icon(),
            TrayIcon {
                level: Some(VolumeLevel::High),
                badge: Some(DeviceType::Headphones),
                large: false,
                look: TrayLook::Windows
            }
        );
        // Same device, default style: no badge.
        let mut app = english(app.engine);
        assert_eq!(app.tray_icon().badge, None);
        // The colourful look (FR-084).
        app.settings.general.tray_icon_look = TrayIconLook::Colourful;
        assert_eq!(app.tray_icon().look, TrayLook::Colourful);
    }

    #[test]
    fn volume_steps_match_windows() {
        assert_eq!(volume_level(0, false), VolumeLevel::Zero);
        assert_eq!(volume_level(1, false), VolumeLevel::Low);
        assert_eq!(volume_level(33, false), VolumeLevel::Low);
        assert_eq!(volume_level(34, false), VolumeLevel::Medium);
        assert_eq!(volume_level(66, false), VolumeLevel::Medium);
        assert_eq!(volume_level(67, false), VolumeLevel::High);
        assert_eq!(volume_level(100, false), VolumeLevel::High);
        for percent in [0, 50, 100] {
            assert_eq!(volume_level(percent, true), VolumeLevel::Muted);
        }
    }

    #[test]
    fn device_types_without_a_drawing_keep_the_windows_speaker() {
        assert_eq!(device_type(FormFactor::Headset), Some(DeviceType::Headset));
        assert_eq!(device_type(FormFactor::Handset), Some(DeviceType::Headset));
        assert_eq!(device_type(FormFactor::Display), Some(DeviceType::Display));
        assert_eq!(device_type(FormFactor::Spdif), None);
        assert_eq!(device_type(FormFactor::Unknown), None);
    }

    #[test]
    fn tooltip_follows_the_media_device_not_the_calls_one() {
        let mut app = english(speakers().with_device(Flow::Output, device("hp", "Headphones")));
        app.engine.set_default_device(Flow::Output, "hp", SwitchScope::CallsOnly).unwrap();
        assert_eq!(app.engine.default_device(Flow::Output, Role::Calls), Some("hp"));
        assert_eq!(app.tooltip(), "Speakers: 40%");
    }
}
