//! Metric and typography tokens, in DIPs (1 DIP = 1 pixel at 100 % scaling).
//!
//! Values follow the Windows 11 design resources (WinUI 3 control templates and the shell
//! surfaces they sit next to). Changing a value here restyles every surface; nothing
//! outside this crate may hard-code one (NFR-04).

/// Typography: font families and the Windows 11 type ramp.
pub mod typography {
    /// Text font (optical size for 14 px text and below).
    pub const TEXT_FAMILY: &str = "Segoe UI Variable Text";
    /// Icon font shipped with Windows 11 (the system's own glyphs).
    pub const ICON_FAMILY: &str = "Segoe Fluent Icons";

    /// A type ramp entry.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct TextStyle {
        /// Font size before text scaling.
        pub size: f32,
        /// Line height before text scaling.
        pub line_height: f32,
        /// Weight (400 regular, 600 semibold).
        pub weight: u16,
    }

    /// "Body": menu entries, labels.
    pub const BODY: TextStyle = TextStyle { size: 14.0, line_height: 20.0, weight: 400 };
    /// "Body strong".
    pub const BODY_STRONG: TextStyle = TextStyle { size: 14.0, line_height: 20.0, weight: 600 };
    /// "Caption": secondary lines.
    pub const CAPTION: TextStyle = TextStyle { size: 12.0, line_height: 16.0, weight: 400 };
    /// "Caption", semibold: section titles, the current tile.
    pub const CAPTION_STRONG: TextStyle = TextStyle { size: 12.0, line_height: 16.0, weight: 600 };
    /// "Title": the title of a settings page.
    pub const TITLE: TextStyle = TextStyle { size: 28.0, line_height: 36.0, weight: 600 };
    /// "Subtitle": the title of a dialog (WinUI ContentDialog).
    pub const SUBTITLE: TextStyle = TextStyle { size: 20.0, line_height: 28.0, weight: 600 };
}

/// Popup surfaces (flyout, menu).
pub mod popup {
    /// Corner radius of popups: the radius of the Windows "round" window corners, which
    /// the frame draws (`bv-shell` asks Windows for those corners).
    pub const CORNER_RADIUS: f32 = 8.0;
    /// Whether popups have the Windows round corners (of [`CORNER_RADIUS`]) or square ones.
    pub const ROUND_CORNERS: bool = true;
}

/// The flyout panel (FR-020, FR-023).
pub mod flyout {
    /// Width of the panel.
    pub const WIDTH: f32 = 360.0;
    /// Gap between the panel and the work-area edges (taskbar side and screen side).
    pub const MARGIN: f32 = 12.0;
    /// Horizontal padding of the content.
    pub const PADDING_X: f32 = 16.0;
    /// Space above the first section.
    pub const PADDING_TOP: f32 = 16.0;
    /// Space between the last control and the footer.
    pub const PADDING_BOTTOM: f32 = 8.0;
    /// Space on each side of the line separating two sections.
    pub const SECTION_GAP: f32 = 12.0;
    /// Space between the tab bar and the tiles.
    pub const TABS_GAP: f32 = 12.0;
    /// Height of the footer band.
    pub const FOOTER_HEIGHT: f32 = 48.0;
    /// Inset of the footer buttons from the panel edges.
    pub const FOOTER_INSET: f32 = 8.0;
    /// Height of the area showing "no device" in place of tiles.
    pub const EMPTY_HEIGHT: f32 = 48.0;
    /// Width of the scroll indicator of the application list (FR-023).
    pub const SCROLL_INDICATOR: f32 = 3.0;
}

/// Tab bar (FR-030).
pub mod tabs {
    /// Height of the bar.
    pub const HEIGHT: f32 = 36.0;
    /// Padding between the bar edge and the tabs.
    pub const PADDING: f32 = 3.0;
    /// Corner radius of the bar.
    pub const RADIUS: f32 = 6.0;
    /// Corner radius of a tab.
    pub const TAB_RADIUS: f32 = 4.0;
    /// Selection indicator under the selected tab: width, height.
    pub const PILL: (f32, f32) = (16.0, 3.0);
    /// Size of the tab icon and gap to its label.
    pub const ICON_SIZE: f32 = 16.0;
    /// Gap between the tab icon and its label.
    pub const ICON_GAP: f32 = 8.0;
}

