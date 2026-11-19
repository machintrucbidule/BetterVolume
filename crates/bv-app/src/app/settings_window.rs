//! The settings window (FR-070 to FR-081, FR-095): what it shows and what its controls do.
//!
//! The application describes each page as design-system cards ([`SettingsContent`]);
//! every change applies at once (FR-070) and is saved by the caller (`take_dirty`).
//! The update cards of the About page are described in `update.rs`, hiding the Windows
//! volume icon in `volume_icon.rs`. The Devices page (FR-085 to FR-089) lists every known
//! device of each flow in the configured order; the Home Assistant page (`ha_page.rs`)
//! holds the link settings.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use bv_audio::{AudioEngine, Flow};
use bv_core::devices;
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{
    FlyoutHeight, SwitchScope, TrayIconSize, TrayIconStyle, WHEEL_STEP_MAX, WHEEL_STEP_MIN,
};
use bv_ui_kit::controls::device_icon::DeviceIcon;
use bv_ui_kit::controls::settings::{
    Block, Card, CardControl, DeviceRow, DeviceTexts, NavEntry, PageKey, SettingKey,
    SettingsAction, SettingsContent,
};
use bv_ui_kit::icons::Icon;

use super::ha_page::{HaKey, HaPage};
use super::update::UpdateKey;
use super::App;

/// A page of the settings window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    /// General settings (section 5.1).
    #[default]
    General,
    /// Device lists (section 5.2).
    Devices,
    /// Home Assistant (section 5.3).
    HomeAssistant,
    /// About (section 5.4).
    About,
}

impl Page {
    const ALL: [Page; 4] = [Page::General, Page::Devices, Page::HomeAssistant, Page::About];

    pub(super) fn key(self) -> PageKey {
        PageKey(self as u64)
    }

    fn from_key(key: PageKey) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// The settings of the window, as card keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Setting {
    Autostart,
    Language,
    Microphones,
    Scope,
    Wheel,
    SwitchNotification,
    VolumeBubble,
    FlyoutHeight,
    TrayStyle,
    TraySize,
    HideIcon,
    HideIconNotice,
    Version,
    Licence,
    Project,
    Log,
}

impl Setting {
    const ALL: [Setting; 16] = [
        Setting::Autostart,
        Setting::Language,
        Setting::Microphones,
        Setting::Scope,
        Setting::Wheel,
        Setting::SwitchNotification,
        Setting::VolumeBubble,
        Setting::FlyoutHeight,
        Setting::TrayStyle,
        Setting::TraySize,
        Setting::HideIcon,
        Setting::HideIconNotice,
        Setting::Version,
        Setting::Licence,
        Setting::Project,
        Setting::Log,
    ];

    fn key(self) -> SettingKey {
        SettingKey(self as u64 + 1)
    }

    fn from_key(key: SettingKey) -> Option<Setting> {
        Setting::ALL.into_iter().find(|s| s.key() == key)
    }
}

const SCOPES: [(SwitchScope, Msg); 3] = [
    (SwitchScope::MediaAndCalls, Msg::SettingsScopeBoth),
    (SwitchScope::MediaOnly, Msg::SettingsScopeMedia),
    (SwitchScope::CallsOnly, Msg::SettingsScopeCalls),
];

const TRAY_STYLES: [(TrayIconStyle, Msg); 2] = [
    (TrayIconStyle::SameAsWindows, Msg::SettingsTraySame),
    (TrayIconStyle::WithDeviceBadge, Msg::SettingsTrayBadge),
];

const TRAY_SIZES: [(TrayIconSize, Msg); 2] = [
    (TrayIconSize::SameAsWindows, Msg::SettingsTraySizeSame),
    (TrayIconSize::Large, Msg::SettingsTraySizeLarge),
];

const FLYOUT_HEIGHTS: [(FlyoutHeight, Msg); 2] = [
    (FlyoutHeight::Full, Msg::SettingsFlyoutHeightFull),
    (FlyoutHeight::Content, Msg::SettingsFlyoutHeightContent),
];

/// What the caller must do after a settings action, beyond showing the new state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsEffect {
    /// Show the new state everywhere (tray, flyout, settings window).
    Refresh,
    /// Turn starting with Windows on or off, then report it with
    /// [`App::set_autostart_state`].
    SetAutostart(bool),
    /// Close the settings window (the close question was answered).
    Close,
    /// Open the folder of the log file.
    OpenLogFolder,
    /// Open the project page ([`App::project_page`]) in the browser.
    OpenProjectPage,
    /// Hide (`true`) or show again the Windows volume icon (FR-079); when hiding, also
    /// show BetterVolume's own icon on the taskbar.
    HideVolumeIcon(bool),
}

/// State of the settings window kept by the application.
#[derive(Default)]
pub(super) struct SettingsSession {
    pub(super) page: Page,
    /// Whether the Windows autostart entry is on (read by the caller).
    pub(super) autostart: bool,
    /// The Home Assistant page (its draft form, the last test).
    pub(super) ha: HaPage,
    /// The project page, for a copy built from the public repository (FR-095).
    pub(super) project_page: Option<String>,
}

impl<E: AudioEngine> App<E> {
    /// Sets the project page the About page links to; `None` (a copy built elsewhere)
    /// leaves the link out.
    pub fn set_project_page(&mut self, page: Option<String>) {
        self.settings_window.project_page = page;
    }

