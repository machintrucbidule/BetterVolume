//! Device rows of the settings window (FR-085 to FR-089): reordering by drag and by
//! keyboard, inline rename with a text box, the icon picker and the forget confirmation.

use super::content::{DeviceRow, SettingKey, SettingsAction};
use super::layout::{self, ConfirmLayout};
use super::{ids, Part, Picker, Response, SettingsKey, SettingsView, Target};
use crate::access::{AccessNode, AccessRole};
use crate::controls::device_icon::DeviceIcon;
use crate::controls::flyout::TextMeasure;
use crate::tokens::settings as t;
use crate::tokens::typography::BODY;
use crate::{Point, Rect};

/// Text being edited: a rename in progress, or a text field with the focus.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Edit {
    pub key: SettingKey,
    pub text: String,
    /// Caret and selection anchor, in characters.
    pub caret: usize,
    pub anchor: usize,
    /// Horizontal scroll of a text wider than the box.
    pub scroll: f32,
    /// The name was refused (already displayed by another device of the list).
    pub refused: bool,
    /// Longest text, in characters.
    pub limit: usize,
    /// Shown as bullets, never copied (a password).
    pub masked: bool,
    /// Only digits are accepted.
    pub digits: bool,
}

impl Edit {
    /// A rename of a device: the whole name selected.
    fn new(key: SettingKey, text: &str) -> Edit {
        let len = text.chars().count();
        Edit {
            key,
            text: text.to_owned(),
            caret: len,
            anchor: 0,
            scroll: 0.0,
            refused: false,
            limit: t::NAME_MAX_CHARS,
            masked: false,
            digits: false,
        }
    }

    /// The text field of card `key` taking the focus: its whole text selected.
    pub fn field(key: SettingKey, field: &super::content::TextField) -> Edit {
        Edit {
            limit: field.max_chars.max(1),
            masked: field.password,
            digits: field.digits_only,
            ..Edit::new(key, &field.value)
        }
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, index: usize) -> usize {
        self.text.char_indices().nth(index).map_or(self.text.len(), |(b, _)| b)
    }

