//! The flyout window (FR-020 to FR-024): anchored in the tray corner of the taskbar's
//! monitor, rises from behind the taskbar, closes on focus loss, Escape and a second
//! click on the tray icon. It hosts the design system's flyout view: the application
//! hands over content, the window forwards pointer, wheel and keyboard input to the view
//! and reports what the user asked for as [`Event::Flyout`]. The height follows the
//! content smoothly; tooltips show over it; screen readers see it through UI
//! Automation.

use std::rc::Rc;
use std::time::Instant;

use bv_ui_kit::access::AccessAction;
use bv_ui_kit::backend::Graphics;
use bv_ui_kit::controls::flyout::{
    FlyoutAction, FlyoutContent, FlyoutKey, FlyoutView, GraphicsMeasure, Response, VolumeIcon,
};
use bv_ui_kit::controls::tooltip;
use bv_ui_kit::motion::{
    FLYOUT_CLOSE, FLYOUT_OPEN, FLYOUT_RESIZE, RESIZE_FRAME_MS, ROW_REMOVAL_HOLD_MS,
    TOOLTIP_DELAY_MS,
};
use bv_ui_kit::theme::Material;
use bv_ui_kit::tokens::flyout::MARGIN;
use bv_ui_kit::tokens::volume_row::APP_ICON;
use bv_ui_kit::{Point, Rect, Theme};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, PostMessageW, SetTimer, EVENT_SYSTEM_FOREGROUND, WA_INACTIVE, WHEEL_DELTA,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_ACTIVATE, WM_CAPTURECHANGED, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_TIMER,
};

use crate::appicon::IconCache;
use crate::host::{Event, Shell, TrayActivation};
use crate::input::ToggleAction;
use crate::placement::flyout_origin;
use crate::popup::{Popup, Unfocused};
use crate::screen::Layout;
use crate::uia::{self, AccessChange, AccessSource};

/// Timer of the flyout window driving a height change.
const TIMER_RESIZE: usize = 11;
/// Timer of the flyout window delaying a tooltip.
const TIMER_TOOLTIP: usize = 12;
/// Timer of the flyout window ending the hold of application rows that vanished.
const TIMER_ROWS: usize = 13;

/// Posted to the message window when the flyout has been hidden.
pub(crate) const WM_FLYOUT_CLOSED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 4;

/// A height change in progress (DIPs).
#[derive(Clone, Copy, Debug)]
struct Resize {
    from: f32,
    to: f32,
    started: Instant,
}

/// The flyout's state while it is shown.
#[derive(Default)]
pub(crate) struct FlyoutState {
    view: Option<FlyoutView>,
    theme: Option<Theme>,
    layout: Option<Layout>,
    /// Height the window shows now, in DIPs.
    shown: f32,
    resize: Option<Resize>,
    /// Tooltip waiting for its delay, and the one shown.
    tooltip_pending: Option<(String, Rect)>,
    tooltip_shown: Option<String>,
    /// Wheel movement smaller than a notch, kept for the next message.
    wheel_rest: i32,
    tracking_leave: bool,
    icons: IconCache,
    /// What the window's surface holds when it was last painted at rest: an opening
    /// with the same content, look and size shows it again without painting (latency).
    last_frame: Option<Frame>,
    /// The latest content, while application rows it no longer has are still shown for a
    /// moment (they may come back at once: no jump).
    held: Option<FlyoutContent>,
}

/// What a frame painted at rest depends on.
#[derive(Clone, PartialEq)]
struct Frame {
    content: FlyoutContent,
    theme: Theme,
    size: (i32, i32),
    scale: f32,
}

impl FlyoutState {
    /// The surface no longer holds the last frame (graphics rebuilt).
    pub(crate) fn forget_frame(&mut self) {
        self.last_frame = None;
    }

    fn scale(&self) -> f32 {
        self.layout.map_or(1.0, |l| l.scale)
    }

    fn max_height(&self) -> f32 {
        match self.layout {
            Some(layout) => layout.area.height() as f32 / layout.scale.max(0.1) - 2.0 * MARGIN,
            None => f32::MAX,
        }
    }
}

impl Shell {
    /// Whether the flyout is shown.
    pub fn flyout_visible(&self) -> bool {
        self.flyout_popup.borrow().as_ref().is_some_and(|p| p.is_visible())
    }

    /// The primary action on the tray icon: opens the flyout with the content `content`
    /// returns if closed, closes it if open (FR-013). A click that itself just closed the
    /// flyout (by taking the focus away from it) does not open it again. `content` is only
    /// called when the flyout opens.
    pub fn toggle_flyout(
        &self,
        activation: TrayActivation,
        content: impl FnOnce() -> FlyoutContent,
    ) {
        let action = self.flyout_toggle.borrow_mut().primary(
            activation.time,
            self.flyout_visible(),
            activation.keyboard,
        );
        match action {
            ToggleAction::Open => self.show_flyout(content()),
            ToggleAction::Close => self.hide_flyout(),
            ToggleAction::Nothing => {}
        }
    }

