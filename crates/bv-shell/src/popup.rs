//! Popup windows (flyout and context menu): a borderless, topmost, composition-drawn
//! window with the Windows 11 frame (rounded corners, system translucent material), and
//! its entrance and exit motion.
//!
//! Motion: the system flyouts rise from behind the taskbar. The window is placed just
//! below the taskbar in the z-order and moved, frame by frame, from behind the taskbar's
//! inner edge to its place. The whole window moves (material and frame included), which
//! a composition animation of the content alone cannot do. The loop runs on the UI
//! thread for the duration of the transition only (a quarter of a second) and paces
//! itself on the compositor's frames: nothing runs once the popup is still.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use bv_ui_kit::backend::{Canvas, Graphics, GraphicsError, WindowSurface};
use bv_ui_kit::motion::Transition;
use bv_ui_kit::theme::Material;
use bv_ui_kit::tokens::popup::ROUND_CORNERS;
use bv_ui_kit::{Theme, ThemeMode};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmFlush, DwmSetWindowAttribute, DWMSBT_NONE,
    DWMSBT_TRANSIENTWINDOW, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND,
    DWMWINDOWATTRIBUTE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetForegroundWindow, IsWindow, IsWindowVisible, LoadCursorW,
    RegisterClassW, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WINDOW_EX_STYLE, WNDCLASSW, WNDPROC,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::placement::{hidden_origin, lerp, visible_part};
use crate::screen::Layout;

/// Class of the context menu window.
pub(crate) const MENU_CLASS: PCWSTR = w!("BetterVolume.Menu");
/// Class of the flyout window. Also how the measurement tool finds the flyout.
pub(crate) const FLYOUT_CLASS: PCWSTR = w!("BetterVolume.Flyout");
/// Class of the tooltip window.
pub(crate) const TOOLTIP_CLASS: PCWSTR = w!("BetterVolume.Tooltip");
/// Class of the bubble window (FR-105). Also how checks find the bubble.
pub(crate) const BUBBLE_CLASS: PCWSTR = w!("BetterVolume.Bubble");

/// Registers the popup classes (once per process; a second call fails harmlessly).
pub(crate) fn register_classes(procedure: WNDPROC) -> windows::core::Result<()> {
    // SAFETY: a null module name returns the handle of the current executable.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    let class = WNDCLASSW {
        lpfnWndProc: procedure,
        hInstance: instance,
        // SAFETY: loads a shared system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        lpszClassName: MENU_CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and its strings are constants.
    unsafe { RegisterClassW(&class) };
    let class = WNDCLASSW { lpszClassName: FLYOUT_CLASS, ..class };
    // SAFETY: as above.
    unsafe { RegisterClassW(&class) };
    let class = WNDCLASSW { lpszClassName: TOOLTIP_CLASS, ..class };
    // SAFETY: as above.
    unsafe { RegisterClassW(&class) };
    let class = WNDCLASSW { lpszClassName: BUBBLE_CLASS, ..class };
    // SAFETY: as above.
    unsafe { RegisterClassW(&class) };
    Ok(())
}

/// Duration of one compositor frame at 60 Hz, in milliseconds.
const FRAME_MS: f32 = 1000.0 / 60.0;

/// Where a popup stands in the z-order.
#[derive(Clone, Copy)]
enum Layer {
    /// Right below the taskbar (still above every ordinary window): while it moves.
    BelowTaskbar,
    /// Above every window, the hidden-icons panel included: once in place.
    Top,
    /// Unchanged.
    Keep,
}

/// What a popup does when Windows refuses it the focus.
pub(crate) enum Unfocused<'a> {
    /// Hide again at once.
    Hide,
    /// Stay, after running this first (before the entrance).
    Then(&'a dyn Fn()),
}

/// One popup window and its drawing surface.
pub(crate) struct Popup {
    window: HWND,
    surface: RefCell<Option<WindowSurface>>,
    /// Final position and size of the last showing, in physical pixels.
    origin: Cell<(i32, i32)>,
    size: Cell<(i32, i32)>,
    scale: Cell<f32>,
    layout: Cell<Option<Layout>>,
}

impl Popup {
    /// Creates the (hidden) window. `owner` is the hidden message window: the popup
    /// procedure reaches the shell through it.
    pub fn create(owner: HWND, class: PCWSTR) -> windows::core::Result<Popup> {
        Popup::create_with(owner, class, WINDOW_EX_STYLE(0), true)
    }

