//! The safe drawing surface handed to painters.

use std::mem::ManuallyDrop;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Brush, ID2D1Factory1, ID2D1RenderTarget, ID2D1SolidColorBrush,
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_CLIP,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_NO_SNAP, D2D1_ELLIPSE,
    D2D1_EXTEND_MODE_CLAMP, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_GAMMA_2_2, D2D1_LAYER_OPTIONS_NONE,
    D2D1_LAYER_PARAMETERS, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING,
    DWRITE_TEXT_ALIGNMENT_TRAILING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::IWICBitmap;

use windows_numerics::{Matrix3x2, Vector2};

use super::{Graphics, GraphicsError};
use crate::path::Path;
use crate::tokens::typography::TextStyle;
use crate::{Color, Point, Rect};

/// Horizontal alignment of a text line in its box (it is always centred vertically).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    /// Left (right in right-to-left languages).
    #[default]
    Leading,
    /// Centre.
    Center,
    /// Right (left in right-to-left languages).
    Trailing,
}

fn color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn rect(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x, top: r.y, right: r.right(), bottom: r.bottom() }
}

/// A linear gradient between two points, in DIPs of the canvas it is used on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    /// Where the first colour is.
    pub from: Point,
    /// Where the last colour is.
    pub to: Point,
    /// The colours, from `from` to `to`.
    pub colors: [Color; 2],
}

/// Drawing operations in DIPs. Failures inside a frame are reported once, by the
/// function that ends the frame.
pub struct Canvas<'a> {
    target: &'a ID2D1RenderTarget,
    graphics: &'a Graphics,
    brush: ID2D1SolidColorBrush,
    text_scale: f32,
    error: Option<GraphicsError>,
    /// Inside a vertical stretch: icon glyphs are placed exactly, not snapped to pixels.
    exact: bool,
    /// Inside [`Canvas::with_gradient`]: the brush every drawing uses instead of a colour.
    gradient: Option<ID2D1Brush>,
}

