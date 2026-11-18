//! What the application gives the flyout to show (already translated), and what the
//! flyout asks the application to do. No audio vocabulary: devices and applications are
//! opaque keys chosen by the application.

use std::rc::Rc;

use crate::backend::Pixels;
use crate::controls::device_icon::DeviceIcon;
use crate::icons::VolumeLevel;

/// An opaque, stable key the application gives to a tile or an application row, and
/// receives back in [`FlyoutAction`]s. Equal keys mean "the same item" across updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey(pub u64);

/// The two views of the flyout (FR-030).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlyoutTab {
    /// Output devices, applications, master volume.
    Output,
    /// Input devices and microphone level.
    Input,
}

/// Which volume control an action is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SliderTarget {
    /// The row at the bottom: master volume (output view) or microphone level (input
    /// view).
    Main,
    /// An application row.
    App(ItemKey),
}

/// What the user asked for through the flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlyoutAction {
    /// Show this view.
    SelectTab(FlyoutTab),
    /// A tile was clicked (also the highlighted one: what that does is the application's rule).
    SelectDevice(ItemKey),
    /// Set this volume, in percent.
    SetVolume(SliderTarget, u8),
    /// Toggle the mute state of this volume.
    ToggleMute(SliderTarget),
    /// Expand or collapse the application list.
    SetAppsExpanded(bool),
    /// Open the Windows sound settings.
    OpenSoundSettings,
    /// Open the BetterVolume settings.
    OpenSettings,
    /// The button of the update line was used (install, or try again).
    InstallUpdate,
    /// The update line was closed.
    DismissUpdate,
}

/// The tab bar (shown only when microphone management is on).
#[derive(Clone, Debug, PartialEq)]
pub struct Tabs {
    /// Name of the bar for screen readers.
    pub name: String,
    /// The view shown.
    pub selected: FlyoutTab,
    /// Label of the output tab.
    pub output: String,
    /// Label of the input tab.
    pub input: String,
}

/// One device tile.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceTile {
    /// Stable key of the device.
    pub key: ItemKey,
    /// Display name.
    pub name: String,
    /// Icon.
    pub icon: DeviceIcon,
    /// Highlighted: the current device (FR-041).
    pub current: bool,
    /// The device used for calls when it differs from the highlighted one: the text of
    /// its marker's tooltip.
    pub calls: Option<String>,
}

/// The device tiles of the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Devices {
    /// Name of the tile group for screen readers.
    pub name: String,
    /// The tiles, in order.
    pub tiles: Vec<DeviceTile>,
    /// Text shown instead of tiles when there is none (FR-047).
    pub empty: String,
}

/// The picture of a volume row's mute button.
#[derive(Clone, Debug, PartialEq)]
pub enum VolumeIcon {
    /// The Windows volume glyph of this step (master volume).
    Level(VolumeLevel),
    /// A microphone, crossed out when muted.
    Microphone,
    /// Windows system sounds.
    SystemSounds,
    /// An application: its icon when it could be loaded (the shell loads it from
    /// [`VolumeControl::icon_source`]), a generic picture otherwise.
    App(Option<Rc<Pixels>>),
}

/// One volume row: mute button, slider, value, and a label above.
#[derive(Clone, Debug, PartialEq)]
pub struct VolumeControl {
    /// Which volume this row controls.
    pub target: SliderTarget,
    /// Text above the row (device or application name).
    pub label: String,
    /// Picture of the mute button.
    pub icon: VolumeIcon,
    /// Where the application icon can be loaded from (application rows).
    pub icon_source: Option<String>,
    /// Volume, in percent.
    pub volume: u8,
    /// Muted.
    pub muted: bool,
    /// Whether the row can be used (FR-047: no device).
    pub enabled: bool,
    /// Screen-reader name of the mute button.
    pub mute_name: String,
    /// Tooltip of the mute button.
    pub mute_tooltip: String,
    /// Screen-reader name of the slider.
    pub slider_name: String,
}

/// The collapsible application list (FR-060 to FR-065).
#[derive(Clone, Debug, PartialEq)]
pub struct AppsSection {
    /// Title of the section.
    pub title: String,
    /// Expanded.
    pub expanded: bool,
    /// The rows (System sounds first), shown when expanded.
    pub rows: Vec<VolumeControl>,
}

/// The footer (FR-033).
#[derive(Clone, Debug, PartialEq)]
pub struct Footer {
    /// Label of the Windows sound settings link.
    pub sound_settings: String,
    /// Name and tooltip of the settings button.
    pub settings: String,
    /// Whether the settings button can be used.
    pub settings_enabled: bool,
}