    /// The project page the About page links to, if any.
    pub fn project_page(&self) -> Option<&str> {
        self.settings_window.project_page.as_deref()
    }

    /// The settings window shows the General page (it is opened or brought forward for
    /// something there).
    pub fn settings_show_general(&mut self) {
        self.settings_window.page = Page::General;
    }

    /// The settings window is being opened: it starts on the General page, showing the
    /// autostart state read by the caller.
    pub fn settings_opening(&mut self, autostart: bool) {
        let window = &mut self.settings_window;
        window.page = Page::General;
        window.autostart = autostart;
        window.ha = Default::default();
    }

    /// The autostart entry as Windows now has it. Returns whether it changed.
    pub fn set_autostart_state(&mut self, on: bool) -> bool {
        std::mem::replace(&mut self.settings_window.autostart, on) != on
    }

    /// What the settings window shows.
    pub fn settings_content(&self) -> SettingsContent {
        let language = self.language;
        let text = |msg| language.text(msg).to_owned();
        let page = self.settings_window.page;
        let nav = [
            (Page::General, Icon::Settings, Msg::SettingsPageGeneral),
            (Page::Devices, Icon::Devices, Msg::SettingsPageDevices),
            (Page::HomeAssistant, Icon::Home, Msg::SettingsPageHa),
            (Page::About, Icon::Info, Msg::SettingsPageAbout),
        ]
        .into_iter()
        .map(|(page, icon, label)| NavEntry { key: page.key(), icon, label: text(label) })
        .collect();
        let (title, blocks) = match page {
            Page::General => (text(Msg::SettingsPageGeneral), self.general_cards()),
            Page::Devices => (text(Msg::SettingsPageDevices), self.device_blocks()),
            Page::HomeAssistant => (text(Msg::SettingsPageHa), self.ha_blocks()),
            Page::About => (text(Msg::SettingsPageAbout), self.about_cards()),
        };
        SettingsContent {
            name: text(Msg::MenuSettings),
            nav_name: text(Msg::SettingsNav),
            nav,
            page: page.key(),
            title,
            blocks,
            devices: self.device_texts(),
            close_question: self.close_question(),
            question: self.volume_icon_question(),
            reveal: self
                .volume_icon_notice(Setting::HideIconNotice.key())
                .map(|_| Setting::HideIconNotice.key()),
        }
    }

    fn device_texts(&self) -> DeviceTexts {
        let text = |msg| self.language.text(msg).to_owned();
        DeviceTexts {
            change_icon: text(Msg::SettingsChangeIcon),
            rename: text(Msg::SettingsRename),
            hide: text(Msg::SettingsHide),
            show: text(Msg::SettingsShow),
            forget: text(Msg::SettingsForget),
            move_help: text(Msg::SettingsMoveHelp),
            name_box: text(Msg::SettingsNameBox),
            duplicate: text(Msg::SettingsDuplicate),
            automatic: text(Msg::SettingsIconAuto),
            icon_names: DeviceIcon::ALL.iter().map(|icon| text(icon_name(*icon))).collect(),
            forget_question: text(Msg::SettingsForgetQuestion),
            forget_detail: text(Msg::SettingsForgetDetail),
            forget_confirm: text(Msg::SettingsForgetConfirm),
            cancel: text(Msg::SettingsCancel),
        }
    }

    /// The Devices page: help line, then one list per flow (inputs only with microphone
    /// management, FR-076), every known device in the configured order (FR-085, FR-089).
    fn device_blocks(&self) -> Vec<Block> {
        let text = |msg| self.language.text(msg).to_owned();
        let mut blocks = vec![Block::Text(text(Msg::SettingsDevicesHelp))];
        let keys = device_keys(&self.settings);
        for flow in Flow::ALL {
            if flow == Flow::Input && !self.settings.general.microphone_enabled {
                continue;
            }
            blocks.push(Block::Header(text(match flow {
                Flow::Output => Msg::SettingsOutputs,
                Flow::Input => Msg::SettingsInputs,
            })));
            let known = self.settings.devices.list(flow);
            if known.is_empty() {
                blocks.push(Block::Text(text(Msg::SettingsNoDevices)));
                continue;
            }
            let live = self.engine.devices(flow);
            let names = devices::display_names(known, live);
            for entry in known {
                let Some(key) = keys.iter().find(|(_, f, id)| *f == flow && *id == entry.id) else {
                    continue;
                };
                let info = live.iter().find(|d| d.id == entry.id);
                let connected = info.is_some_and(|i| i.state.is_available());
                let windows_name =
                    info.map_or(entry.windows_name.as_str(), |i| i.windows_name.as_str());
                let automatic = info
                    .map_or_else(|| generic_icon(flow), |i| super::flyout::device_icon(flow, i));
                let chosen = entry.icon.as_deref().and_then(DeviceIcon::from_id);
                let name = names
                    .iter()
                    .find(|(id, _)| *id == entry.id)
                    .map_or_else(|| windows_name.to_owned(), |(_, n)| n.clone());
                let state = text(if connected {
                    Msg::SettingsConnected
                } else {
                    Msg::SettingsDisconnected
                });
                blocks.push(Block::Device(DeviceRow {
                    key: key.0,
                    list: flow.index() as u32,
                    icon: chosen.unwrap_or(automatic),
                    chosen_icon: chosen,
                    automatic_icon: automatic,
                    name,
                    detail: self.language.format(
                        Msg::SettingsDeviceDetail,
                        &[("name", windows_name), ("state", &state)],
                    ),
                    hidden: entry.hidden,
                    connected,
                }));
            }
        }
        blocks
    }

