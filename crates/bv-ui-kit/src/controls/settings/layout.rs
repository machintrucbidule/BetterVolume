//! Placement of everything in the settings window, in DIPs relative to the client area.

use super::content::{Block, CardControl, SettingKey, SettingsContent};
use crate::controls::device_icon::DeviceIcon;
use crate::controls::flyout::TextMeasure;
use crate::tokens::settings as t;
use crate::tokens::typography::{TextStyle, BODY, BODY_STRONG, CAPTION, SUBTITLE, TITLE};
use crate::{Rect, Size};

/// Height of a control holding one line of `style`: its nominal height at 100 % text,
/// growing with the text.
pub(crate) fn grown(nominal: f32, style: TextStyle, text_scale: f32) -> f32 {
    nominal.max(nominal - style.line_height + style.line_height * text_scale).round()
}

/// Where a card and its parts are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CardLayout {
    pub rect: Rect,
    pub icon: Option<Rect>,
    pub title: Rect,
    pub description: Option<(Rect, u32)>,
    /// The whole control (toggle: label and switch; choice: the box; link: the button).
    pub control: Option<Rect>,
    /// The switch itself, for a toggle.
    pub switch: Option<Rect>,
    /// The label of a toggle.
    pub switch_label: Option<Rect>,
}

/// Where a device row and its parts are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RowLayout {
    pub key: SettingKey,
    pub rect: Rect,
    pub grip: Rect,
    pub icon: Rect,
    pub name: Rect,
    pub detail: Rect,
    /// Text box of a rename (in place of the name and details).
    pub text_box: Rect,
    /// Message of a refused name, under the text box.
    pub error: Option<Rect>,
    pub rename: Rect,
    pub visibility: Rect,
    pub forget: Option<Rect>,
}

/// The icon picker panel under a row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PickerLayout {
    pub key: SettingKey,
    pub rect: Rect,
    /// "Automatic" first, then the catalogue in its order.
    pub cells: Vec<Rect>,
    pub columns: usize,
}

/// Interaction state that changes the layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Extras {
    /// The row whose icon picker is open.
    pub picker: Option<SettingKey>,
    /// The row being renamed, and whether its name is refused.
    pub editing: Option<(SettingKey, bool)>,
}

/// Where a notice and its parts are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NoticeLayout {
    pub key: SettingKey,
    pub rect: Rect,
    pub icon: Rect,
    pub title: Rect,
    /// The message, and its number of lines.
    pub message: (Rect, u32),
    pub action: Option<Rect>,
}

/// The laid-out window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layout {
    pub size: Size,
    pub nav: Vec<Rect>,
    pub title: Rect,
    pub headers: Vec<Rect>,
    pub texts: Vec<(Rect, u32)>,
    pub cards: Vec<CardLayout>,
    pub rows: Vec<RowLayout>,
    pub picker: Option<PickerLayout>,
    /// The notices.
    pub notices: Vec<NoticeLayout>,
    /// The buttons of the button rows.
    pub buttons: Vec<(SettingKey, Rect)>,
    /// The page area (right of the navigation pane).
    pub page: Rect,
    /// Height of the whole page content (unscrolled).
    pub content_height: f32,
    /// How far the page is scrolled (set by the view, within `0..=max_scroll`).
    pub scroll: f32,
}

impl Layout {
    /// A page rectangle as it shows in the window.
    pub fn on_screen(&self, r: Rect) -> Rect {
        Rect { y: r.y - self.scroll, ..r }
    }

    /// Sets the scroll offset, kept in range and on whole DIPs.
    pub fn scroll_to(&mut self, scroll: f32) {
        self.scroll = scroll.clamp(0.0, self.max_scroll()).round();
    }

