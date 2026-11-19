//! The settings window (FR-070): a retained view in the Windows 11 Settings layout.
//!
//! The application describes the page to show ([`SettingsContent`]: navigation entries,
//! a title, group headers, help text, setting cards with a toggle switch, a drop-down
//! choice or a link, and device rows); the view lays it out at the window size, paints
//! it, handles the pointer, the wheel and the keyboard (FR-024), keeps the interaction
//! state across content updates (hover, focus, an open drop-down list, a rename in
//! progress, the icon picker, a confirmation, the scroll position) and reports what the
//! user asked for as [`SettingsAction`]s. It also describes itself for screen readers.
//!
//! Input methods take a [`TextMeasure`]: the text box places its caret with it, and some
//! inputs change the layout (a rename refused, the icon picker opening).
//!
//! The shell hosts the view in a standard resizable window and forwards input to it.

mod content;
mod devices;
mod fields;
mod layout;
mod paint;
#[cfg(test)]
mod tests;

pub use content::{
    Block, Button, Card, CardControl, CloseQuestion, DeviceRow, DeviceTexts, NavEntry, Notice,
    PageKey, Question, RowPicture, SettingKey, SettingsAction, SettingsContent, Severity,
    TextField,
};

use crate::access::{AccessAction, AccessNode, AccessRole};
use crate::backend::Canvas;
use crate::controls::flyout::TextMeasure;
use crate::tokens::settings as t;
use crate::{Point, Rect, Size, Theme};
use devices::{Drag, Edit};
use layout::{Extras, Layout};

/// A key the settings window reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsKey {
    /// Tab (`back`: with Shift).
    Tab {
        /// Shift was held.
        back: bool,
    },
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow (`select`: with Shift, in a text box).
    Left {
        /// Shift was held.
        select: bool,
    },
    /// Right arrow.
    Right {
        /// Shift was held.
        select: bool,
    },
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home {
        /// Shift was held.
        select: bool,
    },
    /// End.
    End {
        /// Shift was held.
        select: bool,
    },
    /// Enter.
    Enter,
    /// Space.
    Space,
    /// Escape.
    Escape,
    /// F4: opens a drop-down list.
    Open,
    /// Alt+Up: moves a device row up; nothing elsewhere.
    AltUp,
    /// Alt+Down: opens a drop-down list, or moves a device row down.
    AltDown,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Ctrl+A.
    SelectAll,
    /// Ctrl+C.
    Copy,
    /// Ctrl+X.
    Cut,
}

/// What the view asks its host to do after an input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Paint again.
    pub redraw: bool,
    /// The user asked for this.
    pub action: Option<SettingsAction>,
    /// Start (`Some(true)`) or stop (`Some(false)`) capturing the pointer.
    pub capture: Option<bool>,
    /// The keyboard focus moved (screen readers are told).
    pub focus_moved: bool,
    /// Put this text on the clipboard (copy, cut).
    pub clipboard: Option<String>,
    /// Closing was asked but the window must stay: the close question is shown.
    pub keep_open: bool,
}

impl Response {
    fn redraw() -> Response {
        Response { redraw: true, ..Response::default() }
    }

    fn act(action: SettingsAction) -> Response {
        Response { redraw: true, action: Some(action), ..Response::default() }
    }

    fn moved() -> Response {
        Response { redraw: true, focus_moved: true, ..Response::default() }
    }

    fn act_moved(action: Option<SettingsAction>) -> Response {
        Response { redraw: true, focus_moved: true, action, ..Response::default() }
    }
}

/// A part of a device row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    /// The row itself (a press there starts a drag).
    Body,
    /// The icon button (opens the icon picker).
    Icon,
    /// The rename button.
    Rename,
    /// The show/hide button.
    Visibility,
    /// The forget button (disconnected devices).
    Forget,
    /// The text box of a rename in progress.
    TextBox,
}

/// What the pointer or the focus is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// A navigation entry, by index.
    Nav(usize),
    /// The control of a card, by key.
    Control(SettingKey),
    /// An entry of the open list, by index.
    Option(usize),
    /// The scroll bar thumb.
    Thumb,
    /// A part of a device row.
    Row(SettingKey, Part),
    /// A cell of the icon picker (0 = automatic, then the catalogue).
    Cell(usize),
    /// The confirm button of the confirmation flyout.
    ConfirmOk,
    /// Its cancel button.
    ConfirmCancel,
    /// "Save" of the close question.
    DialogSave,
    /// "Don't save" of the close question.
    DialogDiscard,
    /// "Cancel" of the close question.
    DialogCancel,
}

/// An open drop-down list.
#[derive(Clone, Copy, Debug, PartialEq)]
struct OpenList {
    key: SettingKey,
    /// Highlighted entry (keyboard).
    highlight: usize,
    /// Scroll offset of a list taller than the window.
    scroll: f32,
}

/// The open icon picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Picker {
    key: SettingKey,
    /// Cell with the keyboard focus.
    highlight: usize,
}

/// The settings view.
#[derive(Clone, Debug)]
pub struct SettingsView {
    content: SettingsContent,
    layout: Layout,
    text_scale: f32,
    hot: Option<Target>,
    pressed: Option<Target>,
    /// Keyboard focus.
    focus: Option<Target>,
    /// The focus indicator shows (after keyboard use).
    keyboard: bool,
    list: Option<OpenList>,
    /// Pointer offset inside the scroll thumb while it is dragged.
    thumb_grab: Option<f32>,
    edit: Option<Edit>,
    picker: Option<Picker>,
    /// The row whose "forget" asks for confirmation.
    confirm: Option<SettingKey>,
    drag: Option<Drag>,
    /// The text field with the focus, being edited.
    field: Option<Edit>,
    /// The dialog shown, if any.
    dialog: Option<Dialog>,
    /// The content's `reveal` last scrolled into view.
    revealed: Option<SettingKey>,
}

/// Which dialog is shown over the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialog {
    /// The close question: save, don't save, cancel.
    Close,
    /// The content's [`Question`]: primary, secondary.
    Ask,
}