impl Canvas<'_> {
    fn keep(&mut self, result: Result<(), GraphicsError>) {
        if let Err(error) = result {
            self.error.get_or_insert(error);
        }
    }

    /// The brush of a drawing in `c`: a solid one, or the gradient set by
    /// [`Canvas::with_gradient`] at the opacity of `c`.
    fn brush(&self, c: Color) -> ID2D1Brush {
        if let Some(gradient) = &self.gradient {
            // SAFETY: plain setter on our own brush.
            unsafe { gradient.SetOpacity(c.a) };
            return gradient.clone();
        }
        // SAFETY: plain setter on our own brush.
        unsafe { self.brush.SetColor(&color(c)) };
        (*self.brush).clone()
    }

    /// Runs `draw` with every drawing painted with `gradient` instead of its colour (the
    /// colour's opacity still applies).
    pub fn with_gradient(&mut self, gradient: &Gradient, draw: impl FnOnce(&mut Self)) {
        let stops = gradient
            .colors
            .iter()
            .enumerate()
            .map(|(i, c)| D2D1_GRADIENT_STOP {
                position: i as f32 / (gradient.colors.len() - 1) as f32,
                color: color(*c),
            })
            .collect::<Vec<_>>();
        let properties = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
            startPoint: Vector2 { X: gradient.from.x, Y: gradient.from.y },
            endPoint: Vector2 { X: gradient.to.x, Y: gradient.to.y },
        };
        // SAFETY: plain creations on a valid render target; `stops` outlives the call.
        let brush = unsafe {
            self.target
                .CreateGradientStopCollection(&stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)
                .and_then(|collection| {
                    self.target.CreateLinearGradientBrush(&properties, None, &collection)
                })
        };
        match brush {
            Ok(brush) => {
                let previous = self.gradient.replace((*brush).clone());
                draw(self);
                self.gradient = previous;
            }
            Err(error) => self.keep(Err(error.into())),
        }
    }

    /// Windows text scaling factor this canvas draws text at.
    pub fn text_scale(&self) -> f32 {
        self.text_scale
    }

    /// Replaces every pixel with `c` (transparent included).
    pub fn clear(&mut self, c: Color) {
        // SAFETY: plain call inside BeginDraw/EndDraw.
        unsafe { self.target.Clear(Some(&color(c))) };
    }

    /// Runs `draw` as one picture drawn at `opacity`: where its parts overlap, the overlap
    /// is not darker (each part is drawn opaque inside the group, the group is blended
    /// once).
    pub fn with_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
        let parameters = D2D1_LAYER_PARAMETERS {
            contentBounds: D2D_RECT_F {
                left: f32::MIN,
                top: f32::MIN,
                right: f32::MAX,
                bottom: f32::MAX,
            },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS_NONE,
        };
        // SAFETY: plain calls inside BeginDraw/EndDraw; the parameters outlive the call
        // and hold no object (nothing to release); no layer object is needed since
        // Windows 8. Popped below (draw cannot pop it: no such method).
        unsafe { self.target.PushLayer(&parameters, None) };
        draw(self);
        // SAFETY: pops the layer pushed above.
        unsafe { self.target.PopLayer() };
    }

    /// Runs `draw` with drawing limited to the rectangle `r`.
    pub fn clipped(&mut self, r: Rect, draw: impl FnOnce(&mut Self)) {
        // SAFETY: plain call inside BeginDraw/EndDraw; popped below.
        unsafe { self.target.PushAxisAlignedClip(&rect(r), D2D1_ANTIALIAS_MODE_ALIASED) };
        draw(self);
        // SAFETY: pops the clip pushed above (draw cannot pop it: no such method).
        unsafe { self.target.PopAxisAlignedClip() };
    }

    /// Runs `draw` with everything stretched vertically by `factor` about the horizontal
    /// line `y = centre` (DIPs); widths are unchanged. Text is drawn there without grid
    /// fitting: its outlines keep their exact stretched geometry (grid fitting would move
    /// their vertical extremes by up to half a pixel, past the edge of a drawing that fills
    /// its height).
    pub fn stretched_vertically(&mut self, factor: f32, centre: f32, draw: impl FnOnce(&mut Self)) {
        self.scaled(1.0, factor, Point::new(0.0, centre), draw);
    }

    /// Runs `draw` with everything scaled by `sx` horizontally and `sy` vertically about
    /// the point `about` (DIPs). Icon glyphs are drawn there without grid fitting, as in
    /// [`Canvas::stretched_vertically`].
    pub fn scaled(&mut self, sx: f32, sy: f32, about: Point, draw: impl FnOnce(&mut Self)) {
        self.exactly([sx, sy], [about.x * (1.0 - sx), about.y * (1.0 - sy)], draw);
    }

    /// Runs `draw` scaled by `scale` with its origin moved to `origin` (DIPs): a drawing
    /// made for a square of `n` DIPs fills the square of `n · scale` DIPs at `origin`.
    /// Icon glyphs are drawn there without grid fitting, as in [`Canvas::scaled`].
    pub fn placed(&mut self, origin: Point, scale: f32, draw: impl FnOnce(&mut Self)) {
        self.exactly([scale, scale], [origin.x, origin.y], draw);
    }

    /// Runs `draw` transformed (see `transformed`), icon glyphs placed exactly.
    fn exactly(&mut self, scale: [f32; 2], shift: [f32; 2], draw: impl FnOnce(&mut Self)) {
        // SAFETY: plain getter; an error means "no parameters set" (the default ones).
        let previous = unsafe { self.target.GetTextRenderingParams() }.ok();
        let exact = self.graphics.ungridded_text_params();
        if let Some(params) = &exact {
            // SAFETY: plain setter inside BeginDraw/EndDraw; restored below.
            unsafe { self.target.SetTextRenderingParams(params) };
        }
        let was_exact = std::mem::replace(&mut self.exact, true);
        self.transformed(scale, shift, draw);
        self.exact = was_exact;
        if exact.is_some() {
            // SAFETY: as above.
            unsafe { self.target.SetTextRenderingParams(previous.as_ref()) };
        }
    }

    /// Runs `draw` moved down by `dy` DIPs (tests: room around a drawing to see that
    /// nothing of it is cut).
    #[cfg(test)]
    pub(crate) fn shifted_down(&mut self, dy: f32, draw: impl FnOnce(&mut Self)) {
        self.transformed([1.0, 1.0], [0.0, dy], draw);
    }

    /// Runs `draw` with `x' = scale[0]·x + shift[0]`, `y' = scale[1]·y + shift[1]` applied
    /// before the current transform.
    fn transformed(&mut self, scale: [f32; 2], shift: [f32; 2], draw: impl FnOnce(&mut Self)) {
        let mut current = Matrix3x2::default();
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid out-pointer.
        unsafe { self.target.GetTransform(&mut current) };
        let changed = Matrix3x2 {
            M11: scale[0] * current.M11,
            M12: scale[0] * current.M12,
            M21: scale[1] * current.M21,
            M22: scale[1] * current.M22,
            M31: shift[0] * current.M11 + shift[1] * current.M21 + current.M31,
            M32: shift[0] * current.M12 + shift[1] * current.M22 + current.M32,
        };
        // SAFETY: plain calls inside BeginDraw/EndDraw; the transform is restored below
        // (draw cannot change it: no such method).
        unsafe { self.target.SetTransform(&changed) };
        draw(self);
        // SAFETY: as above.
        unsafe { self.target.SetTransform(&current) };
    }

    /// Fills a rectangle.
    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        let brush = self.brush(c).clone();
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.FillRectangle(&rect(r), &brush) };
    }

    /// Fills a rounded rectangle.
    pub fn fill_rounded(&mut self, r: Rect, radius: f32, c: Color) {
        let brush = self.brush(c).clone();
        let rounded = D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius };
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.FillRoundedRectangle(&rounded, &brush) };
    }

    /// Draws the outline of a rounded rectangle, centred on its edges.
    pub fn stroke_rounded(&mut self, r: Rect, radius: f32, width: f32, c: Color) {
        let brush = self.brush(c).clone();
        let rounded = D2D1_ROUNDED_RECT { rect: rect(r), radiusX: radius, radiusY: radius };
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.DrawRoundedRectangle(&rounded, &brush, width, None) };
    }

    /// Draws a straight line.
    pub fn line(&mut self, from: Point, to: Point, width: f32, c: Color) {
        let brush = self.brush(c).clone();
        let (a, b) = (Vector2 { X: from.x, Y: from.y }, Vector2 { X: to.x, Y: to.y });
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.DrawLine(a, b, &brush, width, None) };
    }

    /// Fills a circle.
    pub fn fill_circle(&mut self, center: Point, radius: f32, c: Color) {
        let brush = self.brush(c).clone();
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: center.x, Y: center.y },
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.FillEllipse(&ellipse, &brush) };
    }

    /// Draws the outline of a circle, centred on its edge.
    pub fn stroke_circle(&mut self, center: Point, radius: f32, width: f32, c: Color) {
        let brush = self.brush(c).clone();
        let ellipse = D2D1_ELLIPSE {
            point: Vector2 { X: center.x, Y: center.y },
            radiusX: radius,
            radiusY: radius,
        };
        // SAFETY: plain call inside BeginDraw/EndDraw with a valid brush.
        unsafe { self.target.DrawEllipse(&ellipse, &brush, width, None) };
    }

    /// Draws the outline of a path with round caps and joins (the Fluent icon stroke).
    pub fn stroke_path(&mut self, path: &Path, width: f32, c: Color) {
        let result = self.graphics.geometry(path).and_then(|geometry| {
            let style = self.graphics.round_stroke()?;
            let brush = self.brush(c).clone();
            // SAFETY: plain call inside BeginDraw/EndDraw with valid objects of one factory.
            unsafe { self.target.DrawGeometry(&geometry, &brush, width, &style) };
            Ok(())
        });
        self.keep(result);
    }

    /// Fills a path.
    pub fn fill_path(&mut self, path: &Path, c: Color) {
        let result = self.graphics.geometry(path).map(|geometry| {
            let brush = self.brush(c).clone();
            // SAFETY: plain call inside BeginDraw/EndDraw with valid objects of one factory.
            unsafe { self.target.FillGeometry(&geometry, &brush, None) };
        });
        self.keep(result);
    }

    /// Draws one line of text in `r` (clipped to it), centred vertically.
    pub fn text(&mut self, text: &str, style: TextStyle, r: Rect, align: TextAlign, c: Color) {
        let result = self.graphics.text_format(style, self.text_scale).and_then(|format| {
            let alignment = match align {
                TextAlign::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                TextAlign::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
            };
            // SAFETY: plain setter; formats are only used on this thread, one draw at a time.
            unsafe { format.SetTextAlignment(alignment) }?;
            let wide: Vec<u16> = text.encode_utf16().collect();
            let brush = self.brush(c).clone();
            // SAFETY: plain call inside BeginDraw/EndDraw; buffers outlive it.
            unsafe {
                self.target.DrawText(
                    &wide,
                    &format,
                    &rect(r),
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    DWRITE_MEASURING_MODE_NATURAL,
                )
            };
            Ok(())
        });
        self.keep(result);
    }

    /// Draws a glyph of the Windows icon font, `size` DIPs, centred in `r`.
    pub fn glyph(&mut self, glyph: char, size: f32, r: Rect, c: Color) {
        let result = self.graphics.icon_format(size).map(|format| {
            let mut buffer = [0u16; 2];
            let wide = glyph.encode_utf16(&mut buffer);
            let brush = self.brush(c).clone();
            // SAFETY: plain call inside BeginDraw/EndDraw; buffers outlive it.
            unsafe {
                self.target.DrawText(
                    wide,
                    &format,
                    &rect(r),
                    &brush,
                    if self.exact {
                        D2D1_DRAW_TEXT_OPTIONS_NO_SNAP
                    } else {
                        D2D1_DRAW_TEXT_OPTIONS_NONE
                    },
                    DWRITE_MEASURING_MODE_NATURAL,
                )
            };
        });
        self.keep(result);
    }
}