    /// The largest useful scroll offset.
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.size.height).max(0.0)
    }

    /// The scroll bar thumb, when the page scrolls.
    pub fn scroll_thumb(&self) -> Option<Rect> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return None;
        }
        let track = self.size.height;
        let length = (track * track / self.content_height).max(24.0).min(track);
        let y = (track - length) * (self.scroll / max);
        let (_, grab) = t::SCROLL_BAR;
        Some(Rect::new(self.size.width - grab, y, grab, length))
    }

    /// The layout of a row.
    pub fn row(&self, key: SettingKey) -> Option<&RowLayout> {
        self.rows.iter().find(|r| r.key == key)
    }

    /// A button of a button row, or the button of a notice (page coordinates).
    pub fn button(&self, key: SettingKey) -> Option<Rect> {
        self.buttons
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, r)| *r)
            .or_else(|| self.notices.iter().find(|n| n.key == key).and_then(|n| n.action))
    }
}

/// Size of the control of a card.
fn control_size(control: &CardControl, measure: &dyn TextMeasure, text_scale: f32) -> Option<Size> {
    match control {
        CardControl::None => None,
        CardControl::Toggle { on_label, off_label, .. } => {
            let label = measure.width(on_label, BODY).max(measure.width(off_label, BODY)).ceil();
            let (w, _) = t::TOGGLE;
            Some(Size::new(label + t::TOGGLE_LABEL_GAP + w, grown(32.0, BODY, text_scale)))
        }
        CardControl::Choice { options, .. } => {
            let widest = options.iter().map(|o| measure.width(o, BODY)).fold(0.0_f32, f32::max);
            let (left, right) = t::COMBO_PADDING;
            let (chevron, gap) = t::COMBO_CHEVRON;
            let width = (left + widest.ceil() + gap + chevron + right).max(t::COMBO_MIN_WIDTH);
            Some(Size::new(width, grown(t::COMBO_HEIGHT, BODY, text_scale)))
        }
        CardControl::Link { label } => {
            let (pad, height) = t::LINK;
            Some(Size::new(
                measure.width(label, BODY).ceil() + 2.0 * pad,
                grown(height, BODY, text_scale),
            ))
        }
        CardControl::TextField(_) => {
            Some(Size::new(t::FIELD_WIDTH, grown(t::TEXT_BOX.0, BODY, text_scale)))
        }
        CardControl::Button { label, .. } => Some(button_size(label, measure, text_scale)),
    }
}

/// Size of a push button showing `label` (WinUI: 120 DIP at least).
pub(crate) fn button_size(label: &str, measure: &dyn TextMeasure, text_scale: f32) -> Size {
    let (height, pad, _) = t::BUTTON;
    let width = (measure.width(label, BODY).ceil() + 2.0 * pad).max(120.0);
    Size::new(width, grown(height, BODY, text_scale))
}