/// Accessibility identifiers. Setting keys must stay below 2^24 (they are small enums or
/// indices in the application).
mod ids {
    pub const ROOT: u64 = 1;
    pub const NAV: u64 = 2;
    pub const PAGE: u64 = 3;
    pub const TITLE: u64 = 4;
    pub const CONFIRM: u64 = 5;
    pub const CONFIRM_OK: u64 = 6;
    pub const CONFIRM_CANCEL: u64 = 7;
    pub const PICKER: u64 = 8;
    pub const DIALOG: u64 = 9;
    pub const DIALOG_SAVE: u64 = 10;
    pub const DIALOG_DISCARD: u64 = 11;
    pub const DIALOG_CANCEL: u64 = 12;
    pub const NOTICE: u64 = 6 << 32;
    pub const NAV_ITEM: u64 = 0x10_0000;
    pub const HEADER: u64 = 0x20_0000;
    pub const TEXT: u64 = 0x30_0000;
    pub const CARD: u64 = 1 << 32;
    pub const CONTROL: u64 = 2 << 32;
    pub const OPTION: u64 = 3 << 32;
    pub const ROW: u64 = 4 << 32;
    pub const CELL: u64 = 5 << 32;
}

impl SettingsView {
    /// A view of `content` in a window of `size` DIPs.
    pub fn new(
        content: SettingsContent,
        size: Size,
        measure: &dyn TextMeasure,
        text_scale: f32,
    ) -> SettingsView {
        let layout = layout::layout(&content, size, Extras::default(), measure, text_scale);
        let mut view = SettingsView {
            content,
            layout,
            text_scale,
            hot: None,
            pressed: None,
            focus: None,
            keyboard: false,
            list: None,
            thumb_grab: None,
            edit: None,
            picker: None,
            confirm: None,
            drag: None,
            field: None,
            dialog: None,
            revealed: None,
        };
        view.open_question();
        view.reveal_asked();
        view
    }

    /// Scrolls the card or notice the content asks for into view, once per change.
    fn reveal_asked(&mut self) {
        if self.revealed == self.content.reveal {
            return;
        }
        self.revealed = self.content.reveal;
        let Some(key) = self.revealed else { return };
        let rect = self
            .card_layout(key)
            .map(|c| c.rect)
            .or_else(|| self.layout.notices.iter().find(|n| n.key == key).map(|n| n.rect));
        let Some(rect) = rect else { return };
        let margin = t::CARD_GAP * 3.0;
        let (scroll, height) = (self.layout.scroll, self.layout.size.height);
        if rect.bottom() + margin > scroll + height {
            self.layout.scroll_to(rect.bottom() + margin - height);
        } else if rect.y - margin < scroll {
            self.layout.scroll_to(rect.y - margin);
        }
    }

    /// What the view shows.
    pub fn content(&self) -> &SettingsContent {
        &self.content
    }

    /// The window size the view was laid out for.
    pub fn size(&self) -> Size {
        self.layout.size
    }

    /// How far the page is scrolled (DIPs).
    pub fn scroll(&self) -> f32 {
        self.layout.scroll
    }

    /// Whether a drop-down list is open.
    pub fn list_open(&self) -> bool {
        self.list.is_some()
    }

    /// Whether text is being edited: a device rename or a text field with the focus (the
    /// host then forwards typed characters).
    pub fn editing(&self) -> bool {
        self.edit.is_some() || self.field.is_some()
    }

    fn extras(&self) -> Extras {
        Extras {
            picker: self.picker.map(|p| p.key),
            editing: self.edit.as_ref().map(|e| (e.key, e.refused)),
        }
    }

    /// Replaces the content, keeping the interaction state of the controls that remain.
    /// Showing another page starts at its top.
    pub fn set_content(&mut self, content: SettingsContent, measure: &dyn TextMeasure) {
        let new_page = content.page != self.content.page;
        self.content = content;
        let scroll = if new_page { 0.0 } else { self.layout.scroll };
        if new_page {
            self.list = None;
            self.hot = None;
            self.pressed = None;
            self.edit = None;
            self.picker = None;
            self.confirm = None;
            self.drag = None;
            if !matches!(self.focus, Some(Target::Nav(_))) {
                self.focus = None;
            }
        }
        // Forget the state of rows and cards that went away.
        if self.edit.as_ref().is_some_and(|e| self.content.row(e.key).is_none()) {
            self.edit = None;
        }
        if self.picker.is_some_and(|p| self.content.row(p.key).is_none()) {
            self.picker = None;
        }
        if self.confirm.is_some_and(|k| self.content.row(k).is_none_or(|r| !r.forgettable)) {
            self.confirm = None;
        }
        if self.drag.as_ref().is_some_and(|d| self.content.row(d.key).is_none()) {
            self.drag = None;
        }
        // The application echoes a field's text; a different one (reset after "Save" or
        // "Don't save") replaces what the editor holds.
        if let Some(edit) = self.field.as_mut() {
            match self.content.field(edit.key) {
                Some(field) => edit.replace(&field.value),
                None => self.field = None,
            }
        }
        let gone = match self.dialog {
            Some(Dialog::Close) => self.content.close_question.is_none(),
            Some(Dialog::Ask) => self.content.question.is_none(),
            None => false,
        };
        if gone {
            self.dialog = None;
            self.focus = None;
        }
        self.open_question();
        if let Some(list) = self.list {
            let count = options(&self.content, list.key).map_or(0, <[String]>::len);
            if count == 0 || !usable(&self.content, list.key) {
                self.list = None;
            } else if list.highlight >= count {
                self.list = Some(OpenList { highlight: count - 1, ..list });
            }
        }
        self.relayout(self.layout.size, scroll, measure);
        if let Some(focus @ Target::Row(..)) = self.focus.filter(|_| self.keyboard) {
            self.reveal(focus);
        }
        self.reveal_asked();
        for target in [&mut self.focus, &mut self.hot, &mut self.pressed] {
            if !target.is_some_and(|t| {
                exists(&self.content, &self.layout, self.list, self.edit.as_ref(), t)
            }) {
                *target = None;
            }
        }
        self.sync_field();
    }

    /// The window changed size, or the text scaling changed.
    pub fn resize(&mut self, size: Size, text_scale: f32, measure: &dyn TextMeasure) {
        self.text_scale = text_scale;
        let scroll = self.layout.scroll;
        self.relayout(size, scroll, measure);
    }

