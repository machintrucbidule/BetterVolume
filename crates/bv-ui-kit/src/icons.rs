//! Icons: glyphs of the Windows 11 icon font (Segoe Fluent Icons), the font Windows
//! draws its own volume icon and menus with.

/// An icon of the design system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Speaker with a cross: muted.
    VolumeMute,
    /// Speaker, no wave: volume at zero.
    Volume0,
    /// Speaker, one wave.
    Volume1,
    /// Speaker, two waves.
    Volume2,
    /// Speaker, three waves.
    Volume3,
    /// Windows sound settings.
    SoundSettings,
    /// Volume mixer.
    Mixer,
    /// Classic Sound control panel.
    SoundPanel,
    /// BetterVolume settings.
    Settings,
    /// Quit.
    Quit,
    /// Microphone.
    Microphone,
    /// Microphone, muted.
    MicrophoneMuted,
    /// Telephone handset: the device used for calls.
    Calls,
    /// Chevron pointing down: a collapsed section.
    ChevronDown,
    /// Chevron pointing up: an expanded section.
    ChevronUp,
    /// An application without an icon of its own.
    App,
    /// Windows system sounds.
    SystemSounds,
    /// Devices (settings page).
    Devices,
    /// Information ("About" page).
    Info,
    /// Power: starting with Windows.
    Power,
    /// Language.
    Language,
    /// Mouse: the wheel step.
    Mouse,
    /// Speaker: which device a switch changes.
    Speaker,
    /// Taskbar: the tray icon style.
    Taskbar,
    /// Folder.
    Folder,
    /// Document: the licence.
    Document,
    /// Drag handle of a reorderable row.
    Grip,
    /// Pencil: rename.
    Edit,
    /// Eye: shown.
    Shown,
    /// Crossed eye: hidden.
    Hidden,
    /// Bin: forget.
    Delete,
    /// Notification: the device-switch bubble setting.
    Notification,
    /// Full screen: the flyout height setting.
    PanelHeight,
    /// Magnifier with a plus: the tray icon size.
    IconSize,
    /// House: the Home Assistant page.
    Home,
    /// Globe: a server address.
    Globe,
    /// Network cable: a port.
    Ethernet,
    /// Padlock: an encrypted connection.
    Lock,
    /// Person: a user name.
    Contact,
    /// Key: a password.
    Key,
    /// Computer: the name of this PC.
    Pc,
    /// Label: the discovery prefix.
    Tag,
    /// Filled circle behind the severity glyph of a notice (WinUI InfoBar).
    NoticeBackground,
    /// Severity glyph of an information notice.
    NoticeInfo,
    /// Severity glyph of a success notice.
    NoticeSuccess,
    /// Severity glyph of a warning notice.
    NoticeWarning,
    /// Severity glyph of an error notice.
    NoticeError,
    /// Two circling arrows: updates (Windows Update).
    Update,
    /// Arrow into a tray: a download.
    Download,
    /// A cross: closes a line or a panel.
    Close,
    /// Developer tools: preview versions.
    Preview,
}

impl Icon {
    /// The glyph in the Windows icon font.
    pub fn glyph(self) -> char {
        // Every code is a valid scalar value of the private use area (tested below).
        char::from_u32(self.code()).unwrap_or(char::REPLACEMENT_CHARACTER)
    }