    /// The selection, as a character range.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    /// What the box shows: the text, or one bullet per character for a password.
    pub fn shown(&self) -> std::borrow::Cow<'_, str> {
        if self.masked {
            std::borrow::Cow::Owned(std::iter::repeat_n(t::PASSWORD_BULLET, self.len()).collect())
        } else {
            std::borrow::Cow::Borrowed(&self.text)
        }
    }

    /// The x offset of the character position `index` from the text start.
    pub fn offset(&self, index: usize, measure: &dyn TextMeasure) -> f32 {
        let shown = self.shown();
        let end = shown.char_indices().nth(index).map_or(shown.len(), |(b, _)| b);
        measure.width(&shown[..end], BODY)
    }

    /// Takes a new text from the application (the field was changed from outside), keeping
    /// the caret within it.
    pub fn replace(&mut self, text: &str) {
        if self.text != text {
            self.text = text.to_owned();
            let len = self.len();
            self.caret = self.caret.min(len);
            self.anchor = self.anchor.min(len);
        }
    }

    /// The character position nearest to `x` (from the text start).
    pub fn index_at(&self, x: f32, measure: &dyn TextMeasure) -> usize {
        (0..=self.len())
            .min_by(|a, b| {
                let da = (self.offset(*a, measure) - x).abs();
                let db = (self.offset(*b, measure) - x).abs();
                da.total_cmp(&db)
            })
            .unwrap_or(0)
    }

    /// Scrolls the text so the caret shows in `text_box`.
    pub fn keep_caret_visible(&mut self, text_box: Rect, measure: &dyn TextMeasure) {
        let (_, pad) = t::TEXT_BOX;
        let room = (text_box.width - 2.0 * pad).max(1.0);
        let caret = self.offset(self.caret, measure);
        if caret - self.scroll > room {
            self.scroll = caret - room;
        } else if caret < self.scroll {
            self.scroll = caret;
        }
        let full = self.offset(self.len(), measure);
        self.scroll = self.scroll.clamp(0.0, (full - room).max(0.0));
    }

    /// Replaces the selection with `text` (cut to the longest text; digits only in a
    /// digits field).
    pub fn insert(&mut self, text: &str) {
        let (start, end) = self.selection();
        let room = self.limit.saturating_sub(self.len() - (end - start));
        let piece: String = text
            .chars()
            .filter(|c| !c.is_control() && (!self.digits || c.is_ascii_digit()))
            .take(room)
            .collect();
        let (a, b) = (self.byte(start), self.byte(end));
        self.text.replace_range(a..b, &piece);
        self.caret = start + piece.chars().count();
        self.anchor = self.caret;
        self.refused = false;
    }

    fn selected_text(&self) -> String {
        let (start, end) = self.selection();
        self.text[self.byte(start)..self.byte(end)].to_owned()
    }

    /// Applies an editing key (arrows, Home/End, Backspace/Delete, select all, copy, cut).
    /// Returns what goes to the clipboard, and whether the key was an editing key. A
    /// password is never copied (WinUI PasswordBox).
    pub fn key(&mut self, input: SettingsKey) -> (Option<String>, bool) {
        let len = self.len();
        let mut clipboard = None;
        match input {
            SettingsKey::Left { select } | SettingsKey::Right { select } => {
                let left = matches!(input, SettingsKey::Left { .. });
                let (start, end) = self.selection();
                self.caret = match (left, select, start != end) {
                    (true, false, true) => start,
                    (false, false, true) => end,
                    (true, _, _) => self.caret.saturating_sub(1),
                    (false, _, _) => (self.caret + 1).min(len),
                };
                if !select {
                    self.anchor = self.caret;
                }
            }
            SettingsKey::Home { select } | SettingsKey::End { select } => {
                self.caret = if matches!(input, SettingsKey::Home { .. }) { 0 } else { len };
                if !select {
                    self.anchor = self.caret;
                }
            }
            SettingsKey::Backspace | SettingsKey::Delete => {
                let (start, end) = self.selection();
                if start == end {
                    if input == SettingsKey::Backspace && start > 0 {
                        self.anchor = start - 1;
                    } else if input == SettingsKey::Delete && end < len {
                        self.anchor = end + 1;
                    }
                }
                self.insert("");
            }
            SettingsKey::SelectAll => {
                self.anchor = 0;
                self.caret = len;
            }
            SettingsKey::Copy | SettingsKey::Cut if !self.masked => {
                let selected = self.selected_text();
                if !selected.is_empty() {
                    clipboard = Some(selected);
                    if input == SettingsKey::Cut {
                        self.insert("");
                    }
                }
            }
            SettingsKey::Copy | SettingsKey::Cut => {}
            _ => return (None, false),
        }
        (clipboard, true)
    }
}

/// A press on a row, which becomes a drag once the pointer moved far enough.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Drag {
    pub key: SettingKey,
    pub start_y: f32,
    pub y: f32,
    pub active: bool,
}

impl Drag {
    pub fn new(key: SettingKey, y: f32) -> Drag {
        Drag { key, start_y: y, y, active: false }
    }

    /// How far the dragged row moved.
    pub fn offset(&self) -> f32 {
        if self.active {
            self.y - self.start_y
        } else {
            0.0
        }
    }
}

impl SettingsView {
    /// The row part under `p` (page area, nothing modal open).
    pub(super) fn row_hit(&self, p: Point) -> Option<Target> {
        let row = self.layout.rows.iter().find(|r| self.layout.on_screen(r.rect).contains(p))?;
        let key = row.key;
        let on = |r: Rect| self.layout.on_screen(r).contains(p);
        let editing = self.edit.as_ref().is_some_and(|e| e.key == key);
        let icon_button = self.content.row(key).is_some_and(DeviceRow::has_icon_button);
        Some(if icon_button && on(row.icon) {
            Target::Row(key, Part::Icon)
        } else if row.rename.is_some_and(on) {
            Target::Row(key, Part::Rename)
        } else if on(row.visibility) {
            Target::Row(key, Part::Visibility)
        } else if row.forget.is_some_and(on) {
            Target::Row(key, Part::Forget)
        } else if editing && on(row.text_box) {
            Target::Row(key, Part::TextBox)
        } else {
            Target::Row(key, Part::Body)
        })
    }

