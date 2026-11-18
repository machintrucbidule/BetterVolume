//! Painting of the flyout controls: tabs, tiles, section header, volume rows (mute
//! button, slider, value), footer, focus indicator, scroll indicator.

use super::content::UpdateLine;
use super::content::{FlyoutContent, FlyoutTab, VolumeControl, VolumeIcon};
use super::layout::{tab_style, tile_style, value_rect, Block, Layout, NodeKey};
use super::{thumb_x, FlyoutView};
use crate::backend::{Canvas, TextAlign};
use crate::controls::device_icon;
use crate::icons::Icon;
use crate::tokens::typography::{BODY, CAPTION, CAPTION_STRONG};
use crate::tokens::{flyout, focus, footer, section, slider, tabs, tile, volume_row};
use crate::{Color, Point, Rect, Theme};

/// The state of a control for painting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct State {
    hot: bool,
    pressed: bool,
    enabled: bool,
}

pub(super) fn paint(view: &FlyoutView, canvas: &mut Canvas<'_>, theme: &Theme) {
    let size = crate::Size::new(view.layout.width, view.shown_height);
    super::super::paint_surface(canvas, theme, size);
    let width = view.layout.width;

    // Top block.
    paint_tabs(view, canvas, theme);
    paint_tiles(view, canvas, theme);

    // Middle block, cut to its visible part; the previous content while shrinking.
    let clip = view.middle_clip();
    if clip.height > 0.0 {
        let (content, layout, live) = match &view.ghost {
            Some(ghost) => (&ghost.content, &ghost.layout, false),
            None => (&view.content, &view.layout, true),
        };
        let origin = view.block_origin(Block::Middle);
        canvas.clipped(clip, |c| {
            if let Some(y) = layout.middle_divider {
                divider(c, origin + y, width, theme);
            }
            paint_middle(view, content, layout, origin, live, c, theme);
        });
        if view.scrolls() {
            paint_scroll_indicator(view, canvas, theme, clip);
        }
    }

    // Bottom block.
    let origin = view.block_origin(Block::Bottom);
    divider(canvas, origin + view.layout.bottom_divider, width, theme);
    let footer = view.placed(Block::Bottom, view.layout.footer);
    canvas.fill_rect(footer, theme.controls.footer);
    divider(canvas, footer.y, width, theme);
    paint_volume(view, &view.content.main, &view.layout, origin, true, canvas, theme);
    paint_footer(view, canvas, theme);
    if let (Some(update), Some(text)) = (&view.content.update, view.layout.update_text) {
        paint_update(view, update, view.placed(Block::Bottom, text), canvas, theme);
    }

    // Keyboard focus indicator, over everything.
    if view.focus_visible {
        if let Some(key) = view.focus {
            if let Some(rect) = view.node_rect(key) {
                let in_middle = view.layout.node(key).is_some_and(|n| n.block == Block::Middle);
                if in_middle {
                    canvas.clipped(clip, |c| focus_ring(c, rect, radius_of(key), theme));
                } else {
                    focus_ring(canvas, rect, radius_of(key), theme);
                }
            }
        }
    }
}

fn state(view: &FlyoutView, key: NodeKey) -> State {
    State {
        hot: view.hot == Some(key) && view.pressed.is_none_or(|p| p == key),
        pressed: view.pressed == Some(key),
        enabled: view.enabled(key),
    }
}

fn radius_of(key: NodeKey) -> f32 {
    match key {
        NodeKey::Tab(_) => tabs::TAB_RADIUS,
        NodeKey::Tile(_) => tile::RADIUS,
        NodeKey::Header => section::RADIUS,
        NodeKey::Mute(_) => volume_row::BUTTON_RADIUS,
        NodeKey::Slider(_) => volume_row::BUTTON_RADIUS,
        NodeKey::SoundSettings | NodeKey::Settings => footer::RADIUS,
        NodeKey::UpdateAction | NodeKey::UpdateClose => footer::RADIUS,
    }
}

/// A separator line across the whole panel, its top edge at `y`.
fn divider(canvas: &mut Canvas<'_>, y: f32, width: f32, theme: &Theme) {
    let y = y.floor() + 0.5;
    canvas.line(Point::new(0.0, y), Point::new(width, y), 1.0, theme.divider);
}