    /// Opens the flyout with this content (an open flyout takes the content instead).
    pub fn show_flyout(&self, content: FlyoutContent) {
        if self.flyout_visible() {
            self.update_flyout(content);
            return;
        }
        // The flyout shows the state itself: no bubble next to it.
        self.hide_bubble();
        self.flyout_toggle.borrow_mut().opened();
        self.refresh_appearance();
        let Some(popup) = self.flyout_window() else { return };
        let Some(graphics) = self.graphics() else { return };
        let theme = popup.apply_theme(self.theme());
        let layout = crate::screen::taskbar_layout();
        let content = {
            let mut state = self.flyout_state.borrow_mut();
            state.layout = Some(layout);
            state.theme = Some(theme);
            state.resize = None;
            state.wheel_rest = 0;
            resolve_icons(content, &mut state.icons, layout.scale, &graphics)
        };
        let max_height = self.flyout_state.borrow().max_height();
        let measure = GraphicsMeasure { graphics: &graphics, text_scale: theme.text_scale };
        let view = FlyoutView::new(content, &measure, theme.text_scale, max_height);
        let height = view.height();
        let size = pixels(view.width(), height, layout.scale);
        let frame = Frame { content: view.content().clone(), theme, size, scale: layout.scale };
        let kept =
            popup.has_surface() && self.flyout_state.borrow().last_frame.as_ref() == Some(&frame);
        let graphics = if kept {
            // The surface still holds this very picture from the last opening.
            graphics
        } else {
            let paint = |c: &mut bv_ui_kit::backend::Canvas<'_>| view.paint(c, &theme);
            let Some(graphics) =
                self.paint_popup(&popup, "flyout", size, layout.scale, theme.text_scale, &paint)
            else {
                self.flyout_state.borrow_mut().last_frame = None;
                return;
            };
            graphics
        };
        {
            let mut state = self.flyout_state.borrow_mut();
            state.last_frame = Some(frame);
            state.view = Some(view);
            state.shown = height;
        }
        let margin = (MARGIN * layout.scale).round() as i32;
        let origin = flyout_origin(size, &layout.area, layout.edge, margin, layout.rtl);
        // Opened from the hidden-icons panel: appears in place, over the panel.
        let rises = theme.animations && !crate::screen::hidden_icons_open();
        let motion = rises.then_some(FLYOUT_OPEN);
        // Without the focus the flyout would never learn of a click elsewhere: watch the
        // foreground instead, from before the entrance (a switch during it counts).
        let watch = || {
            crate::log::info("flyout shown without the focus: watching the foreground");
            self.watch_foreground();
        };
        popup.show(&graphics, layout, origin, motion, Unfocused::Then(&watch));
    }

    /// Shows new content in the open flyout (state changed from anywhere, FR-003). The
    /// controls keep their hover, focus and drag; the height follows smoothly.
    /// Nothing happens while the flyout is closed.
    pub fn update_flyout(&self, content: FlyoutContent) {
        if !self.flyout_visible() {
            return;
        }
        let Some(popup) = self.flyout_popup.borrow().clone() else { return };
        // Application rows that vanish are kept a moment: after an output switch, an
        // application's sound reaches the new output a little later, and its row would
        // otherwise disappear and come back. A real removal shows once the
        // hold is over.
        let shown = self.flyout_state.borrow().view.as_ref().map(|v| v.content().clone());
        let mut showing = content.clone();
        let kept = shown.as_ref().is_some_and(|s| showing.keep_rows_of(s));
        let holding = {
            let mut state = self.flyout_state.borrow_mut();
            let holding = state.held.is_some();
            state.held = kept.then_some(content);
            holding
        };
        // SAFETY: a one-shot timer of our own window (not restarted while it runs, so a
        // removal is never held longer than the hold).
        unsafe {
            if kept && !holding {
                SetTimer(Some(popup.window()), TIMER_ROWS, ROW_REMOVAL_HOLD_MS, None);
            } else if !kept {
                let _ = KillTimer(Some(popup.window()), TIMER_ROWS);
            }
        }
        self.apply_content(Some(showing));
    }