    /// Does what a click (or Enter) on a row button does.
    pub(super) fn activate_row(
        &mut self,
        key: SettingKey,
        part: Part,
        measure: &dyn TextMeasure,
    ) -> Option<SettingsAction> {
        let row = self.content.row(key)?.clone();
        match part {
            Part::Icon if !row.has_icon_button() => None,
            Part::Rename if !row.renamable => None,
            Part::Icon => {
                if self.picker.is_some_and(|p| p.key == key) {
                    self.picker = None;
                    self.focus = Some(Target::Row(key, Part::Icon));
                } else {
                    let highlight = row
                        .chosen_icon()
                        .and_then(|icon| DeviceIcon::ALL.iter().position(|i| *i == icon))
                        .map_or(0, |i| i + 1);
                    self.picker = Some(Picker { key, highlight });
                    self.refresh(measure);
                    self.focus = Some(Target::Cell(highlight));
                    self.reveal(Target::Cell(highlight));
                }
                None
            }
            Part::Rename => {
                let action = if self.edit.as_ref().is_some_and(|e| e.key == key) {
                    // The pencil of the row being renamed commits it.
                    let action = self.finish_edit(false, measure);
                    if self.edit.is_none() {
                        self.focus = Some(Target::Row(key, Part::Rename));
                    }
                    action
                } else {
                    let action = self.finish_edit(true, measure);
                    self.edit = Some(Edit::new(key, &row.name));
                    self.focus = Some(Target::Row(key, Part::TextBox));
                    action
                };
                self.refresh(measure);
                action
            }
            Part::Visibility => Some(SettingsAction::SetHidden(key, !row.hidden)),
            Part::Forget => {
                if !row.forgettable {
                    return None;
                }
                self.confirm = Some(key);
                self.focus = Some(Target::ConfirmCancel);
                None
            }
            Part::Body | Part::TextBox => None,
        }
    }

    /// Chooses picker cell `index` (0 = automatic) and closes the picker.
    pub(super) fn pick(
        &mut self,
        index: usize,
        measure: &dyn TextMeasure,
    ) -> Option<SettingsAction> {
        let picker = self.picker.take()?;
        let row = self.content.row(picker.key)?;
        let icon = index.checked_sub(1).and_then(|i| DeviceIcon::ALL.get(i).copied());
        let changed = icon != row.chosen_icon();
        self.focus = Some(Target::Row(picker.key, Part::Icon));
        self.refresh(measure);
        changed.then_some(SettingsAction::SetIcon(picker.key, icon))
    }

    /// The position of a row in its list, and the list's length.
    fn position(&self, key: SettingKey) -> Option<(usize, usize)> {
        let list = self.content.row(key)?.list;
        let rows: Vec<&DeviceRow> = self.content.list(list).collect();
        Some((rows.iter().position(|r| r.key == key)?, rows.len()))
    }

    /// Alt+Up / Alt+Down: one place up or down in its list (FR-086).
    pub(super) fn move_by(&mut self, key: SettingKey, up: bool) -> Option<SettingsAction> {
        let (at, len) = self.position(key)?;
        let to = if up { at.checked_sub(1)? } else { (at + 1 < len).then_some(at + 1)? };
        Some(SettingsAction::MoveDevice(key, to))
    }

    pub(super) fn drag_move(&mut self, p: Point) -> Response {
        let Some(drag) = self.drag.as_mut() else { return Response::default() };
        drag.y = p.y;
        if !drag.active && (drag.y - drag.start_y).abs() >= t::DRAG_THRESHOLD {
            drag.active = true;
            self.edit = None;
            self.picker = None;
        }
        if drag.active {
            Response::redraw()
        } else {
            Response::default()
        }
    }