/// Device tiles (FR-040).
pub mod tile {
    /// Most tiles per row.
    pub const PER_ROW: usize = 3;
    /// Gap between two tiles, both directions.
    pub const GAP: f32 = 8.0;
    /// Corner radius.
    pub const RADIUS: f32 = 4.0;
    /// Space above the icon.
    pub const PADDING_TOP: f32 = 12.0;
    /// Space below the label area.
    pub const PADDING_BOTTOM: f32 = 8.0;
    /// Horizontal padding of the label.
    pub const PADDING_X: f32 = 6.0;
    /// Size of the device icon.
    pub const ICON_SIZE: f32 = 24.0;
    /// Gap between the icon and the label.
    pub const ICON_GAP: f32 = 6.0;
    /// Lines of the label before it is ellipsised.
    pub const LABEL_LINES: u32 = 2;
    /// Size of the calls marker glyph, and its inset from the top-right corner.
    pub const MARKER_SIZE: f32 = 12.0;
    /// Inset of the calls marker from the tile's top and right edges.
    pub const MARKER_INSET: f32 = 6.0;
}

/// Volume rows: master volume, microphone level, applications (FR-050, FR-062).
pub mod volume_row {
    /// Height of the row holding the button, the slider and the value.
    pub const HEIGHT: f32 = 40.0;
    /// Size of the mute button (square).
    pub const BUTTON: f32 = 36.0;
    /// Corner radius of the mute button.
    pub const BUTTON_RADIUS: f32 = 4.0;
    /// Size of the glyph in the mute button.
    pub const GLYPH: f32 = 16.0;
    /// Size of an application icon in the mute button.
    pub const APP_ICON: f32 = 20.0;
    /// Diameter of the mute badge on an application icon.
    pub const BADGE: f32 = 14.0;
    /// Gap between the button and the slider.
    pub const SLIDER_GAP: f32 = 12.0;
    /// Width of the numeric value.
    pub const VALUE_WIDTH: f32 = 32.0;
    /// Gap between the slider and the value.
    pub const VALUE_GAP: f32 = 4.0;
    /// Gap between the label line and the row below it.
    pub const LABEL_GAP: f32 = 0.0;
    /// Indent of the master label from the content edge.
    pub const MASTER_LABEL_INDENT: f32 = 4.0;
    /// Vertical space between two application rows.
    pub const ROW_SPACING: f32 = 4.0;
}

/// Slider (FR-051).
pub mod slider {
    /// Height of the hit area.
    pub const HEIGHT: f32 = 32.0;
    /// Thickness of the track.
    pub const TRACK: f32 = 4.0;
    /// Diameter of the thumb.
    pub const THUMB: f32 = 20.0;
    /// Diameter of the thumb's inner disc at rest, under the pointer, and pressed.
    pub const INNER: (f32, f32, f32) = (12.0, 14.0, 10.0);
}

/// Section header of the application list (FR-060).
pub mod section {
    /// Height of the header button.
    pub const HEIGHT: f32 = 32.0;
    /// Corner radius of its highlight.
    pub const RADIUS: f32 = 4.0;
    /// Horizontal padding of its label.
    pub const PADDING_X: f32 = 8.0;
    /// Size of the chevron glyph.
    pub const CHEVRON: f32 = 12.0;
    /// Gap between the header and the first row.
    pub const GAP: f32 = 4.0;
}

/// Footer buttons (FR-033).
pub mod footer {
    /// Height of the footer buttons.
    pub const BUTTON: f32 = 36.0;
    /// Corner radius of their highlight.
    pub const RADIUS: f32 = 4.0;
    /// Horizontal padding of the text button.
    pub const PADDING_X: f32 = 8.0;
    /// Size of the settings glyph.
    pub const GLYPH: f32 = 16.0;
    /// Height of the update line, above the footer buttons (at least: it grows with
    /// the text).
    pub const UPDATE_HEIGHT: f32 = 40.0;
    /// Height of the buttons of the update line (at least; they grow with the text).
    pub const UPDATE_BUTTON: f32 = 32.0;
    /// Gap between the glyph, the text and the buttons of the update line.
    pub const UPDATE_GAP: f32 = 8.0;
    /// Size of the close glyph of the update line.
    pub const CLOSE_GLYPH: f32 = 12.0;
}

/// Tooltips.
pub mod tooltip {
    /// Horizontal padding.
    pub const PADDING_X: f32 = 8.0;
    /// Vertical padding.
    pub const PADDING_Y: f32 = 5.0;
    /// Corner radius.
    pub const RADIUS: f32 = 4.0;
    /// Widest tooltip before its text wraps.
    pub const MAX_WIDTH: f32 = 320.0;
    /// Gap between the tooltip and the control it describes.
    pub const OFFSET: f32 = 8.0;
}