    /// The device a row key designates.
    fn device_of(&self, key: SettingKey) -> Option<(Flow, String)> {
        device_keys(&self.settings)
            .into_iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, f, id)| (f, id))
    }

    fn card(
        &self,
        setting: Setting,
        icon: Icon,
        title: Msg,
        help: Option<Msg>,
        control: CardControl,
    ) -> Block {
        Block::Card(Card {
            key: setting.key(),
            icon: Some(icon),
            title: self.language.text(title).to_owned(),
            description: help.map(|m| self.language.text(m).to_owned()),
            control,
            enabled: true,
        })
    }

    fn toggle(&self, on: bool) -> CardControl {
        CardControl::Toggle {
            on,
            on_label: self.language.text(Msg::SettingsOn).to_owned(),
            off_label: self.language.text(Msg::SettingsOff).to_owned(),
        }
    }

    /// The language choices: automatic (naming the language it gives now), then every
    /// shipped language in its own name (FR-075).
    fn language_choice(&self) -> CardControl {
        let automatic = Language::resolve(None::<&str>, &self.os_languages);
        let mut options = vec![self
            .language
            .format(Msg::SettingsLanguageAuto, &[("language", automatic.native_name())])];
        options.extend(Language::all().map(|l| l.native_name().to_owned()));
        let selected = self
            .settings
            .general
            .language
            .as_deref()
            .and_then(Language::from_tag)
            .and_then(|chosen| Language::all().position(|l| l == chosen))
            .map_or(0, |i| i + 1);
        CardControl::Choice { options, selected }
    }

    fn general_cards(&self) -> Vec<Block> {
        let g = &self.settings.general;
        let text = |msg| self.language.text(msg).to_owned();
        let scope = CardControl::Choice {
            options: SCOPES.iter().map(|(_, m)| text(*m)).collect(),
            selected: SCOPES.iter().position(|(s, _)| *s == g.switch_scope).unwrap_or(0),
        };
        let wheel = CardControl::Choice {
            options: (WHEEL_STEP_MIN..=WHEEL_STEP_MAX)
                .map(|step| {
                    self.language.format(Msg::SettingsWheelValue, &[("step", &step.to_string())])
                })
                .collect(),
            selected: usize::from(g.wheel_step.saturating_sub(WHEEL_STEP_MIN)),
        };
        let style = CardControl::Choice {
            options: TRAY_STYLES.iter().map(|(_, m)| text(*m)).collect(),
            selected: TRAY_STYLES.iter().position(|(s, _)| *s == g.tray_icon_style).unwrap_or(0),
        };
        let size = CardControl::Choice {
            options: TRAY_SIZES.iter().map(|(_, m)| text(*m)).collect(),
            selected: TRAY_SIZES.iter().position(|(s, _)| *s == g.tray_icon_size).unwrap_or(0),
        };
        let height = CardControl::Choice {
            options: FLYOUT_HEIGHTS.iter().map(|(_, m)| text(*m)).collect(),
            selected: FLYOUT_HEIGHTS.iter().position(|(h, _)| *h == g.flyout_height).unwrap_or(0),
        };
        let mut cards = vec![
            self.card(
                Setting::Autostart,
                Icon::Power,
                Msg::SettingsAutostart,
                Some(Msg::SettingsAutostartHelp),
                self.toggle(self.settings_window.autostart),
            ),
            self.card(
                Setting::Language,
                Icon::Language,
                Msg::SettingsLanguage,
                Some(Msg::SettingsLanguageHelp),
                self.language_choice(),
            ),
            self.card(
                Setting::Microphones,
                Icon::Microphone,
                Msg::SettingsMicrophones,
                Some(Msg::SettingsMicrophonesHelp),
                self.toggle(g.microphone_enabled),
            ),
            self.card(
                Setting::Scope,
                Icon::Speaker,
                Msg::SettingsScope,
                Some(Msg::SettingsScopeHelp),
                scope,
            ),
            self.card(
                Setting::Wheel,
                Icon::Mouse,
                Msg::SettingsWheel,
                Some(Msg::SettingsWheelHelp),
                wheel,
            ),
            self.card(
                Setting::SwitchNotification,
                Icon::Notification,
                Msg::SettingsSwitchNotification,
                Some(Msg::SettingsSwitchNotificationHelp),
                self.toggle(g.switch_notification),
            ),
            self.card(
                Setting::VolumeBubble,
                Icon::Volume3,
                Msg::SettingsVolumeBubble,
                Some(Msg::SettingsVolumeBubbleHelp),
                self.toggle(g.volume_bubble),
            ),
            self.card(
                Setting::FlyoutHeight,
                Icon::PanelHeight,
                Msg::SettingsFlyoutHeight,
                Some(Msg::SettingsFlyoutHeightHelp),
                height,
            ),
            self.card(
                Setting::TrayStyle,
                Icon::Taskbar,
                Msg::SettingsTrayStyle,
                Some(Msg::SettingsTrayStyleHelp),
                style,
            ),
            self.card(
                Setting::TraySize,
                Icon::IconSize,
                Msg::SettingsTraySize,
                Some(Msg::SettingsTraySizeHelp),
                size,
            ),
            // FR-079: the risks are always under the switch.
            Block::Card(Card {
                key: Setting::HideIcon.key(),
                icon: Some(Icon::Hidden),
                title: self.language.text(Msg::SettingsHideIcon).to_owned(),
                description: Some(self.volume_icon_help()),
                control: self.toggle(g.hide_windows_volume_icon),
                enabled: true,
            }),
        ];
        // Why the option turned itself off, next to the switch.
        if let Some(notice) = self.volume_icon_notice(Setting::HideIconNotice.key()) {
            cards.push(Block::Notice(notice));
        }
        cards
    }

    fn about_cards(&self) -> Vec<Block> {
        let version = self.language.format(Msg::SettingsVersion, &[("version", crate::VERSION)]);
        let mut blocks = vec![Block::Card(Card {
            key: Setting::Version.key(),
            icon: Some(Icon::Info),
            title: self.language.text(Msg::AppName).to_owned(),
            description: Some(version),
            control: CardControl::None,
            enabled: true,
        })];
        // FR-095: the update status, "check now" and the update switches.
        blocks.extend(self.update_cards());
        blocks.extend([self.card(
            Setting::Licence,
            Icon::Document,
            Msg::SettingsLicence,
            Some(Msg::SettingsLicenceHelp),
            CardControl::None,
        )]);
        if self.settings_window.project_page.is_some() {
            blocks.push(self.card(
                Setting::Project,
                Icon::Globe,
                Msg::SettingsProject,
                Some(Msg::SettingsProjectHelp),
                CardControl::Link { label: self.language.text(Msg::SettingsOpenPage).to_owned() },
            ));
        }
        blocks.extend([self.card(
            Setting::Log,
            Icon::Folder,
            Msg::SettingsLog,
            Some(Msg::SettingsLogHelp),
            CardControl::Link { label: self.language.text(Msg::SettingsOpenFolder).to_owned() },
        )]);
        blocks
    }

    /// The general settings, for a change (marks them for saving).
    fn general_mut(&mut self) -> &mut bv_core::settings::General {
        self.dirty = true;
        &mut self.settings.general
    }

    /// Applies what the user asked for in the settings window.
    pub fn settings_action(&mut self, action: SettingsAction) -> SettingsEffect {
        match action {
            SettingsAction::SelectPage(key) => {
                if let Some(page) = Page::from_key(key) {
                    self.settings_window.page = page;
                }
            }
            SettingsAction::Toggle(key, on) if HaKey::from_key(key).is_some() => {
                match HaKey::from_key(key) {
                    Some(HaKey::Enabled) => self.ha_set_enabled(on),
                    Some(HaKey::Tls) => self.ha_set_tls(on),
                    _ => {}
                }
            }
            SettingsAction::Toggle(key, on) if UpdateKey::from_key(key).is_some() => {
                if let Some(update) = UpdateKey::from_key(key) {
                    self.update_toggle(update, on);
                }
            }
            SettingsAction::Toggle(key, on) => match Setting::from_key(key) {
                Some(Setting::Autostart) => return SettingsEffect::SetAutostart(on),
                Some(Setting::Microphones) => {
                    self.general_mut().microphone_enabled = on;
                    // The flyout loses its microphone tab (FR-076).
                    self.microphones_changed();
                }
                Some(Setting::SwitchNotification) => self.general_mut().switch_notification = on,
                Some(Setting::VolumeBubble) => self.general_mut().volume_bubble = on,
                Some(Setting::HideIcon) => return self.set_volume_icon_hidden(on),
                _ => {}
            },
            SettingsAction::QuestionAnswer(hide) => return self.volume_icon_answer(hide),
            SettingsAction::Choose(key, index) => match Setting::from_key(key) {
                Some(Setting::Language) => {
                    let tag = index
                        .checked_sub(1)
                        .and_then(|i| Language::all().nth(i))
                        .map(|l| l.tag().to_owned());
                    self.general_mut().language = tag;
                    self.language = Language::resolve(
                        self.settings.general.language.as_deref(),
                        &self.os_languages,
                    );
                }
                Some(Setting::Scope) => {
                    if let Some((scope, _)) = SCOPES.get(index) {
                        self.general_mut().switch_scope = *scope;
                    }
                }
                Some(Setting::Wheel) => {
                    let step = u8::try_from(index).ok().and_then(|i| i.checked_add(WHEEL_STEP_MIN));
                    if let Some(step) = step.filter(|s| *s <= WHEEL_STEP_MAX) {
                        self.general_mut().wheel_step = step;
                    }
                }
                Some(Setting::TrayStyle) => {
                    if let Some((style, _)) = TRAY_STYLES.get(index) {
                        self.general_mut().tray_icon_style = *style;
                    }
                }
                Some(Setting::TraySize) => {
                    if let Some((size, _)) = TRAY_SIZES.get(index) {
                        self.general_mut().tray_icon_size = *size;
                    }
                }
                Some(Setting::FlyoutHeight) => {
                    if let Some((height, _)) = FLYOUT_HEIGHTS.get(index) {
                        self.general_mut().flyout_height = *height;
                    }
                }
                _ => {}
            },
            SettingsAction::MoveDevice(key, to) => {
                if let Some((flow, id)) = self.device_of(key) {
                    self.dirty |= devices::move_to(self.settings.devices.list_mut(flow), &id, to);
                }
            }
            SettingsAction::SetHidden(key, hidden) => {
                if let Some((flow, id)) = self.device_of(key) {
                    self.dirty |=
                        devices::set_hidden(self.settings.devices.list_mut(flow), &id, hidden);
                }
            }
            SettingsAction::Rename(key, name) => {
                if let Some((flow, id)) = self.device_of(key) {
                    let live = self.engine.devices(flow).to_vec();
                    let known = self.settings.devices.list_mut(flow);
                    // A refused name (another device shows it) changes nothing: the view
                    // already refused it, so this only guards a race with a live change.
                    self.dirty |= devices::rename(known, &live, &id, &name).unwrap_or(false);
                }
            }
            SettingsAction::SetIcon(key, icon) => {
                if let Some((flow, id)) = self.device_of(key) {
                    let known = self.settings.devices.list_mut(flow);
                    self.dirty |= devices::set_icon(known, &id, icon.map(DeviceIcon::id));
                }
            }
            SettingsAction::Forget(key) => {
                if let Some((flow, id)) = self.device_of(key) {
                    let live = self.engine.devices(flow).to_vec();
                    self.dirty |= devices::forget(self.settings.devices.list_mut(flow), &live, &id);
                }
            }
            SettingsAction::Invoke(key) => {
                if Setting::from_key(key) == Some(Setting::Log) {
                    return SettingsEffect::OpenLogFolder;
                }
                if Setting::from_key(key) == Some(Setting::Project) {
                    return SettingsEffect::OpenProjectPage;
                }
                if let Some(update) = UpdateKey::from_key(key) {
                    self.update_invoke(update);
                }
                match HaKey::from_key(key) {
                    Some(HaKey::Test) => self.ha_test(),
                    Some(HaKey::Save) => {
                        self.ha_save();
                    }
                    Some(HaKey::Notice) => self.ha_trust(),
                    _ => {}
                }
            }
            SettingsAction::SetText(key, text) => {
                if let Some(field) = HaKey::from_key(key) {
                    self.ha_set_text(field, text);
                }
            }
            // The window closes once the answer is applied (a refused form stays,
            // with its message).
            SettingsAction::CloseAnswer(true) => {
                if self.ha_save() {
                    return SettingsEffect::Close;
                }
                // Refused: show the page with its message, wherever the user was.
                self.settings_window.page = Page::HomeAssistant;
            }
            SettingsAction::CloseAnswer(false) => {
                self.ha_discard();
                return SettingsEffect::Close;
            }
        }
        SettingsEffect::Refresh
    }
}

