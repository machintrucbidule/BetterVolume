//! Painting of the settings window.

use super::content::{Block, CardControl, DeviceRow, RowPicture};
use super::layout::RowLayout;
use super::{Part, SettingsView, Target};
use crate::backend::{Canvas, TextAlign};
use crate::controls::device_icon::{self, DeviceIcon};
use crate::controls::flyout::TextMeasure;
use crate::icons::Icon;
use crate::tokens::focus;
use crate::tokens::settings as t;
use crate::tokens::typography::{BODY, BODY_STRONG, CAPTION, SUBTITLE, TITLE};
use crate::{Color, Point, Rect, Theme};

/// Pointer state of one control.
#[derive(Clone, Copy, Default)]
struct State {
    hot: bool,
    pressed: bool,
    focused: bool,
    enabled: bool,
    /// A drop-down list of this control is open.
    open: bool,
}

fn focus_ring(canvas: &mut Canvas<'_>, rect: Rect, radius: f32, theme: &Theme) {
    let outer = focus::MARGIN - focus::OUTER / 2.0;
    canvas.stroke_rounded(
        rect.inset(-outer, -outer),
        radius + outer,
        focus::OUTER,
        theme.focus_outer,
    );
    let inner = focus::MARGIN - focus::OUTER - focus::INNER / 2.0;
    canvas.stroke_rounded(
        rect.inset(-inner, -inner),
        radius + inner,
        focus::INNER,
        theme.focus_inner,
    );
}