/// Lays the window out at `size`. Page rectangles are in page coordinates (unscrolled):
/// [`Layout::on_screen`] places them in the window.
pub(crate) fn layout(
    content: &SettingsContent,
    size: Size,
    extras: Extras,
    measure: &dyn TextMeasure,
    text_scale: f32,
) -> Layout {
    let line = |style: TextStyle| (style.line_height * text_scale).round();

    let item_height = grown(t::NAV_ITEM_HEIGHT, BODY, text_scale);
    let nav = (0..content.nav.len())
        .map(|i| {
            Rect::new(
                t::NAV_INSET,
                t::NAV_PADDING_TOP + i as f32 * (item_height + t::NAV_ITEM_GAP),
                t::NAV_WIDTH - 2.0 * t::NAV_INSET,
                item_height,
            )
        })
        .collect();

    // Without navigation entries (the installer's window) the page takes the
    // whole width.
    let nav_width = if content.nav.is_empty() { 0.0 } else { t::NAV_WIDTH };
    let page = Rect::new(nav_width, 0.0, (size.width - nav_width).max(0.0), size.height);
    let x = page.x + t::PAGE_PADDING_X;
    let width = (page.width - 2.0 * t::PAGE_PADDING_X).clamp(1.0, t::PAGE_MAX_WIDTH);

    let mut y = t::PAGE_PADDING_TOP;
    let title = Rect::new(x, y, width, line(TITLE));
    y += title.height + t::TITLE_GAP;
    let mut laid = Layout {
        size,
        nav,
        title,
        headers: Vec::new(),
        texts: Vec::new(),
        cards: Vec::new(),
        rows: Vec::new(),
        picker: None,
        notices: Vec::new(),
        buttons: Vec::new(),
        page,
        content_height: 0.0,
        scroll: 0.0,
    };
    let (pad_x, pad_y) = t::CARD_PADDING;
    let (icon_size, icon_gap) = t::CARD_ICON;
    for (position, block) in content.blocks.iter().enumerate() {
        match block {
            Block::Header(_) => {
                if position > 0 {
                    y += t::HEADER_GAP_TOP - t::CARD_GAP;
                }
                laid.headers.push(Rect::new(x, y, width, line(BODY_STRONG)));
                y += line(BODY_STRONG) + t::HEADER_GAP_BOTTOM;
            }
            Block::Text(text) => {
                let lines = measure.lines(text, BODY, width).clamp(1, 8);
                let height = lines as f32 * line(BODY);
                laid.texts.push((Rect::new(x, y, width, height), lines));
                y += height + t::TEXT_GAP;
            }
            Block::Card(card) => {
                let control = control_size(&card.control, measure, text_scale);
                let text_x =
                    x + pad_x + if card.icon.is_some() { icon_size + icon_gap } else { 0.0 };
                let control_room = control.map_or(0.0, |c| c.width + t::CARD_CONTROL_GAP);
                let text_width = (x + width - pad_x - control_room - text_x).max(1.0);
                let description = card.description.as_ref().map(|d| {
                    let lines = measure.lines(d, CAPTION, text_width).clamp(1, 6);
                    (lines, lines as f32 * line(CAPTION))
                });
                let text_height = line(BODY) + description.map_or(0.0, |(_, h)| h);
                let height = (text_height + 2.0 * pad_y).max(t::CARD_MIN_HEIGHT).round();
                let rect = Rect::new(x, y, width, height);
                let text_top = (y + (height - text_height) / 2.0).round();
                let control_rect = control.map(|c| {
                    Rect::new(
                        rect.right() - pad_x - c.width,
                        (y + (height - c.height) / 2.0).round(),
                        c.width,
                        c.height,
                    )
                });
                let (switch, switch_label) = match (&card.control, control_rect) {
                    (CardControl::Toggle { .. }, Some(r)) => {
                        let (w, h) = t::TOGGLE;
                        let switch =
                            Rect::new(r.right() - w, (r.y + (r.height - h) / 2.0).round(), w, h);
                        let label =
                            Rect::new(r.x, r.y, r.width - w - t::TOGGLE_LABEL_GAP, r.height);
                        (Some(switch), Some(label))
                    }
                    _ => (None, None),
                };
                laid.cards.push(CardLayout {
                    rect,
                    icon: card.icon.map(|_| {
                        Rect::new(
                            x + pad_x,
                            (y + (height - icon_size) / 2.0).round(),
                            icon_size,
                            icon_size,
                        )
                    }),
                    title: Rect::new(text_x, text_top, text_width, line(BODY)),
                    description: description.map(|(lines, h)| {
                        (Rect::new(text_x, text_top + line(BODY), text_width, h), lines)
                    }),
                    control: control_rect,
                    switch,
                    switch_label,
                });
                y += height + t::CARD_GAP;
            }
            Block::Notice(notice) => {
                let (min_height, notice_pad_x, notice_pad_y) = t::NOTICE;
                let (icon, icon_gap) = t::NOTICE_ICON;
                let action = notice.action.as_ref().map(|a| button_size(a, measure, text_scale));
                let text_x = x + notice_pad_x + icon + icon_gap;
                let right = x + width - notice_pad_x - action.map_or(0.0, |a| a.width + 12.0);
                let room = (right - text_x).max(1.0);
                let title_width = measure.width(&notice.title, BODY_STRONG).ceil();
                let message_width = measure.width(&notice.message, BODY).ceil();
                // Title and message on one line when they fit, the message under it
                // otherwise (WinUI InfoBar).
                let inline = title_width + t::NOTICE_TITLE_GAP + message_width <= room;
                let (title, message, text_height) = if inline || notice.message.is_empty() {
                    let message_x = text_x + title_width + t::NOTICE_TITLE_GAP;
                    (
                        Rect::new(text_x, 0.0, title_width.min(room), line(BODY)),
                        (Rect::new(message_x, 0.0, (right - message_x).max(1.0), line(BODY)), 1),
                        line(BODY),
                    )
                } else {
                    let lines = measure.lines(&notice.message, BODY, room).clamp(1, 6);
                    let message_height = lines as f32 * line(BODY);
                    (
                        Rect::new(text_x, 0.0, room, line(BODY)),
                        (Rect::new(text_x, line(BODY), room, message_height), lines),
                        line(BODY) + message_height,
                    )
                };
                let action_height = action.map_or(0.0, |a| a.height);
                let height =
                    (text_height.max(action_height) + 2.0 * notice_pad_y).max(min_height).round();
                let rect = Rect::new(x, y, width, height);
                let text_top = (y + (height - text_height) / 2.0).round();
                let shift = |r: Rect| Rect { y: r.y + text_top, ..r };
                laid.notices.push(NoticeLayout {
                    key: notice.key,
                    rect,
                    icon: Rect::new(
                        x + notice_pad_x,
                        text_top + (line(BODY) - icon) / 2.0,
                        icon,
                        icon,
                    ),
                    title: shift(title),
                    message: (shift(message.0), message.1),
                    action: action.map(|a| {
                        Rect::new(
                            x + width - notice_pad_x - a.width,
                            (y + (height - a.height) / 2.0).round(),
                            a.width,
                            a.height,
                        )
                    }),
                });
                y += height + t::CARD_GAP;
            }
            Block::Buttons(buttons) => {
                y += t::HEADER_GAP_BOTTOM;
                let mut left = x;
                let mut tallest: f32 = 0.0;
                for button in buttons {
                    let size = button_size(&button.label, measure, text_scale);
                    laid.buttons.push((button.key, Rect::new(left, y, size.width, size.height)));
                    left += size.width + t::BUTTON.2;
                    tallest = tallest.max(size.height);
                }
                y += tallest + t::CARD_GAP;
            }
            Block::Device(row) => {
                let editing = extras.editing.filter(|(key, _)| *key == row.key);
                let refused = editing.is_some_and(|(_, refused)| refused);
                let text_box_height = grown(t::TEXT_BOX.0, BODY, text_scale);
                let text_height = if editing.is_some() {
                    text_box_height + if refused { line(CAPTION) } else { 0.0 }
                } else {
                    line(BODY) + line(CAPTION)
                };
                let height = (text_height + 2.0 * pad_y).max(t::ROW_MIN_HEIGHT).round();
                let rect = Rect::new(x, y, width, height);
                let middle = |h: f32| (y + (height - h) / 2.0).round();
                let (grip_size, grip_gap) = t::GRIP;
                let grip = Rect::new(x + t::ROW_PADDING_X, middle(grip_size), grip_size, grip_size);
                let (icon_box, _, icon_after) = t::ROW_ICON;
                let icon = Rect::new(grip.right() + grip_gap, middle(icon_box), icon_box, icon_box);
                let (button, _, button_gap) = t::ROW_BUTTON;
                let visibility =
                    Rect::new(rect.right() - pad_x - button, middle(button), button, button);
                let rename = Rect { x: visibility.x - button_gap - button, ..visibility };
                let forget = (!row.connected)
                    .then_some(Rect { x: rename.x - button_gap - button, ..rename });
                let text_x = icon.right() + icon_after;
                let text_right = forget.unwrap_or(rename).x - t::CARD_CONTROL_GAP;
                let text_width = (text_right - text_x).max(1.0);
                let text_top = middle(text_height);
                laid.rows.push(RowLayout {
                    key: row.key,
                    rect,
                    grip,
                    icon,
                    name: Rect::new(text_x, text_top, text_width, line(BODY)),
                    detail: Rect::new(text_x, text_top + line(BODY), text_width, line(CAPTION)),
                    text_box: Rect::new(text_x, text_top, text_width, text_box_height),
                    error: refused.then(|| {
                        Rect::new(text_x, text_top + text_box_height, text_width, line(CAPTION))
                    }),
                    rename,
                    visibility,
                    forget,
                });
                y += height + t::CARD_GAP;
                if extras.picker == Some(row.key) {
                    let picker = picker(row.key, Rect::new(x, y, width, 0.0), text_scale);
                    y += picker.rect.height + t::CARD_GAP;
                    laid.picker = Some(picker);
                }
            }
        }
    }
    laid.content_height = y - t::CARD_GAP + t::PAGE_PADDING_BOTTOM;
    laid
}