    /// Code point of the glyph in the Windows icon font.
    pub fn code(self) -> u32 {
        match self {
            Icon::VolumeMute => 0xE74F,
            Icon::Volume0 => 0xE992,
            Icon::Volume1 => 0xE993,
            Icon::Volume2 => 0xE994,
            Icon::Volume3 => 0xE995,
            Icon::SoundSettings => 0xE767,
            Icon::Mixer => 0xF4C3,
            Icon::SoundPanel => 0xE7F5,
            Icon::Settings => 0xE713,
            Icon::Quit => 0xE711,
            Icon::Microphone => 0xE720,
            Icon::MicrophoneMuted => 0xEC54,
            Icon::Calls => 0xE717,
            Icon::ChevronDown => 0xE70D,
            Icon::ChevronUp => 0xE70E,
            Icon::App => 0xECAA,
            Icon::SystemSounds => 0xE767,
            Icon::Devices => 0xE772,
            Icon::Info => 0xE946,
            Icon::Power => 0xE7E8,
            Icon::Language => 0xF2B7,
            Icon::Mouse => 0xE962,
            Icon::Speaker => 0xE7F5,
            Icon::Taskbar => 0xE90E,
            Icon::Folder => 0xE8B7,
            Icon::Document => 0xE8A5,
            Icon::Grip => 0xE76F,
            Icon::Edit => 0xE70F,
            Icon::Shown => 0xE890,
            Icon::Hidden => 0xED1A,
            Icon::Delete => 0xE74D,
            Icon::Notification => 0xE7E7,
            Icon::PanelHeight => 0xE740,
            Icon::IconSize => 0xE8A3,
            Icon::Home => 0xE80F,
            Icon::Globe => 0xE774,
            Icon::Ethernet => 0xE839,
            Icon::Lock => 0xE72E,
            Icon::Contact => 0xE77B,
            Icon::Key => 0xE8D7,
            Icon::Pc => 0xE7F8,
            Icon::Tag => 0xE8EC,
            Icon::NoticeBackground => 0xF136,
            Icon::NoticeInfo => 0xF13F,
            Icon::NoticeSuccess => 0xF13E,
            Icon::NoticeWarning => 0xF13C,
            Icon::NoticeError => 0xF13D,
            Icon::Update => 0xE895,
            Icon::Download => 0xE896,
            Icon::Close => 0xE711,
            Icon::Preview => 0xEC7A,
        }
    }
}

/// The pictures of the Windows 11 volume icon (FR-010). Which one a volume shows is the
/// application's rule, not the design system's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VolumeLevel {
    /// Speaker with a cross.
    Muted,
    /// Speaker without waves.
    Zero,
    /// One wave.
    Low,
    /// Two waves.
    Medium,
    /// Three waves.
    High,
}

impl VolumeLevel {
    /// The icon of this step.
    pub fn icon(self) -> Icon {
        match self {
            VolumeLevel::Muted => Icon::VolumeMute,
            VolumeLevel::Zero => Icon::Volume0,
            VolumeLevel::Low => Icon::Volume1,
            VolumeLevel::Medium => Icon::Volume2,
            VolumeLevel::High => Icon::Volume3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_private_use_glyph() {
        let all = [
            Icon::VolumeMute,
            Icon::Volume0,
            Icon::Volume1,
            Icon::Volume2,
            Icon::Volume3,
            Icon::SoundSettings,
            Icon::Mixer,
            Icon::SoundPanel,
            Icon::Settings,
            Icon::Quit,
            Icon::Microphone,
            Icon::MicrophoneMuted,
            Icon::Calls,
            Icon::ChevronDown,
            Icon::ChevronUp,
            Icon::App,
            Icon::SystemSounds,
            Icon::Devices,
            Icon::Info,
            Icon::Power,
            Icon::Language,
            Icon::Mouse,
            Icon::Speaker,
            Icon::Taskbar,
            Icon::Folder,
            Icon::Document,
            Icon::Grip,
            Icon::Edit,
            Icon::Shown,
            Icon::Hidden,
            Icon::Delete,
            Icon::Notification,
            Icon::PanelHeight,
            Icon::IconSize,
            Icon::Home,
            Icon::Globe,
            Icon::Ethernet,
            Icon::Lock,
            Icon::Contact,
            Icon::Key,
            Icon::Pc,
            Icon::Tag,
            Icon::NoticeBackground,
            Icon::NoticeInfo,
            Icon::NoticeSuccess,
            Icon::NoticeWarning,
            Icon::NoticeError,
            Icon::Update,
            Icon::Download,
            Icon::Close,
            Icon::Preview,
        ];
        for icon in all {
            let code = u32::from(icon.glyph());
            assert!((0xE000..=0xF8FF).contains(&code), "{icon:?}");
        }
    }
}