/// The Windows keyboard focus indicator: a 2-DIP outer ring and a 1-DIP inner ring,
/// outside the control.
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

/// Background of a subtle control (transparent at rest).
fn subtle(canvas: &mut Canvas<'_>, rect: Rect, radius: f32, s: State, theme: &Theme) {
    if !s.enabled {
        return;
    }
    if s.pressed {
        canvas.fill_rounded(rect, radius, theme.subtle_pressed);
    } else if s.hot {
        canvas.fill_rounded(rect, radius, theme.subtle_hover);
    }
}

/// Text colour of a subtle control.
fn subtle_text(s: State, theme: &Theme, rest: Color) -> Color {
    if !s.enabled {
        theme.text_disabled
    } else if s.hot || s.pressed {
        if theme.high_contrast {
            theme.subtle_hover_text
        } else if s.pressed {
            theme.text_secondary
        } else {
            rest
        }
    } else {
        rest
    }
}

fn paint_tabs(view: &FlyoutView, canvas: &mut Canvas<'_>, theme: &Theme) {
    let Some(bar) = &view.content.tabs else { return };
    let (Some(first), Some(second)) = (
        view.node_rect(NodeKey::Tab(FlyoutTab::Output)),
        view.node_rect(NodeKey::Tab(FlyoutTab::Input)),
    ) else {
        return;
    };
    let outline = Rect::new(
        first.x - tabs::PADDING,
        first.y - tabs::PADDING,
        second.right() - first.x + 2.0 * tabs::PADDING,
        first.height + 2.0 * tabs::PADDING,
    );
    canvas.fill_rounded(outline, tabs::RADIUS, theme.controls.container);
    canvas.stroke_rounded(outline.inset(0.5, 0.5), tabs::RADIUS - 0.5, 1.0, theme.controls.stroke);
    for (tab, rect, label, icon, label_w) in [
        (FlyoutTab::Output, first, &bar.output, Icon::Volume3, view.layout.tab_labels[0]),
        (FlyoutTab::Input, second, &bar.input, Icon::Microphone, view.layout.tab_labels[1]),
    ] {
        let selected = bar.selected == tab;
        let s = state(view, NodeKey::Tab(tab));
        if selected {
            let fill = if s.hot && theme.high_contrast {
                theme.controls.fill_hover
            } else {
                theme.controls.fill
            };
            canvas.fill_rounded(rect, tabs::TAB_RADIUS, fill);
            canvas.stroke_rounded(
                rect.inset(0.5, 0.5),
                tabs::TAB_RADIUS - 0.5,
                1.0,
                theme.controls.stroke,
            );
            let (pw, ph) = tabs::PILL;
            let pill =
                Rect::new(rect.x + (rect.width - pw) / 2.0, rect.bottom() - ph - 1.0, pw, ph);
            canvas.fill_rounded(pill, ph / 2.0, theme.accent_fill);
        } else {
            subtle(canvas, rect, tabs::TAB_RADIUS, s, theme);
        }
        let color =
            if selected { theme.text_primary } else { subtle_text(s, theme, theme.text_secondary) };
        // Icon and label centred together.
        let style = tab_style(selected);
        let total = tabs::ICON_SIZE + tabs::ICON_GAP + label_w;
        let x = rect.x + ((rect.width - total) / 2.0).max(4.0);
        let icon_box = Rect::new(x, rect.y, tabs::ICON_SIZE, rect.height);
        canvas.glyph(icon.glyph(), tabs::ICON_SIZE, icon_box, color);
        let text_x = icon_box.right() + tabs::ICON_GAP;
        let text_box = Rect::new(text_x, rect.y, rect.right() - text_x - 4.0, rect.height);
        canvas.text(label, style, text_box, TextAlign::Leading, color);
    }
}