/// The context menu (FR-100).
pub mod menu {
    /// Padding between the menu edge and its first and last entries.
    pub const PADDING_Y: f32 = 4.0;
    /// Horizontal inset of the entry highlights inside the menu.
    pub const ITEM_INSET_X: f32 = 4.0;
    /// Vertical gap between two entries.
    pub const ITEM_GAP: f32 = 2.0;
    /// Height of an entry at 100 % text scaling (grows with the text).
    pub const ITEM_HEIGHT: f32 = 32.0;
    /// Corner radius of an entry highlight.
    pub const ITEM_RADIUS: f32 = 4.0;
    /// Space between the highlight's left edge and the icon.
    pub const ICON_LEFT: f32 = 12.0;
    /// Size of the entry icon.
    pub const ICON_SIZE: f32 = 16.0;
    /// Space between the icon and the label.
    pub const ICON_GAP: f32 = 12.0;
    /// Space after the label, up to the highlight's right edge.
    pub const LABEL_RIGHT: f32 = 24.0;
    /// Total height of a separator (line included).
    pub const SEPARATOR_HEIGHT: f32 = 9.0;
    /// Narrowest menu.
    pub const MIN_WIDTH: f32 = 200.0;
    /// Gap between the menu and the taskbar.
    pub const TASKBAR_GAP: f32 = 8.0;
}

/// Keyboard focus indicator.
pub mod focus {
    /// Width of the outer ring.
    pub const OUTER: f32 = 2.0;
    /// Width of the inner ring.
    pub const INNER: f32 = 1.0;
    /// Distance between the control edge and the outer edge of the indicator.
    pub const MARGIN: f32 = 3.0;
}