pub(super) fn paint(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    measure: &dyn TextMeasure,
) {
    let layout = &view.layout;
    let w = &theme.window;
    if w.mica {
        // The system material shows through: Windows draws it behind the client area.
        canvas.clear(Color::TRANSPARENT);
    } else {
        canvas.clear(w.base);
    }
    let modal = view.list.is_some()
        || view.confirm.is_some()
        || view.dialog.is_some()
        || view.drag.is_some_and(|d| d.active);
    let state = |target: Target, enabled: bool| State {
        hot: view.hot == Some(target) && !modal,
        pressed: view.pressed == Some(target) && view.hot == Some(target),
        focused: view.keyboard
            && view.list.is_none()
            && view.dialog.is_none()
            && view.focus == Some(target),
        enabled,
        open: false,
    };

    // Navigation pane.
    for (i, (entry, rect)) in view.content.nav.iter().zip(&layout.nav).enumerate() {
        let s = state(Target::Nav(i), true);
        let selected = entry.key == view.content.page;
        let fill = if s.pressed {
            Some(theme.subtle_pressed)
        } else if s.hot || selected {
            Some(theme.subtle_hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            canvas.fill_rounded(*rect, t::NAV_RADIUS, fill);
        }
        let text = if theme.high_contrast && (s.hot || selected) {
            theme.subtle_hover_text
        } else {
            theme.text_primary
        };
        if selected {
            let (pw, ph) = t::NAV_PILL;
            let pill = Rect::new(rect.x, rect.y + (rect.height - ph) / 2.0, pw, ph);
            canvas.fill_rounded(pill, pw / 2.0, theme.accent_fill);
        }
        let (icon_x, icon_size, label_x) = t::NAV_ICON;
        let icon = Rect::new(
            rect.x + icon_x,
            rect.y + (rect.height - icon_size) / 2.0,
            icon_size,
            icon_size,
        );
        canvas.glyph(entry.icon.glyph(), icon_size, icon, text);
        let label =
            Rect::new(rect.x + label_x, rect.y, (rect.width - label_x - 8.0).max(1.0), rect.height);
        canvas.text(&entry.label, BODY, label, TextAlign::Leading, text);
        if s.focused {
            focus_ring(canvas, *rect, t::NAV_RADIUS, theme);
        }
    }

    // Page.
    canvas.clipped(layout.page, |canvas| {
        canvas.text(
            &view.content.title,
            TITLE,
            layout.on_screen(layout.title),
            TextAlign::Leading,
            theme.text_primary,
        );
        let mut headers = layout.headers.iter();
        let mut texts = layout.texts.iter();
        let mut cards = layout.cards.iter();
        for block in &view.content.blocks {
            match block {
                Block::Text(text) => {
                    if let Some((r, lines)) = texts.next() {
                        let r = layout.on_screen(*r);
                        canvas.text_wrapped(
                            text,
                            BODY,
                            r,
                            TextAlign::Leading,
                            *lines,
                            theme.text_secondary,
                        );
                    }
                }
                Block::Device(row) => {
                    let dragged = view.drag.filter(|d| d.active && d.key == row.key);
                    if dragged.is_none() {
                        if let Some(laid) = layout.row(row.key) {
                            paint_row(
                                Ctx { view, theme, measure, state: &state },
                                canvas,
                                row,
                                laid,
                                0.0,
                            );
                        }
                    }
                }
                Block::Header(text) => {
                    if let Some(r) = headers.next() {
                        canvas.text(
                            text,
                            BODY_STRONG,
                            layout.on_screen(*r),
                            TextAlign::Leading,
                            theme.text_primary,
                        );
                    }
                }
                Block::Card(card) => {
                    let Some(laid) = cards.next() else { continue };
                    let rect = layout.on_screen(laid.rect);
                    if rect.bottom() < 0.0 || rect.y > layout.size.height {
                        continue;
                    }
                    canvas.fill_rounded(rect, t::CARD_RADIUS, w.card);
                    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CARD_RADIUS, 1.0, w.card_stroke);
                    let text = if card.enabled { theme.text_primary } else { theme.text_disabled };
                    let secondary =
                        if card.enabled { theme.text_secondary } else { theme.text_disabled };
                    if let (Some(icon), Some(r)) = (card.icon, laid.icon) {
                        let (size, _) = t::CARD_ICON;
                        canvas.glyph(icon.glyph(), size, layout.on_screen(r), text);
                    }
                    canvas.text(
                        &card.title,
                        BODY,
                        layout.on_screen(laid.title),
                        TextAlign::Leading,
                        text,
                    );
                    if let (Some(description), Some((r, lines))) =
                        (&card.description, laid.description)
                    {
                        canvas.text_wrapped(
                            description,
                            CAPTION,
                            layout.on_screen(r),
                            TextAlign::Leading,
                            lines,
                            secondary,
                        );
                    }
                    let open = view.list.is_some_and(|l| l.key == card.key);
                    let s = State { open, ..state(Target::Control(card.key), card.enabled) };
                    if let CardControl::TextField(field) = &card.control {
                        if let Some(r) = laid.control {
                            // The field with the focus shows its editor (caret, selection).
                            let editing = view.field_of(card.key);
                            let shown = match editing {
                                Some(edit) => TextBoxShow::Editing(edit),
                                None => TextBoxShow::Rest {
                                    value: &field.value,
                                    placeholder: &field.placeholder,
                                    masked: field.password,
                                },
                            };
                            let focused = editing.is_some() && view.dialog.is_none();
                            let r = layout.on_screen(r);
                            text_box(
                                canvas,
                                theme,
                                measure,
                                r,
                                shown,
                                focused,
                                s.hot,
                                card.enabled,
                            );
                        }
                    } else {
                        paint_control(view, canvas, theme, &card.control, laid, s);
                    }
                }
                Block::Notice(notice) => paint_notice(view, canvas, theme, notice, &state),
                Block::Buttons(buttons) => {
                    for button in buttons {
                        let Some(r) = layout.button(button.key) else { continue };
                        let s = state(Target::Control(button.key), button.enabled);
                        push_button(
                            canvas,
                            theme,
                            layout.on_screen(r),
                            &button.label,
                            button.accent,
                            s,
                        );
                    }
                }
            }
        }
    });

    paint_picker(view, canvas, theme, &state);
    // The dragged row, lifted above the others, and where it would go.
    if let Some(drag) = view.drag.filter(|d| d.active) {
        if let (Some(row), Some(laid)) = (view.content.row(drag.key), layout.row(drag.key)) {
            paint_drop_line(view, canvas, theme, &drag);
            canvas.clipped(layout.page, |canvas| {
                paint_row(
                    Ctx { view, theme, measure, state: &state },
                    canvas,
                    row,
                    laid,
                    drag.offset(),
                );
            });
        }
    }

    if let Some(thumb) = layout.scroll_thumb() {
        let (bar, _) = t::SCROLL_BAR;
        let wide = view.hot == Some(Target::Thumb) || view.thumb_grab.is_some();
        let width = if wide { bar * 2.0 } else { bar };
        let r = Rect::new(
            thumb.right() - width - 3.0,
            thumb.y + 2.0,
            width,
            (thumb.height - 4.0).max(1.0),
        );
        canvas.fill_rounded(r, width / 2.0, theme.controls.scroll_indicator);
    }

    paint_list(view, canvas, theme);
    paint_confirm(view, canvas, theme, measure);
    paint_dialog(view, canvas, theme, measure);
}

