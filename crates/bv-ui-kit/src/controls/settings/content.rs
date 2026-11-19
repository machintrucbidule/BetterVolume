//! What the settings window shows, as described by the application (already translated).

use std::rc::Rc;

use crate::backend::Pixels;
use crate::controls::device_icon::DeviceIcon;
use crate::icons::Icon;

/// Identifies a page of the settings window (opaque to the design system).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageKey(pub u64);

/// Identifies a setting card or a device row (opaque to the design system; unique in its
/// page, below 2^24).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SettingKey(pub u64);

/// One entry of the navigation pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavEntry {
    /// The page it opens.
    pub key: PageKey,
    /// Its icon.
    pub icon: Icon,
    /// Its label.
    pub label: String,
}

/// The control at the right of a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardControl {
    /// Nothing (an information card).
    None,
    /// A toggle switch.
    Toggle {
        /// Current state.
        on: bool,
        /// Text shown next to the switch when on ("On").
        on_label: String,
        /// Text shown next to the switch when off ("Off").
        off_label: String,
    },
    /// A drop-down choice (combo box).
    Choice {
        /// The choices, in order.
        options: Vec<String>,
        /// Index of the current choice.
        selected: usize,
    },
    /// A hyperlink button.
    Link {
        /// Its text.
        label: String,
    },
    /// A single-line text field (WinUI TextBox, or PasswordBox when `password`). Every
    /// change is reported at once ([`SettingsAction::SetText`]); the application echoes
    /// the text back in `value`.
    TextField(TextField),
    /// A push button (reported as [`SettingsAction::Invoke`]).
    Button {
        /// Its text.
        label: String,
        /// Accent (primary) style.
        accent: bool,
    },
}

/// A text field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextField {
    /// The text.
    pub value: String,
    /// Shown in place of an empty text.
    pub placeholder: String,
    /// The text is hidden (one bullet per character), never copied.
    pub password: bool,
    /// Only digits can be typed (a port).
    pub digits_only: bool,
    /// Longest text, in characters.
    pub max_chars: usize,
}

/// How serious a notice is (WinUI InfoBar severities).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    /// Information.
    #[default]
    Info,
    /// Something worked.
    Success,
    /// Something needs attention.
    Caution,
    /// Something failed.
    Critical,
}

/// A notice (WinUI InfoBar): a coloured band with a title, a message and an optional
/// button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// Identifier, reported back when its button is used.
    pub key: SettingKey,
    /// Severity (colour and icon).
    pub severity: Severity,
    /// Title (strong text).
    pub title: String,
    /// Message, after the title.
    pub message: String,
    /// Text of its button, if it has one ([`SettingsAction::Invoke`] of `key`).
    pub action: Option<String>,
}

/// A push button of a button row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    /// Identifier, reported back ([`SettingsAction::Invoke`]).
    pub key: SettingKey,
    /// Its text.
    pub label: String,
    /// Accent (primary) style.
    pub accent: bool,
    /// Whether it can be used.
    pub enabled: bool,
}

/// The question asked when the window is closed with unsaved changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseQuestion {
    /// Title of the dialog.
    pub title: String,
    /// Its text.
    pub text: String,
    /// The button saving, then closing (accent).
    pub save: String,
    /// The button closing without saving.
    pub discard: String,
    /// The button keeping the window open.
    pub cancel: String,
}

/// A question shown at once as a dialog over the window (WinUI ContentDialog), such as
/// the first-run offer to hide the Windows volume icon. Answered with
/// [`SettingsAction::QuestionAnswer`]; closing it any other way (Escape, closing the
/// window) answers the secondary button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// Title of the dialog.
    pub title: String,
    /// Its text.
    pub text: String,
    /// The primary (accent) button.
    pub primary: String,
    /// The other button.
    pub secondary: String,
}

/// A setting card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// Identifier, reported back in actions.
    pub key: SettingKey,
    /// Icon at the left.
    pub icon: Option<Icon>,
    /// Title (one line).
    pub title: String,
    /// Description under the title (wraps).
    pub description: Option<String>,
    /// Control at the right.
    pub control: CardControl,
    /// Whether the control can be used.
    pub enabled: bool,
}

/// The picture at the left of a list row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowPicture {
    /// A device drawing in an icon button that opens the icon picker (FR-088).
    Device {
        /// The icon shown (automatic or chosen).
        shown: DeviceIcon,
        /// The icon chosen by the user, if any (`None` = automatic).
        chosen: Option<DeviceIcon>,
        /// The icon the device would have automatically (shown on the "Automatic" choice).
        automatic: DeviceIcon,
    },
    /// An application icon, not a button (FR-096): its pixels once the shell loaded them
    /// from `source`, a generic picture otherwise.
    App {
        /// The loaded icon.
        pixels: Option<Rc<Pixels>>,
        /// Where the icon can be loaded from (an executable path).
        source: Option<String>,
    },
    /// Windows system sounds, not a button.
    SystemSounds,
}

/// One row of a device or application list (FR-085, FR-096): drag handle, picture, name
/// and details, rename (devices), show/hide, and "forget" when allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    /// Identifier, reported back in actions.
    pub key: SettingKey,
    /// The list it belongs to: rows are reordered within their list only.
    pub list: u32,
    /// The picture at the left.
    pub picture: RowPicture,
    /// Displayed name.
    pub name: String,
    /// Secondary line (a device: Windows name and availability; an application: its
    /// executable and whether it runs).
    pub detail: String,
    /// Hidden from the flyout.
    pub hidden: bool,
    /// The row has a rename button (devices).
    pub renamable: bool,
    /// The row has a forget button (a disconnected device, any application but System
    /// sounds).
    pub forgettable: bool,
}