fn paint_tiles(view: &FlyoutView, canvas: &mut Canvas<'_>, theme: &Theme) {
    if let Some(empty) = view.layout.empty {
        let rect = view.placed(Block::Top, empty);
        canvas.text(
            &view.content.devices.empty,
            CAPTION,
            rect,
            TextAlign::Center,
            theme.text_secondary,
        );
        return;
    }
    for device in &view.content.devices.tiles {
        let key = NodeKey::Tile(device.key);
        let Some(rect) = view.node_rect(key) else { continue };
        let s = state(view, key);
        let (fill, fg) = if device.current {
            let fill = if s.pressed {
                theme.controls.accent_pressed
            } else if s.hot {
                theme.accent_fill_hover
            } else {
                theme.accent_fill
            };
            (fill, theme.text_on_accent)
        } else {
            let fill = if s.pressed {
                theme.controls.fill_pressed
            } else if s.hot {
                theme.controls.fill_hover
            } else {
                theme.controls.fill
            };
            let fg = if theme.high_contrast && (s.hot || s.pressed) {
                theme.controls.hover_text
            } else if s.pressed {
                theme.text_secondary
            } else {
                theme.text_primary
            };
            (fill, fg)
        };
        canvas.fill_rounded(rect, tile::RADIUS, fill);
        if !device.current || theme.high_contrast {
            canvas.stroke_rounded(
                rect.inset(0.5, 0.5),
                tile::RADIUS - 0.5,
                1.0,
                theme.controls.stroke,
            );
        }
        let icon_x = (rect.x + (rect.width - tile::ICON_SIZE) / 2.0).round();
        let icon_y = rect.y + tile::PADDING_TOP;
        device_icon::paint(canvas, device.icon, Point::new(icon_x, icon_y), tile::ICON_SIZE, fg);
        let label_y = icon_y + tile::ICON_SIZE + tile::ICON_GAP;
        let label = Rect::new(
            rect.x + tile::PADDING_X,
            label_y,
            rect.width - 2.0 * tile::PADDING_X,
            rect.bottom() - tile::PADDING_BOTTOM - label_y,
        );
        canvas.text_wrapped(
            &device.name,
            tile_style(device.current),
            label,
            TextAlign::Center,
            tile::LABEL_LINES,
            fg,
        );
        if device.calls.is_some() {
            let marker = Rect::new(
                rect.right() - tile::MARKER_INSET - tile::MARKER_SIZE,
                rect.y + tile::MARKER_INSET,
                tile::MARKER_SIZE,
                tile::MARKER_SIZE,
            );
            canvas.glyph(Icon::Calls.glyph(), tile::MARKER_SIZE, marker, fg);
        }
    }
}