    /// Lays out `content` (or the current content again) and repaints or resizes.
    fn apply_content(&self, content: Option<FlyoutContent>) {
        let Some(popup) = self.flyout_popup.borrow().clone() else { return };
        let Some(graphics) = self.graphics() else { return };
        let change = {
            let mut guard = self.flyout_state.borrow_mut();
            let state = &mut *guard;
            let (Some(theme), Some(layout)) = (state.theme, state.layout) else { return };
            let max_height = state.max_height();
            let Some(view) = state.view.as_mut() else { return };
            let content = content.unwrap_or_else(|| view.content().clone());
            let content = resolve_icons(content, &mut state.icons, layout.scale, &graphics);
            let measure = GraphicsMeasure { graphics: &graphics, text_scale: theme.text_scale };
            view.set_content(content, &measure, theme.text_scale, max_height)
        };
        if change.restructured {
            uia::structure_changed(popup.window());
        }
        if change.resized {
            self.start_resize(&popup, &graphics);
        } else {
            self.repaint_flyout(&popup, &graphics);
        }
        self.refresh_tooltip(&popup);
    }

    /// Closes the flyout (nothing happens if it is closed). The application hears of it
    /// through [`Event::FlyoutClosed`].
    pub fn hide_flyout(&self) {
        let Some(popup) = self.flyout_popup.borrow().clone() else { return };
        if !popup.is_visible() || self.flyout_closing.replace(true) {
            return;
        }
        self.stop_watching_foreground();
        self.hide_tooltip();
        // SAFETY: timers of our own window; killing a timer that does not run is harmless.
        unsafe {
            let _ = KillTimer(Some(popup.window()), TIMER_RESIZE);
            let _ = KillTimer(Some(popup.window()), TIMER_TOOLTIP);
            let _ = KillTimer(Some(popup.window()), TIMER_ROWS);
            // A drag in progress ends (the window procedure sees the capture change).
            let _ = ReleaseCapture();
        }
        let animated = self.theme().animations;
        let graphics = self.graphics();
        popup.hide(graphics.as_deref(), animated.then_some(FLYOUT_CLOSE));
        {
            let mut state = self.flyout_state.borrow_mut();
            state.view = None;
            state.resize = None;
            state.held = None;
            state.tracking_leave = false;
        }
        self.flyout_closing.set(false);
        // SAFETY: posting a message without pointers to our own window.
        let _ = unsafe { PostMessageW(Some(self.window), WM_FLYOUT_CLOSED, WPARAM(0), LPARAM(0)) };
    }

    /// The appearance changed: an open flyout takes the new look at once.
    pub(crate) fn restyle_flyout(&self) {
        let Some(popup) = self.flyout_popup.borrow().clone() else { return };
        if !popup.is_visible() {
            return;
        }
        let theme = popup.apply_theme(self.theme());
        self.flyout_state.borrow_mut().theme = Some(theme);
        self.apply_content(None);
    }

    /// Paints the flyout again at its shown height.
    fn repaint_flyout(&self, popup: &Popup, graphics: &Graphics) {
        let mut guard = self.flyout_state.borrow_mut();
        let state = &mut *guard;
        let (Some(view), Some(theme), Some(layout)) =
            (state.view.as_ref(), state.theme, state.layout)
        else {
            return;
        };
        state.last_frame = None;
        match popup.repaint(graphics, theme.text_scale, |c| view.paint(c, &theme)) {
            Ok(()) if view.at_rest() => {
                let size = pixels(view.width(), view.height(), layout.scale);
                state.last_frame = Some(Frame {
                    content: view.content().clone(),
                    theme,
                    size,
                    scale: layout.scale,
                });
            }
            Ok(()) => {}
            Err(error) => {
                crate::log::error(&format!("flyout not redrawn: {error}"));
                drop(guard);
                self.recover_flyout(popup);
            }
        }
    }

    /// The flyout could not be drawn (lost device, display change) or lost its surface
    /// while open: rebuilds the graphics objects and paints it again (NFR-05).
    fn recover_flyout(&self, popup: &Popup) {
        self.reset_graphics();
        // The view is taken out while painting: sizing the window sends it messages,
        // whose handling looks the state up.
        let (view, theme, layout) = {
            let mut state = self.flyout_state.borrow_mut();
            (state.view.take(), state.theme, state.layout)
        };
        let (Some(view), Some(theme), Some(layout)) = (view, theme, layout) else { return };
        let size = pixels(view.width(), view.height(), layout.scale);
        let paint = |c: &mut bv_ui_kit::backend::Canvas<'_>| view.paint(c, &theme);
        let painted =
            self.paint_popup(popup, "flyout", size, layout.scale, theme.text_scale, &paint);
        let mut state = self.flyout_state.borrow_mut();
        state.shown = view.height();
        state.resize = None;
        state.view = Some(view);
        if painted.is_none() {
            drop(state);
            self.hide_flyout();
        }
    }

