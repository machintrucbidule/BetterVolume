//! Rendering backend: Direct2D and DirectWrite on the software rasteriser (WARP), shown
//! through DirectComposition (`docs/ARCHITECTURE.md` §1).
//!
//! The only place of the design system where `unsafe` code is allowed. Everything
//! above it draws through the safe [`Canvas`].
//!
//! Two levels of cost, both created on first need:
//! - [`Graphics::new`] creates the factories only (Direct2D, DirectWrite, WIC). Enough to
//!   rasterise the tray icon in memory ([`Graphics::rasterize`]).
//! - The graphics device (WARP by default, hardware on request) is created the first time
//!   a window surface is needed, then kept: re-creating it on each opening would cost the
//!   ~40 ms of the first opening every time.
#![allow(unsafe_code)]

mod canvas;
mod surface;

use std::cell::{OnceCell, RefCell};

use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_BEZIER_SEGMENT, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FIGURE_END_OPEN,
    D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Device, ID2D1DeviceContext, ID2D1Factory, ID2D1Factory1,
    ID2D1PathGeometry, ID2D1StrokeStyle, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE,
    D2D1_ARC_SIZE_SMALL, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_DEBUG_LEVEL_NONE, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_OPTIONS,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_LINE_JOIN_ROUND, D2D1_STROKE_STYLE_PROPERTIES,
    D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
    D3D11_CREATE_DEVICE_SINGLETHREADED, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectComposition::{DCompositionCreateDevice, IDCompositionDevice};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory2, IDWriteRenderingParams,
    IDWriteRenderingParams1, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_GRID_FIT_MODE_DISABLED, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS,
    DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapCacheOnLoad,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

pub use canvas::{Canvas, Gradient, TextAlign};
pub use surface::WindowSurface;

use crate::path::{Path, Segment};
use crate::tokens::typography::{TextStyle, ICON_FAMILY, TEXT_FAMILY};
use crate::Size;
use windows_numerics::Vector2;

/// A graphics failure (device lost, out of memory, Windows refused an operation). Never
/// shown to the user; the caller logs it and carries on without the surface.
#[derive(Debug, Clone)]
pub struct GraphicsError(windows::core::Error);

impl std::fmt::Display for GraphicsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The code, not the system text: the log stays in English.
        write!(f, "graphics: Windows error 0x{:08X}", self.0.code().0 as u32)
    }
}

impl std::error::Error for GraphicsError {}

impl From<windows::core::Error> for GraphicsError {
    fn from(error: windows::core::Error) -> Self {
        Self(error)
    }
}

/// Which device rasterises window surfaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceKind {
    /// The Windows software rasteriser: ~2 MB and no extra thread (measured choice).
    #[default]
    Software,
    /// The graphics card: ~12 MB and dozens of driver threads; kept as a switch.
    Hardware,
}

/// An image in memory: premultiplied BGRA, 4 bytes per pixel, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pixels {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub data: Vec<u8>,
}

impl Pixels {
    /// The four bytes (blue, green, red, alpha, premultiplied) of one pixel.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }
}

/// The device objects behind window surfaces.
pub(crate) struct Gpu {
    pub(crate) d3d: ID3D11Device,
    pub(crate) context: ID2D1DeviceContext,
    pub(crate) composition: IDCompositionDevice,
    _device: ID2D1Device,
}

/// Keeps COM initialised on this thread for as long as the graphics objects live.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Apartment {
        // SAFETY: plain call; balanced by `CoUninitialize` in `drop` when it succeeded
        // (also when COM was already initialised on this thread: S_FALSE counts).
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Apartment(result.is_ok())
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `enter`, same thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Key of a cached text format.
#[derive(Clone, Copy, PartialEq)]
struct FormatKey {
    icon: bool,
    size: f32,
    weight: u16,
}

