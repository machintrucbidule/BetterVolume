//! Control renderers. Each control owns its layout (in DIPs), its interaction state and its
//! painting; the shell only forwards input and hosts the window.

pub mod bubble;
pub mod device_icon;
pub mod flyout;
pub mod menu;
pub mod settings;
pub mod tooltip;
pub mod tray;

use crate::backend::Canvas;
use crate::theme::Material;
use crate::tokens::popup::CORNER_RADIUS;
use crate::{Rect, Size, Theme};

/// Paints the background of a popup surface (flyout, menu). With the translucent
/// material, the system draws the blurred backdrop behind the window and this only adds
/// the tint; the rounded corners and the outline come from the window frame.
pub fn paint_surface(canvas: &mut Canvas<'_>, theme: &Theme, size: Size) {
    let full = Rect::from_size(size);
    match theme.material {
        Material::Acrylic { tint, .. } => {
            canvas.clear(crate::Color::TRANSPARENT);
            canvas.fill_rect(full, tint);
        }
        Material::Solid(color) => canvas.clear(color),
    }
    if theme.high_contrast {
        // High contrast: a solid outline inside the edge, as system surfaces draw.
        canvas.stroke_rounded(full.inset(0.5, 0.5), CORNER_RADIUS, 1.0, theme.surface_stroke);
    }
}