    fn relayout(&mut self, size: Size, scroll: f32, measure: &dyn TextMeasure) {
        self.layout = layout::layout(&self.content, size, self.extras(), measure, self.text_scale);
        self.layout.scroll_to(scroll);
        if let Some(list) = self.list.as_mut() {
            list.scroll = 0.0;
        }
        if let Some(list) = self.list {
            self.reveal_option(list.highlight);
        }
        if let Some(edit) = self.edit.as_mut() {
            if let Some(row) = self.layout.row(edit.key) {
                edit.keep_caret_visible(row.text_box, measure);
            }
        }
    }

    /// Lays out again for a change of the interaction state (same size and scroll).
    fn refresh(&mut self, measure: &dyn TextMeasure) {
        let (size, scroll) = (self.layout.size, self.layout.scroll);
        self.relayout(size, scroll, measure);
    }

    fn card_layout(&self, key: SettingKey) -> Option<&layout::CardLayout> {
        let index = self.content.cards().position(|c| c.key == key)?;
        self.layout.cards.get(index)
    }

    /// The index of the navigation entry of the page shown.
    fn current_nav(&self) -> Option<usize> {
        self.content.nav.iter().position(|e| e.key == self.content.page)
    }

    /// The open list's rectangle (window coordinates) and entry height.
    fn list_geometry(&self) -> Option<(OpenList, Rect, f32, usize)> {
        let list = self.list?;
        let anchor = self.layout.on_screen(self.card_layout(list.key)?.control?);
        let (selected, count) = match self.content.card(list.key)?.control {
            CardControl::Choice { ref options, selected } => (selected, options.len()),
            _ => return None,
        };
        let selected = selected.min(count.saturating_sub(1));
        let (rect, item) =
            layout::open_list(anchor, count, selected, self.layout.size, self.text_scale);
        Some((list, rect, item, count))
    }

    /// Scrolls the open list so that entry `index` shows.
    fn reveal_option(&mut self, index: usize) {
        let Some((list, rect, item, count)) = self.list_geometry() else { return };
        let visible = rect.height - 2.0 * t::LIST_PADDING;
        let max = (count as f32 * item - visible).max(0.0);
        let top = index as f32 * item;
        let mut scroll = list.scroll;
        if top < scroll {
            scroll = top;
        } else if top + item > scroll + visible {
            scroll = top + item - visible;
        }
        if let Some(open) = self.list.as_mut() {
            open.scroll = scroll.clamp(0.0, max);
        }
    }

    fn hit(&self, p: Point, measure: &dyn TextMeasure) -> Option<Target> {
        if self.dialog.is_some() {
            return self.dialog_hit(p, measure);
        }
        if let Some((list, rect, item, count)) = self.list_geometry() {
            if !rect.contains(p) {
                return None;
            }
            let offset = p.y - rect.y - t::LIST_PADDING + list.scroll;
            let index = (offset / item).floor();
            return (index >= 0.0 && (index as usize) < count)
                .then_some(Target::Option(index as usize));
        }
        if let Some(confirm) = self.confirm_geometry(measure) {
            return if confirm.ok.contains(p) {
                Some(Target::ConfirmOk)
            } else if confirm.cancel.contains(p) {
                Some(Target::ConfirmCancel)
            } else {
                None
            };
        }
        if self.layout.scroll_thumb().is_some_and(|r| r.contains(p)) {
            return Some(Target::Thumb);
        }
        if let Some(i) = self.layout.nav.iter().position(|r| r.contains(p)) {
            return Some(Target::Nav(i));
        }
        if !self.layout.page.contains(p) {
            return None;
        }
        if let Some(picker) = &self.layout.picker {
            if let Some(i) = picker.cells.iter().position(|c| self.layout.on_screen(*c).contains(p))
            {
                return Some(Target::Cell(i));
            }
        }
        if let Some(target) = self.row_hit(p) {
            return Some(target);
        }
        if let Some(target) = self.button_hit(p) {
            return Some(target);
        }
        self.content.cards().zip(&self.layout.cards).find_map(|(card, laid)| {
            let control = self.layout.on_screen(laid.control?);
            (control.contains(p) && usable(&self.content, card.key))
                .then_some(Target::Control(card.key))
        })
    }

    /// The pointer moved to `p` (window DIPs).
    pub fn pointer_move(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        if let Some(grab) = self.thumb_grab {
            let before = self.layout.scroll;
            if let Some(thumb) = self.layout.scroll_thumb() {
                let travel = (self.layout.size.height - thumb.height).max(1.0);
                let top = (p.y - grab).clamp(0.0, travel);
                self.layout.scroll_to(top / travel * self.layout.max_scroll());
            }
            return if self.layout.scroll != before {
                Response::redraw()
            } else {
                Response::default()
            };
        }
        if self.drag.is_some() {
            return self.drag_move(p);
        }
        if self.pressed.is_some_and(|t| matches!(t, Target::Row(_, Part::TextBox))) {
            return self.edit_drag(p, measure);
        }
        if self.pressing_field() {
            return self.field_drag(p, measure);
        }
        let hot = self.hit(p, measure);
        if hot == self.hot {
            return Response::default();
        }
        self.hot = hot;
        Response::redraw()
    }

    /// The pointer left the window.
    pub fn pointer_leave(&mut self) -> Response {
        if self.hot.is_none() || self.thumb_grab.is_some() || self.drag.is_some() {
            return Response::default();
        }
        self.hot = None;
        Response::redraw()
    }

    /// The primary button went down at `p`.
    pub fn press(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        let response = self.press_inner(p, measure);
        if let Some(Target::Control(key)) = self.pressed.filter(|_| self.focus == self.pressed) {
            if self.content.field(key).is_some() {
                self.field_press(key, p, measure);
                return response;
            }
        }
        self.sync_field();
        response
    }