/// The icon picker panel at `top` (its height is computed): "Automatic" then the
/// catalogue, in as many columns as fit.
fn picker(key: SettingKey, top: Rect, text_scale: f32) -> PickerLayout {
    let (cell_w, cell_h) = t::PICKER_CELL;
    let cell_h = grown(cell_h, CAPTION, text_scale);
    let (pad, gap) = t::PICKER_PADDING;
    let count = DeviceIcon::ALL.len() + 1;
    let fit = ((top.width - 2.0 * pad + gap) / (cell_w + gap)).floor().max(1.0) as usize;
    let columns = fit.min(count);
    let rows = count.div_ceil(columns);
    let height = 2.0 * pad + rows as f32 * cell_h + (rows - 1) as f32 * gap;
    let cells = (0..count)
        .map(|i| {
            let (column, line) = (i % columns, i / columns);
            Rect::new(
                top.x + pad + column as f32 * (cell_w + gap),
                top.y + pad + line as f32 * (cell_h + gap),
                cell_w,
                cell_h,
            )
        })
        .collect();
    PickerLayout { key, rect: Rect { height, ..top }, cells, columns }
}

/// Where the open list of a choice goes: the selected entry over the box (WinUI), kept
/// inside the window; a list taller than the window scrolls. Returns the list rectangle
/// and the height of one entry.
pub(crate) fn open_list(
    anchor: Rect,
    count: usize,
    selected: usize,
    window: Size,
    text_scale: f32,
) -> (Rect, f32) {
    let item = grown(t::LIST_ITEM_HEIGHT, BODY, text_scale);
    let natural = count as f32 * item + 2.0 * t::LIST_PADDING;
    let room = (window.height - 2.0 * t::LIST_MARGIN).max(item + 2.0 * t::LIST_PADDING);
    let height = natural.min(room);
    let wanted =
        anchor.y + anchor.height / 2.0 - (t::LIST_PADDING + selected as f32 * item + item / 2.0);
    let top =
        wanted.clamp(t::LIST_MARGIN, (window.height - t::LIST_MARGIN - height).max(t::LIST_MARGIN));
    let width = anchor.width + 2.0 * t::LIST_INSET;
    let x = (anchor.x - t::LIST_INSET).min(window.width - t::LIST_MARGIN - width).max(0.0);
    (Rect::new(x, top.round(), width, height), item)
}