    /// Starts the height change towards the view's height, or applies it at once
    /// when animations are off.
    fn start_resize(&self, popup: &Popup, graphics: &Graphics) {
        let animated = self.flyout_state.borrow().theme.is_some_and(|t| t.animations);
        {
            let mut state = self.flyout_state.borrow_mut();
            let Some(to) = state.view.as_ref().map(FlyoutView::height) else { return };
            let from = state.shown;
            state.resize = animated.then(|| Resize { from, to, started: Instant::now() });
        }
        if animated {
            // SAFETY: a timer of our own window, killed when the change ends.
            unsafe { SetTimer(Some(popup.window()), TIMER_RESIZE, RESIZE_FRAME_MS, None) };
            self.resize_step(popup, graphics);
        } else {
            self.finish_resize(popup, graphics);
        }
    }

    /// One frame of a height change.
    fn resize_step(&self, popup: &Popup, graphics: &Graphics) {
        let resize = self.flyout_state.borrow().resize;
        let Some(resize) = resize else {
            // SAFETY: a timer of our own window.
            let _ = unsafe { KillTimer(Some(popup.window()), TIMER_RESIZE) };
            return;
        };
        let elapsed = resize.started.elapsed().as_secs_f32() * 1000.0;
        if elapsed >= FLYOUT_RESIZE.duration_ms as f32 {
            self.finish_resize(popup, graphics);
            return;
        }
        let progress = FLYOUT_RESIZE.progress(elapsed);
        let height = resize.from + (resize.to - resize.from) * progress;
        self.show_height(popup, graphics, height);
    }

    /// The end of a height change: the final height, the previous content forgotten.
    fn finish_resize(&self, popup: &Popup, graphics: &Graphics) {
        // SAFETY: a timer of our own window.
        let _ = unsafe { KillTimer(Some(popup.window()), TIMER_RESIZE) };
        let height = {
            let mut state = self.flyout_state.borrow_mut();
            state.resize = None;
            let Some(view) = state.view.as_mut() else { return };
            view.end_transition();
            view.height()
        };
        self.show_height(popup, graphics, height);
    }

    /// Gives the window `height` DIPs, bottom (or top, for a top taskbar) edge in place.
    fn show_height(&self, popup: &Popup, graphics: &Graphics, height: f32) {
        let (size, origin) = {
            let mut guard = self.flyout_state.borrow_mut();
            let state = &mut *guard;
            let (Some(theme), Some(layout)) = (state.theme, state.layout) else { return };
            let Some(view) = state.view.as_mut() else { return };
            view.set_shown_height(height);
            state.shown = height;
            let size = pixels(view.width(), height, layout.scale);
            let margin = (MARGIN * layout.scale).round() as i32;
            let origin = flyout_origin(size, &layout.area, layout.edge, margin, layout.rtl);
            let view = &*view;
            state.last_frame = None;
            if !popup.has_surface() {
                drop(guard);
                self.recover_flyout(popup);
                return;
            }
            match popup.resize_render(graphics, size, theme.text_scale, |c| view.paint(c, &theme)) {
                Ok(()) if view.at_rest() => {
                    state.last_frame = Some(Frame {
                        content: view.content().clone(),
                        theme,
                        size,
                        scale: layout.scale,
                    });
                }
                Ok(()) => {}
                Err(error) => {
                    crate::log::error(&format!("flyout not resized: {error}"));
                    drop(guard);
                    self.recover_flyout(popup);
                    return;
                }
            }
            (size, origin)
        };
        // Without holding the state: moving the window sends it messages.
        popup.set_frame(graphics, origin, size);
    }

    fn flyout_window(&self) -> Option<Rc<Popup>> {
        // A window destroyed behind our back (it should not happen: close requests only
        // hide popups) is replaced rather than reused.
        let existing = self.flyout_popup.borrow().clone();
        match existing {
            Some(popup) if popup.is_alive() => return Some(popup),
            Some(_) => drop(self.flyout_popup.borrow_mut().take()),
            None => {}
        }
        self.flyout_state.borrow_mut().forget_frame();
        match Popup::create(self.window, crate::popup::FLYOUT_CLASS) {
            Ok(popup) => {
                let popup = Rc::new(popup);
                *self.flyout_popup.borrow_mut() = Some(popup.clone());
                Some(popup)
            }
            Err(error) => {
                crate::log::error(&format!(
                    "flyout window not created: {}",
                    crate::ShellError::from(error)
                ));
                None
            }
        }
    }

    pub(crate) fn is_flyout_window(&self, window: HWND) -> bool {
        self.flyout_popup.borrow().as_ref().is_some_and(|p| p.window() == window)
    }

