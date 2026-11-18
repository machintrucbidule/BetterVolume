//! What the bubble above the taskbar shows (FR-017, FR-105, FR-106).
//!
//! The application remembers the default devices it last saw. After every event the
//! caller asks [`App::device_bubble`]: a default that changed since, from any source,
//! gives a bubble, unless the flyout is open (it shows the change itself, FR-106), the
//! notification is off (FR-077), or it is a microphone while microphone management is
//! off (FR-076). The very first defaults, read at start, are never announced.

use bv_audio::{AudioEngine, Flow, Role};
use bv_core::devices;
use bv_core::i18n::Msg;
use bv_ui_kit::controls::bubble::BubbleContent;

use super::flyout::{chosen_icon, device_icon};
use super::{volume_level, App};

/// The default devices of each flow (output, input) and role (media, calls).
pub(super) type Defaults = [[Option<String>; 2]; 2];

/// Reads the default devices from the engine.
pub(super) fn read_defaults(engine: &impl AudioEngine) -> Defaults {
    Flow::ALL.map(|flow| {
        [Role::Media, Role::Calls].map(|role| engine.default_device(flow, role).map(str::to_owned))
    })
}

impl<E: AudioEngine> App<E> {
    /// Whether a default device changed since the last call, and if so what the bubble
    /// shows: the output before the microphone; the media device before the calls
    /// one; "calls" is said only when the calls device changed alone. Remembers the
    /// current defaults, except a new default whose device is not listed yet: it is
    /// announced on a later call, once the device list has caught up.
    pub fn device_bubble(&mut self, flyout_visible: bool) -> Option<BubbleContent> {
        let now = read_defaults(&self.engine);
        let before = std::mem::replace(&mut self.defaults, now.clone());
        if flyout_visible || !self.settings.general.switch_notification {
            return None;
        }
        for flow in Flow::ALL {
            if flow == Flow::Input && !self.settings.general.microphone_enabled {
                continue;
            }
            let [media, calls] = &now[flow.index()];
            let [media_before, calls_before] = &before[flow.index()];
            let changed = if media.is_some() && media != media_before {
                media.as_deref().map(|id| (id, false))
            } else if calls.is_some() && calls != calls_before {
                calls.as_deref().map(|id| (id, true))
            } else {
                None
            };
            let Some((id, calls_only)) = changed else { continue };
            match self.device_content(flow, id, calls_only) {
                Some(content) => return Some(content),
                // Not listed yet: keep the old defaults of this flow, to try again.
                None => self.defaults[flow.index()] = before[flow.index()].clone(),
            }
        }
        None
    }

    fn device_content(&self, flow: Flow, id: &str, calls_only: bool) -> Option<BubbleContent> {
        let live = self.engine.devices(flow);
        let info = live.iter().find(|d| d.id == id)?;
        let known = self.settings.devices.list(flow);
        let kind = match (flow, calls_only) {
            (Flow::Output, false) => Msg::BubbleOutput,
            (Flow::Output, true) => Msg::BubbleOutputCalls,
            (Flow::Input, false) => Msg::BubbleInput,
            (Flow::Input, true) => Msg::BubbleInputCalls,
        };
        Some(BubbleContent::Device {
            icon: chosen_icon(known, id).unwrap_or_else(|| device_icon(flow, info)),
            name: devices::display_name(known, live, info),
            kind: self.language.text(kind).to_owned(),
        })
    }

    /// What the volume bubble shows after a wheel turn or a middle click (FR-017),
    /// or nothing: no output with a volume, the flyout open, or the bubble turned
    /// off (FR-081).
    pub fn volume_bubble(&self, flyout_visible: bool) -> Option<BubbleContent> {
        if flyout_visible || !self.settings.general.volume_bubble {
            return None;
        }
        let endpoint = self.engine.endpoint(Flow::Output)?;
        let percent = endpoint.percent();
        Some(BubbleContent::Volume { level: volume_level(percent, endpoint.muted), percent })
    }
}

#[cfg(test)]
mod tests {
    use bv_audio::fake::FakeEngine;
    use bv_audio::{Changes, DeviceInfo, SwitchScope};
    use bv_core::audio::FormFactor;
    use bv_core::devices::KnownDevice;
    use bv_core::i18n::Language;
    use bv_core::settings::Settings;
    use bv_ui_kit::controls::device_icon::DeviceIcon;
    use bv_ui_kit::icons::VolumeLevel;

    use super::*;

    fn device(id: &str, name: &str, form_factor: FormFactor) -> DeviceInfo {
        DeviceInfo { id: id.into(), windows_name: name.into(), form_factor, ..Default::default() }
    }

    fn engine() -> FakeEngine {
        let mut engine = FakeEngine::new()
            .with_device(Flow::Output, device("spk", "Speakers", FormFactor::Speakers))
            .with_device(Flow::Output, device("hp", "Headphones", FormFactor::Headphones))
            .with_device(Flow::Input, device("mic", "Mic", FormFactor::Microphone))
            .with_device(Flow::Input, device("cam", "Webcam mic", FormFactor::Microphone));
        engine.levels.insert("spk".into(), (0.4, false));
        engine.levels.insert("hp".into(), (0.75, false));
        engine.sync();
        engine
    }

    fn app() -> App<FakeEngine> {
        App::new(engine(), Settings::default(), Language::ENGLISH)
    }