fn paint_middle(
    view: &FlyoutView,
    content: &FlyoutContent,
    layout: &Layout,
    origin: f32,
    live: bool,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
) {
    let Some(apps) = &content.apps else { return };
    if let Some(node) = layout.node(NodeKey::Header) {
        let rect = Rect::new(node.rect.x, node.rect.y + origin, node.rect.width, node.rect.height);
        let s = if live {
            state(view, NodeKey::Header)
        } else {
            State { enabled: true, ..State::default() }
        };
        subtle(canvas, rect, section::RADIUS, s, theme);
        let color = subtle_text(s, theme, theme.text_secondary);
        let text = Rect::new(
            rect.x + section::PADDING_X,
            rect.y,
            rect.width - 2.0 * section::PADDING_X - section::CHEVRON - 8.0,
            rect.height,
        );
        canvas.text(&apps.title, CAPTION_STRONG, text, TextAlign::Leading, color);
        let chevron = if apps.expanded { Icon::ChevronUp } else { Icon::ChevronDown };
        let glyph_box = Rect::new(
            rect.right() - section::PADDING_X - section::CHEVRON,
            rect.y,
            section::CHEVRON,
            rect.height,
        );
        canvas.glyph(chevron.glyph(), section::CHEVRON, glyph_box, color);
    }
    for row in content.visible_rows() {
        paint_volume(view, row, layout, origin, live, canvas, theme);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_volume(
    view: &FlyoutView,
    row: &VolumeControl,
    layout: &Layout,
    origin: f32,
    live: bool,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
) {
    let at = |rect: Rect| Rect::new(rect.x, rect.y + origin, rect.width, rect.height);
    let target = row.target;
    if let Some((_, label, _)) = layout.labels.iter().find(|(t, _, _)| *t == target) {
        let color = if row.enabled { theme.text_secondary } else { theme.text_disabled };
        canvas.text_wrapped(&row.label, CAPTION, at(*label), TextAlign::Leading, 1, color);
    }
    let still = State { enabled: row.enabled, ..State::default() };
    if let Some(node) = layout.node(NodeKey::Mute(target)) {
        let rect = at(node.rect);
        let s = if live { state(view, NodeKey::Mute(target)) } else { still };
        subtle(canvas, rect, volume_row::BUTTON_RADIUS, s, theme);
        paint_volume_icon(canvas, row, rect, subtle_text(s, theme, theme.text_primary), theme);
    }
    if let Some(node) = layout.node(NodeKey::Slider(target)) {
        let rect = at(node.rect);
        let s = if live { state(view, NodeKey::Slider(target)) } else { still };
        let value = match view.drag {
            Some(drag) if live && drag.target == target => drag.value,
            _ => row.volume,
        };
        paint_slider(canvas, rect, value, s, theme);
        let color = if row.enabled { theme.text_secondary } else { theme.text_disabled };
        let text = if row.enabled { value.to_string() } else { String::new() };
        canvas.text(&text, BODY, value_rect(rect), TextAlign::Trailing, color);
    }
}

fn paint_volume_icon(
    canvas: &mut Canvas<'_>,
    row: &VolumeControl,
    rect: Rect,
    color: Color,
    theme: &Theme,
) {
    let glyph = |canvas: &mut Canvas<'_>, icon: Icon| {
        canvas.glyph(icon.glyph(), volume_row::GLYPH, rect, color);
    };
    match &row.icon {
        VolumeIcon::Level(level) => glyph(canvas, level.icon()),
        VolumeIcon::Microphone => {
            glyph(canvas, if row.muted { Icon::MicrophoneMuted } else { Icon::Microphone })
        }
        VolumeIcon::SystemSounds | VolumeIcon::App(_) => {
            let size = volume_row::APP_ICON;
            let icon = Rect::new(
                (rect.x + (rect.width - size) / 2.0).round(),
                (rect.y + (rect.height - size) / 2.0).round(),
                size,
                size,
            );
            let opacity = if row.muted || !row.enabled { 0.5 } else { 1.0 };
            match &row.icon {
                VolumeIcon::App(Some(image)) => canvas.image(image, icon, opacity),
                VolumeIcon::App(None) => {
                    canvas.glyph(Icon::App.glyph(), size, icon, color.with_alpha(color.a * opacity))
                }
                _ => canvas.glyph(
                    Icon::SystemSounds.glyph(),
                    volume_row::GLYPH,
                    icon,
                    color.with_alpha(color.a * opacity),
                ),
            }
            if row.muted {
                // A small crossed speaker on an opaque disc at the bottom-right corner.
                let d = volume_row::BADGE;
                let centre =
                    Point::new(icon.right() - d / 2.0 + 3.0, icon.bottom() - d / 2.0 + 3.0);
                canvas.fill_circle(centre, d / 2.0, theme.controls.solid);
                canvas.stroke_circle(centre, d / 2.0 - 0.5, 1.0, theme.controls.solid_stroke);
                let badge = Rect::new(centre.x - d / 2.0, centre.y - d / 2.0, d, d);
                canvas.glyph(Icon::VolumeMute.glyph(), d - 4.0, badge, theme.text_primary);
            }
        }
    }
}

/// The Windows 11 slider: a 4-DIP track filled with the accent up to the thumb, a
/// 20-DIP thumb with an accent disc that grows under the pointer and shrinks pressed.
fn paint_slider(canvas: &mut Canvas<'_>, rect: Rect, value: u8, s: State, theme: &Theme) {
    let cy = rect.y + rect.height / 2.0;
    let x = thumb_x(rect, value);
    let track = Rect::new(rect.x, cy - slider::TRACK / 2.0, rect.width, slider::TRACK);
    let radius = slider::TRACK / 2.0;
    let (fill, rest, inner) = if s.enabled {
        let inner = if theme.high_contrast && (s.hot || s.pressed) {
            theme.controls.fill_hover
        } else if s.pressed {
            theme.controls.accent_pressed
        } else if s.hot {
            theme.accent_fill_hover
        } else {
            theme.accent_fill
        };
        (theme.accent_fill, theme.controls.slider_track, inner)
    } else {
        (
            theme.controls.accent_disabled,
            theme.controls.accent_disabled,
            theme.controls.accent_disabled,
        )
    };
    canvas.fill_rounded(track, radius, rest);
    if x > track.x {
        canvas.fill_rounded(Rect::new(track.x, track.y, x - track.x, track.height), radius, fill);
    }
    let centre = Point::new(x, cy);
    canvas.fill_circle(centre, slider::THUMB / 2.0, theme.controls.slider_thumb);
    canvas.stroke_circle(
        centre,
        slider::THUMB / 2.0 - 0.5,
        1.0,
        theme.controls.slider_thumb_stroke,
    );
    let (rest_d, hot_d, pressed_d) = slider::INNER;
    let d = if !s.enabled {
        rest_d
    } else if s.pressed {
        pressed_d
    } else if s.hot {
        hot_d
    } else {
        rest_d
    };
    canvas.fill_circle(centre, d / 2.0, inner);
}

fn paint_footer(view: &FlyoutView, canvas: &mut Canvas<'_>, theme: &Theme) {
    if let Some(rect) = view.node_rect(NodeKey::SoundSettings) {
        let s = state(view, NodeKey::SoundSettings);
        subtle(canvas, rect, footer::RADIUS, s, theme);
        let text = rect.inset(footer::PADDING_X, 0.0);
        let color = subtle_text(s, theme, theme.text_secondary);
        canvas.text(&view.content.footer.sound_settings, CAPTION, text, TextAlign::Leading, color);
    }
    if let Some(rect) = view.node_rect(NodeKey::Settings) {
        let s = state(view, NodeKey::Settings);
        subtle(canvas, rect, footer::RADIUS, s, theme);
        let color = subtle_text(s, theme, theme.text_primary);
        canvas.glyph(Icon::Settings.glyph(), footer::GLYPH, rect, color);
    }
}

/// The update line: download glyph in the accent colour, the text, then a
/// hyperlink-style button and a close button.
fn paint_update(
    view: &FlyoutView,
    update: &UpdateLine,
    text: Rect,
    canvas: &mut Canvas<'_>,
    theme: &Theme,
) {
    let glyph_x = text.x - footer::UPDATE_GAP - footer::GLYPH;
    let glyph = Rect::new(glyph_x, text.y, footer::GLYPH, text.height);
    canvas.glyph(Icon::Download.glyph(), footer::GLYPH, glyph, theme.window.link);
    canvas.text_wrapped(
        &update.text,
        CAPTION,
        text.inset(0.0, (text.height - CAPTION.line_height * canvas.text_scale()).max(0.0) / 2.0),
        TextAlign::Leading,
        1,
        theme.text_primary,
    );
    if let (Some(label), Some(rect)) = (&update.action, view.node_rect(NodeKey::UpdateAction)) {
        let s = state(view, NodeKey::UpdateAction);
        subtle(canvas, rect, footer::RADIUS, s, theme);
        let color = if s.hot || s.pressed { theme.window.link_hover } else { theme.window.link };
        canvas.text(label, CAPTION_STRONG, rect, TextAlign::Center, color);
    }
    if let Some(rect) = view.node_rect(NodeKey::UpdateClose) {
        let s = state(view, NodeKey::UpdateClose);
        subtle(canvas, rect, footer::RADIUS, s, theme);
        let color = subtle_text(s, theme, theme.text_secondary);
        canvas.glyph(Icon::Close.glyph(), footer::CLOSE_GLYPH, rect, color);
    }
}

fn paint_scroll_indicator(view: &FlyoutView, canvas: &mut Canvas<'_>, theme: &Theme, clip: Rect) {
    let total = view.layout.middle_height.max(1.0);
    let visible = clip.height;
    let length = (visible * visible / total).max(16.0).min(visible);
    let range = view.scroll_range().max(1.0);
    let y = clip.y + (visible - length) * (view.scroll / range);
    let w = flyout::SCROLL_INDICATOR;
    let bar = Rect::new(clip.right() - w - 3.0, y + 2.0, w, (length - 4.0).max(4.0));
    canvas.fill_rounded(bar, w / 2.0, theme.controls.scroll_indicator);
}