    /// A message for the flyout window; `None` lets the default procedure handle it.
    /// Requests of the user are handed to `dispatch` (the application's handler).
    pub(crate) fn flyout_message(
        &self,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        dispatch: &dyn Fn(Event),
    ) -> Option<LRESULT> {
        let popup = self.flyout_popup.borrow().clone()?;
        let to_dip = |x: i32, y: i32| {
            let scale = self.flyout_state.borrow().scale().max(0.1);
            Point::new(x as f32 / scale, y as f32 / scale)
        };
        let client = || {
            let x = i32::from((lparam.0 & 0xFFFF) as u16 as i16);
            let y = i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16);
            to_dip(x, y)
        };
        let response = match message {
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
                if self.flyout_visible() && !self.flyout_closing.get() {
                    // `lparam`: the window taking the focus (may be unknown).
                    self.closed_by_focus_loss(HWND(lparam.0 as *mut core::ffi::c_void));
                }
                return Some(LRESULT(0));
            }
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                self.hide_flyout();
                return Some(LRESULT(0));
            }
            WM_KEYDOWN => {
                let key = flyout_key(wparam.0 as u16, shift_down())?;
                // Bit 30: the key was already down (auto-repeat). Holding Enter or Space
                // must not toggle a mute or a section on and off.
                let repeat = (lparam.0 >> 30) & 1 == 1;
                if repeat && key == FlyoutKey::Activate {
                    return Some(LRESULT(0));
                }
                self.hide_tooltip();
                self.with_view(|view| view.key(key))?
            }
            WM_MOUSEMOVE => {
                self.track_leave(&popup);
                let p = client();
                self.with_view(|view| view.pointer_move(p))?
            }
            WM_MOUSELEAVE => {
                if let Ok(mut state) = self.flyout_state.try_borrow_mut() {
                    state.tracking_leave = false;
                }
                self.hide_tooltip();
                self.with_view(FlyoutView::pointer_leave)?
            }
            WM_LBUTTONDOWN => {
                self.hide_tooltip();
                let p = client();
                self.with_view(|view| view.press(p))?
            }
            WM_LBUTTONUP => {
                let p = client();
                self.with_view(|view| view.release(p))?
            }
            WM_CAPTURECHANGED => {
                let ours = HWND(lparam.0 as *mut core::ffi::c_void) == popup.window();
                if ours {
                    return Some(LRESULT(0));
                }
                self.with_view(|view| {
                    if view.dragging() {
                        view.cancel_pointer()
                    } else {
                        Response::default()
                    }
                })?
            }
            WM_MOUSEWHEEL => {
                self.hide_tooltip();
                let delta = i32::from((wparam.0 >> 16) as u16 as i16);
                let mut point = POINT {
                    x: i32::from((lparam.0 & 0xFFFF) as u16 as i16),
                    y: i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16),
                };
                // SAFETY: our own window and a valid point.
                let _ = unsafe { ScreenToClient(popup.window(), &mut point) };
                let p = to_dip(point.x, point.y);
                let notches = {
                    let mut state = self.flyout_state.try_borrow_mut().ok()?;
                    let total = state.wheel_rest + delta;
                    state.wheel_rest = total % WHEEL_DELTA as i32;
                    total / WHEEL_DELTA as i32
                };
                if notches == 0 {
                    return Some(LRESULT(0));
                }
                self.with_view(|view| view.wheel(p, notches))?
            }
            WM_TIMER if wparam.0 == TIMER_RESIZE => {
                if let Some(graphics) = self.graphics() {
                    self.resize_step(&popup, &graphics);
                }
                return Some(LRESULT(0));
            }
            WM_TIMER if wparam.0 == TIMER_ROWS => {
                // SAFETY: a timer of our own window; one-shot.
                let _ = unsafe { KillTimer(Some(popup.window()), TIMER_ROWS) };
                let held = self.flyout_state.borrow_mut().held.take();
                if held.is_some() {
                    self.apply_content(held);
                }
                return Some(LRESULT(0));
            }
            WM_TIMER if wparam.0 == TIMER_TOOLTIP => {
                // SAFETY: a timer of our own window; one-shot.
                let _ = unsafe { KillTimer(Some(popup.window()), TIMER_TOOLTIP) };
                self.show_tooltip(&popup);
                return Some(LRESULT(0));
            }
            _ => return None,
        };
        self.apply_response(&popup, response, dispatch);
        if matches!(message, WM_MOUSEMOVE | WM_MOUSELEAVE) {
            self.refresh_tooltip(&popup);
        }
        Some(LRESULT(0))
    }

    /// Runs `f` on the view, if it exists and nothing else uses it right now.
    fn with_view(&self, f: impl FnOnce(&mut FlyoutView) -> Response) -> Option<Response> {
        let mut state = self.flyout_state.try_borrow_mut().ok()?;
        state.view.as_mut().map(f)
    }

    /// Carries out what the view asked for after an input.
    fn apply_response(&self, popup: &Popup, response: Response, dispatch: &dyn Fn(Event)) {
        match response.capture {
            // SAFETY: our own window; the capture ends with the drag.
            Some(true) => unsafe {
                SetCapture(popup.window());
            },
            // SAFETY: plain call.
            Some(false) => unsafe {
                let _ = ReleaseCapture();
            },
            None => {}
        }
        if response.resized && !self.flyout_closing.get() {
            // Content that waited for the end of a drag (FR-063).
            let pending =
                self.flyout_state.borrow_mut().view.as_mut().and_then(FlyoutView::take_pending);
            if let Some(pending) = pending {
                self.apply_content(Some(pending));
            }
        }
        if response.redraw {
            if let Some(graphics) = self.graphics() {
                self.repaint_flyout(popup, &graphics);
            }
        }
        if response.focus_moved {
            let focused =
                self.flyout_state.borrow().view.as_ref().and_then(FlyoutView::focused_access_id);
            uia::focus_changed(popup.window(), focused);
        }
        if let Some(action) = response.action {
            dispatch(Event::Flyout(action));
            // The application applied it (its handler ran): screen readers re-read.
            let change = match action {
                FlyoutAction::SetVolume(..) => Some(AccessChange::Value),
                FlyoutAction::ToggleMute(_) => Some(AccessChange::Toggled),
                FlyoutAction::SelectTab(_) | FlyoutAction::SelectDevice(_) => {
                    Some(AccessChange::Selected)
                }
                FlyoutAction::SetAppsExpanded(_) => Some(AccessChange::Expanded),
                FlyoutAction::OpenSoundSettings
                | FlyoutAction::OpenSettings
                | FlyoutAction::InstallUpdate
                | FlyoutAction::DismissUpdate => None,
            };
            if let (Some(change), Some(id)) = (change, FlyoutView::action_access_id(action)) {
                uia::property_changed(popup.window(), id, change);
            }
        }
    }

    fn track_leave(&self, popup: &Popup) {
        let Ok(mut state) = self.flyout_state.try_borrow_mut() else { return };
        if state.tracking_leave {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: popup.window(),
            dwHoverTime: 0,
        };
        // SAFETY: valid structure for our own window.
        state.tracking_leave = unsafe { TrackMouseEvent(&mut track) }.is_ok();
    }

    // ----- Tooltips -----

    /// The pointer moved: the tooltip follows the control under it, after a delay.
    fn refresh_tooltip(&self, popup: &Popup) {
        let wanted = {
            let Ok(state) = self.flyout_state.try_borrow() else { return };
            state.view.as_ref().and_then(FlyoutView::tooltip)
        };
        let (same_shown, same_pending) = {
            let state = self.flyout_state.borrow();
            let text = wanted.as_ref().map(|(t, _)| t);
            (
                state.tooltip_shown.as_ref() == text,
                state.tooltip_pending.as_ref().map(|(t, _)| t) == text,
            )
        };
        if same_shown || (same_pending && wanted.is_some()) {
            return;
        }
        self.hide_tooltip();
        if let Some(wanted) = wanted {
            self.flyout_state.borrow_mut().tooltip_pending = Some(wanted);
            // SAFETY: a one-shot timer of our own window.
            unsafe { SetTimer(Some(popup.window()), TIMER_TOOLTIP, TOOLTIP_DELAY_MS, None) };
        }
    }

    fn hide_tooltip(&self) {
        {
            let Ok(mut state) = self.flyout_state.try_borrow_mut() else { return };
            state.tooltip_pending = None;
            state.tooltip_shown = None;
        }
        if let Some(flyout) = self.flyout_popup.borrow().as_ref() {
            // SAFETY: a timer of our own window.
            let _ = unsafe { KillTimer(Some(flyout.window()), TIMER_TOOLTIP) };
        }
        let tooltip = self.tooltip_popup.borrow().clone();
        if let Some(tooltip) = tooltip {
            tooltip.hide(None, None);
        }
    }

    /// The delay is over: shows the tooltip next to its control.
    fn show_tooltip(&self, flyout: &Popup) {
        let (text, rect, layout, theme) = {
            let mut state = self.flyout_state.borrow_mut();
            let Some((text, rect)) = state.tooltip_pending.take() else { return };
            // Only if the pointer still rests on the same control.
            let current = state.view.as_ref().and_then(FlyoutView::tooltip);
            if current.as_ref().map(|(t, _)| t) != Some(&text) {
                return;
            }
            let (Some(layout), Some(theme)) = (state.layout, state.theme) else { return };
            state.tooltip_shown = Some(text.clone());
            (text, rect, layout, theme)
        };
        let Some(graphics) = self.graphics() else { return };
        let Some(popup) = self.tooltip_window() else { return };
        let scale = layout.scale;
        let solid = Theme { material: Material::Solid(theme.controls.solid), ..theme };
        let solid = popup.apply_theme(solid);
        let size_dip = tooltip::size(&text, &graphics, theme.text_scale);
        let size = pixels(size_dip.width, size_dip.height, scale);
        let paint =
            |c: &mut bv_ui_kit::backend::Canvas<'_>| tooltip::paint(c, &solid, &text, size_dip);
        let Some(graphics) =
            self.paint_popup(&popup, "tooltip", size, scale, theme.text_scale, &paint)
        else {
            return;
        };
        // Centred on the control, above it (below if there is no room).
        let (fx, fy) = flyout.origin();
        let gap = (tooltip::offset() * scale).round() as i32;
        let left = fx + ((rect.x + rect.width / 2.0) * scale) as i32 - size.0 / 2;
        let top = fy + (rect.y * scale) as i32 - gap - size.1;
        let below = fy + (rect.bottom() * scale) as i32 + gap;
        let area = layout.area;
        // Kept on the monitor, the same gap away from its edges as from the control.
        let x = left.clamp(area.left + gap, (area.right - gap - size.0).max(area.left + gap));
        let y = if top >= area.top { top } else { below };
        popup.show_inactive(&graphics, layout, (x, y));
    }

    fn tooltip_window(&self) -> Option<Rc<Popup>> {
        let existing = self.tooltip_popup.borrow().clone();
        match existing {
            Some(popup) if popup.is_alive() => return Some(popup),
            Some(_) => drop(self.tooltip_popup.borrow_mut().take()),
            None => {}
        }
        match Popup::create_passive(self.window) {
            Ok(popup) => {
                let popup = Rc::new(popup);
                *self.tooltip_popup.borrow_mut() = Some(popup.clone());
                Some(popup)
            }
            Err(error) => {
                crate::log::error(&format!(
                    "tooltip window not created: {}",
                    crate::ShellError::from(error)
                ));
                None
            }
        }
    }

    /// Destroys the popups (the application is ending).
    pub(crate) fn destroy_popups(&self) {
        self.close_settings();
        // Taken out first, dropped after: destroying a window sends it messages, whose
        // handling looks these cells up again.
        let menu = self.menu_popup.borrow_mut().take();
        let flyout = self.flyout_popup.borrow_mut().take();
        let tooltip = self.tooltip_popup.borrow_mut().take();
        let bubble = self.bubble_popup.borrow_mut().take();
        for popup in [&menu, &flyout, &tooltip].into_iter().flatten() {
            uia::disconnect(popup.window());
        }
        drop(menu);
        drop(flyout);
        drop(tooltip);
        drop(bubble);
    }
}