/// The parts of the close dialog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DialogLayout {
    pub rect: Rect,
    pub title: (Rect, u32),
    pub text: (Rect, u32),
    /// The band holding the buttons (darker, WinUI ContentDialog).
    pub footer: Rect,
    /// The buttons, left to right.
    pub buttons: Vec<Rect>,
}

/// A dialog centred in the window (WinUI ContentDialog): title, text, then buttons
/// (their `labels`) sharing the width. The dialog grows (within the window) when its
/// buttons need more room for their labels (long translations, larger text).
pub(crate) fn dialog(
    title: &str,
    text: &str,
    labels: &[&str],
    window: Size,
    measure: &dyn TextMeasure,
    text_scale: f32,
) -> DialogLayout {
    let line = |style: TextStyle| (style.line_height * text_scale).round();
    let (wanted, pad, footer_pad) = t::DIALOG;
    let (_, button_pad, gap) = t::BUTTON;
    let count = labels.len().max(1);
    let widest = labels.iter().map(|l| measure.width(l, BODY)).fold(0.0f32, f32::max);
    let needed =
        count as f32 * (widest + 2.0 * button_pad).ceil() + (count - 1) as f32 * gap + 2.0 * pad;
    let width = wanted.max(needed).min(window.width - 2.0 * t::LIST_MARGIN).max(1.0);
    let inner = (width - 2.0 * pad).max(1.0);

    let title_lines = measure.lines(title, SUBTITLE, inner).clamp(1, 3);
    let text_lines = measure.lines(text, BODY, inner).clamp(1, 8);
    let title_h = title_lines as f32 * line(SUBTITLE);
    let text_h = text_lines as f32 * line(BODY);
    let (button_h, _, _) = t::BUTTON;
    let button_h = grown(button_h, BODY, text_scale);
    let footer_h = button_h + 2.0 * footer_pad;
    let height = pad + title_h + t::DIALOG_TITLE_GAP + text_h + pad + footer_h;
    let x = ((window.width - width) / 2.0).round();
    let y = ((window.height - height) / 2.0).max(t::LIST_MARGIN).round();
    let rect = Rect::new(x, y, width, height);
    let title = Rect::new(x + pad, y + pad, inner, title_h);
    let text = Rect::new(x + pad, title.bottom() + t::DIALOG_TITLE_GAP, inner, text_h);
    let footer = Rect::new(x, rect.bottom() - footer_h, width, footer_h);
    let each = (inner - (count - 1) as f32 * gap) / count as f32;
    let top = footer.y + footer_pad;
    let buttons = (0..count)
        .map(|i| Rect::new(x + pad + i as f32 * (each + gap), top, each, button_h))
        .collect();
    DialogLayout { rect, title: (title, title_lines), text: (text, text_lines), footer, buttons }
}

