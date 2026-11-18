//! The bubble above the taskbar: the device-switch notice (FR-105) and the volume
//! feedback of the wheel and middle click (FR-017), a copy of the Windows 11
//! volume indicator. It never takes the focus and lets clicks through; the shell shows
//! it bottom centre above the taskbar and hides it after a while.
//!
//! The window frame draws the rounded corners and the shadow; the bubble paints an opaque
//! surface, as the Windows indicator looks (its material is all but opaque).

use crate::backend::{Canvas, Graphics, TextAlign};
use crate::controls::device_icon::{self, DeviceIcon};
use crate::icons::{Icon, VolumeLevel};
use crate::theme::Material;
use crate::tokens::bubble::{
    BAR, DEVICE_ICON, DEVICE_MAX_WIDTH, DEVICE_PADDING_RIGHT, DEVICE_PADDING_Y, GLYPH, HEIGHT,
    PADDING_X, VALUE_MIN_WIDTH, WIDTH,
};
use crate::tokens::typography::{BODY, CAPTION};
use crate::{Point, Rect, Size, Theme};

/// What the bubble shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BubbleContent {
    /// The current device changed (FR-105): its drawing, its display name, and what it is
    /// ("Output", "Microphone · calls"...), all ready to show.
    Device {
        /// Drawing of the device.
        icon: DeviceIcon,
        /// Display name (never translated, FR-122).
        name: String,
        /// Kind line, translated.
        kind: String,
    },
    /// A short notice of BetterVolume itself, such as "updated": a glyph, a
    /// title and a detail line, laid out like the device notice.
    Notice {
        /// The glyph, in the accent colour.
        icon: Icon,
        /// First line, translated.
        title: String,
        /// Second line, translated.
        detail: String,
    },
    /// The output volume after a wheel turn or a middle click (FR-017).
    Volume {
        /// Step of the volume glyph (muted included).
        level: VolumeLevel,
        /// Volume in whole percent.
        percent: u8,
    },
}

/// The theme the bubble is painted with: the shell theme on an opaque surface.
pub fn theme(shell: Theme) -> Theme {
    Theme { material: Material::Solid(shell.controls.solid), ..shell }
}

/// Size of the bubble for `content`, in DIPs.
pub fn size(content: &BubbleContent, graphics: &Graphics, text_scale: f32) -> Size {
    match content {
        BubbleContent::Volume { .. } => {
            let value = value_width(graphics, text_scale);
            let width = WIDTH - VALUE_MIN_WIDTH + value;
            Size::new(width.ceil(), HEIGHT.max(BODY.line_height * text_scale + 8.0).ceil())
        }
        BubbleContent::Device { name, kind, .. }
        | BubbleContent::Notice { title: name, detail: kind, .. } => {
            let measure = |text: &str, style| {
                graphics.measure_text(text, style, text_scale).map_or(0.0, |s| s.width)
            };
            let text = measure(name, BODY).max(measure(kind, CAPTION));
            let fixed = PADDING_X + DEVICE_ICON.0 + DEVICE_ICON.1 + DEVICE_PADDING_RIGHT;
            let width = (fixed + text).clamp(WIDTH, DEVICE_MAX_WIDTH);
            let lines = (BODY.line_height + CAPTION.line_height) * text_scale;
            Size::new(width.ceil(), HEIGHT.max(lines + 2.0 * DEVICE_PADDING_Y).ceil())
        }
    }
}

/// Room for the number: the widest value, "100", or the minimum.
fn value_width(graphics: &Graphics, text_scale: f32) -> f32 {
    graphics
        .measure_text("100", BODY, text_scale)
        .map_or(VALUE_MIN_WIDTH, |s| s.width.ceil())
        .max(VALUE_MIN_WIDTH)
}

/// Where the parts of a volume bubble go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeLayout {
    /// The volume glyph's box.
    pub glyph: Rect,
    /// The whole level bar.
    pub bar: Rect,
    /// The number's box (right-aligned in it).
    pub value: Rect,
}