/// The settings window (FR-070): Windows 11 Settings layout, navigation on the
/// left, setting cards on the right (WinUI 3 NavigationView, SettingsCard, ToggleSwitch,
/// ComboBox and HyperlinkButton templates).
pub mod settings {
    /// Size of a new window (before it was ever moved), in DIPs.
    pub const DEFAULT_SIZE: (f32, f32) = (980.0, 700.0);
    /// Smallest window client area, in DIPs.
    pub const MIN_SIZE: (f32, f32) = (680.0, 460.0);
    /// Size of the installer's and uninstaller's window (no navigation pane), in
    /// DIPs.
    pub const COMPACT_SIZE: (f32, f32) = (680.0, 460.0);
    /// Width of the navigation pane.
    pub const NAV_WIDTH: f32 = 290.0;
    /// Space above the first navigation entry.
    pub const NAV_PADDING_TOP: f32 = 8.0;
    /// Horizontal inset of the navigation entries in their pane.
    pub const NAV_INSET: f32 = 4.0;
    /// Height of a navigation entry at 100 % text.
    pub const NAV_ITEM_HEIGHT: f32 = 36.0;
    /// Vertical gap between two navigation entries.
    pub const NAV_ITEM_GAP: f32 = 4.0;
    /// Corner radius of a navigation entry highlight.
    pub const NAV_RADIUS: f32 = 4.0;
    /// Left offset of the entry icon inside the entry, its size, and the label offset.
    pub const NAV_ICON: (f32, f32, f32) = (12.0, 16.0, 44.0);
    /// Selection indicator of the current entry: width, height.
    pub const NAV_PILL: (f32, f32) = (3.0, 16.0);
    /// Horizontal padding of a page.
    pub const PAGE_PADDING_X: f32 = 36.0;
    /// Space above the page title.
    pub const PAGE_PADDING_TOP: f32 = 12.0;
    /// Space below the last card.
    pub const PAGE_PADDING_BOTTOM: f32 = 36.0;
    /// Widest page content.
    pub const PAGE_MAX_WIDTH: f32 = 1000.0;
    /// Space between the page title and the first card.
    pub const TITLE_GAP: f32 = 20.0;
    /// Space above a group header (between two groups of cards).
    pub const HEADER_GAP_TOP: f32 = 26.0;
    /// Space between a group header and its first card.
    pub const HEADER_GAP_BOTTOM: f32 = 8.0;
    /// Vertical gap between two cards.
    pub const CARD_GAP: f32 = 4.0;
    /// Smallest height of a card.
    pub const CARD_MIN_HEIGHT: f32 = 68.0;
    /// Padding inside a card: horizontal, vertical.
    pub const CARD_PADDING: (f32, f32) = (16.0, 12.0);
    /// Corner radius of a card.
    pub const CARD_RADIUS: f32 = 4.0;
    /// Size of the card icon, and the gap between it and the title.
    pub const CARD_ICON: (f32, f32) = (20.0, 20.0);
    /// Smallest gap between the card text and its control.
    pub const CARD_CONTROL_GAP: f32 = 16.0;
    /// Width under which a card puts its control under its text (WinUI `SettingsCard`
    /// wrap thresholds): with an icon, without one.
    pub const CARD_WRAP_THRESHOLD: (f32, f32) = (476.0, 286.0);
    /// Gap between the text of a wrapped card and its control.
    pub const CARD_WRAP_GAP: f32 = 8.0;
    /// Toggle switch track: width, height.
    pub const TOGGLE: (f32, f32) = (40.0, 20.0);
    /// Knob diameter of the toggle switch: at rest, under the pointer.
    pub const TOGGLE_KNOB: (f32, f32) = (12.0, 14.0);
    /// Width of the pressed (stretched) knob.
    pub const TOGGLE_KNOB_PRESSED: f32 = 17.0;
    /// Gap between the "On"/"Off" label and the switch.
    pub const TOGGLE_LABEL_GAP: f32 = 12.0;
    /// Height of a combo box at 100 % text.
    pub const COMBO_HEIGHT: f32 = 32.0;
    /// Narrowest combo box.
    pub const COMBO_MIN_WIDTH: f32 = 160.0;
    /// Text padding of a combo box: left, right of the chevron.
    pub const COMBO_PADDING: (f32, f32) = (11.0, 11.0);
    /// Chevron size, and the gap between the text and the chevron.
    pub const COMBO_CHEVRON: (f32, f32) = (12.0, 8.0);
    /// Corner radius of combo boxes and buttons.
    pub const CONTROL_RADIUS: f32 = 4.0;
    /// Padding of the open list around its entries.
    pub const LIST_PADDING: f32 = 4.0;
    /// Height of an entry of the open list at 100 % text.
    pub const LIST_ITEM_HEIGHT: f32 = 36.0;
    /// Horizontal inset of the list entries in the list.
    pub const LIST_INSET: f32 = 4.0;
    /// Corner radius of the open list.
    pub const LIST_RADIUS: f32 = 8.0;
    /// Smallest gap between the open list and the window edges.
    pub const LIST_MARGIN: f32 = 8.0;
    /// Hyperlink button: horizontal padding, height.
    pub const LINK: (f32, f32) = (11.0, 32.0);
    /// Lines scrolled by one wheel notch, in DIPs.
    pub const WHEEL_STEP: f32 = 64.0;
    /// Width of the scroll bar indicator, and of its grab area.
    pub const SCROLL_BAR: (f32, f32) = (3.0, 12.0);
    /// Space under a paragraph of help text.
    pub const TEXT_GAP: f32 = 12.0;
    /// Smallest height of a device row.
    pub const ROW_MIN_HEIGHT: f32 = 64.0;
    /// Space between the row's left edge and the drag handle.
    pub const ROW_PADDING_X: f32 = 8.0;
    /// Size of the drag handle glyph, and the gap after it.
    pub const GRIP: (f32, f32) = (16.0, 8.0);
    /// Size of the icon button of a row, the device icon in it, and the gap after it.
    pub const ROW_ICON: (f32, f32, f32) = (40.0, 24.0, 12.0);
    /// An application row (no button around its picture): size of the application icon,
    /// and of the glyph drawn for System sounds or an application without an icon.
    pub const ROW_APP_ICON: (f32, f32) = (24.0, 20.0);
    /// Opacity of an application icon on a hidden row (its text is in secondary colour).
    pub const HIDDEN_PICTURE_OPACITY: f32 = 0.6;
    /// Size of the other row buttons (square), their glyph, and the gap between them.
    pub const ROW_BUTTON: (f32, f32, f32) = (36.0, 16.0, 4.0);
    /// Height of a text box at 100 % text, and its horizontal padding.
    pub const TEXT_BOX: (f32, f32) = (32.0, 10.0);
    /// Vertical insets in the text box: of the selection band, of the caret.
    pub const TEXT_BOX_INSETS: (f32, f32) = (6.0, 7.0);
    /// Thickness of the text box's bottom line: at rest, focused.
    pub const TEXT_BOX_LINE: (f32, f32) = (1.0, 2.0);
    /// Longest device name, in characters.
    pub const NAME_MAX_CHARS: usize = 64;
    /// Icon picker: cell width and height, padding, gap, icon size.
    pub const PICKER_CELL: (f32, f32) = (88.0, 76.0);
    /// Padding of the icon picker panel, and the gap between cells.
    pub const PICKER_PADDING: (f32, f32) = (12.0, 4.0);
    /// Size of the icons of the picker.
    pub const PICKER_ICON: f32 = 24.0;
    /// Picture choices (a card of tiles, like the Start layout choice of Windows 11):
    /// least tile width, and height of the picture (a strip of the taskbar colour).
    pub const PICTURE_TILE: (f32, f32) = (76.0, 52.0);
    /// Size of the tray icon drawn on a tile (twice the taskbar size at 100 %).
    pub const PICTURE_ICON: f32 = 32.0;
    /// Padding inside a tile, gap between the picture and the label, corner radius of
    /// the picture.
    pub const PICTURE_TILE_INSETS: (f32, f32, f32) = (4.0, 6.0, 4.0);
    /// Room left and right of a tile's label.
    pub const PICTURE_LABEL_PADDING: f32 = 6.0;
    /// Gaps: between two tiles, between two groups side by side, between two rows of
    /// groups (a narrow window wraps them). Three groups of two tiles fit side by side
    /// in the window at its default size.
    pub const PICTURE_GAPS: (f32, f32, f32) = (4.0, 20.0, 16.0);
    /// Gaps: under the card's text, under a group's label.
    pub const PICTURE_TEXT_GAPS: (f32, f32) = (16.0, 8.0);
    /// Confirmation flyout: width, padding.
    pub const CONFIRM: (f32, f32) = (320.0, 16.0);
    /// Push buttons: height at 100 % text, horizontal padding, gap between two.
    pub const BUTTON: (f32, f32, f32) = (32.0, 12.0, 8.0);
    /// Space under a button row followed by more content (a list under its buttons).
    pub const BUTTONS_GAP_AFTER: f32 = 12.0;
    /// Pointer travel before a press on a row becomes a drag.
    pub const DRAG_THRESHOLD: f32 = 4.0;
    /// Width of a text field in a card.
    pub const FIELD_WIDTH: f32 = 260.0;
    /// The character a password field shows for each typed character (WinUI PasswordBox).
    pub const PASSWORD_BULLET: char = '\u{25CF}';
    /// Notice (WinUI InfoBar): smallest height, horizontal padding, vertical padding.
    pub const NOTICE: (f32, f32, f32) = (48.0, 16.0, 14.0);
    /// Severity icon of a notice: size, gap after it.
    pub const NOTICE_ICON: (f32, f32) = (16.0, 12.0);
    /// Gap between a notice's title and its message on one line.
    pub const NOTICE_TITLE_GAP: f32 = 12.0;
    /// Width of a dialog (WinUI ContentDialog), its padding, and the gap above its buttons.
    pub const DIALOG: (f32, f32, f32) = (480.0, 24.0, 24.0);
    /// Corner radius of a dialog.
    pub const DIALOG_RADIUS: f32 = 8.0;
    /// Gap between the dialog title and its text.
    pub const DIALOG_TITLE_GAP: f32 = 12.0;
}