    /// Another program changes a default (what Windows then reports).
    fn switched_elsewhere(app: &mut App<FakeEngine>, flow: Flow, id: &str, scope: SwitchScope) {
        app.engine.set_default_device(flow, id, scope).unwrap();
        app.engine.pending = Changes { defaults: true, output: true, ..Changes::default() };
        app.on_audio_signals();
    }

    fn device_bubble(name: &str, kind: &str, icon: DeviceIcon) -> Option<BubbleContent> {
        Some(BubbleContent::Device { icon, name: name.into(), kind: kind.into() })
    }

    #[test]
    fn nothing_is_announced_at_start_or_without_change() {
        let mut app = app();
        assert_eq!(app.device_bubble(false), None);
        app.on_resumed();
        assert_eq!(app.device_bubble(false), None);
    }

    #[test]
    fn a_switch_made_elsewhere_is_announced_once() {
        let mut app = app();
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::MediaAndCalls);
        assert_eq!(
            app.device_bubble(false),
            device_bubble("Headphones", "Audio output", DeviceIcon::WiredHeadphones)
        );
        assert_eq!(app.device_bubble(false), None, "announced once");
    }

    #[test]
    fn a_calls_only_change_says_so() {
        let mut app = app();
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::CallsOnly);
        assert_eq!(
            app.device_bubble(false),
            device_bubble("Headphones", "Audio output · calls", DeviceIcon::WiredHeadphones)
        );
        // Media alone: no "calls".
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::MediaOnly);
        assert_eq!(
            app.device_bubble(false),
            device_bubble("Headphones", "Audio output", DeviceIcon::WiredHeadphones)
        );
    }

    #[test]
    fn microphones_are_announced_only_with_microphone_management() {
        let mut app = app();
        switched_elsewhere(&mut app, Flow::Input, "cam", SwitchScope::MediaAndCalls);
        assert_eq!(
            app.device_bubble(false),
            device_bubble("Webcam mic", "Microphone", DeviceIcon::GenericInput)
        );
        app.settings.general.microphone_enabled = false;
        switched_elsewhere(&mut app, Flow::Input, "mic", SwitchScope::MediaAndCalls);
        assert_eq!(app.device_bubble(false), None);
    }

    #[test]
    fn the_output_wins_when_both_change_together() {
        let mut app = app();
        app.engine.set_default_device(Flow::Input, "cam", SwitchScope::MediaAndCalls).unwrap();
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::MediaAndCalls);
        let shown = app.device_bubble(false);
        assert!(
            matches!(shown, Some(BubbleContent::Device { ref name, .. }) if name == "Headphones")
        );
    }

    #[test]
    fn no_bubble_while_the_flyout_is_open_or_when_turned_off() {
        let mut app = app();
        // A tile click: the flyout shows the change (FR-106).
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::MediaAndCalls);
        assert_eq!(app.device_bubble(true), None);
        assert_eq!(app.device_bubble(false), None, "the change is not announced later either");

        app.settings.general.switch_notification = false;
        switched_elsewhere(&mut app, Flow::Output, "spk", SwitchScope::MediaAndCalls);
        assert_eq!(app.device_bubble(false), None);
    }

    #[test]
    fn the_bubble_uses_the_custom_name_and_icon() {
        let mut settings = Settings::default();
        settings.devices.outputs.push(KnownDevice {
            custom_name: Some("Desk".into()),
            icon: Some(DeviceIcon::Soundbar.id().into()),
            ..KnownDevice::new("hp")
        });
        let mut app = App::new(engine(), settings, Language::from_tag("fr").unwrap());
        switched_elsewhere(&mut app, Flow::Output, "hp", SwitchScope::MediaAndCalls);
        assert_eq!(
            app.device_bubble(false),
            device_bubble("Desk", "Sortie audio", DeviceIcon::Soundbar)
        );
    }

    #[test]
    fn a_default_announced_before_its_device_is_listed_waits_for_it() {
        let mut app = app();
        app.engine.defaults[Flow::Output.index()] = [Some("new".into()), Some("new".into())];
        assert_eq!(app.device_bubble(false), None, "not listed yet");
        app.engine.devices[Flow::Output.index()].push(device("new", "New", FormFactor::Speakers));
        assert_eq!(
            app.device_bubble(false),
            device_bubble("New", "Audio output", DeviceIcon::DesktopSpeakers)
        );
        assert_eq!(app.device_bubble(false), None);
    }

    #[test]
    fn a_device_gone_is_not_announced() {
        let mut app = app();
        app.engine.defaults[Flow::Output.index()] = [None, None];
        app.engine.pending = Changes::ALL;
        app.on_audio_signals();
        assert_eq!(app.device_bubble(false), None);
    }

    #[test]
    fn the_volume_bubble_shows_the_level() {
        let mut app = app();
        assert_eq!(
            app.volume_bubble(false),
            Some(BubbleContent::Volume { level: VolumeLevel::Medium, percent: 40 })
        );
        app.toggle_output_mute();
        assert_eq!(
            app.volume_bubble(false),
            Some(BubbleContent::Volume { level: VolumeLevel::Muted, percent: 40 })
        );
        assert_eq!(app.volume_bubble(true), None, "the flyout shows it");
        app.settings.general.volume_bubble = false;
        assert_eq!(app.volume_bubble(false), None, "turned off (FR-081)");
        assert_eq!(
            App::new(FakeEngine::new(), Settings::default(), Language::ENGLISH)
                .volume_bubble(false),
            None
        );
    }
}