    /// Where a drag would drop its row: the position in its list among the other rows.
    pub(super) fn drop_target(&self, drag: &Drag) -> Option<usize> {
        let row = self.content.row(drag.key)?;
        let dragged = self.layout.row(drag.key)?.rect;
        let center = dragged.y + dragged.height / 2.0 + drag.offset();
        let others = self
            .content
            .list(row.list)
            .filter(|r| r.key != drag.key)
            .filter_map(|r| self.layout.row(r.key))
            .filter(|r| r.rect.y + r.rect.height / 2.0 < center)
            .count();
        let (at, _) = self.position(drag.key)?;
        (others != at).then_some(others)
    }

    // ----- Text box -----

    pub(super) fn edit_press(&mut self, key: SettingKey, p: Point, measure: &dyn TextMeasure) {
        let Some(text_box) = self.layout.row(key).map(|r| self.layout.on_screen(r.text_box)) else {
            return;
        };
        let Some(edit) = self.edit.as_mut().filter(|e| e.key == key) else { return };
        let (_, pad) = t::TEXT_BOX;
        let index = edit.index_at(p.x - text_box.x - pad + edit.scroll, measure);
        edit.caret = index;
        edit.anchor = index;
    }

    pub(super) fn edit_drag(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        let Some(Target::Row(key, _)) = self.pressed else { return Response::default() };
        let Some(text_box) = self.layout.row(key).map(|r| self.layout.on_screen(r.text_box)) else {
            return Response::default();
        };
        let Some(edit) = self.edit.as_mut() else { return Response::default() };
        let (_, pad) = t::TEXT_BOX;
        let index = edit.index_at(p.x - text_box.x - pad + edit.scroll, measure);
        if index == edit.caret {
            return Response::default();
        }
        edit.caret = index;
        edit.keep_caret_visible(text_box, measure);
        Response::redraw()
    }

    /// Ends the rename in progress. `leaving`: the focus goes elsewhere, so a refused
    /// name is dropped instead of kept for correction. Returns the rename to apply.
    pub(super) fn finish_edit(
        &mut self,
        leaving: bool,
        measure: &dyn TextMeasure,
    ) -> Option<SettingsAction> {
        let edit = self.edit.take()?;
        let row = self.content.row(edit.key)?;
        let name = edit.text.trim().to_owned();
        if name == row.name {
            return None;
        }
        let taken = self
            .content
            .list(row.list)
            .any(|other| other.key != edit.key && other.name.to_lowercase() == name.to_lowercase());
        if taken && !name.is_empty() {
            if !leaving {
                self.edit = Some(Edit { refused: true, ..edit });
                self.refresh(measure);
            }
            return None;
        }
        Some(SettingsAction::Rename(edit.key, name))
    }

    /// A key while the text box has the focus. `None`: not a text box key.
    pub(super) fn edit_key(
        &mut self,
        key: SettingKey,
        input: SettingsKey,
        measure: &dyn TextMeasure,
    ) -> Option<Response> {
        let text_box = self.layout.row(key).map(|r| self.layout.on_screen(r.text_box))?;
        match input {
            SettingsKey::Enter => {
                let action = self.finish_edit(false, measure);
                if self.edit.is_none() {
                    self.focus = Some(Target::Row(key, Part::Rename));
                    self.refresh(measure);
                    return Some(Response::act_moved(action));
                }
                return Some(Response::redraw());
            }
            SettingsKey::Escape => {
                self.edit = None;
                self.focus = Some(Target::Row(key, Part::Rename));
                self.refresh(measure);
                return Some(Response::moved());
            }
            SettingsKey::Tab { .. } => return None,
            _ => {}
        }
        let edit = self.edit.as_mut().filter(|e| e.key == key)?;
        let was_refused = edit.refused;
        // Space reaches the text as a typed character; other keys do nothing here.
        let (clipboard, _) = edit.key(input);
        edit.keep_caret_visible(text_box, measure);
        // Editing a refused name removes the message (and its line).
        if was_refused && !edit.refused {
            self.refresh(measure);
        }
        Some(Response { redraw: true, clipboard, ..Response::default() })
    }