/// A subtle square button with a glyph (row buttons).
fn glyph_button(canvas: &mut Canvas<'_>, theme: &Theme, rect: Rect, icon: Icon, s: State) {
    if s.pressed {
        canvas.fill_rounded(rect, t::CONTROL_RADIUS, theme.subtle_pressed);
    } else if s.hot {
        canvas.fill_rounded(rect, t::CONTROL_RADIUS, theme.subtle_hover);
    }
    let (_, glyph, _) = t::ROW_BUTTON;
    let color = if theme.high_contrast && (s.hot || s.pressed) {
        theme.subtle_hover_text
    } else {
        theme.text_primary
    };
    canvas.glyph(icon.glyph(), glyph, rect, color);
    if s.focused {
        focus_ring(canvas, rect, t::CONTROL_RADIUS, theme);
    }
}

/// What every row painting needs.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    view: &'a SettingsView,
    theme: &'a Theme,
    measure: &'a dyn TextMeasure,
    state: &'a dyn Fn(Target, bool) -> State,
}

fn paint_row(ctx: Ctx<'_>, canvas: &mut Canvas<'_>, row: &DeviceRow, laid: &RowLayout, dy: f32) {
    let Ctx { view, theme, measure, state } = ctx;
    let layout = &view.layout;
    let at = |r: Rect| {
        let r = layout.on_screen(r);
        Rect { y: r.y + dy, ..r }
    };
    let rect = at(laid.rect);
    if rect.bottom() < 0.0 || rect.y > layout.size.height {
        return;
    }
    let w = &theme.window;
    let lifted = dy != 0.0 || view.drag.is_some_and(|d| d.active && d.key == row.key);
    canvas.fill_rounded(rect, t::CARD_RADIUS, if lifted { theme.controls.solid } else { w.card });
    let stroke = if lifted { theme.accent_fill } else { w.card_stroke };
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CARD_RADIUS, 1.0, stroke);
    let (main, secondary) = if row.hidden {
        (theme.text_secondary, theme.text_disabled)
    } else {
        (theme.text_primary, theme.text_secondary)
    };
    let (grip, _) = t::GRIP;
    canvas.glyph(Icon::Grip.glyph(), grip, at(laid.grip), theme.text_secondary);

    let key = row.key;
    let icon_rect = at(laid.icon);
    let (_, icon_size, _) = t::ROW_ICON;
    let origin = Point::new(
        icon_rect.x + (icon_rect.width - icon_size) / 2.0,
        icon_rect.y + (icon_rect.height - icon_size) / 2.0,
    );
    let (app_size, glyph_size) = t::ROW_APP_ICON;
    let centred = |size: f32| {
        Rect::new(
            (icon_rect.x + (icon_rect.width - size) / 2.0).round(),
            (icon_rect.y + (icon_rect.height - size) / 2.0).round(),
            size,
            size,
        )
    };
    match &row.picture {
        RowPicture::Device { shown, .. } => {
            // Icon button: outlined, like the mockup.
            let s = state(Target::Row(key, Part::Icon), true);
            let open = view.picker.is_some_and(|p| p.key == key);
            let fill = if s.pressed || open {
                theme.controls.fill_pressed
            } else if s.hot {
                theme.controls.fill_hover
            } else {
                theme.controls.fill
            };
            canvas.fill_rounded(icon_rect, t::CONTROL_RADIUS, fill);
            canvas.stroke_rounded(
                icon_rect.inset(0.5, 0.5),
                t::CONTROL_RADIUS,
                1.0,
                theme.controls.stroke,
            );
            device_icon::paint(canvas, *shown, origin, icon_size, main);
            if s.focused {
                focus_ring(canvas, icon_rect, t::CONTROL_RADIUS, theme);
            }
        }
        // An application: its own icon, dimmed with the row when hidden.
        RowPicture::App { pixels: Some(image), .. } => {
            let opacity = if row.hidden { t::HIDDEN_PICTURE_OPACITY } else { 1.0 };
            canvas.image(image, centred(app_size), opacity);
        }
        RowPicture::App { pixels: None, .. } => {
            canvas.glyph(Icon::App.glyph(), glyph_size, centred(glyph_size), main);
        }
        RowPicture::SystemSounds => {
            canvas.glyph(Icon::SystemSounds.glyph(), glyph_size, centred(glyph_size), main);
        }
    }

    match view.edit_of(key) {
        Some(edit) => {
            let focused = view.focus == Some(Target::Row(key, Part::TextBox));
            let shown = TextBoxShow::Editing(edit);
            text_box(canvas, theme, measure, at(laid.text_box), shown, focused, false, true);
            if let Some(error) = laid.error {
                canvas.text(
                    &view.content.devices.duplicate,
                    CAPTION,
                    at(error),
                    TextAlign::Leading,
                    w.critical,
                );
            }
        }
        None => {
            canvas.text(&row.name, BODY, at(laid.name), TextAlign::Leading, main);
            canvas.text(&row.detail, CAPTION, at(laid.detail), TextAlign::Leading, secondary);
        }
    }

    if let Some(rename) = laid.rename {
        glyph_button(
            canvas,
            theme,
            at(rename),
            Icon::Edit,
            state(Target::Row(key, Part::Rename), true),
        );
    }
    let eye = if row.hidden { Icon::Hidden } else { Icon::Shown };
    glyph_button(
        canvas,
        theme,
        at(laid.visibility),
        eye,
        state(Target::Row(key, Part::Visibility), true),
    );
    if let Some(forget) = laid.forget {
        glyph_button(
            canvas,
            theme,
            at(forget),
            Icon::Delete,
            state(Target::Row(key, Part::Forget), true),
        );
    }
}

