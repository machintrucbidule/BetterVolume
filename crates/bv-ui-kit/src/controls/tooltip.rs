//! Tooltips: a small opaque card with one or more lines of text (Windows 11 tooltip).

use crate::backend::{Canvas, Graphics, TextAlign};
use crate::tokens::tooltip::{MAX_WIDTH, OFFSET, PADDING_X, PADDING_Y, RADIUS};
use crate::tokens::typography::CAPTION;
use crate::{Rect, Size, Theme};

/// Size of the tooltip for `text` (lines separated by `\n`, long lines wrapped).
pub fn size(text: &str, graphics: &Graphics, text_scale: f32) -> Size {
    let measured = graphics
        .measure_wrapped(text, CAPTION, text_scale, MAX_WIDTH - 2.0 * PADDING_X)
        .unwrap_or(Size::new(MAX_WIDTH / 2.0, CAPTION.line_height * text_scale));
    Size::new(
        (measured.width + 2.0 * PADDING_X).ceil().min(MAX_WIDTH),
        (measured.height + 2.0 * PADDING_Y).ceil(),
    )
}

/// Gap between the tooltip and the control it describes.
pub fn offset() -> f32 {
    OFFSET
}

/// Paints the tooltip (background included).
pub fn paint(canvas: &mut Canvas<'_>, theme: &Theme, text: &str, size: Size) {
    let full = Rect::from_size(size);
    canvas.clear(crate::Color::TRANSPARENT);
    canvas.fill_rounded(full, RADIUS, theme.controls.solid);
    canvas.stroke_rounded(full.inset(0.5, 0.5), RADIUS - 0.5, 1.0, theme.controls.solid_stroke);
    let lines = text.lines().count().max(1) as u32 + 4;
    canvas.text_wrapped(
        text,
        CAPTION,
        full.inset(PADDING_X, PADDING_Y),
        TextAlign::Leading,
        lines,
        theme.text_primary,
    );
}