/// Screen-reader access to the flyout: reads the view through the message window.
pub(crate) struct FlyoutAccess {
    pub(crate) owner: HWND,
}

impl FlyoutAccess {
    fn with_shell<R>(&self, f: impl FnOnce(&crate::host::Host<'_>) -> Option<R>) -> Option<R> {
        // SAFETY: UI Automation calls the providers on the UI thread (COM threading), where
        // the message window's user data is the host or 0.
        let host = unsafe { crate::host::host_of(self.owner) }?;
        f(host)
    }
}

impl AccessSource for FlyoutAccess {
    fn tree(&self) -> Option<bv_ui_kit::access::AccessNode> {
        self.with_shell(|host| {
            let state = host.shell.flyout_state.try_borrow().ok()?;
            state.view.as_ref().map(FlyoutView::access_tree)
        })
    }

    fn scale(&self) -> f32 {
        self.with_shell(|host| host.shell.flyout_state.try_borrow().ok().map(|s| s.scale()))
            .unwrap_or(1.0)
    }

    fn act(&self, id: u64, action: AccessAction) -> bool {
        self.with_shell(|host| {
            let shell = &host.shell;
            let popup = shell.flyout_popup.borrow().clone()?;
            let response = shell.with_view(|view| view.access_action(id, action))?;
            let acted = response != Response::default();
            shell.apply_response(&popup, response, &|event| host.dispatch(event));
            Some(acted)
        })
        .unwrap_or(false)
    }