/// What a text box shows.
#[derive(Clone, Copy)]
enum TextBoxShow<'a> {
    /// Text being edited (caret and selection when focused).
    Editing(&'a super::devices::Edit),
    /// A field at rest: its text (bullets for a password) or its placeholder.
    Rest { value: &'a str, placeholder: &'a str, masked: bool },
}

/// A WinUI text box: control fill, outline, bottom line (accent and thicker when focused),
/// text clipped to the inner area.
#[allow(clippy::too_many_arguments)]
fn text_box(
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    measure: &dyn TextMeasure,
    rect: Rect,
    shown: TextBoxShow<'_>,
    focused: bool,
    hot: bool,
    enabled: bool,
) {
    let w = &theme.window;
    let fill = if !enabled {
        theme.controls.fill_disabled
    } else if focused {
        w.input_focused
    } else if hot {
        theme.controls.fill_hover
    } else {
        theme.controls.fill
    };
    canvas.fill_rounded(rect, t::CONTROL_RADIUS, fill);
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CONTROL_RADIUS, 1.0, theme.controls.stroke);
    let (rest, active) = t::TEXT_BOX_LINE;
    let line = if focused { active } else { rest };
    let underline = Rect::new(rect.x + 1.0, rect.bottom() - line, rect.width - 2.0, line);
    canvas.fill_rect(underline, if focused { theme.accent_fill } else { w.input_line });
    let (_, pad) = t::TEXT_BOX;
    let inner = Rect::new(rect.x + pad, rect.y, (rect.width - 2.0 * pad).max(1.0), rect.height);
    canvas.clipped(inner, |canvas| match shown {
        TextBoxShow::Editing(edit) => {
            let x0 = inner.x - edit.scroll;
            let (start, end) = edit.selection();
            if start != end && focused {
                let (a, b) = (edit.offset(start, measure), edit.offset(end, measure));
                let (band_inset, _) = t::TEXT_BOX_INSETS;
                let band =
                    Rect::new(x0 + a, inner.y + band_inset, b - a, inner.height - 2.0 * band_inset);
                canvas.fill_rect(band, theme.accent_fill.with_alpha(0.45));
            }
            let full = Rect::new(x0, inner.y, edit.offset(edit.len(), measure) + 4.0, inner.height);
            canvas.text(&edit.shown(), BODY, full, TextAlign::Leading, theme.text_primary);
            if focused {
                let x = (x0 + edit.offset(edit.caret, measure)).round() + 0.5;
                canvas.line(
                    Point::new(x, inner.y + t::TEXT_BOX_INSETS.1),
                    Point::new(x, inner.bottom() - t::TEXT_BOX_INSETS.1),
                    1.0,
                    theme.text_primary,
                );
            }
        }
        TextBoxShow::Rest { value, placeholder, masked } => {
            if value.is_empty() {
                canvas.text(placeholder, BODY, inner, TextAlign::Leading, theme.text_secondary);
            } else if masked {
                let bullets: String =
                    std::iter::repeat_n(t::PASSWORD_BULLET, value.chars().count()).collect();
                canvas.text(&bullets, BODY, inner, TextAlign::Leading, theme.text_primary);
            } else {
                let color = if enabled { theme.text_primary } else { theme.text_disabled };
                canvas.text(value, BODY, inner, TextAlign::Leading, color);
            }
        }
    });
}