/// A paragraph of text laid out for drawing or measuring.
pub(crate) struct Paragraph {
    pub(crate) layout: windows::Win32::Graphics::DirectWrite::IDWriteTextLayout,
}

impl Canvas<'_> {
    /// Draws text wrapped on at most `max_lines` lines in `r` (top-aligned), the last line
    /// ellipsised if the text does not fit.
    pub fn text_wrapped(
        &mut self,
        text: &str,
        style: TextStyle,
        r: Rect,
        align: TextAlign,
        max_lines: u32,
        c: Color,
    ) {
        let height = style.line_height * self.text_scale * max_lines as f32;
        let result = self
            .graphics
            .paragraph(text, style, self.text_scale, r.width, height, align, true)
            .map(|paragraph| {
                let brush = self.brush(c).clone();
                // SAFETY: plain call inside BeginDraw/EndDraw with valid objects.
                unsafe {
                    self.target.DrawTextLayout(
                        Vector2 { X: r.x, Y: r.y },
                        &paragraph.layout,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    )
                };
            });
        self.keep(result);
    }

    /// Draws an image (premultiplied BGRA) scaled into `r`, with an opacity.
    pub fn image(&mut self, image: &super::Pixels, r: Rect, opacity: f32) {
        if image.width == 0 || image.height == 0 {
            return;
        }
        let properties = windows::Win32::Graphics::Direct2D::D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let size = windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U {
            width: image.width,
            height: image.height,
        };
        // SAFETY: `data` holds `width * height * 4` bytes (contract of `Pixels`), read
        // during the call only; the pitch matches.
        let bitmap = unsafe {
            self.target.CreateBitmap(
                size,
                Some(image.data.as_ptr().cast()),
                image.width * 4,
                &properties,
            )
        };
        match bitmap {
            // SAFETY: plain call inside BeginDraw/EndDraw with a bitmap of this target.
            Ok(bitmap) => unsafe {
                self.target.DrawBitmap(
                    &bitmap,
                    Some(&rect(r)),
                    opacity,
                    windows::Win32::Graphics::Direct2D::D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                )
            },
            Err(error) => self.keep(Err(error.into())),
        }
    }
}