    fn press_inner(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        let hit = self.hit(p, measure);
        self.keyboard = false;
        if self.dialog.is_some() {
            // The dialog is modal: only its buttons react.
            self.pressed = hit;
            return Response { capture: hit.map(|_| true), ..Response::redraw() };
        }
        if self.list.is_some() && !matches!(hit, Some(Target::Option(_))) {
            // A click outside the open list only closes it.
            self.list = None;
            self.hot = self.hit(p, measure);
            return Response::redraw();
        }
        if self.confirm.is_some() && !matches!(hit, Some(Target::ConfirmOk | Target::ConfirmCancel))
        {
            // A click outside the confirmation cancels it, and only that.
            let key = self.confirm.take();
            self.focus = key.map(|k| Target::Row(k, Part::Forget));
            self.hot = self.hit(p, measure);
            return Response::moved();
        }
        let mut response = Response::redraw();
        // Leaving a rename in progress commits it (a refused name is dropped). Its own
        // pencil commits it on release instead (see `activate_row`).
        let mut edit_ended = false;
        if let Some(edit) = &self.edit {
            let own = [Target::Row(edit.key, Part::TextBox), Target::Row(edit.key, Part::Rename)];
            if !hit.is_some_and(|h| own.contains(&h)) {
                response.action = self.finish_edit(true, measure);
                edit_ended = true;
            }
        }
        self.pressed = hit;
        match hit {
            Some(Target::Thumb) => {
                let top = self.layout.scroll_thumb().map_or(0.0, |r| r.y);
                self.thumb_grab = Some(p.y - top);
            }
            Some(Target::Row(key, Part::Body)) => {
                self.drag = Some(Drag::new(key, p.y));
            }
            Some(Target::Row(key, Part::TextBox)) => {
                self.edit_press(key, p, measure);
                if self.focus != Some(Target::Row(key, Part::TextBox)) {
                    self.focus = Some(Target::Row(key, Part::TextBox));
                    response.focus_moved = true;
                }
            }
            Some(
                target @ (Target::Nav(_) | Target::Control(_) | Target::Row(..) | Target::Cell(_)),
            ) => {
                if self.focus != Some(target) {
                    self.focus = Some(target);
                    response.focus_moved = true;
                }
            }
            Some(
                Target::Option(_)
                | Target::ConfirmOk
                | Target::ConfirmCancel
                | Target::DialogSave
                | Target::DialogDiscard
                | Target::DialogCancel,
            )
            | None => {}
        }
        if hit.is_some() {
            response.capture = Some(true);
        }
        if edit_ended {
            self.refresh(measure);
        }
        response
    }

    /// The primary button went up at `p`.
    pub fn release(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        let pressed = self.pressed.take();
        let grabbed = self.thumb_grab.take().is_some();
        let mut response = Response { capture: Some(false), ..Response::default() };
        if grabbed {
            self.hot = self.hit(p, measure);
            response.redraw = true;
            return response;
        }
        if let Some(drag) = self.drag.take() {
            response.redraw = true;
            if drag.active {
                response.action =
                    self.drop_target(&drag).map(|to| SettingsAction::MoveDevice(drag.key, to));
                self.hot = self.hit(p, measure);
                return response;
            }
        }
        let Some(target) = pressed else { return response };
        response.redraw = true;
        if matches!(target, Target::Row(_, Part::TextBox | Part::Body)) {
            return response;
        }
        if self.hit(p, measure) == Some(target) {
            let focus_before = self.focus;
            response.action = self.activate(target, measure);
            response.focus_moved = self.focus != focus_before;
        }
        response
    }

    /// The pointer was taken away (capture lost): nothing pressed any more.
    pub fn cancel_pointer(&mut self) -> Response {
        let had = self.pressed.take().is_some()
            | self.thumb_grab.take().is_some()
            | self.drag.take().is_some();
        Response { redraw: had, ..Response::default() }
    }

    /// Does what a click (or Enter) on `target` does.
    fn activate(&mut self, target: Target, measure: &dyn TextMeasure) -> Option<SettingsAction> {
        match target {
            Target::Nav(i) => {
                let entry = self.content.nav.get(i)?;
                (entry.key != self.content.page).then_some(SettingsAction::SelectPage(entry.key))
            }
            Target::Control(key) if self.content.card(key).is_none() => {
                fields::button_usable(&self.content, key).then_some(SettingsAction::Invoke(key))
            }
            Target::Control(key) => match &self.content.card(key)?.control {
                CardControl::Toggle { on, .. } => Some(SettingsAction::Toggle(key, !on)),
                CardControl::Choice { selected, .. } => {
                    self.list = Some(OpenList { key, highlight: *selected, scroll: 0.0 });
                    self.reveal_option(*selected);
                    None
                }
                CardControl::Link { .. } => Some(SettingsAction::Invoke(key)),
                CardControl::None | CardControl::TextField(_) => None,
                CardControl::Button { .. } => Some(SettingsAction::Invoke(key)),
            },
            Target::Option(i) => self.choose(i),
            Target::Row(key, part) => self.activate_row(key, part, measure),
            Target::Cell(i) => self.pick(i, measure),
            Target::ConfirmOk => {
                let key = self.confirm.take()?;
                self.focus = None;
                Some(SettingsAction::Forget(key))
            }
            Target::ConfirmCancel => {
                let key = self.confirm.take()?;
                self.focus = Some(Target::Row(key, Part::Forget));
                None
            }
            Target::Thumb => None,
            Target::DialogSave | Target::DialogDiscard | Target::DialogCancel => {
                self.dialog_activate(target)
            }
        }
    }

    /// Picks entry `index` of the open list and closes it.
    fn choose(&mut self, index: usize) -> Option<SettingsAction> {
        let list = self.list.take()?;
        let CardControl::Choice { selected, .. } = self.content.card(list.key)?.control else {
            return None;
        };
        self.focus = Some(Target::Control(list.key));
        (index != selected).then_some(SettingsAction::Choose(list.key, index))
    }

    /// The wheel turned by `notches` (positive = away from the user) at `p`.
    pub fn wheel(&mut self, p: Point, notches: i32, measure: &dyn TextMeasure) -> Response {
        if self.dialog.is_some() {
            return Response::default();
        }
        if let Some((list, rect, item, count)) = self.list_geometry() {
            if rect.contains(p) {
                let visible = rect.height - 2.0 * t::LIST_PADDING;
                let max = (count as f32 * item - visible).max(0.0);
                let scroll = (list.scroll - notches as f32 * item).clamp(0.0, max);
                if let Some(open) = self.list.as_mut() {
                    open.scroll = scroll;
                }
                self.hot = self.hit(p, measure);
                return Response::redraw();
            }
            return Response::default();
        }
        if self.confirm.is_some() {
            return Response::default();
        }
        let before = self.layout.scroll;
        self.layout.scroll_to(before - notches as f32 * t::WHEEL_STEP);
        if self.layout.scroll == before {
            return Response::default();
        }
        if self.drag.is_none() {
            self.hot = self.hit(p, measure);
        }
        Response::redraw()
    }