/// A WinUI push button (accent or standard).
fn push_button(
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    rect: Rect,
    label: &str,
    accent: bool,
    s: State,
) {
    let (fill, text) = if accent && s.enabled {
        let fill = if s.pressed {
            theme.controls.accent_pressed
        } else if s.hot {
            theme.accent_fill_hover
        } else {
            theme.accent_fill
        };
        (fill, theme.text_on_accent)
    } else {
        let fill = if !s.enabled {
            theme.controls.fill_disabled
        } else if s.pressed {
            theme.controls.fill_pressed
        } else if s.hot {
            theme.controls.fill_hover
        } else {
            theme.controls.fill
        };
        let text = if !s.enabled {
            theme.text_disabled
        } else if theme.high_contrast && (s.hot || s.pressed) {
            theme.controls.hover_text
        } else {
            theme.text_primary
        };
        (fill, text)
    };
    canvas.fill_rounded(rect, t::CONTROL_RADIUS, fill);
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CONTROL_RADIUS, 1.0, theme.controls.stroke);
    canvas.text(label, BODY, rect, TextAlign::Center, text);
    if s.focused {
        focus_ring(canvas, rect, t::CONTROL_RADIUS, theme);
    }
}

/// A notice (WinUI InfoBar): tinted band, severity icon, title, message, button.
fn paint_notice(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    notice: &super::content::Notice,
    state: &dyn Fn(Target, bool) -> State,
) {
    let layout = &view.layout;
    let Some(laid) = layout.notices.iter().find(|n| n.key == notice.key) else { return };
    let rect = layout.on_screen(laid.rect);
    if rect.bottom() < 0.0 || rect.y > layout.size.height {
        return;
    }
    let w = &theme.window;
    let index = notice.severity as usize;
    canvas.fill_rounded(rect, t::CONTROL_RADIUS, w.notice_background[index]);
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CONTROL_RADIUS, 1.0, w.card_stroke);
    let icon = layout.on_screen(laid.icon);
    let (size, _) = t::NOTICE_ICON;
    canvas.glyph(Icon::NoticeBackground.glyph(), size, icon, w.notice_icon[index]);
    let glyph = match notice.severity {
        super::content::Severity::Info => Icon::NoticeInfo,
        super::content::Severity::Success => Icon::NoticeSuccess,
        super::content::Severity::Caution => Icon::NoticeWarning,
        super::content::Severity::Critical => Icon::NoticeError,
    };
    canvas.glyph(glyph.glyph(), size, icon, w.notice_glyph);
    canvas.text(
        &notice.title,
        BODY_STRONG,
        layout.on_screen(laid.title),
        TextAlign::Leading,
        theme.text_primary,
    );
    let (message, lines) = laid.message;
    canvas.text_wrapped(
        &notice.message,
        BODY,
        layout.on_screen(message),
        TextAlign::Leading,
        lines,
        theme.text_primary,
    );
    if let (Some(label), Some(r)) = (&notice.action, laid.action) {
        let s = state(Target::Control(notice.key), true);
        push_button(canvas, theme, layout.on_screen(r), label, false, s);
    }
}

/// The dialog shown (WinUI ContentDialog) over a veil on the whole window.
fn paint_dialog(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    measure: &dyn TextMeasure,
) {
    let Some((d, title_text, body, buttons)) = view.dialog_geometry(measure) else { return };
    canvas.fill_rect(Rect::from_size(view.layout.size), theme.window.smoke);
    let rect = d.rect;
    canvas.fill_rounded(
        Rect { y: rect.y + 1.0, ..rect }.inset(-1.0, 0.0),
        t::DIALOG_RADIUS + 1.0,
        theme.window.popup_stroke,
    );
    canvas.fill_rounded(rect, t::DIALOG_RADIUS, theme.controls.solid);
    canvas.clipped(rect, |canvas| {
        canvas.fill_rect(d.footer, theme.controls.footer);
    });
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::DIALOG_RADIUS, 1.0, theme.window.popup_stroke);
    let (title, title_lines) = d.title;
    canvas.text_wrapped(
        title_text,
        SUBTITLE,
        title,
        TextAlign::Leading,
        title_lines,
        theme.text_primary,
    );
    let (text, text_lines) = d.text;
    canvas.text_wrapped(body, BODY, text, TextAlign::Leading, text_lines, theme.text_primary);
    for (target, label, accent, r) in buttons {
        let hot = view.hot == Some(target);
        let s = State {
            hot,
            pressed: hot && view.pressed == Some(target),
            focused: view.keyboard && view.focus == Some(target),
            enabled: true,
            open: false,
        };
        push_button(canvas, theme, r, label, accent, s);
    }
}