/// A software render target drawing into a WIC bitmap, in DIPs at `scale`.
pub(crate) fn wic_target(
    factory: &ID2D1Factory1,
    bitmap: &IWICBitmap,
    scale: f32,
) -> Result<ID2D1RenderTarget, GraphicsError> {
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0 * scale,
        dpiY: 96.0 * scale,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    // SAFETY: `bitmap` is a valid WIC bitmap in a format Direct2D supports.
    Ok(unsafe { factory.CreateWicBitmapRenderTarget(bitmap, &properties) }?)
}

/// Runs one frame on `target`: clears nothing, lets `paint` draw, reports the first
/// failure of the frame.
pub(crate) fn draw(
    graphics: &Graphics,
    target: &ID2D1RenderTarget,
    text_scale: f32,
    paint: impl FnOnce(&mut Canvas<'_>),
) -> Result<(), GraphicsError> {
    // SAFETY: plain calls on a valid render target.
    let brush = unsafe { target.CreateSolidColorBrush(&color(Color::BLACK), None) }?;
    // SAFETY: as above. Grayscale: ClearType is not allowed on transparent surfaces.
    unsafe {
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.BeginDraw();
    }
    let mut canvas =
        Canvas { target, graphics, brush, text_scale, error: None, exact: false, gradient: None };
    paint(&mut canvas);
    let error = canvas.error.take();
    drop(canvas);
    // SAFETY: ends the frame begun above.
    let ended = unsafe { target.EndDraw(None, None) };
    if let Some(error) = error {
        return Err(error);
    }
    Ok(ended?)
}