    /// A character typed (from the keyboard or an input method) while renaming or in a
    /// text field.
    pub fn text_input(&mut self, text: &str, measure: &dyn TextMeasure) -> Response {
        if let Some(response) = self.field_text(text, measure) {
            return response;
        }
        let Some(Target::Row(key, Part::TextBox)) = self.focus else { return Response::default() };
        let Some(edit) = self.edit.as_mut().filter(|e| e.key == key) else {
            return Response::default();
        };
        let was_refused = edit.refused;
        edit.insert(text);
        if was_refused {
            self.refresh(measure);
        }
        if let (Some(edit), Some(text_box)) =
            (self.edit.as_mut(), self.layout.row(key).map(|r| self.layout.on_screen(r.text_box)))
        {
            edit.keep_caret_visible(text_box, measure);
        }
        Response::redraw()
    }

    /// Text pasted from the clipboard while renaming or in a text field (its first
    /// non-empty line).
    pub fn paste(&mut self, text: &str, measure: &dyn TextMeasure) -> Response {
        let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        self.text_input(line, measure)
    }

    /// The text box being edited, for painting: its text, caret, selection and scroll.
    pub(super) fn edit_of(&self, key: SettingKey) -> Option<&Edit> {
        self.edit.as_ref().filter(|e| e.key == key)
    }

    // ----- Icon picker -----

    pub(super) fn picker_key(
        &mut self,
        cell: usize,
        key: SettingsKey,
        measure: &dyn TextMeasure,
    ) -> Option<Response> {
        let columns = self.layout.picker.as_ref()?.columns;
        let count = DeviceIcon::ALL.len() + 1;
        let next = match key {
            SettingsKey::Left { .. } => cell.checked_sub(1),
            SettingsKey::Right { .. } => (cell + 1 < count).then_some(cell + 1),
            SettingsKey::Up => cell.checked_sub(columns),
            SettingsKey::Down => (cell + columns < count).then_some(cell + columns),
            SettingsKey::Home { .. } => Some(0),
            SettingsKey::End { .. } => Some(count - 1),
            SettingsKey::Enter | SettingsKey::Space => {
                let action = self.pick(cell, measure);
                return Some(Response::act_moved(action));
            }
            SettingsKey::Escape => {
                let picker = self.picker.take()?;
                self.focus = Some(Target::Row(picker.key, Part::Icon));
                self.refresh(measure);
                return Some(Response::moved());
            }
            _ => return None,
        };
        Some(match next {
            Some(i) => self.focus_on(Target::Cell(i)),
            None => Response::redraw(),
        })
    }

    // ----- Forget confirmation -----

    pub(super) fn confirm_geometry(&self, measure: &dyn TextMeasure) -> Option<ConfirmLayout> {
        let key = self.confirm?;
        let anchor = self.layout.on_screen(self.layout.row(key)?.forget?);
        let texts = &self.content.devices;
        Some(layout::confirm(
            anchor,
            [&texts.forget_question, &texts.forget_detail, &texts.forget_confirm, &texts.cancel],
            self.layout.size,
            measure,
            self.text_scale,
        ))
    }

    pub(super) fn confirm_key(&mut self, key: SettingsKey, measure: &dyn TextMeasure) -> Response {
        match key {
            SettingsKey::Tab { .. } | SettingsKey::Left { .. } | SettingsKey::Right { .. } => {
                let next = if self.focus == Some(Target::ConfirmOk) {
                    Target::ConfirmCancel
                } else {
                    Target::ConfirmOk
                };
                self.focus_on(next)
            }
            SettingsKey::Enter | SettingsKey::Space => {
                let target = self
                    .focus
                    .filter(|f| matches!(f, Target::ConfirmOk | Target::ConfirmCancel))
                    .unwrap_or(Target::ConfirmCancel);
                let action = self.activate(target, measure);
                self.refresh(measure);
                Response::act_moved(action)
            }
            SettingsKey::Escape => {
                let action = self.activate(Target::ConfirmCancel, measure);
                Response::act_moved(action)
            }
            _ => Response::default(),
        }
    }