/// The accent line where a dragged row would go.
fn paint_drop_line(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    drag: &super::devices::Drag,
) {
    let layout = &view.layout;
    let Some(row) = view.content.row(drag.key) else { return };
    let to = view.drop_target(drag);
    let Some(to) = to else { return };
    let others: Vec<&RowLayout> = view
        .content
        .list(row.list)
        .filter(|r| r.key != drag.key)
        .filter_map(|r| layout.row(r.key))
        .collect();
    let y = match (others.get(to.wrapping_sub(1)).filter(|_| to > 0), others.get(to)) {
        (_, Some(next)) if to == 0 => next.rect.y - t::CARD_GAP / 2.0,
        (Some(before), _) => before.rect.bottom() + t::CARD_GAP / 2.0,
        (None, Some(next)) => next.rect.y - t::CARD_GAP / 2.0,
        (None, None) => return,
    };
    let Some(first) = others.first() else { return };
    let line = layout.on_screen(Rect::new(first.rect.x, y - 1.0, first.rect.width, 2.0));
    canvas.fill_rounded(line, 1.0, theme.accent_fill);
}

/// The icon picker panel under its row.
fn paint_picker(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    state: &dyn Fn(Target, bool) -> State,
) {
    let layout = &view.layout;
    let Some(picker) = &layout.picker else { return };
    let Some(row) = view.content.row(picker.key) else { return };
    let RowPicture::Device { automatic, .. } = row.picture else { return };
    let texts = &view.content.devices;
    canvas.clipped(layout.page, |canvas| {
        let panel = layout.on_screen(picker.rect);
        canvas.fill_rounded(panel, t::CARD_RADIUS, theme.window.card);
        canvas.stroke_rounded(panel.inset(0.5, 0.5), t::CARD_RADIUS, 1.0, theme.window.card_stroke);
        let chosen = row
            .chosen_icon()
            .and_then(|c| DeviceIcon::ALL.iter().position(|i| *i == c))
            .map_or(0, |p| p + 1);
        for (i, cell) in picker.cells.iter().enumerate() {
            let cell = layout.on_screen(*cell);
            let s = state(Target::Cell(i), true);
            if i == chosen {
                canvas.fill_rounded(cell, t::CONTROL_RADIUS, theme.subtle_hover);
                canvas.stroke_rounded(
                    cell.inset(1.0, 1.0),
                    t::CONTROL_RADIUS,
                    2.0,
                    theme.accent_fill,
                );
            } else if s.pressed {
                canvas.fill_rounded(cell, t::CONTROL_RADIUS, theme.subtle_pressed);
            } else if s.hot {
                canvas.fill_rounded(cell, t::CONTROL_RADIUS, theme.subtle_hover);
            }
            let size = t::PICKER_ICON;
            let origin = Point::new(cell.x + (cell.width - size) / 2.0, cell.y + 10.0);
            let icon = i.checked_sub(1).and_then(|k| DeviceIcon::ALL.get(k).copied());
            let color = theme.text_primary;
            match icon {
                Some(icon) => device_icon::paint(canvas, icon, origin, size, color),
                // "Automatic": the icon chosen from the type Windows declares.
                None => device_icon::paint(canvas, automatic, origin, size, color),
            }
            let name = if i == 0 {
                &texts.automatic
            } else {
                texts.icon_names.get(i - 1).unwrap_or(&texts.automatic)
            };
            let label = Rect::new(
                cell.x + 4.0,
                origin.y + size + 6.0,
                cell.width - 8.0,
                cell.bottom() - origin.y - size - 8.0,
            );
            canvas.text_wrapped(name, CAPTION, label, TextAlign::Center, 2, theme.text_secondary);
            if s.focused {
                focus_ring(canvas, cell, t::CONTROL_RADIUS, theme);
            }
        }
    });
}