    /// The focus stops, in Tab order: the navigation list (one stop), every usable card
    /// control, and the controls of each device row (the icon picker as one stop after
    /// its row's icon button).
    fn stops(&self) -> Vec<Target> {
        let nav = match self.focus {
            Some(Target::Nav(i)) => Some(i),
            _ => self.current_nav().or((!self.content.nav.is_empty()).then_some(0)),
        };
        let mut stops: Vec<Target> = nav.map(Target::Nav).into_iter().collect();
        for block in &self.content.blocks {
            match block {
                Block::Card(card) if usable(&self.content, card.key) => {
                    stops.push(Target::Control(card.key));
                }
                Block::Buttons(buttons) => {
                    stops.extend(
                        buttons.iter().filter(|b| b.enabled).map(|b| Target::Control(b.key)),
                    );
                }
                Block::Notice(notice) if notice.action.is_some() => {
                    stops.push(Target::Control(notice.key));
                }
                Block::Device(row) => {
                    let key = row.key;
                    if row.has_icon_button() {
                        stops.push(Target::Row(key, Part::Icon));
                    }
                    if let Some(picker) = self.picker.filter(|p| p.key == key) {
                        stops.push(Target::Cell(picker.highlight));
                    }
                    if self.edit.as_ref().is_some_and(|e| e.key == key) {
                        stops.push(Target::Row(key, Part::TextBox));
                    }
                    if row.renamable {
                        stops.push(Target::Row(key, Part::Rename));
                    }
                    stops.push(Target::Row(key, Part::Visibility));
                    if row.forgettable {
                        stops.push(Target::Row(key, Part::Forget));
                    }
                }
                _ => {}
            }
        }
        stops
    }

    /// Scrolls the page so the focused control shows.
    fn reveal(&mut self, target: Target) {
        let rect = match target {
            Target::Control(key) => self.card_layout(key).map(|c| c.rect),
            Target::Row(key, _) => self.layout.row(key).map(|r| r.rect),
            Target::Cell(i) => self.layout.picker.as_ref().and_then(|p| p.cells.get(i).copied()),
            _ => None,
        };
        let Some(rect) = rect else { return };
        let margin = t::CARD_GAP * 3.0;
        let (scroll, height) = (self.layout.scroll, self.layout.size.height);
        if rect.y - margin < scroll {
            self.layout.scroll_to(rect.y - margin);
        } else if rect.bottom() + margin > scroll + height {
            self.layout.scroll_to(rect.bottom() + margin - height);
        }
    }

    fn focus_on(&mut self, target: Target) -> Response {
        self.focus = Some(target);
        if let Target::Cell(i) = target {
            if let Some(picker) = self.picker.as_mut() {
                picker.highlight = i;
            }
        }
        self.reveal(target);
        Response::moved()
    }

    /// A key was pressed while the window has the keyboard focus.
    pub fn key(&mut self, key: SettingsKey, measure: &dyn TextMeasure) -> Response {
        let response = self.key_inner(key, measure);
        self.sync_field();
        response
    }

    fn key_inner(&mut self, key: SettingsKey, measure: &dyn TextMeasure) -> Response {
        self.keyboard = true;
        if self.dialog.is_some() {
            return self.dialog_key(key);
        }
        if key == SettingsKey::Escape && self.drag.take().is_some() {
            return Response::redraw();
        }
        if let Some(Target::Control(field)) = self.focus.filter(|_| self.field.is_some()) {
            if let Some(response) = self.field_key(field, key, measure) {
                return response;
            }
        }
        if let Some(list) = self.list {
            return self.list_key(list, key, measure);
        }
        if self.confirm.is_some() {
            return self.confirm_key(key, measure);
        }
        if let Some(Target::Row(row, Part::TextBox)) = self.focus {
            if let Some(response) = self.edit_key(row, key, measure) {
                return response;
            }
        }
        if let Some(Target::Cell(i)) = self.focus {
            if let Some(response) = self.picker_key(i, key, measure) {
                return response;
            }
        }
        let page = self.layout.size.height * 0.9;
        match (key, self.focus) {
            (SettingsKey::Tab { back }, focus) => self.tab(focus, back, measure),
            (SettingsKey::AltUp | SettingsKey::AltDown, Some(Target::Row(row, _))) => {
                let up = key == SettingsKey::AltUp;
                Response { action: self.move_by(row, up), ..Response::redraw() }
            }
            (SettingsKey::Up | SettingsKey::Left { .. }, Some(Target::Nav(i))) if i > 0 => {
                self.focus_on(Target::Nav(i - 1))
            }
            (SettingsKey::Down | SettingsKey::Right { .. }, Some(Target::Nav(i)))
                if i + 1 < self.content.nav.len() =>
            {
                self.focus_on(Target::Nav(i + 1))
            }
            (SettingsKey::Home { .. }, Some(Target::Nav(_))) => self.focus_on(Target::Nav(0)),
            (SettingsKey::End { .. }, Some(Target::Nav(_))) if !self.content.nav.is_empty() => {
                self.focus_on(Target::Nav(self.content.nav.len() - 1))
            }
            (
                SettingsKey::Enter | SettingsKey::Space,
                Some(target @ (Target::Nav(_) | Target::Row(..))),
            ) => {
                let before = self.focus;
                let action = self.activate(target, measure);
                self.refresh(measure);
                if let Some(focus) = self.focus.filter(|f| Some(*f) != before) {
                    self.reveal(focus);
                    return Response::act_moved(action);
                }
                Response { action, ..Response::redraw() }
            }
            (
                SettingsKey::Enter | SettingsKey::Space | SettingsKey::Open | SettingsKey::AltDown,
                Some(Target::Control(k)),
            ) => {
                let open = matches!(key, SettingsKey::Open | SettingsKey::AltDown);
                let is_choice = matches!(
                    self.content.card(k).map(|c| &c.control),
                    Some(CardControl::Choice { .. })
                );
                if open && !is_choice {
                    return Response::default();
                }
                match self.activate(Target::Control(k), measure) {
                    Some(action) => Response::act(action),
                    None => Response::redraw(),
                }
            }
            (SettingsKey::Up | SettingsKey::Down, Some(Target::Control(k))) => {
                // A closed choice steps through its options, like a WinUI combo box.
                let Some(CardControl::Choice { options, selected }) =
                    self.content.card(k).map(|c| &c.control)
                else {
                    let step = if key == SettingsKey::Up { -t::WHEEL_STEP } else { t::WHEEL_STEP };
                    return self.scroll_by(step);
                };
                let next = if key == SettingsKey::Up {
                    selected.saturating_sub(1)
                } else {
                    (selected + 1).min(options.len().saturating_sub(1))
                };
                if next == *selected {
                    Response::redraw()
                } else {
                    Response::act(SettingsAction::Choose(k, next))
                }
            }
            (SettingsKey::Up, _) => self.scroll_by(-t::WHEEL_STEP),
            (SettingsKey::Down, _) => self.scroll_by(t::WHEEL_STEP),
            (SettingsKey::PageUp, _) => self.scroll_by(-page),
            (SettingsKey::PageDown, _) => self.scroll_by(page),
            (SettingsKey::Home { .. }, _) => self.scroll_by(-self.layout.content_height),
            (SettingsKey::End { .. }, _) => self.scroll_by(self.layout.content_height),
            _ => Response::redraw(),
        }
    }