    pub(super) fn confirm_node(&self, measure: &dyn TextMeasure) -> Option<AccessNode> {
        let geometry = self.confirm_geometry(measure)?;
        let texts = &self.content.devices;
        let mut group = AccessNode::new(
            ids::CONFIRM,
            AccessRole::Group,
            texts.forget_question.clone(),
            geometry.rect,
        );
        group.help = Some(texts.forget_detail.clone());
        for (id, name, rect) in [
            (ids::CONFIRM_OK, &texts.forget_confirm, geometry.ok),
            (ids::CONFIRM_CANCEL, &texts.cancel, geometry.cancel),
        ] {
            let mut button = AccessNode::new(id, AccessRole::Button, name.clone(), rect);
            button.focusable = true;
            group.children.push(button);
        }
        Some(group)
    }

    // ----- Accessibility of rows -----

    pub(super) fn row_nodes(&self, row: &DeviceRow) -> Vec<AccessNode> {
        let Some(laid) = self.layout.row(row.key) else { return Vec::new() };
        let texts = &self.content.devices;
        let screen = |r: Rect| self.layout.on_screen(r);
        let mut group = AccessNode::new(
            ids::ROW | (row.key.0 << 4),
            AccessRole::Group,
            row.name.clone(),
            screen(laid.rect),
        );
        group.help = Some(format!("{} {}", row.detail, texts.move_help).trim().to_owned());
        let id = |code: u64| ids::ROW | (row.key.0 << 4) | code;
        let button = |code: u64, name: &str, rect: Rect| {
            let mut node =
                AccessNode::new(id(code), AccessRole::Button, name.to_owned(), screen(rect));
            node.focusable = true;
            node.help = Some(row.name.clone());
            node
        };
        if row.has_icon_button() {
            let mut icon = button(1, &texts.change_icon, laid.icon);
            icon.role = AccessRole::Expander;
            icon.expanded = Some(self.picker.is_some_and(|p| p.key == row.key));
            group.children.push(icon);
        }
        if let Some(edit) = self.edit_of(row.key) {
            let mut text = AccessNode::new(
                id(5),
                AccessRole::Edit,
                texts.name_box.clone(),
                screen(laid.text_box),
            );
            text.value = Some(edit.text.clone());
            text.focusable = true;
            if edit.refused {
                text.help = Some(texts.duplicate.clone());
            }
            group.children.push(text);
        }
        if let Some(rename) = laid.rename {
            group.children.push(button(2, &texts.rename, rename));
        }
        let visibility = if row.hidden { &texts.show } else { &texts.hide };
        group.children.push(button(3, visibility, laid.visibility));
        if let Some(forget) = laid.forget {
            group.children.push(button(4, &texts.forget, forget));
        }
        let mut nodes = vec![group];
        if let Some(picker) = self.layout.picker.as_ref().filter(|p| p.key == row.key) {
            let mut list = AccessNode::new(
                ids::PICKER,
                AccessRole::List,
                texts.change_icon.clone(),
                screen(picker.rect),
            );
            for (i, cell) in picker.cells.iter().enumerate() {
                let name = if i == 0 {
                    texts.automatic.clone()
                } else {
                    texts.icon_names.get(i - 1).cloned().unwrap_or_default()
                };
                let mut item = AccessNode::new(
                    ids::CELL | (row.key.0 << 8) | i as u64,
                    AccessRole::ListItem,
                    name,
                    screen(*cell),
                );
                let chosen = row
                    .chosen_icon()
                    .and_then(|c| DeviceIcon::ALL.iter().position(|i| *i == c))
                    .map_or(0, |p| p + 1);
                item.selected = Some(i == chosen);
                item.focusable = true;
                list.children.push(item);
            }
            nodes.push(list);
        }
        nodes
    }
}