/// Layout of a volume bubble of `size` (measured on the Windows indicator: glyph 13 from
/// the left, bar from 42 for 110, number right-aligned 12 from the right).
pub fn volume_layout(size: Size) -> VolumeLayout {
    let cy = size.height / 2.0;
    let glyph = Rect::new(GLYPH.0, cy - GLYPH.1 / 2.0, GLYPH.1, GLYPH.1);
    let bar = Rect::new(BAR.0, cy - BAR.2 / 2.0, BAR.1, BAR.2);
    let value_x = bar.right() + BAR.3;
    let value = Rect::new(value_x, 0.0, (size.width - PADDING_X - value_x).max(0.0), size.height);
    VolumeLayout { glyph, bar, value }
}

/// Paints the bubble (background included).
pub fn paint(canvas: &mut Canvas<'_>, theme: &Theme, content: &BubbleContent, size: Size) {
    crate::controls::paint_surface(canvas, theme, size);
    match content {
        BubbleContent::Volume { level, percent } => {
            let layout = volume_layout(size);
            canvas.glyph(level.icon().glyph(), GLYPH.1, layout.glyph, theme.text_primary);
            let radius = BAR.2 / 2.0;
            canvas.fill_rounded(layout.bar, radius, theme.controls.slider_track);
            let filled = layout.bar.width * f32::from((*percent).min(100)) / 100.0;
            if filled > 0.0 {
                let part = Rect::new(layout.bar.x, layout.bar.y, filled.max(BAR.2), BAR.2);
                canvas.fill_rounded(part, radius, theme.accent_fill);
            }
            canvas.text(
                &percent.to_string(),
                BODY,
                layout.value,
                TextAlign::Trailing,
                theme.text_primary,
            );
        }
        BubbleContent::Device { icon, name, kind } => {
            let icon_y = (size.height - DEVICE_ICON.0) / 2.0;
            device_icon::paint(
                canvas,
                *icon,
                Point::new(PADDING_X, icon_y),
                DEVICE_ICON.0,
                theme.accent_fill,
            );
            two_lines(canvas, theme, name, kind, size);
        }
        BubbleContent::Notice { icon, title, detail } => {
            let icon_y = (size.height - DEVICE_ICON.0) / 2.0;
            let place = Rect::new(PADDING_X, icon_y, DEVICE_ICON.0, DEVICE_ICON.0);
            canvas.glyph(icon.glyph(), DEVICE_ICON.0 * 0.75, place, theme.accent_fill);
            two_lines(canvas, theme, title, detail, size);
        }
    }
}

/// The two lines of text right of the picture of a device or notice bubble.
fn two_lines(canvas: &mut Canvas<'_>, theme: &Theme, name: &str, kind: &str, size: Size) {
    let text_scale = canvas.text_scale();
    let x = PADDING_X + DEVICE_ICON.0 + DEVICE_ICON.1;
    let width = (size.width - x - DEVICE_PADDING_RIGHT).max(0.0);
    let name_h = BODY.line_height * text_scale;
    let kind_h = CAPTION.line_height * text_scale;
    let top = (size.height - name_h - kind_h) / 2.0;
    canvas.text_wrapped(
        name,
        BODY,
        Rect::new(x, top, width, name_h),
        TextAlign::Leading,
        1,
        theme.text_primary,
    );
    canvas.text_wrapped(
        kind,
        CAPTION,
        Rect::new(x, top + name_h, width, kind_h),
        TextAlign::Leading,
        1,
        theme.text_secondary,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_layout_matches_the_windows_indicator() {
        let layout = volume_layout(Size::new(WIDTH, HEIGHT));
        assert_eq!(layout.glyph, Rect::new(13.0, 16.0, 16.0, 16.0));
        assert_eq!(layout.bar, Rect::new(42.0, 22.0, 110.0, 4.0));
        assert_eq!(layout.value.right(), WIDTH - 12.0);
        assert_eq!(layout.value.x, 156.0);
        assert_eq!(layout.value.width, VALUE_MIN_WIDTH);
    }

    #[test]
    fn the_bubble_is_opaque() {
        let shell = Theme::shell(&crate::ThemeInputs::default());
        assert_eq!(theme(shell).material, Material::Solid(shell.controls.solid));
    }
}
