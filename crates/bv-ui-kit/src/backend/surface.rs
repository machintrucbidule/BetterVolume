//! A window's drawing surface: a composition swap chain shown through DirectComposition.

use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1RenderTarget, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1,
};
use windows::Win32::Graphics::DirectComposition::{IDCompositionTarget, IDCompositionVisual};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGISurface, IDXGISwapChain1, DXGI_CREATE_FACTORY_FLAGS,
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};

use super::canvas::{self, Canvas};
use super::{Graphics, GraphicsError};

/// The surface of one window. The window must have been created with
/// `WS_EX_NOREDIRECTIONBITMAP` (its pixels come from composition only).
pub struct WindowSurface {
    swap: IDXGISwapChain1,
    visual: IDCompositionVisual,
    offset: (i32, i32),
    _target: IDCompositionTarget,
    bitmap: Option<ID2D1Bitmap1>,
    width: u32,
    height: u32,
    scale: f32,
    shown: bool,
}

impl WindowSurface {
    /// Attaches a surface of `width` × `height` physical pixels to the window `hwnd` (its
    /// raw handle value). Creates the graphics device on first use.
    pub fn new(
        graphics: &Graphics,
        hwnd: isize,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<WindowSurface, GraphicsError> {
        let gpu = graphics.gpu()?;
        // SAFETY: plain factory creation.
        let factory: IDXGIFactory2 = unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width.max(1),
            Height: height.max(1),
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
            ..Default::default()
        };
        // SAFETY: the device and the description are valid.
        let swap = unsafe { factory.CreateSwapChainForComposition(&gpu.d3d, &desc, None) }?;
        let window = HWND(hwnd as *mut core::ffi::c_void);
        // SAFETY: `window` is a live window of this thread (caller's contract); a stale
        // handle only makes the call fail.
        let target = unsafe { gpu.composition.CreateTargetForHwnd(window, true) }?;
        // SAFETY: plain calls on valid composition objects.
        let visual = unsafe {
            let visual = gpu.composition.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            visual
        };
        Ok(WindowSurface {
            swap,
            visual,
            offset: (0, 0),
            _target: target,
            bitmap: None,
            width: width.max(1),
            height: height.max(1),
            scale,
            shown: false,
        })
    }

    /// Moves the content inside the window by `(x, y)` physical pixels (a window cut at
    /// its top or left edge shows the rest of its content at the right place).
    pub fn set_offset(&mut self, graphics: &Graphics, x: i32, y: i32) -> Result<(), GraphicsError> {
        if self.offset == (x, y) {
            return Ok(());
        }
        let gpu = graphics.gpu()?;
        // SAFETY: plain calls on valid composition objects of this thread.
        unsafe {
            self.visual.SetOffsetX2(x as f32)?;
            self.visual.SetOffsetY2(y as f32)?;
            gpu.composition.Commit()?;
        }
        self.offset = (x, y);
        Ok(())
    }

    /// Size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Changes the size (physical pixels) and scale; the next [`render`](Self::render)
    /// paints at the new size.
    pub fn resize(
        &mut self,
        graphics: &Graphics,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<(), GraphicsError> {
        let (width, height) = (width.max(1), height.max(1));
        self.scale = scale;
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        // Every reference to the buffers must be gone before they are resized.
        self.bitmap = None;
        // SAFETY: plain call on the device context of this thread.
        unsafe { graphics.gpu()?.context.SetTarget(None) };
        // SAFETY: no outstanding buffer reference (released above).
        unsafe {
            self.swap.ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
        }?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Paints a frame (in DIPs) and shows it.
    pub fn render(
        &mut self,
        graphics: &Graphics,
        text_scale: f32,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<(), GraphicsError> {
        let gpu = graphics.gpu()?;
        let dpi = 96.0 * self.scale;
        let bitmap = match &self.bitmap {
            Some(bitmap) => bitmap.clone(),
            None => {
                // SAFETY: buffer 0 is the back buffer of a flip-model swap chain.
                let surface: IDXGISurface = unsafe { self.swap.GetBuffer(0) }?;
                let properties = D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: dpi,
                    dpiY: dpi,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                };
                // SAFETY: `surface` belongs to the same device as the context.
                let bitmap = unsafe {
                    gpu.context.CreateBitmapFromDxgiSurface(&surface, Some(&properties))
                }?;
                self.bitmap = Some(bitmap.clone());
                bitmap
            }
        };
        // SAFETY: plain setters on the device context of this thread.
        unsafe {
            gpu.context.SetTarget(&bitmap);
            gpu.context.SetDpi(dpi, dpi);
        }
        let target: ID2D1RenderTarget = gpu.context.cast()?;
        let drawn = canvas::draw(graphics, &target, text_scale, paint);
        // SAFETY: releases the target so the buffer can be presented or resized.
        unsafe { gpu.context.SetTarget(None) };
        drawn?;
        // SAFETY: plain call. Interval 0: composition never tears, and waiting for the
        // next vertical blank would only delay the frame.
        unsafe { self.swap.Present(0, DXGI_PRESENT(0)) }.ok()?;
        if !self.shown {
            // SAFETY: plain call; makes the visual tree (set up in `new`) live.
            unsafe { gpu.composition.Commit() }?;
            self.shown = true;
        }
        Ok(())
    }
}