    fn focused(&self) -> Option<u64> {
        self.with_shell(|host| {
            let state = host.shell.flyout_state.try_borrow().ok()?;
            state.view.as_ref().and_then(FlyoutView::focused_access_id)
        })
    }
}

/// Loads the application icons of the rows at the size they are drawn (FR-062).
fn resolve_icons(
    mut content: FlyoutContent,
    icons: &mut IconCache,
    scale: f32,
    graphics: &Graphics,
) -> FlyoutContent {
    let pixels = (APP_ICON * scale).round().max(1.0) as u32;
    if let Some(apps) = content.apps.as_mut() {
        for row in &mut apps.rows {
            if let (VolumeIcon::App(None), Some(source)) = (&row.icon, &row.icon_source) {
                row.icon = VolumeIcon::App(icons.get(source, pixels, graphics));
            }
        }
    }
    content
}

/// A size in DIPs as whole physical pixels.
fn pixels(width: f32, height: f32, scale: f32) -> (i32, i32) {
    ((width * scale).round() as i32, (height * scale).round() as i32)
}

fn shift_down() -> bool {
    // SAFETY: plain query of the keyboard state of this thread.
    unsafe { GetKeyState(i32::from(VK_SHIFT.0)) < 0 }
}

/// The flyout key for a virtual key, if the flyout reacts to it.
fn flyout_key(key: u16, shift: bool) -> Option<FlyoutKey> {
    Some(match key {
        k if k == VK_TAB.0 => FlyoutKey::Tab { back: shift },
        k if k == VK_LEFT.0 => FlyoutKey::Left,
        k if k == VK_RIGHT.0 => FlyoutKey::Right,
        k if k == VK_UP.0 => FlyoutKey::Up,
        k if k == VK_DOWN.0 => FlyoutKey::Down,
        k if k == VK_PRIOR.0 => FlyoutKey::PageUp,
        k if k == VK_NEXT.0 => FlyoutKey::PageDown,
        k if k == VK_HOME.0 => FlyoutKey::Home,
        k if k == VK_END.0 => FlyoutKey::End,
        k if k == VK_RETURN.0 || k == VK_SPACE.0 => FlyoutKey::Activate,
        _ => return None,
    })
}

/// Posted to the message window when another window took the foreground while the
/// flyout was shown without the focus.
pub(crate) const WM_FOREGROUND_CHANGED: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 3;

thread_local! {
    /// The message window to notify (raw handle), while a foreground watch is active.
    static WATCH_OWNER: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

/// Out-of-context window-event callback: runs on the thread that set the hook (ours), from
/// its message loop. Only posts a message: the shell acts when it handles it.
unsafe extern "system" fn foreground_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    window: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let owner = WATCH_OWNER.with(|o| o.get());
    if owner != 0 {
        // SAFETY: posting a message without pointers to our own window (or to a stale
        // handle, which only makes the call fail).
        let _ = unsafe {
            PostMessageW(
                Some(HWND(owner as *mut core::ffi::c_void)),
                WM_FOREGROUND_CHANGED,
                WPARAM(window.0 as usize),
                LPARAM(0),
            )
        };
    }
}

impl Shell {
    /// The flyout is shown but Windows refused it the focus: it would never learn of a
    /// click elsewhere. Watch the foreground instead (event-driven, no polling): when the
    /// user brings any other program forward, the flyout closes.
    fn watch_foreground(&self) {
        if self.foreground_hook.get().is_some() {
            return;
        }
        WATCH_OWNER.with(|o| o.set(self.window.0 as isize));
        // SAFETY: an out-of-context hook with a callback that lives for the whole program.
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() {
            crate::log::error("flyout without focus: cannot watch the foreground");
        } else {
            self.foreground_hook.set(Some(hook));
        }
    }