/// Row keys of every known device (FR-086): stable while the device is known (a hash of
/// its flow and identifier, so the window keeps its focus and edits across updates), below
/// 2^24 as the design system requires, and never equal to a card key.
fn device_keys(settings: &bv_core::settings::Settings) -> Vec<(SettingKey, Flow, String)> {
    const BASE: u64 = 0x40_0000;
    const MASK: u64 = 0x1F_FFFF;
    let mut used = HashSet::new();
    let mut keys = Vec::new();
    for flow in Flow::ALL {
        for entry in settings.devices.list(flow) {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            entry.id.hash(&mut hasher);
            let mut slot = hasher.finish() & MASK;
            // Two identifiers of one flow hashing alike: the later one takes the next slot.
            let key = loop {
                let key = BASE | ((flow.index() as u64) << 21) | slot;
                if used.insert(key) {
                    break key;
                }
                slot = (slot + 1) & MASK;
            };
            keys.push((SettingKey(key), flow, entry.id.clone()));
        }
    }
    keys
}

fn generic_icon(flow: Flow) -> DeviceIcon {
    match flow {
        Flow::Output => DeviceIcon::GenericOutput,
        Flow::Input => DeviceIcon::GenericInput,
    }
}

/// The translated name of a catalogue icon.
fn icon_name(icon: DeviceIcon) -> Msg {
    match icon {
        DeviceIcon::DesktopSpeakers => Msg::IconDesktopSpeakers,
        DeviceIcon::BookshelfSpeakers => Msg::IconBookshelfSpeakers,
        DeviceIcon::Soundbar => Msg::IconSoundbar,
        DeviceIcon::AvReceiver => Msg::IconAvReceiver,
        DeviceIcon::Subwoofer => Msg::IconSubwoofer,
        DeviceIcon::PortableSpeaker => Msg::IconPortableSpeaker,
        DeviceIcon::WiredHeadphones => Msg::IconWiredHeadphones,
        DeviceIcon::BluetoothHeadphones => Msg::IconBluetoothHeadphones,
        DeviceIcon::GamingHeadset => Msg::IconGamingHeadset,
        DeviceIcon::Headset => Msg::IconHeadset,
        DeviceIcon::WiredEarbuds => Msg::IconWiredEarbuds,
        DeviceIcon::WirelessEarbuds => Msg::IconWirelessEarbuds,
        DeviceIcon::BoneConduction => Msg::IconBoneConduction,
        DeviceIcon::Monitor => Msg::IconMonitor,
        DeviceIcon::Tv => Msg::IconTv,
        DeviceIcon::Projector => Msg::IconProjector,
        DeviceIcon::Laptop => Msg::IconLaptop,
        DeviceIcon::Phone => Msg::IconPhone,
        DeviceIcon::VrHeadset => Msg::IconVrHeadset,
        DeviceIcon::DeskMicrophone => Msg::IconDeskMicrophone,
        DeviceIcon::StudioMicrophone => Msg::IconStudioMicrophone,
        DeviceIcon::LavalierMicrophone => Msg::IconLavalierMicrophone,
        DeviceIcon::Webcam => Msg::IconWebcam,
        DeviceIcon::AudioInterface => Msg::IconAudioInterface,
        DeviceIcon::VirtualDevice => Msg::IconVirtualDevice,
        DeviceIcon::GenericOutput => Msg::IconGenericOutput,
        DeviceIcon::GenericInput => Msg::IconGenericInput,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bv_audio::fake::FakeEngine;
    use bv_core::settings::Settings;

    fn app() -> App<FakeEngine> {
        let mut app = App::new(FakeEngine::default(), Settings::default(), Language::ENGLISH);
        // German is not shipped: the automatic language is English.
        app.set_os_languages(vec!["de-DE".into(), "en-US".into()]);
        app.take_dirty();
        app
    }

    fn card(content: &SettingsContent, setting: Setting) -> &Card {
        content.card(setting.key()).expect("card shown")
    }

    #[test]
    fn the_general_page_shows_the_settings() {
        let mut app = app();
        app.settings_opening(true);
        let content = app.settings_content();
        assert_eq!(content.page, Page::General.key());
        assert_eq!(content.nav.len(), 4, "General, Devices, Home Assistant, About");
        assert!(matches!(
            card(&content, Setting::Autostart).control,
            CardControl::Toggle { on: true, .. }
        ));
        let CardControl::Choice { options, selected } = &card(&content, Setting::Language).control
        else {
            panic!("language is a choice")
        };
        assert_eq!(*selected, 0, "automatic by default");
        assert_eq!(options[0], "Automatic (English)", "names the language it gives");
        assert_eq!(options.len(), 1 + Language::all().count());
        let CardControl::Choice { options, selected } = &card(&content, Setting::Wheel).control
        else {
            panic!("wheel step is a choice")
        };
        assert_eq!((options.len(), *selected), (10, 1));
        assert_eq!(options[1], "2%");
        // The update switches are on the About page; hiding the Windows volume icon is
        // the last card.
        assert_eq!(content.cards().count(), 11);
        assert_eq!(content.cards().last().map(|c| c.key), Some(Setting::HideIcon.key()));
        // The icon size (FR-083) follows the icon style, "Same as Windows" by default.
        let keys: Vec<_> = content.cards().map(|c| c.key).collect();
        let style = keys.iter().position(|k| *k == Setting::TrayStyle.key());
        assert_eq!(keys.get(style.unwrap_or(0) + 1), Some(&Setting::TraySize.key()));
        let CardControl::Choice { options, selected } = &card(&content, Setting::TraySize).control
        else {
            panic!("icon size is a choice")
        };
        assert_eq!(
            (options.clone(), *selected),
            (vec!["Same as Windows".into(), "Large".into()], 0)
        );
    }

    #[test]
    fn changes_apply_at_once_and_are_saved() {
        let mut app = app();
        app.settings_opening(false);
        assert_eq!(
            app.settings_action(SettingsAction::Choose(Setting::Wheel.key(), 9)),
            SettingsEffect::Refresh
        );
        assert_eq!(app.settings().general.wheel_step, 10);
        assert!(app.take_dirty());
        assert!(!app.take_dirty(), "reported once");
        app.settings_action(SettingsAction::Choose(Setting::Scope.key(), 2));
        assert_eq!(app.settings().general.switch_scope, SwitchScope::CallsOnly);
        app.settings_action(SettingsAction::Choose(Setting::TrayStyle.key(), 1));
        assert_eq!(app.settings().general.tray_icon_style, TrayIconStyle::WithDeviceBadge);
        assert!(!app.tray_icon().large);
        app.settings_action(SettingsAction::Choose(Setting::TraySize.key(), 1));
        assert_eq!(app.settings().general.tray_icon_size, TrayIconSize::Large);
        assert!(app.tray_icon().large, "applied at once (FR-083)");
        app.settings_action(SettingsAction::Toggle(Setting::Microphones.key(), false));
        assert!(!app.settings().general.microphone_enabled);
        assert!(app.flyout_content().tabs.is_none(), "the flyout loses its tabs (FR-076)");
        // The two bubbles (FR-077, FR-081).
        app.settings_action(SettingsAction::Toggle(Setting::SwitchNotification.key(), false));
        assert!(!app.settings().general.switch_notification);
        app.settings_action(SettingsAction::Toggle(Setting::VolumeBubble.key(), false));
        assert!(!app.settings().general.volume_bubble);
        assert!(app.volume_bubble(false).is_none());
        // The flyout height: full by default, fitting the content on demand.
        assert_eq!(app.flyout_content().height, bv_ui_kit::controls::flyout::PanelHeight::Full);
        app.settings_action(SettingsAction::Choose(Setting::FlyoutHeight.key(), 1));
        assert_eq!(app.settings().general.flyout_height, FlyoutHeight::Content);
        assert_eq!(app.flyout_content().height, bv_ui_kit::controls::flyout::PanelHeight::Content);
        // Out-of-range choices change nothing.
        app.settings_action(SettingsAction::Choose(Setting::Wheel.key(), 10));
        app.settings_action(SettingsAction::Choose(Setting::Scope.key(), 3));
        assert_eq!(app.settings().general.wheel_step, 10);
        assert_eq!(app.settings().general.switch_scope, SwitchScope::CallsOnly);
    }

    #[test]
    fn language_changes_apply_without_restart() {
        let mut app = app();
        app.settings_opening(false);
        let french = Language::all().position(|l| l.tag() == "fr").expect("French ships") + 1;
        app.settings_action(SettingsAction::Choose(Setting::Language.key(), french));
        assert_eq!(app.settings().general.language.as_deref(), Some("fr"));
        assert_eq!(app.language().tag(), "fr");
        assert_eq!(app.settings_content().title, "Général");
        let english = Language::all().position(|l| l.tag() == "en").expect("English ships") + 1;
        app.settings_action(SettingsAction::Choose(Setting::Language.key(), english));
        assert_eq!(app.language().tag(), "en");
        // Back to automatic: the Windows languages decide again.
        app.settings_action(SettingsAction::Choose(Setting::Language.key(), 0));
        assert_eq!(app.settings().general.language, None);
        assert_eq!(app.language().tag(), "en");
        // Windows in French: the automatic choice follows it at once.
        app.set_os_languages(vec!["fr-CA".into()]);
        assert_eq!(app.language().tag(), "fr");
        let content = app.settings_content();
        let CardControl::Choice { options, .. } = &card(&content, Setting::Language).control else {
            panic!("language is a choice")
        };
        assert_eq!(options[0], "Automatique (Français)");
    }

    #[test]
    fn autostart_and_links_are_left_to_the_caller() {
        let mut app = app();
        app.settings_opening(false);
        assert_eq!(
            app.settings_action(SettingsAction::Toggle(Setting::Autostart.key(), true)),
            SettingsEffect::SetAutostart(true)
        );
        assert!(!app.take_dirty(), "not stored in the settings file");
        assert!(app.set_autostart_state(true));
        assert!(!app.set_autostart_state(true));
        app.settings_action(SettingsAction::SelectPage(Page::About.key()));
        let content = app.settings_content();
        assert_eq!(content.page, Page::About.key());
        assert_eq!(
            app.settings_action(SettingsAction::Invoke(Setting::Log.key())),
            SettingsEffect::OpenLogFolder
        );
        assert!(content
            .card(Setting::Version.key())
            .unwrap()
            .description
            .as_deref()
            .unwrap()
            .contains(crate::VERSION));
        // Opening again starts on the General page.
        app.settings_opening(false);
        assert_eq!(app.settings_content().page, Page::General.key());
    }

    #[test]
    fn the_project_page_link_shows_only_for_a_build_that_names_its_repository() {
        let mut app = app();
        app.settings_opening(false);
        app.settings_action(SettingsAction::SelectPage(Page::About.key()));
        assert!(app.settings_content().card(Setting::Project.key()).is_none());
        app.set_project_page(Some("https://github.com/owner/BetterVolume".into()));
        let content = app.settings_content();
        let project = content.card(Setting::Project.key()).expect("the link");
        assert_eq!(project.title, "Project page");
        assert!(matches!(&project.control, CardControl::Link { label } if label == "Open"));
        // FR-095 order: licence, then the project page, then the log folder.
        let keys: Vec<_> = content.cards().map(|c| c.key).collect();
        let at = |s: Setting| keys.iter().position(|k| *k == s.key()).expect("a card");
        assert!(
            at(Setting::Licence) < at(Setting::Project) && at(Setting::Project) < at(Setting::Log)
        );
        assert_eq!(
            app.settings_action(SettingsAction::Invoke(Setting::Project.key())),
            SettingsEffect::OpenProjectPage
        );
        assert_eq!(app.project_page(), Some("https://github.com/owner/BetterVolume"));
    }

    // ----- Devices page -----

    use bv_audio::{DeviceInfo, DeviceState};
    use bv_core::audio::FormFactor;
    use bv_ui_kit::controls::settings::DeviceRow;

    fn output(id: &str, name: &str, form_factor: FormFactor) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            windows_name: name.into(),
            form_factor,
            state: DeviceState::Active,
            ..Default::default()
        }
    }

    fn devices_app() -> App<FakeEngine> {
        let engine = FakeEngine::new()
            .with_device(Flow::Output, output("spk", "Speakers", FormFactor::Speakers))
            .with_device(Flow::Output, output("hs", "Headset", FormFactor::Headset))
            .with_device(Flow::Input, output("mic", "Microphone", FormFactor::Microphone));
        let mut app = App::new(engine, Settings::default(), Language::ENGLISH);
        app.settings_opening(false);
        app.settings_action(SettingsAction::SelectPage(Page::Devices.key()));
        app.take_dirty();
        app
    }

    fn rows(app: &App<FakeEngine>) -> Vec<DeviceRow> {
        app.settings_content().rows().cloned().collect()
    }

    fn row(app: &App<FakeEngine>, name: &str) -> DeviceRow {
        rows(app).into_iter().find(|r| r.name == name).expect("row shown")
    }

    #[test]
    fn the_devices_page_lists_both_flows_in_order() {
        let mut app = devices_app();
        let content = app.settings_content();
        assert_eq!(content.nav.len(), 4);
        let names: Vec<String> = rows(&app).into_iter().map(|r| r.name).collect();
        // Learnt in natural order at start: Headset, Speakers; then the microphone.
        assert_eq!(names, ["Headset", "Speakers", "Microphone"]);
        let speakers = row(&app, "Speakers");
        assert_eq!(speakers.detail, "Speakers · connected");
        assert_eq!(speakers.icon, DeviceIcon::DesktopSpeakers);
        assert!(speakers.connected && !speakers.hidden);
        assert_ne!(speakers.list, row(&app, "Microphone").list);
        // Without microphone management the inputs list goes away (FR-076).
        app.settings.general.microphone_enabled = false;
        assert!(rows(&app).iter().all(|r| r.name != "Microphone"));
    }

    #[test]
    fn row_actions_change_the_settings_and_the_flyout() {
        let mut app = devices_app();
        let speakers = row(&app, "Speakers").key;
        app.settings_action(SettingsAction::MoveDevice(speakers, 0));
        assert!(app.take_dirty());
        assert_eq!(rows(&app)[0].name, "Speakers");
        let tiles: Vec<String> =
            app.flyout_content().devices.tiles.iter().map(|t| t.name.clone()).collect();
        assert_eq!(tiles, ["Speakers", "Headset"], "the flyout follows the order");

        app.settings_action(SettingsAction::Rename(speakers, "Desk".into()));
        assert_eq!(rows(&app)[0].name, "Desk");
        assert!(app.tooltip().starts_with("Desk") || !app.tooltip().contains("Speakers"));
        // A duplicate never gets through, even if asked.
        app.settings_action(SettingsAction::Rename(speakers, "headset".into()));
        assert_eq!(rows(&app)[0].name, "Desk");

        app.settings_action(SettingsAction::SetHidden(speakers, true));
        assert!(rows(&app)[0].hidden);

        app.settings_action(SettingsAction::SetIcon(speakers, Some(DeviceIcon::Soundbar)));
        let desk = row(&app, "Desk");
        assert_eq!(
            (desk.icon, desk.chosen_icon),
            (DeviceIcon::Soundbar, Some(DeviceIcon::Soundbar))
        );
        assert_eq!(desk.automatic_icon, DeviceIcon::DesktopSpeakers);
        assert_eq!(app.settings().devices.outputs[0].icon.as_deref(), Some("soundbar"));
        app.settings_action(SettingsAction::SetIcon(speakers, None));
        assert_eq!(row(&app, "Desk").icon, DeviceIcon::DesktopSpeakers);
        assert!(app.take_dirty());
    }

    #[test]
    fn chosen_icons_reach_the_tiles_and_the_tray_badge() {
        let mut app = devices_app();
        app.settings.general.tray_icon_style = TrayIconStyle::WithDeviceBadge;
        let current =
            app.engine.default_device(Flow::Output, bv_audio::Role::Media).unwrap().to_owned();
        let key = rows(&app).into_iter().find(|r| {
            app.settings.devices.outputs.iter().any(|k| {
                k.id == current && k.windows_name == r.detail.split(" · ").next().unwrap_or("")
            })
        });
        let key = key.expect("current output row").key;
        app.settings_action(SettingsAction::SetIcon(key, Some(DeviceIcon::Tv)));
        let tile = app.flyout_content().devices.tiles.into_iter().find(|t| t.current).unwrap();
        assert_eq!(tile.icon, DeviceIcon::Tv);
        assert_eq!(app.tray_icon().badge, Some(bv_ui_kit::controls::tray::DeviceType::Display));
    }

    #[test]
    fn only_disconnected_devices_are_forgotten() {
        let mut app = devices_app();
        let headset = row(&app, "Headset").key;
        app.settings_action(SettingsAction::Forget(headset));
        assert_eq!(rows(&app).len(), 3, "a connected device stays");
        app.engine.devices[Flow::Output.index()].retain(|d| d.id != "hs");
        let gone = row(&app, "Headset");
        assert!(!gone.connected);
        assert_eq!(gone.detail, "Headset · disconnected");
        assert_eq!(gone.icon, DeviceIcon::GenericOutput, "no type known while away");
        app.settings_action(SettingsAction::Forget(gone.key));
        assert!(rows(&app).iter().all(|r| r.name != "Headset"));
        assert!(app.take_dirty());
    }

    #[test]
    fn row_keys_are_stable_and_distinct() {
        let mut settings = Settings::default();
        for i in 0..500 {
            settings.devices.outputs.push(bv_core::devices::KnownDevice::new(&format!("dev{i}")));
        }
        settings.devices.inputs.push(bv_core::devices::KnownDevice::new("dev0"));
        let keys = device_keys(&settings);
        let distinct: HashSet<u64> = keys.iter().map(|(k, _, _)| k.0).collect();
        assert_eq!(distinct.len(), keys.len());
        assert!(keys.iter().all(|(k, _, _)| k.0 < 1 << 24 && k.0 > Setting::ALL.len() as u64));
        assert_eq!(device_keys(&settings), keys, "the same every time");
    }
}