/// The confirmation flyout of a forget button.
fn paint_confirm(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    measure: &dyn TextMeasure,
) {
    let Some(confirm) = view.confirm_geometry(measure) else { return };
    let texts = &view.content.devices;
    let rect = confirm.rect;
    canvas.fill_rounded(
        Rect { y: rect.y + 1.0, ..rect }.inset(-1.0, 0.0),
        t::LIST_RADIUS + 1.0,
        theme.window.popup_stroke,
    );
    canvas.fill_rounded(rect, t::LIST_RADIUS, theme.controls.solid);
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::LIST_RADIUS, 1.0, theme.window.popup_stroke);
    let (q, q_lines) = confirm.question;
    canvas.text_wrapped(
        &texts.forget_question,
        BODY_STRONG,
        q,
        TextAlign::Leading,
        q_lines,
        theme.text_primary,
    );
    let (d, d_lines) = confirm.detail;
    canvas.text_wrapped(
        &texts.forget_detail,
        BODY,
        d,
        TextAlign::Leading,
        d_lines,
        theme.text_secondary,
    );
    for (target, label, rect, accent) in [
        (Target::ConfirmOk, &texts.forget_confirm, confirm.ok, true),
        (Target::ConfirmCancel, &texts.cancel, confirm.cancel, false),
    ] {
        let hot = view.hot == Some(target);
        let pressed = hot && view.pressed == Some(target);
        let (fill, text) = if accent {
            let fill = if pressed {
                theme.controls.accent_pressed
            } else if hot {
                theme.accent_fill_hover
            } else {
                theme.accent_fill
            };
            (fill, theme.text_on_accent)
        } else {
            let fill = if pressed {
                theme.controls.fill_pressed
            } else if hot {
                theme.controls.fill_hover
            } else {
                theme.controls.fill
            };
            (fill, theme.text_primary)
        };
        canvas.fill_rounded(rect, t::CONTROL_RADIUS, fill);
        canvas.stroke_rounded(rect.inset(0.5, 0.5), t::CONTROL_RADIUS, 1.0, theme.controls.stroke);
        canvas.text(label, BODY, rect, TextAlign::Center, text);
        if view.keyboard && view.focus == Some(target) {
            focus_ring(canvas, rect, t::CONTROL_RADIUS, theme);
        }
    }
}

fn paint_control(
    view: &SettingsView,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
    control: &CardControl,
    laid: &super::layout::CardLayout,
    s: State,
) {
    let layout = &view.layout;
    let Some(rect) = laid.control.map(|r| layout.on_screen(r)) else { return };
    let w = &theme.window;
    match control {
        CardControl::None | CardControl::TextField(_) => {}
        CardControl::Button { label, accent } => {
            push_button(canvas, theme, rect, label, *accent, s)
        }
        CardControl::Toggle { on, on_label, off_label } => {
            let (Some(switch), Some(label)) = (laid.switch, laid.switch_label) else { return };
            let (switch, label) = (layout.on_screen(switch), layout.on_screen(label));
            let text = if s.enabled { theme.text_primary } else { theme.text_disabled };
            canvas.text(
                if *on { on_label } else { off_label },
                BODY,
                label,
                TextAlign::Trailing,
                text,
            );
            let radius = switch.height / 2.0;
            let (knob_rest, knob_hot) = t::TOGGLE_KNOB;
            let knob_h = if s.hot || s.pressed { knob_hot } else { knob_rest };
            let knob_w = if s.pressed { t::TOGGLE_KNOB_PRESSED } else { knob_h };
            let knob_color = if *on {
                let fill = if !s.enabled {
                    theme.controls.accent_disabled
                } else if s.pressed {
                    theme.controls.accent_pressed
                } else if s.hot {
                    theme.accent_fill_hover
                } else {
                    theme.accent_fill
                };
                canvas.fill_rounded(switch, radius, fill);
                if theme.high_contrast {
                    canvas.stroke_rounded(switch.inset(0.5, 0.5), radius, 1.0, theme.text_primary);
                }
                theme.text_on_accent
            } else {
                let fill =
                    if s.hot && s.enabled { w.toggle_off_fill_hover } else { w.toggle_off_fill };
                canvas.fill_rounded(switch, radius, fill);
                let stroke = if s.enabled { w.toggle_off_stroke } else { theme.text_disabled };
                canvas.stroke_rounded(switch.inset(0.5, 0.5), radius - 0.5, 1.0, stroke);
                stroke
            };
            // The knob sits 4 DIP from the end of the track it rests on (WinUI).
            let center_y = switch.y + switch.height / 2.0;
            let rest = switch.height / 2.0;
            let x = if *on {
                switch.right() - rest - knob_w / 2.0
            } else {
                switch.x + rest - knob_w / 2.0
            };
            let knob = Rect::new(x, center_y - knob_h / 2.0, knob_w, knob_h);
            canvas.fill_rounded(knob, knob_h / 2.0, knob_color);
            if s.focused {
                focus_ring(canvas, switch, radius, theme);
            }
        }
        CardControl::Choice { options, selected } => {
            let fill = if !s.enabled {
                theme.controls.fill_disabled
            } else if s.pressed || s.open {
                theme.controls.fill_pressed
            } else if s.hot {
                theme.controls.fill_hover
            } else {
                theme.controls.fill
            };
            canvas.fill_rounded(rect, t::CONTROL_RADIUS, fill);
            canvas.stroke_rounded(
                rect.inset(0.5, 0.5),
                t::CONTROL_RADIUS,
                1.0,
                theme.controls.stroke,
            );
            let text = if !s.enabled {
                theme.text_disabled
            } else if (s.hot || s.pressed) && theme.high_contrast {
                theme.controls.hover_text
            } else {
                theme.text_primary
            };
            let (left, right) = t::COMBO_PADDING;
            let (chevron, gap) = t::COMBO_CHEVRON;
            let label = Rect::new(
                rect.x + left,
                rect.y,
                (rect.width - left - gap - chevron - right).max(1.0),
                rect.height,
            );
            if let Some(option) = options.get(*selected) {
                canvas.text(option, BODY, label, TextAlign::Leading, text);
            }
            let glyph = Rect::new(rect.right() - right - chevron, rect.y, chevron, rect.height);
            let secondary = if s.enabled { theme.text_secondary } else { theme.text_disabled };
            canvas.glyph(Icon::ChevronDown.glyph(), chevron, glyph, secondary);
            if s.focused {
                focus_ring(canvas, rect, t::CONTROL_RADIUS, theme);
            }
        }
        CardControl::Link { label } => {
            if s.enabled && (s.hot || s.pressed) {
                let fill = if s.pressed { theme.subtle_pressed } else { theme.subtle_hover };
                canvas.fill_rounded(rect, t::CONTROL_RADIUS, fill);
            }
            let color = if !s.enabled {
                theme.text_disabled
            } else if s.hot || s.pressed {
                if theme.high_contrast {
                    theme.subtle_hover_text
                } else {
                    w.link_hover
                }
            } else {
                w.link
            };
            canvas.text(label, BODY, rect, TextAlign::Center, color);
            if s.focused {
                focus_ring(canvas, rect, t::CONTROL_RADIUS, theme);
            }
        }
    }
}