/// The parts of a confirmation flyout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ConfirmLayout {
    pub rect: Rect,
    pub question: (Rect, u32),
    pub detail: (Rect, u32),
    pub ok: Rect,
    pub cancel: Rect,
}

/// The confirmation flyout of a forget button: below it (above when there is no room),
/// its right edge on the button's, kept inside the window.
pub(crate) fn confirm(
    anchor: Rect,
    texts: [&str; 4],
    window: Size,
    measure: &dyn TextMeasure,
    text_scale: f32,
) -> ConfirmLayout {
    let line = |style: TextStyle| (style.line_height * text_scale).round();
    let (width, pad) = t::CONFIRM;
    let inner = width - 2.0 * pad;
    let [question, detail, _, _] = texts;
    let question_lines = measure.lines(question, BODY_STRONG, inner).clamp(1, 3);
    let detail_lines = measure.lines(detail, BODY, inner).clamp(1, 6);
    let (button_h, _, button_gap) = t::BUTTON;
    let button_h = grown(button_h, BODY, text_scale);
    let question_h = question_lines as f32 * line(BODY_STRONG);
    let detail_h = detail_lines as f32 * line(BODY);
    let height = pad + question_h + 8.0 + detail_h + 16.0 + button_h + pad;
    let x = (anchor.right() - width)
        .min(window.width - t::LIST_MARGIN - width)
        .max(t::LIST_MARGIN.min(window.width - width).max(0.0));
    let below = anchor.bottom() + 4.0;
    let y = if below + height + t::LIST_MARGIN <= window.height {
        below
    } else {
        (anchor.y - 4.0 - height).max(t::LIST_MARGIN)
    };
    let rect = Rect::new(x, y.round(), width, height);
    let question_rect = Rect::new(x + pad, rect.y + pad, inner, question_h);
    let detail_rect = Rect::new(x + pad, question_rect.bottom() + 8.0, inner, detail_h);
    // Two buttons sharing the width equally (WinUI flyout buttons).
    let half = (inner - button_gap) / 2.0;
    let top = detail_rect.bottom() + 16.0;
    let ok_rect = Rect::new(x + pad, top, half, button_h);
    let cancel_rect = Rect::new(ok_rect.right() + button_gap, top, half, button_h);
    ConfirmLayout {
        rect,
        question: (question_rect, question_lines),
        detail: (detail_rect, detail_lines),
        ok: ok_rect,
        cancel: cancel_rect,
    }
}
