//! Text fields, push buttons, notices and the close dialog of the settings window (the
//! Home Assistant page, FR-090 to FR-092).
//!
//! A text field is the control of a card: it shares the [`Target::Control`] focus target
//! with the other card controls; while it has the focus, the view keeps an [`Edit`] for
//! it (caret, selection, scroll) and reports every change as
//! [`SettingsAction::SetText`]. Buttons of a button row and the button of a notice are
//! [`Target::Control`]s too, reported as [`SettingsAction::Invoke`].

use super::content::{Block, SettingKey, SettingsAction};
use super::devices::Edit;
use super::layout::{self, DialogLayout};
use super::{ids, Dialog, Response, SettingsKey, SettingsView, Target};
use crate::access::{AccessNode, AccessRole};
use crate::controls::flyout::TextMeasure;
use crate::tokens::settings as t;
use crate::{Point, Rect};

/// A dialog's title, text and buttons (target, label, accent).
type DialogTexts<'a> = (&'a str, &'a str, Vec<(Target, &'a str, bool)>);
/// A dialog laid out: its geometry, title, text and buttons (target, label, accent, place).
pub(super) type PlacedDialog<'a> =
    (DialogLayout, &'a str, &'a str, Vec<(Target, &'a str, bool, Rect)>);

impl SettingsView {
    /// The text field of card `key`, where it shows in the window.
    pub(super) fn field_rect(&self, key: SettingKey) -> Option<Rect> {
        self.content.field(key)?;
        let laid = self.card_layout(key)?;
        laid.control.map(|r| self.layout.on_screen(r))
    }

    /// Keeps the field editor on the focused text field: created (whole text selected)
    /// when a field takes the focus, dropped when the focus leaves it.
    pub(super) fn sync_field(&mut self) {
        let focused = match self.focus {
            Some(Target::Control(key)) => self.content.field(key).map(|f| (key, f)),
            _ => None,
        };
        match focused {
            Some((key, field)) => {
                if self.field.as_ref().is_none_or(|e| e.key != key) {
                    self.field = Some(Edit::field(key, field));
                }
            }
            None => self.field = None,
        }
    }

    /// The pointer went down on a text field: the caret goes there.
    pub(super) fn field_press(&mut self, key: SettingKey, p: Point, measure: &dyn TextMeasure) {
        self.sync_field();
        let Some(rect) = self.field_rect(key) else { return };
        let Some(edit) = self.field.as_mut().filter(|e| e.key == key) else { return };
        let (_, pad) = t::TEXT_BOX;
        let index = edit.index_at(p.x - rect.x - pad + edit.scroll, measure);
        edit.caret = index;
        edit.anchor = index;
        edit.keep_caret_visible(rect, measure);
    }

    /// The pointer moves with the button down on a text field: the selection follows.
    pub(super) fn field_drag(&mut self, p: Point, measure: &dyn TextMeasure) -> Response {
        let Some(Target::Control(key)) = self.pressed else { return Response::default() };
        let Some(rect) = self.field_rect(key) else { return Response::default() };
        let Some(edit) = self.field.as_mut().filter(|e| e.key == key) else {
            return Response::default();
        };
        let (_, pad) = t::TEXT_BOX;
        let index = edit.index_at(p.x - rect.x - pad + edit.scroll, measure);
        if index == edit.caret {
            return Response::default();
        }
        edit.caret = index;
        edit.keep_caret_visible(rect, measure);
        Response::redraw()
    }

    /// Whether the pointer pressed a text field (a drag then selects).
    pub(super) fn pressing_field(&self) -> bool {
        matches!(self.pressed, Some(Target::Control(key)) if self.content.field(key).is_some())
    }

    /// A key while a text field has the focus. `None`: not a text key (Tab, arrows up and
    /// down, Page Up/Down go on to the page).
    pub(super) fn field_key(
        &mut self,
        key: SettingKey,
        input: SettingsKey,
        measure: &dyn TextMeasure,
    ) -> Option<Response> {
        let rect = self.field_rect(key)?;
        let edit = self.field.as_mut().filter(|e| e.key == key)?;
        let before = edit.text.clone();
        let (clipboard, handled) = edit.key(input);
        if !handled {
            // Enter and Escape do nothing in a single-line field; Space is typed text.
            return matches!(input, SettingsKey::Enter | SettingsKey::Escape | SettingsKey::Space)
                .then(Response::default);
        }
        edit.keep_caret_visible(rect, measure);
        let action = (edit.text != before).then(|| SettingsAction::SetText(key, edit.text.clone()));
        Some(Response { redraw: true, clipboard, action, ..Response::default() })
    }

    /// Text typed or pasted into the focused text field. `None`: no field has the focus.
    pub(super) fn field_text(&mut self, text: &str, measure: &dyn TextMeasure) -> Option<Response> {
        let Some(Target::Control(key)) = self.focus else { return None };
        let rect = self.field_rect(key)?;
        let edit = self.field.as_mut().filter(|e| e.key == key)?;
        let before = edit.text.clone();
        edit.insert(text);
        edit.keep_caret_visible(rect, measure);
        let action = (edit.text != before).then(|| SettingsAction::SetText(key, edit.text.clone()));
        Some(Response { redraw: true, action, ..Response::default() })
    }

    /// The field editor of card `key`, for painting.
    pub(super) fn field_of(&self, key: SettingKey) -> Option<&Edit> {
        self.field.as_ref().filter(|e| e.key == key)
    }

    /// The button (of a button row or a notice) under `p`.
    pub(super) fn button_hit(&self, p: Point) -> Option<Target> {
        let keys = self
            .layout
            .buttons
            .iter()
            .map(|(k, _)| *k)
            .chain(self.layout.notices.iter().filter(|n| n.action.is_some()).map(|n| n.key));
        keys.into_iter().find_map(|key| {
            let rect = self.layout.on_screen(self.layout.button(key)?);
            (rect.contains(p) && super::usable(&self.content, key)).then_some(Target::Control(key))
        })
    }

    // ----- Dialogs: close question, question of the content -----

    /// Shows the content's question when there is one and no dialog is shown.
    pub(super) fn open_question(&mut self) {
        if self.dialog.is_none() && self.content.question.is_some() {
            self.list = None;
            self.confirm = None;
            self.drag = None;
            self.dialog = Some(Dialog::Ask);
            self.focus = Some(Target::DialogSave);
            self.hot = None;
            self.sync_field();
        }
    }

    /// The window is asked to close. A question shown is answered with its secondary
    /// button in the response; with unsaved changes, the close question then opens
    /// and the response says to keep the window (`keep_open`); otherwise it may close.
    pub fn request_close(&mut self, measure: &dyn TextMeasure) -> Response {
        let answered = (self.dialog == Some(Dialog::Ask)).then(|| {
            self.dialog = None;
            self.focus = None;
            SettingsAction::QuestionAnswer(false)
        });
        if self.content.close_question.is_none() {
            return Response { action: answered, ..Response::default() };
        }
        self.list = None;
        self.confirm = None;
        self.drag = None;
        self.dialog = Some(Dialog::Close);
        self.focus = Some(Target::DialogSave);
        self.sync_field();
        self.hot = None;
        let _ = measure;
        Response { keep_open: true, action: answered, ..Response::moved() }
    }

    /// Whether a dialog (close question or question) is shown.
    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Title, text and buttons (target, label, accent) of the dialog shown.
    fn dialog_texts(&self) -> Option<DialogTexts<'_>> {
        match self.dialog? {
            Dialog::Close => {
                let q = self.content.close_question.as_ref()?;
                Some((
                    &q.title,
                    &q.text,
                    vec![
                        (Target::DialogSave, q.save.as_str(), true),
                        (Target::DialogDiscard, q.discard.as_str(), false),
                        (Target::DialogCancel, q.cancel.as_str(), false),
                    ],
                ))
            }
            Dialog::Ask => {
                let q = self.content.question.as_ref()?;
                Some((
                    &q.title,
                    &q.text,
                    vec![
                        (Target::DialogSave, q.primary.as_str(), true),
                        (Target::DialogDiscard, q.secondary.as_str(), false),
                    ],
                ))
            }
        }
    }

    /// The dialog's geometry, title, text and buttons (target, label, accent, rectangle).
    pub(super) fn dialog_geometry(&self, measure: &dyn TextMeasure) -> Option<PlacedDialog<'_>> {
        let (title, text, buttons) = self.dialog_texts()?;
        let labels: Vec<&str> = buttons.iter().map(|(_, label, _)| *label).collect();
        let d = layout::dialog(title, text, &labels, self.layout.size, measure, self.text_scale);
        let placed = buttons
            .into_iter()
            .zip(d.buttons.iter())
            .map(|((target, label, accent), rect)| (target, label, accent, *rect))
            .collect();
        Some((d, title, text, placed))
    }

    pub(super) fn dialog_hit(&self, p: Point, measure: &dyn TextMeasure) -> Option<Target> {
        let (_, _, _, buttons) = self.dialog_geometry(measure)?;
        buttons.into_iter().find(|(_, _, _, r)| r.contains(p)).map(|(t, ..)| t)
    }

    /// Does what the dialog button `target` does.
    pub(super) fn dialog_activate(&mut self, target: Target) -> Option<SettingsAction> {
        let dialog = self.dialog.take();
        self.focus = None;
        match (dialog?, target) {
            (Dialog::Close, Target::DialogSave) => Some(SettingsAction::CloseAnswer(true)),
            (Dialog::Close, Target::DialogDiscard) => Some(SettingsAction::CloseAnswer(false)),
            (Dialog::Ask, Target::DialogSave) => Some(SettingsAction::QuestionAnswer(true)),
            // Any other way of closing the question is its secondary answer.
            (Dialog::Ask, _) => Some(SettingsAction::QuestionAnswer(false)),
            _ => None,
        }
    }

    pub(super) fn dialog_key(&mut self, key: SettingsKey) -> Response {
        let order: &[Target] = match self.dialog {
            Some(Dialog::Close) => {
                &[Target::DialogSave, Target::DialogDiscard, Target::DialogCancel]
            }
            _ => &[Target::DialogSave, Target::DialogDiscard],
        };
        let n = order.len();
        let at = self.focus.and_then(|f| order.iter().position(|t| *t == f)).unwrap_or(0);
        match key {
            SettingsKey::Tab { back: false } | SettingsKey::Right { .. } => {
                self.focus = Some(order[(at + 1) % n]);
                Response::moved()
            }
            SettingsKey::Tab { back: true } | SettingsKey::Left { .. } => {
                self.focus = Some(order[(at + n - 1) % n]);
                Response::moved()
            }
            SettingsKey::Enter | SettingsKey::Space => {
                let action = self.dialog_activate(order[at]);
                Response::act_moved(action)
            }
            SettingsKey::Escape => {
                let action = self.dialog_activate(Target::DialogCancel);
                Response::act_moved(action)
            }
            _ => Response::default(),
        }
    }

    pub(super) fn dialog_node(&self, measure: &dyn TextMeasure) -> Option<AccessNode> {
        let (d, title, text, buttons) = self.dialog_geometry(measure)?;
        let mut group = AccessNode::new(ids::DIALOG, AccessRole::Group, title.to_owned(), d.rect);
        group.help = Some(text.to_owned());
        for (target, label, _, rect) in buttons {
            let id = match target {
                Target::DialogSave => ids::DIALOG_SAVE,
                Target::DialogDiscard => ids::DIALOG_DISCARD,
                _ => ids::DIALOG_CANCEL,
            };
            let mut button = AccessNode::new(id, AccessRole::Button, label.to_owned(), rect);
            button.focusable = true;
            group.children.push(button);
        }
        Some(group)
    }

    // ----- Accessibility of buttons and notices -----

    pub(super) fn button_nodes(&self, block: &Block) -> Vec<AccessNode> {
        match block {
            Block::Buttons(buttons) => buttons
                .iter()
                .filter_map(|b| {
                    let rect = self.layout.on_screen(self.layout.button(b.key)?);
                    let mut node = AccessNode::new(
                        ids::CONTROL | (b.key.0 << 4),
                        AccessRole::Button,
                        b.label.clone(),
                        rect,
                    );
                    node.enabled = b.enabled;
                    node.focusable = true;
                    Some(node)
                })
                .collect(),
            Block::Notice(notice) => {
                let Some(laid) = self.layout.notices.iter().find(|n| n.key == notice.key) else {
                    return Vec::new();
                };
                let mut group = AccessNode::new(
                    ids::NOTICE | (notice.key.0 << 4),
                    AccessRole::Group,
                    notice.title.clone(),
                    self.layout.on_screen(laid.rect),
                );
                group.help = Some(notice.message.clone());
                if let (Some(label), Some(rect)) = (&notice.action, laid.action) {
                    let mut button = AccessNode::new(
                        ids::CONTROL | (notice.key.0 << 4),
                        AccessRole::Button,
                        label.clone(),
                        self.layout.on_screen(rect),
                    );
                    button.focusable = true;
                    button.help = Some(notice.message.clone());
                    group.children.push(button);
                }
                vec![group]
            }
            _ => Vec::new(),
        }
    }

    /// The accessibility node of a text field control.
    pub(super) fn field_node(
        &self,
        key: SettingKey,
        title: &str,
        bounds: Rect,
    ) -> Option<AccessNode> {
        let field = self.content.field(key)?;
        let mut node = AccessNode::new(
            ids::CONTROL | (key.0 << 4),
            AccessRole::Edit,
            title.to_owned(),
            bounds,
        );
        // A password is never exposed (FR-092).
        if !field.password {
            node.value =
                Some(self.field_of(key).map_or_else(|| field.value.clone(), |e| e.text.clone()));
        }
        Some(node)
    }
}

/// Whether `key` designates a usable push button (of a button row or a notice).
pub(super) fn button_usable(content: &super::content::SettingsContent, key: SettingKey) -> bool {
    content.buttons().any(|b| b.key == key && b.enabled)
        || content.notices().any(|n| n.key == key && n.action.is_some())
}