/// The open drop-down list, above everything.
fn paint_list(view: &SettingsView, canvas: &mut Canvas<'_>, theme: &Theme) {
    let Some((list, rect, item, _)) = view.list_geometry() else { return };
    let Some(super::content::Card { control: CardControl::Choice { options, selected }, .. }) =
        view.content.card(list.key)
    else {
        return;
    };
    // A soft edge under the list stands in for its shadow.
    canvas.fill_rounded(
        Rect { y: rect.y + 1.0, ..rect }.inset(-1.0, 0.0),
        t::LIST_RADIUS + 1.0,
        theme.window.popup_stroke,
    );
    canvas.fill_rounded(rect, t::LIST_RADIUS, theme.controls.solid);
    canvas.stroke_rounded(rect.inset(0.5, 0.5), t::LIST_RADIUS, 1.0, theme.window.popup_stroke);
    let inner = rect.inset(0.0, t::LIST_PADDING);
    canvas.clipped(inner, |canvas| {
        for (i, option) in options.iter().enumerate() {
            let top = inner.y + i as f32 * item - list.scroll;
            if top + item < inner.y || top > inner.bottom() {
                continue;
            }
            let row =
                Rect::new(rect.x + t::LIST_INSET, top, rect.width - 2.0 * t::LIST_INSET, item)
                    .inset(0.0, 2.0);
            let hot = view.hot == Some(Target::Option(i));
            let pressed = hot && view.pressed == Some(Target::Option(i));
            let highlighted = view.keyboard && list.highlight == i;
            let fill = if pressed {
                Some(theme.subtle_pressed)
            } else if hot || highlighted || i == *selected {
                Some(theme.subtle_hover)
            } else {
                None
            };
            if let Some(fill) = fill {
                canvas.fill_rounded(row, t::CONTROL_RADIUS, fill);
            }
            if i == *selected {
                let (pw, ph) = t::NAV_PILL;
                let pill = Rect::new(row.x, row.y + (row.height - ph) / 2.0, pw, ph);
                canvas.fill_rounded(pill, pw / 2.0, theme.accent_fill);
            }
            let (left, _) = t::COMBO_PADDING;
            let text = if theme.high_contrast && (hot || highlighted) {
                theme.subtle_hover_text
            } else {
                theme.text_primary
            };
            let label =
                Rect::new(row.x + left, row.y, (row.width - 2.0 * left).max(1.0), row.height);
            canvas.text(option, BODY, label, TextAlign::Leading, text);
            if highlighted {
                focus_ring(canvas, row, t::CONTROL_RADIUS, theme);
            }
        }
    });
}