/// Graphics factories, and the device once a window surface needed it. Lives on the UI
/// thread (single-threaded factories).
pub struct Graphics {
    kind: DeviceKind,
    d2d: ID2D1Factory1,
    dwrite: IDWriteFactory,
    wic: IWICImagingFactory,
    gpu: OnceCell<Gpu>,
    round_stroke: OnceCell<ID2D1StrokeStyle>,
    formats: RefCell<Vec<(FormatKey, IDWriteTextFormat)>>,
    /// Locale name of the texts (empty: none); it picks the fonts of Asian scripts.
    text_locale: RefCell<HSTRING>,
    // Dropped last: COM must outlive every object above.
    _apartment: Apartment,
}

impl Graphics {
    /// Creates the factories. Cheap: no graphics device yet.
    pub fn new(kind: DeviceKind) -> Result<Graphics, GraphicsError> {
        let apartment = Apartment::enter();
        let options = D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE };
        // SAFETY: plain factory creation with valid options.
        let d2d: ID2D1Factory1 =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options)) }?;
        // SAFETY: plain factory creation.
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }?;
        // SAFETY: creates the in-process WIC factory; COM is initialised above.
        let wic: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;
        Ok(Graphics {
            kind,
            d2d,
            dwrite,
            wic,
            gpu: OnceCell::new(),
            round_stroke: OnceCell::new(),
            formats: RefCell::new(Vec::new()),
            text_locale: RefCell::new(HSTRING::new()),
            _apartment: apartment,
        })
    }

    /// Sets the language of the texts drawn from now on, as a locale name (`ja`,
    /// `zh-Hans`, `de`...). DirectWrite picks the fonts of characters the text font lacks
    /// from it: Japanese gets the Japanese fonts of Windows and Simplified Chinese the
    /// Chinese ones (the same characters are drawn differently in each).
    pub fn set_text_locale(&self, locale: &str) {
        if *self.text_locale.borrow() == locale {
            return;
        }
        *self.text_locale.borrow_mut() = HSTRING::from(locale);
        self.formats.borrow_mut().clear();
    }

    /// Whether the graphics device exists already (the next surface is then cheap).
    pub fn has_device(&self) -> bool {
        self.gpu.get().is_some()
    }

    /// The device, created on first call.
    pub(crate) fn gpu(&self) -> Result<&Gpu, GraphicsError> {
        if let Some(gpu) = self.gpu.get() {
            return Ok(gpu);
        }
        let gpu = self.create_gpu()?;
        Ok(self.gpu.get_or_init(|| gpu))
    }

    fn create_gpu(&self) -> Result<Gpu, GraphicsError> {
        let (driver, flags): (D3D_DRIVER_TYPE, _) = match self.kind {
            // Single-threaded and without internal worker threads: the measured
            // low-footprint configuration (ARCHITECTURE §1).
            DeviceKind::Software => (
                D3D_DRIVER_TYPE_WARP,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT
                    | D3D11_CREATE_DEVICE_SINGLETHREADED
                    | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
            ),
            DeviceKind::Hardware => (D3D_DRIVER_TYPE_HARDWARE, D3D11_CREATE_DEVICE_BGRA_SUPPORT),
        };
        let mut d3d = None;
        // SAFETY: all pointers are either null or point to valid locals for the call.
        unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                flags,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d),
                None,
                None,
            )
        }?;
        let d3d: ID3D11Device = d3d.ok_or_else(windows::core::Error::empty)?;
        let dxgi: IDXGIDevice = d3d.cast()?;
        // SAFETY: `dxgi` is a valid device of the same process.
        let device = unsafe { self.d2d.CreateDevice(&dxgi) }?;
        // SAFETY: plain call on a valid device.
        let context = unsafe { device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }?;
        // SAFETY: `dxgi` is a valid device; returns a new composition device.
        let composition: IDCompositionDevice = unsafe { DCompositionCreateDevice(&dxgi) }?;
        Ok(Gpu { d3d, context, composition, _device: device })
    }

    /// The text format for a style at a text scale (cached; formats are immutable).
    pub(crate) fn text_format(
        &self,
        style: TextStyle,
        text_scale: f32,
    ) -> Result<IDWriteTextFormat, GraphicsError> {
        self.format(FormatKey { icon: false, size: style.size * text_scale, weight: style.weight })
    }

    /// The system's text rendering parameters with grid fitting turned off (see
    /// [`Canvas::stretched_vertically`](canvas::Canvas::stretched_vertically)). `None` when
    /// DirectWrite cannot make them: text is then drawn with the default ones.
    pub(crate) fn ungridded_text_params(&self) -> Option<IDWriteRenderingParams> {
        let factory: IDWriteFactory2 = self.dwrite.cast().ok()?;
        // SAFETY: plain calls on valid factories and parameters.
        unsafe {
            let system = self.dwrite.CreateRenderingParams().ok()?;
            let grayscale_contrast = system
                .cast::<IDWriteRenderingParams1>()
                .map_or(system.GetEnhancedContrast(), |p| p.GetGrayscaleEnhancedContrast());
            let params = factory
                .CreateCustomRenderingParams(
                    system.GetGamma(),
                    system.GetEnhancedContrast(),
                    grayscale_contrast,
                    system.GetClearTypeLevel(),
                    system.GetPixelGeometry(),
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_GRID_FIT_MODE_DISABLED,
                )
                .ok()?;
            params.cast().ok()
        }
    }

    /// The icon-font format for glyphs of `size` DIPs, centred in their box.
    pub(crate) fn icon_format(&self, size: f32) -> Result<IDWriteTextFormat, GraphicsError> {
        self.format(FormatKey { icon: true, size, weight: 400 })
    }

    fn format(&self, key: FormatKey) -> Result<IDWriteTextFormat, GraphicsError> {
        if let Some((_, format)) = self.formats.borrow().iter().find(|(k, _)| *k == key) {
            return Ok(format.clone());
        }
        let family = HSTRING::from(if key.icon { ICON_FAMILY } else { TEXT_FAMILY });
        // SAFETY: the strings outlive the call; the values are valid enumerations.
        let format = unsafe {
            self.dwrite.CreateTextFormat(
                &family,
                None,
                DWRITE_FONT_WEIGHT(i32::from(key.weight)),
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                key.size,
                &*self.text_locale.borrow(),
            )
        }?;
        // SAFETY: plain setters on the new format.
        unsafe {
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            if key.icon {
                format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            }
        }
        let mut formats = self.formats.borrow_mut();
        // A handful of styles exist; the cap only guards against unbounded growth.
        if formats.len() >= 32 {
            formats.clear();
        }
        formats.push((key, format.clone()));
        Ok(format)
    }

    /// The stroke style of icon outlines: round caps and joins (created once).
    pub(crate) fn round_stroke(&self) -> Result<ID2D1StrokeStyle, GraphicsError> {
        if let Some(style) = self.round_stroke.get() {
            return Ok(style.clone());
        }
        let properties = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        // SAFETY: valid properties, no dashes.
        let base: &ID2D1Factory = &self.d2d;
        // SAFETY: valid properties, no dashes.
        let style = unsafe { base.CreateStrokeStyle(&properties, None) }?;
        Ok(self.round_stroke.get_or_init(|| style).clone())
    }

    /// Builds the Direct2D geometry of a path.
    pub(crate) fn geometry(&self, path: &Path) -> Result<ID2D1PathGeometry, GraphicsError> {
        let point = |p: crate::Point| Vector2 { X: p.x, Y: p.y };
        // SAFETY: plain creation; the sink is used and closed below, on this thread.
        let base: &ID2D1Factory = &self.d2d;
        // SAFETY: as above.
        let geometry = unsafe { base.CreatePathGeometry() }?;
        // SAFETY: as above.
        let sink = unsafe { geometry.Open() }?;
        let mut open = false;
        for segment in &path.segments {
            // SAFETY: figure calls are balanced: every figure begun is ended (here on the
            // next MoveTo or Close, or after the loop).
            unsafe {
                match *segment {
                    Segment::MoveTo(p) => {
                        if open {
                            sink.EndFigure(D2D1_FIGURE_END_OPEN);
                        }
                        sink.BeginFigure(point(p), D2D1_FIGURE_BEGIN_FILLED);
                        open = true;
                    }
                    Segment::LineTo(p) if open => sink.AddLine(point(p)),
                    Segment::ArcTo { to, radii, clockwise, large } if open => {
                        sink.AddArc(&D2D1_ARC_SEGMENT {
                            point: point(to),
                            size: D2D_SIZE_F { width: radii.width, height: radii.height },
                            rotationAngle: 0.0,
                            sweepDirection: if clockwise {
                                D2D1_SWEEP_DIRECTION_CLOCKWISE
                            } else {
                                D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE
                            },
                            arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                        });
                    }
                    Segment::CubicTo { c1, c2, to } if open => {
                        sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                            point1: point(c1),
                            point2: point(c2),
                            point3: point(to),
                        });
                    }
                    Segment::Close if open => {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    // A segment before any MoveTo has no figure to belong to.
                    _ => {}
                }
            }
        }
        // SAFETY: ends the last figure, then closes the sink once.
        unsafe {
            if open {
                sink.EndFigure(D2D1_FIGURE_END_OPEN);
            }
            sink.Close()?;
        }
        Ok(geometry)
    }

    /// Size of a single line of text in a style, in DIPs.
    pub fn measure_text(
        &self,
        text: &str,
        style: TextStyle,
        text_scale: f32,
    ) -> Result<Size, GraphicsError> {
        let format = self.text_format(style, text_scale)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: `wide` and `format` outlive the call; generous layout box.
        let layout = unsafe { self.dwrite.CreateTextLayout(&wide, &format, 10_000.0, 10_000.0) }?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: `metrics` is a valid out structure.
        unsafe { layout.GetMetrics(&mut metrics) }?;
        Ok(Size::new(metrics.widthIncludingTrailingWhitespace, metrics.height))
    }

    /// Lays out a paragraph: wrapped in `width`, at most `height` high, aligned as asked;
    /// `trim` ends the last visible line with an ellipsis when the text does not fit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paragraph(
        &self,
        text: &str,
        style: TextStyle,
        text_scale: f32,
        width: f32,
        height: f32,
        align: canvas::TextAlign,
        trim: bool,
    ) -> Result<canvas::Paragraph, GraphicsError> {
        use windows::Win32::Graphics::DirectWrite::{
            DWRITE_LINE_SPACING_METHOD_UNIFORM, DWRITE_PARAGRAPH_ALIGNMENT_NEAR,
            DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TRIMMING,
            DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_WRAP,
        };
        let format = self.text_format(style, text_scale)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: `wide` and `format` outlive the call.
        let layout = unsafe {
            self.dwrite.CreateTextLayout(&wide, &format, width.max(1.0), height.max(1.0))
        }?;
        let line = style.line_height * text_scale;
        let alignment = match align {
            canvas::TextAlign::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
            canvas::TextAlign::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            canvas::TextAlign::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
        };
        // SAFETY: plain setters on the new layout (it has its own copy of the format).
        unsafe {
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR)?;
            layout.SetTextAlignment(alignment)?;
            layout.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line, line * 0.75)?;
            if trim {
                let sign = self.dwrite.CreateEllipsisTrimmingSign(&layout)?;
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                layout.SetTrimming(&trimming, &sign)?;
            }
        }
        Ok(canvas::Paragraph { layout })
    }

    /// Number of lines a text takes when wrapped in `width` DIPs.
    pub fn line_count(
        &self,
        text: &str,
        style: TextStyle,
        text_scale: f32,
        width: f32,
    ) -> Result<u32, GraphicsError> {
        let paragraph = self.paragraph(
            text,
            style,
            text_scale,
            width,
            100_000.0,
            canvas::TextAlign::Leading,
            false,
        )?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: `metrics` is a valid out structure.
        unsafe { paragraph.layout.GetMetrics(&mut metrics) }?;
        Ok(metrics.lineCount)
    }

    /// Size of a text wrapped in at most `max_width` DIPs.
    pub fn measure_wrapped(
        &self,
        text: &str,
        style: TextStyle,
        text_scale: f32,
        max_width: f32,
    ) -> Result<Size, GraphicsError> {
        let paragraph = self.paragraph(
            text,
            style,
            text_scale,
            max_width,
            100_000.0,
            canvas::TextAlign::Leading,
            false,
        )?;
        let mut metrics = DWRITE_TEXT_METRICS::default();
        // SAFETY: `metrics` is a valid out structure.
        unsafe { paragraph.layout.GetMetrics(&mut metrics) }?;
        Ok(Size::new(metrics.widthIncludingTrailingWhitespace, metrics.height))
    }

    /// Converts a Windows icon (its handle value) to premultiplied pixels. The icon stays
    /// owned by the caller.
    pub fn icon_pixels(&self, icon: isize) -> Result<Pixels, GraphicsError> {
        use windows::Win32::Graphics::Imaging::{
            IWICBitmapSource, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
        };
        use windows::Win32::UI::WindowsAndMessaging::HICON;
        // SAFETY: the caller passes a live icon handle; WIC copies its pixels.
        let bitmap =
            unsafe { self.wic.CreateBitmapFromHICON(HICON(icon as *mut core::ffi::c_void)) }?;
        // SAFETY: plain calls on valid WIC objects.
        let converter = unsafe { self.wic.CreateFormatConverter() }?;
        let source: IWICBitmapSource = bitmap.cast()?;
        // SAFETY: as above; no palette needed for a 32-bit target.
        unsafe {
            converter.Initialize(
                &source,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }?;
        let (mut width, mut height) = (0u32, 0u32);
        // SAFETY: valid out pointers.
        unsafe { converter.GetSize(&mut width, &mut height) }?;
        let stride = width * 4;
        let mut data = vec![0u8; (stride * height) as usize];
        // SAFETY: `data` holds exactly `stride * height` bytes; null rectangle = whole image.
        unsafe { converter.CopyPixels(std::ptr::null(), stride, &mut data) }?;
        Ok(Pixels { width, height, data })
    }

    /// Paints an image in memory, `width` × `height` physical pixels at `scale` (1.0 =
    /// 100 %; the painter draws in DIPs). Used for the tray icon: no device needed.
    pub fn rasterize(
        &self,
        width: u32,
        height: u32,
        scale: f32,
        text_scale: f32,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<Pixels, GraphicsError> {
        // SAFETY: plain call; returns a new bitmap we own.
        let bitmap = unsafe {
            self.wic.CreateBitmap(
                width,
                height,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapCacheOnLoad,
            )
        }?;
        let target = canvas::wic_target(&self.d2d, &bitmap, scale)?;
        canvas::draw(self, &target, text_scale, paint)?;
        drop(target);
        let stride = width * 4;
        let mut data = vec![0u8; (stride * height) as usize];
        // SAFETY: `data` holds exactly `stride * height` bytes; null rectangle = whole image.
        unsafe { bitmap.CopyPixels(std::ptr::null(), stride, &mut data) }?;
        Ok(Pixels { width, height, data })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::typography::BODY;
    use crate::{Color, Rect};

    fn draw(gfx: &Graphics, text: &str) -> Pixels {
        gfx.rasterize(64, 32, 1.0, 1.0, |c| {
            c.clear(Color::rgb(0xFFFFFF));
            c.text(
                text,
                BODY,
                Rect::new(0.0, 0.0, 64.0, 32.0),
                canvas::TextAlign::Leading,
                Color::rgb(0x000000),
            );
        })
        .expect("rasterize")
    }

    #[test]
    fn the_text_locale_picks_the_japanese_or_the_chinese_form_of_a_character() {
        let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
        // The same characters, drawn differently in Japan and in China.
        let shared = "直骨";
        gfx.set_text_locale("ja");
        let japanese = draw(&gfx, shared);
        gfx.set_text_locale("zh-Hans");
        let chinese = draw(&gfx, shared);
        assert_ne!(japanese, chinese, "the locale must choose the fonts of Asian scripts");
        // Setting a locale again gives the same drawing (the format cache follows it).
        gfx.set_text_locale("ja");
        assert_eq!(draw(&gfx, shared), japanese);
        // Something is drawn at all.
        let blank = draw(&gfx, "");
        assert_ne!(japanese, blank);
    }
}