    /// Tab / Shift+Tab: the next or previous focus stop (a rename in progress is
    /// committed first).
    fn tab(&mut self, focus: Option<Target>, back: bool, measure: &dyn TextMeasure) -> Response {
        let stops = self.stops();
        if stops.is_empty() {
            return Response::redraw();
        }
        let at = focus.and_then(|f| stops.iter().position(|s| *s == f));
        let next = match (at, back) {
            (None, false) => 0,
            (None, true) => stops.len() - 1,
            (Some(i), false) => (i + 1) % stops.len(),
            (Some(i), true) => (i + stops.len() - 1) % stops.len(),
        };
        let target = stops[next];
        let mut action = None;
        if let Some(edit) = &self.edit {
            if target != Target::Row(edit.key, Part::TextBox) {
                action = self.finish_edit(true, measure);
                self.refresh(measure);
            }
        }
        let mut response = self.focus_on(target);
        response.action = action;
        response
    }

    fn scroll_by(&mut self, delta: f32) -> Response {
        let before = self.layout.scroll;
        self.layout.scroll_to(before + delta);
        Response::redraw()
    }

    fn list_key(
        &mut self,
        list: OpenList,
        key: SettingsKey,
        measure: &dyn TextMeasure,
    ) -> Response {
        let count = options(&self.content, list.key).map_or(0, <[String]>::len);
        let last = count.saturating_sub(1);
        let visible = self.list_geometry().map_or(1, |(_, rect, item, _)| {
            ((rect.height - 2.0 * t::LIST_PADDING) / item).floor().max(1.0) as usize
        });
        let highlight = match key {
            SettingsKey::Up | SettingsKey::Left { .. } => list.highlight.saturating_sub(1),
            SettingsKey::Down | SettingsKey::Right { .. } => (list.highlight + 1).min(last),
            SettingsKey::PageUp => list.highlight.saturating_sub(visible),
            SettingsKey::PageDown => (list.highlight + visible).min(last),
            SettingsKey::Home { .. } => 0,
            SettingsKey::End { .. } => last,
            SettingsKey::Enter | SettingsKey::Space => {
                return Response::act_moved(self.choose(list.highlight));
            }
            SettingsKey::Escape | SettingsKey::Open | SettingsKey::AltUp | SettingsKey::AltDown => {
                self.list = None;
                return Response::moved();
            }
            SettingsKey::Tab { .. } => {
                self.list = None;
                return self.key(key, measure);
            }
            _ => return Response::default(),
        };
        self.list = Some(OpenList { highlight, ..list });
        self.reveal_option(highlight);
        Response::moved()
    }

    /// The window is closing: a rename in progress is committed (a refused name is
    /// dropped), as when leaving the box.
    pub fn end_edit(&mut self, measure: &dyn TextMeasure) -> Response {
        let action = self.finish_edit(true, measure);
        Response { action, ..Response::default() }
    }

    /// The window lost the keyboard focus: the open list closes; a rename in progress
    /// stays (the user may come back to it).
    pub fn focus_lost(&mut self) -> Response {
        let had =
            self.list.take().is_some() | self.pressed.take().is_some() | self.drag.take().is_some();
        self.thumb_grab = None;
        Response { redraw: had, ..Response::default() }
    }

    /// Paints the whole window.
    pub fn paint(&self, canvas: &mut Canvas<'_>, theme: &Theme, measure: &dyn TextMeasure) {
        paint::paint(self, canvas, theme, measure);
    }

    /// The accessibility tree of the window (FR-024).
    pub fn access_tree(&self, measure: &dyn TextMeasure) -> AccessNode {
        let full = Rect::from_size(self.layout.size);
        let mut root =
            AccessNode::new(ids::ROOT, AccessRole::Pane, self.content.name.clone(), full);
        let mut nav = AccessNode::new(
            ids::NAV,
            AccessRole::List,
            self.content.nav_name.clone(),
            Rect::new(0.0, 0.0, t::NAV_WIDTH, full.height),
        );
        for (i, (entry, rect)) in self.content.nav.iter().zip(&self.layout.nav).enumerate() {
            let mut item = AccessNode::new(
                ids::NAV_ITEM + i as u64,
                AccessRole::ListItem,
                entry.label.clone(),
                *rect,
            );
            item.focusable = true;
            item.selected = Some(entry.key == self.content.page);
            nav.children.push(item);
        }
        if !self.content.nav.is_empty() {
            root.children.push(nav);
        }

        let mut page = AccessNode::new(
            ids::PAGE,
            AccessRole::Group,
            self.content.title.clone(),
            self.layout.page,
        );
        page.children.push(AccessNode::new(
            ids::TITLE,
            AccessRole::Text,
            self.content.title.clone(),
            self.layout.on_screen(self.layout.title),
        ));
        let mut headers = self.layout.headers.iter();
        let mut texts = self.layout.texts.iter();
        let mut cards = self.layout.cards.iter();
        for (index, block) in self.content.blocks.iter().enumerate() {
            match block {
                Block::Header(text) | Block::Text(text) => {
                    let (rect, base) = if matches!(block, Block::Header(_)) {
                        (headers.next().copied(), ids::HEADER)
                    } else {
                        (texts.next().map(|(r, _)| *r), ids::TEXT)
                    };
                    if let Some(rect) = rect {
                        page.children.push(AccessNode::new(
                            base + index as u64,
                            AccessRole::Text,
                            text.clone(),
                            self.layout.on_screen(rect),
                        ));
                    }
                }
                Block::Card(card) => {
                    if let Some(laid) = cards.next() {
                        page.children.push(self.card_node(card, laid));
                    }
                }
                Block::Device(row) => {
                    page.children.extend(self.row_nodes(row));
                }
                Block::Notice(_) | Block::Buttons(_) => {
                    page.children.extend(self.button_nodes(block));
                }
            }
        }
        root.children.push(page);
        if let Some(node) = self.confirm_node(measure) {
            root.children.push(node);
        }
        if let Some(node) = self.dialog_node(measure) {
            root.children.push(node);
        }
        root
    }