impl DeviceRow {
    /// The device icon chosen by the user, if this is a device row with one.
    pub fn chosen_icon(&self) -> Option<DeviceIcon> {
        match self.picture {
            RowPicture::Device { chosen, .. } => chosen,
            _ => None,
        }
    }

    /// Whether the picture is an icon button opening the icon picker.
    pub fn has_icon_button(&self) -> bool {
        matches!(self.picture, RowPicture::Device { .. })
    }
}

/// Texts of the device rows and of their editors (already translated).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DeviceTexts {
    /// Name of the icon button (screen readers).
    pub change_icon: String,
    /// Name of the rename button.
    pub rename: String,
    /// Name of the button hiding a device.
    pub hide: String,
    /// Name of the button showing a hidden device.
    pub show: String,
    /// Name of the button forgetting a disconnected device.
    pub forget: String,
    /// Help of a row for screen readers (how to move it with the keyboard).
    pub move_help: String,
    /// Name of the text box of a rename.
    pub name_box: String,
    /// Shown under the text box when the name is already used.
    pub duplicate: String,
    /// First choice of the icon picker.
    pub automatic: String,
    /// Names of the catalogue icons, in the order of [`DeviceIcon::ALL`].
    pub icon_names: Vec<String>,
    /// Question of the forget confirmation.
    pub forget_question: String,
    /// Explanation under the question.
    pub forget_detail: String,
    /// Confirm button.
    pub forget_confirm: String,
    /// Cancel button.
    pub cancel: String,
}

/// One element of a page, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Title of a group of cards.
    Header(String),
    /// A card.
    Card(Card),
    /// A paragraph of secondary text.
    Text(String),
    /// A device row.
    Device(DeviceRow),
    /// A notice.
    Notice(Notice),
    /// A row of push buttons, at the left.
    Buttons(Vec<Button>),
}

/// Everything the settings window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsContent {
    /// Name of the window (screen readers).
    pub name: String,
    /// Name of the navigation list (screen readers).
    pub nav_name: String,
    /// Navigation entries.
    pub nav: Vec<NavEntry>,
    /// The page shown.
    pub page: PageKey,
    /// Its title.
    pub title: String,
    /// Its content.
    pub blocks: Vec<Block>,
    /// Texts of the device rows (pages holding some).
    pub devices: DeviceTexts,
    /// Set while closing would lose unsaved changes: closing asks this first.
    pub close_question: Option<CloseQuestion>,
    /// A question to show at once over the window, while set.
    pub question: Option<Question>,
    /// A card or notice to scroll into view when the content arrives with it (once per
    /// change of this value), such as a notice the window was opened for.
    pub reveal: Option<SettingKey>,
}

impl SettingsContent {
    /// The cards of the page, in order.
    pub fn cards(&self) -> impl Iterator<Item = &Card> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Card(card) => Some(card),
            _ => None,
        })
    }

    /// The card with this key.
    pub fn card(&self, key: SettingKey) -> Option<&Card> {
        self.cards().find(|c| c.key == key)
    }

    /// The device rows of the page, in order.
    pub fn rows(&self) -> impl Iterator<Item = &DeviceRow> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Device(row) => Some(row),
            _ => None,
        })
    }

    /// The device row with this key.
    pub fn row(&self, key: SettingKey) -> Option<&DeviceRow> {
        self.rows().find(|r| r.key == key)
    }

    /// The rows of one list, in order.
    pub fn list(&self, list: u32) -> impl Iterator<Item = &DeviceRow> {
        self.rows().filter(move |r| r.list == list)
    }

    /// The buttons of every button row, in order.
    pub fn buttons(&self) -> impl Iterator<Item = &Button> {
        self.blocks.iter().flat_map(|b| match b {
            Block::Buttons(buttons) => buttons.as_slice(),
            _ => &[],
        })
    }

    /// The notices, in order.
    pub fn notices(&self) -> impl Iterator<Item = &Notice> {
        self.blocks.iter().filter_map(|b| match b {
            Block::Notice(notice) => Some(notice),
            _ => None,
        })
    }

    /// The text field of the card `key`, if it is one.
    pub fn field(&self, key: SettingKey) -> Option<&TextField> {
        match &self.card(key)?.control {
            CardControl::TextField(field) => Some(field),
            _ => None,
        }
    }
}

/// What the user asked for in the settings window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    /// Show this page.
    SelectPage(PageKey),
    /// Turn this toggle on or off.
    Toggle(SettingKey, bool),
    /// Choose this option of a drop-down choice.
    Choose(SettingKey, usize),
    /// Follow this link.
    Invoke(SettingKey),
    /// Move this row to this position of its list (FR-086, FR-098).
    MoveDevice(SettingKey, usize),
    /// Show or hide this device or application in the flyout (FR-085, FR-097).
    SetHidden(SettingKey, bool),
    /// Rename this device; an empty name restores the Windows name (FR-087). The view
    /// only reports names no other row of the list displays.
    Rename(SettingKey, String),
    /// Choose this device's icon; `None` = automatic (FR-088).
    SetIcon(SettingKey, Option<DeviceIcon>),
    /// Forget this row (a disconnected device, FR-089; an application, FR-096), after
    /// confirmation.
    Forget(SettingKey),
    /// The text of this field is now this.
    SetText(SettingKey, String),
    /// Answer to the close question: `true` = save, then close; `false` = close
    /// without saving. ("Cancel" only closes the question.)
    CloseAnswer(bool),
    /// Answer to the [`Question`] shown: `true` = its primary button, `false` = the other
    /// button or a dismissal.
    QuestionAnswer(bool),
}