    pub(crate) fn stop_watching_foreground(&self) {
        if let Some(hook) = self.foreground_hook.take() {
            // SAFETY: the hook was set by `watch_foreground` and is removed once.
            let _ = unsafe { UnhookWinEvent(hook) };
        }
        WATCH_OWNER.with(|o| o.set(0));
    }

    /// Another program came forward while the unfocused flyout was shown.
    pub(crate) fn foreground_changed(&self, foreground: HWND) {
        if self.foreground_hook.get().is_none() || !self.flyout_visible() {
            return;
        }
        self.closed_by_focus_loss(foreground);
    }

    /// The flyout lost the focus to `other`: it closes. If the focus went to the taskbar
    /// (or to an unknown window), the click that took it may be on the tray icon, and must
    /// not open the flyout again; a click in any other program is not, so a quick click
    /// on the icon right after it still opens the flyout.
    fn closed_by_focus_loss(&self, other: HWND) {
        if crate::screen::taskbar_surface(other).unwrap_or(true) {
            // The tick count, not the message time: focus messages are sent, not posted,
            // so their message time can be that of an older message. Both clocks count
            // milliseconds since start-up, so they compare.
            // SAFETY: plain call.
            let time = unsafe { GetTickCount() };
            self.flyout_toggle.borrow_mut().closed_by_focus_loss(time);
        }
        self.hide_flyout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_mapped() {
        assert_eq!(flyout_key(VK_TAB.0, true), Some(FlyoutKey::Tab { back: true }));
        assert_eq!(flyout_key(VK_PRIOR.0, false), Some(FlyoutKey::PageUp));
        assert_eq!(flyout_key(VK_SPACE.0, false), Some(FlyoutKey::Activate));
        assert_eq!(flyout_key(0x41, false), None, "letters do nothing");
    }

    #[test]
    fn sizes_round_to_whole_pixels() {
        assert_eq!(pixels(360.0, 200.0, 1.5), (540, 300));
        assert_eq!(pixels(360.0, 200.3, 1.25), (450, 250));
    }
}