    fn card_node(&self, card: &Card, laid: &layout::CardLayout) -> AccessNode {
        let mut group = AccessNode::new(
            ids::CARD | (card.key.0 << 4),
            AccessRole::Group,
            card.title.clone(),
            self.layout.on_screen(laid.rect),
        );
        group.help = card.description.clone();
        let Some(bounds) = laid.control.map(|r| self.layout.on_screen(r)) else {
            group.role = AccessRole::Text;
            return group;
        };
        let id = ids::CONTROL | (card.key.0 << 4);
        let mut node = match &card.control {
            CardControl::Toggle { on, .. } => {
                let mut n =
                    AccessNode::new(id, AccessRole::ToggleButton, card.title.clone(), bounds);
                n.toggled = Some(*on);
                n
            }
            CardControl::Choice { options, selected } => {
                let mut n = AccessNode::new(id, AccessRole::ComboBox, card.title.clone(), bounds);
                let open = self.list.filter(|l| l.key == card.key);
                n.expanded = Some(open.is_some());
                let geometry = open.and(self.list_geometry());
                for (i, option) in options.iter().enumerate() {
                    let rect = geometry.map_or(bounds, |(list, r, item, _)| {
                        Rect::new(
                            r.x,
                            r.y + t::LIST_PADDING + i as f32 * item - list.scroll,
                            r.width,
                            item,
                        )
                    });
                    let mut o = AccessNode::new(
                        ids::OPTION | (card.key.0 << 8) | i as u64,
                        AccessRole::ListItem,
                        option.clone(),
                        rect,
                    );
                    o.selected = Some(i == *selected);
                    o.focusable = open.is_some();
                    n.children.push(o);
                }
                n
            }
            CardControl::Link { label } => {
                AccessNode::new(id, AccessRole::Hyperlink, label.clone(), bounds)
            }
            CardControl::Button { label, .. } => {
                AccessNode::new(id, AccessRole::Button, label.clone(), bounds)
            }
            CardControl::TextField(_) => match self.field_node(card.key, &card.title, bounds) {
                Some(node) => node,
                None => return group,
            },
            CardControl::None => return group,
        };
        node.help = card.description.clone();
        node.enabled = card.enabled;
        node.focusable = true;
        group.children.push(node);
        group
    }

    /// The identifier of the element that has the keyboard focus, if any.
    pub fn focused_access_id(&self) -> Option<u64> {
        if let Some(list) = self.list {
            return Some(ids::OPTION | (list.key.0 << 8) | list.highlight as u64);
        }
        target_id(self.focus?, self.picker)
    }

    /// An assistive tool asked for `action` on element `id`.
    pub fn access_action(
        &mut self,
        id: u64,
        action: AccessAction,
        measure: &dyn TextMeasure,
    ) -> Response {
        let response = self.access_inner(id, action, measure);
        self.sync_field();
        response
    }

    fn access_inner(
        &mut self,
        id: u64,
        action: AccessAction,
        measure: &dyn TextMeasure,
    ) -> Response {
        let Some(target) = self.target_of(id) else { return Response::default() };
        // While a dialog is shown, only its buttons can be used.
        let dialog_target =
            matches!(target, Target::DialogSave | Target::DialogDiscard | Target::DialogCancel);
        if self.dialog.is_some() != dialog_target {
            return Response::default();
        }
        if dialog_target {
            return match action {
                AccessAction::Focus => self.focus_on(target),
                AccessAction::Invoke | AccessAction::Select => {
                    let action = self.dialog_activate(target);
                    Response::act_moved(action)
                }
                _ => Response::default(),
            };
        }
        // A choice is checked against its own list below (it may be closed).
        let choice = matches!(target, Target::Option(_));
        if !choice && !exists(&self.content, &self.layout, self.list, self.edit.as_ref(), target) {
            return Response::default();
        }
        match (action, target) {
            (AccessAction::Focus, Target::Option(_)) => Response::default(),
            (AccessAction::Focus, target) => {
                let mut action = None;
                if self.edit.as_ref().is_some_and(|e| target != Target::Row(e.key, Part::TextBox)) {
                    action = self.finish_edit(true, measure);
                    self.refresh(measure);
                }
                Response { action, ..self.focus_on(target) }
            }
            (AccessAction::Toggle, Target::Control(key)) => {
                match &self.content.card(key).map(|c| &c.control) {
                    Some(CardControl::Toggle { on, .. }) => {
                        Response::act(SettingsAction::Toggle(key, !on))
                    }
                    _ => Response::default(),
                }
            }
            (AccessAction::Expand, Target::Control(key)) => {
                let Some(CardControl::Choice { selected, .. }) =
                    self.content.card(key).map(|c| &c.control)
                else {
                    return Response::default();
                };
                let selected = *selected;
                self.focus = Some(Target::Control(key));
                self.list = Some(OpenList { key, highlight: selected, scroll: 0.0 });
                self.reveal_option(selected);
                Response::moved()
            }
            (AccessAction::Collapse, Target::Control(_)) => {
                self.list = None;
                Response::redraw()
            }
            // The icon button shows or hides the icon picker (an expander).
            (AccessAction::Expand | AccessAction::Collapse, Target::Row(key, Part::Icon)) => {
                let open = self.picker.is_some_and(|p| p.key == key);
                if open == (action == AccessAction::Expand) {
                    return Response::default();
                }
                let before = self.focus;
                self.activate_row(key, Part::Icon, measure);
                Response { focus_moved: self.focus != before, ..Response::redraw() }
            }
            (AccessAction::Invoke, Target::Control(key)) => {
                match self.content.card(key).map(|c| &c.control) {
                    Some(CardControl::Link { .. } | CardControl::Button { .. }) => {
                        Response::act(SettingsAction::Invoke(key))
                    }
                    None if fields::button_usable(&self.content, key) => {
                        Response::act(SettingsAction::Invoke(key))
                    }
                    _ => Response::default(),
                }
            }
            (AccessAction::Select | AccessAction::Invoke, Target::Option(index)) => {
                let Some(key) = option_key(id) else { return Response::default() };
                let selected = match self.content.card(key).map(|c| &c.control) {
                    Some(CardControl::Choice { selected, options }) if index < options.len() => {
                        *selected
                    }
                    _ => return Response::default(),
                };
                if !usable(&self.content, key) {
                    return Response::default();
                }
                self.list = None;
                self.focus = Some(Target::Control(key));
                let action = (index != selected).then_some(SettingsAction::Choose(key, index));
                Response::act_moved(action)
            }
            (
                AccessAction::Select | AccessAction::Invoke,
                target @ (Target::Nav(_)
                | Target::Row(..)
                | Target::Cell(_)
                | Target::ConfirmOk
                | Target::ConfirmCancel),
            ) => {
                let before = self.focus;
                let action = self.activate(target, measure);
                self.refresh(measure);
                Response {
                    focus_moved: self.focus != before,
                    ..Response { action, ..Response::redraw() }
                }
            }
            _ => Response::default(),
        }
    }