/// The update line, at the bottom of the flyout above the footer (FR-132).
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateLine {
    /// Its text ("Version 2.1.0 available", "Downloading version 2.1.0… 45 %").
    pub text: String,
    /// Label of its button ("Install", "Try again"), when it has one.
    pub action: Option<String>,
    /// Name and tooltip of its close button, when it can be closed.
    pub close: Option<String>,
}

/// Everything the flyout shows.
#[derive(Clone, Debug, PartialEq)]
pub struct FlyoutContent {
    /// Name of the panel for screen readers.
    pub name: String,
    /// The tab bar, if microphone management is on.
    pub tabs: Option<Tabs>,
    /// The device tiles.
    pub devices: Devices,
    /// The application list (output view only).
    pub apps: Option<AppsSection>,
    /// The row at the bottom (master volume or microphone level).
    pub main: VolumeControl,
    /// The footer.
    pub footer: Footer,
    /// The update line, when a newer version is offered.
    pub update: Option<UpdateLine>,
    /// Volume change per wheel notch, in percent (FR-073).
    pub wheel_step: u8,
    /// How tall the panel is.
    pub height: PanelHeight,
}

/// How tall the flyout is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelHeight {
    /// The whole height of the work area: the panel never changes size, the tiles never
    /// move; the application list stands right above the master volume.
    #[default]
    Full,
    /// As tall as its content, the application list right below the tiles; height
    /// changes are animated.
    Content,
}

impl FlyoutContent {
    /// The volume control with this target.
    pub fn control(&self, target: SliderTarget) -> Option<&VolumeControl> {
        match target {
            SliderTarget::Main => Some(&self.main),
            SliderTarget::App(_) => {
                self.apps.as_ref()?.rows.iter().find(|row| row.target == target)
            }
        }
    }

    pub(crate) fn control_mut(&mut self, target: SliderTarget) -> Option<&mut VolumeControl> {
        match target {
            SliderTarget::Main => Some(&mut self.main),
            SliderTarget::App(_) => {
                self.apps.as_mut()?.rows.iter_mut().find(|row| row.target == target)
            }
        }
    }

    /// The application rows shown (none when the section is collapsed or absent).
    pub fn visible_rows(&self) -> &[VolumeControl] {
        match &self.apps {
            Some(apps) if apps.expanded => &apps.rows,
            _ => &[],
        }
    }

    /// Puts back the application rows `previous` showed and this content no longer has,
    /// at about their former place (an application whose sound moves to
    /// the new output a moment after a switch must not vanish and come back, which would
    /// make the rows jump). Returns whether any row was kept. Nothing is kept across a
    /// tab change or when either list is collapsed.
    pub fn keep_rows_of(&mut self, previous: &FlyoutContent) -> bool {
        let same_view =
            self.tabs.as_ref().map(|t| t.selected) == previous.tabs.as_ref().map(|t| t.selected);
        let (Some(apps), Some(before)) = (self.apps.as_mut(), previous.apps.as_ref()) else {
            return false;
        };
        if !same_view || !apps.expanded || !before.expanded {
            return false;
        }
        let mut kept = false;
        for (index, row) in before.rows.iter().enumerate() {
            if !apps.rows.iter().any(|r| r.target == row.target) {
                apps.rows.insert(index.min(apps.rows.len()), row.clone());
                kept = true;
            }
        }
        kept
    }

    /// Whether `other` has the same structure (same tabs, tiles, sections and rows, in
    /// the same order): only values and texts differ, so the layout does not move.
    pub(crate) fn same_structure(&self, other: &FlyoutContent) -> bool {
        let keys = |c: &FlyoutContent| -> Vec<SliderTarget> {
            c.visible_rows().iter().map(|r| r.target).collect()
        };
        self.tabs.is_some() == other.tabs.is_some()
            && self.tabs.as_ref().map(|t| t.selected) == other.tabs.as_ref().map(|t| t.selected)
            && self.devices.tiles.len() == other.devices.tiles.len()
            && self.devices.tiles.iter().zip(&other.devices.tiles).all(|(a, b)| a.key == b.key)
            && self.apps.is_some() == other.apps.is_some()
            && keys(self) == keys(other)
            && self.height == other.height
            && self.update.as_ref().map(|u| (u.action.is_some(), u.close.is_some()))
                == other.update.as_ref().map(|u| (u.action.is_some(), u.close.is_some()))
    }
}