    /// Creates a window that never takes the focus and lets clicks through (tooltips).
    pub fn create_passive(owner: HWND) -> windows::core::Result<Popup> {
        Popup::create_with(owner, TOOLTIP_CLASS, WS_EX_TRANSPARENT | WS_EX_NOACTIVATE, false)
    }

    /// Creates a passive window (no focus, clicks pass through) with the Windows frame:
    /// rounded corners and shadow drawn by the system (the bubble).
    /// Without the frame's outline (the Windows volume indicator has none). Layered as
    /// well: only a layered window lets clicks through to other programs' windows.
    pub fn create_passive_framed(owner: HWND, class: PCWSTR) -> windows::core::Result<Popup> {
        let extra = WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_LAYERED;
        let popup = Popup::create_with(owner, class, extra, true)?;
        // SAFETY: our own window; fully opaque layering (the content decides the pixels);
        // the border colour attribute takes a COLORREF.
        unsafe {
            SetLayeredWindowAttributes(popup.window, COLORREF(0), 255, LWA_ALPHA)?;
            set_attribute(popup.window, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE);
        }
        Ok(popup)
    }

    fn create_with(
        owner: HWND,
        class: PCWSTR,
        extra: WINDOW_EX_STYLE,
        framed: bool,
    ) -> windows::core::Result<Popup> {
        // SAFETY: a null module name returns the handle of the current executable.
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
        // SAFETY: the class is registered by `register_class`; plain values otherwise.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | extra,
                class,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        // SAFETY: our own window; the value is the owner's handle, read back by the
        // popup procedure only.
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, owner.0 as isize) };
        // SAFETY: as above.
        unsafe {
            if !framed {
                // A tooltip draws its own corners and outline over a transparent window:
                // no frame, no material.
                set_attribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND);
            } else {
                // The design system chooses the corners; the frame draws them.
                let corner = if ROUND_CORNERS { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
                set_attribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &corner);
                // The system material shows through the whole client area.
                let margins = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                let _ = DwmExtendFrameIntoClientArea(window, &margins);
            }
        }
        Ok(Popup {
            window,
            surface: RefCell::new(None),
            origin: Cell::new((0, 0)),
            size: Cell::new((1, 1)),
            scale: Cell::new(1.0),
            layout: Cell::new(None),
        })
    }

    pub fn window(&self) -> HWND {
        self.window
    }

    /// Whether the window still exists (it is never destroyed on purpose before exit).
    pub fn is_alive(&self) -> bool {
        // SAFETY: plain query on a handle; a stale one answers false.
        unsafe { IsWindow(Some(self.window)) }.as_bool()
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: plain query on our own window.
        unsafe { IsWindowVisible(self.window) }.as_bool()
    }

    pub fn scale(&self) -> f32 {
        self.scale.get()
    }

    /// The frame follows the theme: dark or light frame and material, or opaque. Returns
    /// the theme to paint with: opaque if Windows refused the translucent material (older
    /// builds, policies), so the popup never ends up see-through.
    pub fn apply_theme(&self, theme: Theme) -> Theme {
        let dark = BOOL::from(theme.mode == ThemeMode::Dark);
        let translucent = matches!(theme.material, Material::Acrylic { .. });
        let backdrop = if translucent { DWMSBT_TRANSIENTWINDOW } else { DWMSBT_NONE };
        // SAFETY: our own window; values of the documented types.
        let accepted = unsafe {
            set_attribute(self.window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark);
            set_attribute(self.window, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop)
        };
        if translucent && !accepted {
            // SAFETY: as above.
            unsafe { set_attribute(self.window, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE) };
            return theme.opaque();
        }
        theme
    }

    /// Sizes the surface (physical pixels) and paints it.
    pub fn paint(
        &self,
        graphics: &Graphics,
        size: (i32, i32),
        scale: f32,
        text_scale: f32,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<(), GraphicsError> {
        let (w, h) = (size.0.max(1) as u32, size.1.max(1) as u32);
        self.size.set(size);
        self.scale.set(scale);
        // The window takes the surface's size before it is shown. Done before borrowing the
        // surface: this call sends messages to the popup procedure synchronously.
        // SAFETY: our own window.
        let _ = unsafe {
            SetWindowPos(
                self.window,
                None,
                0,
                0,
                size.0,
                size.1,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        let mut surface = self.surface.borrow_mut();
        match surface.as_mut() {
            Some(s) => s.resize(graphics, w, h, scale)?,
            None => {
                *surface = Some(WindowSurface::new(graphics, self.window.0 as isize, w, h, scale)?)
            }
        }
        surface.as_mut().map_or(Ok(()), |s| s.render(graphics, text_scale, paint))
    }

    /// Paints again at the current size (state change while visible).
    pub fn repaint(
        &self,
        graphics: &Graphics,
        text_scale: f32,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<(), GraphicsError> {
        match self.surface.borrow_mut().as_mut() {
            Some(s) => s.render(graphics, text_scale, paint),
            None => Ok(()),
        }
    }

    /// Whether the window has a drawing surface (it keeps the last frame painted).
    pub fn has_surface(&self) -> bool {
        self.surface.borrow().is_some()
    }

    /// Forgets the drawing surface (the graphics objects are being rebuilt).
    pub fn drop_surface(&self) {
        let old = self.surface.borrow_mut().take();
        drop(old);
    }

    /// Shows the (already painted) popup at `origin`, rising from behind the taskbar when
    /// `motion` is given, and gives it the keyboard focus. Returns whether it got the
    /// focus (Windows refuses it when the request no longer comes from the user); in that
    /// case `unfocused` decides, before the popup moves, whether it is hidden again or
    /// what to do first.
    pub fn show(
        &self,
        graphics: &Graphics,
        layout: Layout,
        origin: (i32, i32),
        motion: Option<Transition>,
        unfocused: Unfocused<'_>,
    ) -> bool {
        self.origin.set(origin);
        self.layout.set(Some(layout));
        let start = match motion {
            Some(_) => hidden_origin(origin, self.size.get(), &layout.monitor, layout.edge),
            None => origin,
        };
        // While it rises, the popup sits right below the taskbar (the part not yet risen
        // is hidden behind it; the first position lies past the monitor's edge, so nothing
        // can flash). Without motion, and once in place, it is on top of everything,
        // including the hidden-icons panel it may have been opened from.
        let rising = if motion.is_some() { Layer::BelowTaskbar } else { Layer::Top };
        self.place(graphics, layout, start, rising);
        // SAFETY: our own window.
        let focused = unsafe {
            let _ = ShowWindow(self.window, SW_SHOWNA);
            // The shell lets the owner of a tray icon take the foreground when the user
            // acts on the icon, so this succeeds for clicks and menu requests.
            let _ = SetForegroundWindow(self.window);
            GetForegroundWindow() == self.window
        };
        if !focused {
            match unfocused {
                Unfocused::Hide => {
                    // SAFETY: our own window.
                    let _ = unsafe { ShowWindow(self.window, SW_HIDE) };
                    return false;
                }
                Unfocused::Then(first) => first(),
            }
        }
        // Taking the foreground raised the window: back to its layer.
        self.place(graphics, layout, start, rising);
        if let Some(transition) = motion {
            self.slide(graphics, layout, start, origin, transition);
            self.place(graphics, layout, origin, Layer::Top);
        }
        focused
    }

    /// Final position of the last showing, in physical pixels.
    pub fn origin(&self) -> (i32, i32) {
        self.origin.get()
    }

    /// Resizes the surface of a shown popup and paints it (height changes of the
    /// flyout); [`set_frame`](Self::set_frame) then gives the window the same size.
    /// Sends no window message, so the caller may hold its state while painting.
    pub fn resize_render(
        &self,
        graphics: &Graphics,
        size: (i32, i32),
        text_scale: f32,
        paint: impl FnOnce(&mut Canvas<'_>),
    ) -> Result<(), GraphicsError> {
        let (w, h) = (size.0.max(1) as u32, size.1.max(1) as u32);
        let mut surface = self.surface.borrow_mut();
        let Some(surface) = surface.as_mut() else { return Ok(()) };
        surface.resize(graphics, w, h, self.scale.get())?;
        surface.render(graphics, text_scale, paint)
    }

    /// Moves and sizes a shown popup in one step, keeping its place in the z-order.
    /// Sends window messages to the popup: call it without holding state its window
    /// procedure looks up.
    pub fn set_frame(&self, graphics: &Graphics, origin: (i32, i32), size: (i32, i32)) {
        self.size.set(size);
        self.origin.set(origin);
        if let Some(layout) = self.layout.get() {
            self.place(graphics, layout, origin, Layer::Keep);
        }
    }

    /// Shows the (already painted) popup at `origin` on top of everything, without
    /// activating it and without motion (tooltips).
    pub fn show_inactive(&self, graphics: &Graphics, layout: Layout, origin: (i32, i32)) {
        self.origin.set(origin);
        self.layout.set(Some(layout));
        self.place(graphics, layout, origin, Layer::Top);
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(self.window, SW_SHOWNA) };
    }

    /// Shows the (already painted) popup at `origin` without activating it, either right
    /// below the taskbar in the z-order (it is about to rise from behind it, moved by its
    /// owner's timer) or on top of everything. Sends window messages to the popup.
    pub fn show_passive(
        &self,
        graphics: &Graphics,
        layout: Layout,
        origin: (i32, i32),
        below_taskbar: bool,
    ) {
        self.origin.set(origin);
        self.layout.set(Some(layout));
        let layer = if below_taskbar { Layer::BelowTaskbar } else { Layer::Top };
        self.place(graphics, layout, origin, layer);
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(self.window, SW_SHOWNA) };
    }

    /// Puts a shown popup on top of everything, where it stands (end of a rise).
    pub fn raise(&self, graphics: &Graphics) {
        if let Some(layout) = self.layout.get() {
            self.place(graphics, layout, self.origin.get(), Layer::Top);
        }
    }

    /// Puts a shown popup right below the taskbar, where it stands (start of a sink).
    pub fn lower(&self, graphics: &Graphics) {
        if let Some(layout) = self.layout.get() {
            self.place(graphics, layout, self.origin.get(), Layer::BelowTaskbar);
        }
    }

    /// Hides the popup at once, wherever it stands.
    pub fn hide_now(&self) {
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(self.window, SW_HIDE) };
    }

    /// Hides the popup, sinking behind the taskbar when `motion` is given.
    pub fn hide(&self, graphics: Option<&Graphics>, motion: Option<Transition>) {
        if !self.is_visible() {
            return;
        }
        if let (Some(graphics), Some(transition), Some(layout)) =
            (graphics, motion, self.layout.get())
        {
            let origin = self.origin.get();
            let end = hidden_origin(origin, self.size.get(), &layout.monitor, layout.edge);
            self.place(graphics, layout, origin, Layer::BelowTaskbar);
            self.slide(graphics, layout, origin, end, transition);
        }
        // SAFETY: our own window.
        let _ = unsafe { ShowWindow(self.window, SW_HIDE) };
    }

    /// Puts the popup at `origin`, cut to its monitor (the part past the monitor's edge is
    /// left out and the content shifted to match), in the given layer of the z-order.
    fn place(&self, graphics: &Graphics, layout: Layout, origin: (i32, i32), layer: Layer) {
        let (part, offset) = visible_part(origin, self.size.get(), &layout.monitor);
        let mut flags = SWP_NOACTIVATE;
        let after = match (layer, layout.taskbar) {
            (Layer::BelowTaskbar, Some(taskbar)) => Some(taskbar),
            (Layer::BelowTaskbar, None) | (Layer::Top, _) => Some(HWND_TOPMOST),
            (Layer::Keep, _) => {
                flags |= SWP_NOZORDER;
                None
            }
        };
        if let Some(surface) = self.surface.borrow_mut().as_mut() {
            let _ = surface.set_offset(graphics, offset.0, offset.1);
        }
        // SAFETY: our own window; the taskbar handle is only used as a z-order anchor.
        let _ = unsafe {
            SetWindowPos(
                self.window,
                after,
                part.left,
                part.top,
                part.width(),
                part.height(),
                flags,
            )
        };
    }

    /// Moves the window from `from` to `to` along the transition, one position per
    /// compositor frame.
    fn slide(
        &self,
        graphics: &Graphics,
        layout: Layout,
        from: (i32, i32),
        to: (i32, i32),
        transition: Transition,
    ) {
        let started = Instant::now();
        loop {
            // One frame ahead: each position is the one due when the compositor shows it
            // (next frame), so the very first frame of an entrance already shows pixels.
            let elapsed = started.elapsed().as_secs_f32() * 1000.0 + FRAME_MS;
            self.place(graphics, layout, lerp(from, to, transition.progress(elapsed)), Layer::Keep);
            if elapsed >= transition.duration_ms as f32 {
                break;
            }
            // SAFETY: plain call; waits for the next composition pass.
            if unsafe { DwmFlush() }.is_err() {
                // No compositor clock (should not happen on Windows 11): a frame's time.
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        }
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        // SAFETY: our own window, destroyed once, on its thread.
        let _ = unsafe { DestroyWindow(self.window) };
    }
}

/// Sets one DWM attribute; returns whether Windows accepted it.
///
/// # Safety
/// `window` must be a window of this process and `T` the documented type of `attribute`.
unsafe fn set_attribute<T>(window: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) -> bool {
    // SAFETY: the caller guarantees the attribute type; size and pointer match `value`.
    unsafe {
        DwmSetWindowAttribute(window, attribute, (value as *const T).cast(), size_of::<T>() as u32)
    }
    .is_ok()
}