/// The bubble above the taskbar (FR-017, FR-105): metrics measured on the
/// Windows 11 volume indicator.
pub mod bubble {
    /// Height of the volume bubble at 100 % text (it grows with the text).
    pub const HEIGHT: f32 = 48.0;
    /// Width of the volume bubble at 100 % text.
    pub const WIDTH: f32 = 192.0;
    /// Gap between the bubble and the taskbar.
    pub const GAP: f32 = 14.0;
    /// Space between the left and right edges and the content.
    pub const PADDING_X: f32 = 12.0;
    /// Left edge of the volume glyph, and its size.
    pub const GLYPH: (f32, f32) = (13.0, 16.0);
    /// Left edge of the level bar, its length, its thickness, and the gap after it.
    pub const BAR: (f32, f32, f32, f32) = (42.0, 110.0, 4.0, 4.0);
    /// Narrowest room for the number (fits "100" at 100 % text).
    pub const VALUE_MIN_WIDTH: f32 = 24.0;
    /// Device bubble: padding above and below the two text lines.
    pub const DEVICE_PADDING_Y: f32 = 8.0;
    /// Size of the device drawing and the gap between it and the text.
    pub const DEVICE_ICON: (f32, f32) = (24.0, 12.0);
    /// Padding after the text.
    pub const DEVICE_PADDING_RIGHT: f32 = 16.0;
    /// Widest device bubble.
    pub const DEVICE_MAX_WIDTH: f32 = 360.0;
    /// How long the bubble stays after its last change, in milliseconds.
    pub const SHOWN_MS: u32 = 2000;
}