    /// The view element an accessibility identifier names.
    fn target_of(&self, id: u64) -> Option<Target> {
        if (ids::NAV_ITEM..ids::NAV_ITEM + self.content.nav.len() as u64).contains(&id) {
            return Some(Target::Nav((id - ids::NAV_ITEM) as usize));
        }
        match id {
            ids::CONFIRM_OK => return Some(Target::ConfirmOk),
            ids::CONFIRM_CANCEL => return Some(Target::ConfirmCancel),
            ids::DIALOG_SAVE => return Some(Target::DialogSave),
            ids::DIALOG_DISCARD => return Some(Target::DialogDiscard),
            ids::DIALOG_CANCEL => return Some(Target::DialogCancel),
            _ => {}
        }
        let low = id & 0xFFFF_FFFF;
        match id >> 32 {
            2 => Some(Target::Control(SettingKey(low >> 4))),
            // The choices of a closed list are reachable too (Selection pattern).
            3 => Some(Target::Option((low & 0xFF) as usize)),
            4 => {
                let key = SettingKey(low >> 4);
                let part = match low & 0xF {
                    1 => Part::Icon,
                    2 => Part::Rename,
                    3 => Part::Visibility,
                    4 => Part::Forget,
                    5 => Part::TextBox,
                    _ => return None,
                };
                Some(Target::Row(key, part))
            }
            5 => {
                let key = SettingKey(low >> 8);
                self.picker.filter(|p| p.key == key).map(|_| Target::Cell((low & 0xFF) as usize))
            }
            _ => None,
        }
    }
}

/// The accessibility identifier of a focus target.
fn target_id(target: Target, picker: Option<Picker>) -> Option<u64> {
    match target {
        Target::Nav(i) => Some(ids::NAV_ITEM + i as u64),
        Target::Control(key) => Some(ids::CONTROL | (key.0 << 4)),
        Target::Row(key, part) => {
            let code = match part {
                Part::Body => return None,
                Part::Icon => 1,
                Part::Rename => 2,
                Part::Visibility => 3,
                Part::Forget => 4,
                Part::TextBox => 5,
            };
            Some(ids::ROW | (key.0 << 4) | code)
        }
        Target::Cell(i) => picker.map(|p| ids::CELL | (p.key.0 << 8) | i as u64),
        Target::ConfirmOk => Some(ids::CONFIRM_OK),
        Target::ConfirmCancel => Some(ids::CONFIRM_CANCEL),
        Target::Option(_) | Target::Thumb => None,
        Target::DialogSave => Some(ids::DIALOG_SAVE),
        Target::DialogDiscard => Some(ids::DIALOG_DISCARD),
        Target::DialogCancel => Some(ids::DIALOG_CANCEL),
    }
}

/// The card key of an option identifier.
fn option_key(id: u64) -> Option<SettingKey> {
    (id >> 32 == 3).then_some(SettingKey((id & 0xFFFF_FFFF) >> 8))
}

/// Whether `target` still designates something of the content.
fn exists(
    content: &SettingsContent,
    layout: &Layout,
    list: Option<OpenList>,
    edit: Option<&Edit>,
    target: Target,
) -> bool {
    match target {
        Target::Nav(i) => i < content.nav.len(),
        Target::Control(key) => usable(content, key),
        Target::Option(i) => {
            list.is_some_and(|l| i < options(content, l.key).map_or(0, <[String]>::len))
        }
        Target::Thumb => layout.scroll_thumb().is_some(),
        Target::Row(key, part) => content.row(key).is_some_and(|row| match part {
            Part::Forget => row.forgettable,
            Part::Icon => row.has_icon_button(),
            Part::Rename => row.renamable,
            Part::TextBox => edit.is_some_and(|e| e.key == key),
            _ => true,
        }),
        Target::Cell(i) => layout.picker.as_ref().is_some_and(|p| i < p.cells.len()),
        Target::ConfirmOk | Target::ConfirmCancel => true,
        Target::DialogSave | Target::DialogDiscard | Target::DialogCancel => true,
    }
}

fn usable(content: &SettingsContent, key: SettingKey) -> bool {
    content.card(key).is_some_and(|c| c.enabled && c.control != CardControl::None)
        || fields::button_usable(content, key)
}

fn options(content: &SettingsContent, key: SettingKey) -> Option<&[String]> {
    match &content.card(key)?.control {
        CardControl::Choice { options, .. } => Some(options),
        _ => None,
    }
}
