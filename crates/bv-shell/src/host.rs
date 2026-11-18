//! The hidden message window, its message loop, and the window procedures.
//!
//! One UI thread owns everything (`docs/ARCHITECTURE.md` §3). The application passes a
//! handler to [`run`]; the window procedures translate raw Windows messages into
//! [`Event`]s and call it. Nothing here polls: the thread sleeps in `GetMessageW` until
//! Windows has something for us. The tray icon lives in `tray.rs`, the context menu in
//! `menu.rs` and the flyout in `flyout.rs`; all three are methods of [`Shell`].

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use bv_ui_kit::backend::{DeviceKind, Graphics};
use bv_ui_kit::controls::flyout::FlyoutAction;
use bv_ui_kit::controls::settings::SettingsAction;
use bv_ui_kit::{Theme, ThemeInputs};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageTime, GetMessageW, GetWindowLongPtrW, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SetWindowLongPtrW, TranslateMessage, GWLP_USERDATA, MSG, MSGFLT_ALLOW,
    PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_CONTEXTMENU,
    WM_DESTROY, WM_DPICHANGED, WM_DWMCOLORIZATIONCOLORCHANGED, WM_ERASEBKGND, WM_GETOBJECT,
    WM_INPUT, WM_MBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_POWERBROADCAST, WM_SETTINGCHANGE,
    WM_SYSCOLORCHANGE, WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

use crate::input::FlyoutToggle;
use crate::popup::{self, Popup};
use crate::tray::TrayState;
use crate::waker::{Waker, WM_WAKE};
use crate::{appearance, ShellError};

/// Class of the hidden message window. Also how a second launch (and the measurement
/// tool) finds the running instance.
pub(crate) const WINDOW_CLASS: PCWSTR = w!("BetterVolume.MessageWindow");

/// Class of the hidden message window of a run without tray icon (the installer and the
/// uninstaller): a second launch and the installer itself never mistake it for
/// the running application.
const WINDOW_ONLY_CLASS: PCWSTR = w!("BetterVolume.SetupMessageWindow");

/// Callback message of the tray icon.
pub(crate) const WM_TRAY: u32 = WM_APP + 1;
/// Keyboard activation of the tray icon (`NIN_SELECT | NINF_KEY` in `shellapi.h`; not
/// exposed by the bindings).
const NIN_KEYSELECT: u32 = NIN_SELECT | 0x1;

/// Timer that retries adding the tray icon when the taskbar was not ready for it.
pub(crate) const TIMER_ICON_RETRY: usize = 1;
/// Timer that follows the pointer while it hovers the tray icon (wheel capture).
pub(crate) const TIMER_HOVER: usize = 2;
/// Timer that looks again for Windows' record of a new tray icon.
pub(crate) const TIMER_PROMOTE: usize = 3;

/// The message a second launch posts to the running instance.
pub(crate) fn activate_message() -> u32 {
    // SAFETY: the name is a valid NUL-terminated constant.
    unsafe { RegisterWindowMessageW(w!("BetterVolume.Activate")) }
}

/// The message the installer and the uninstaller post to the running instance to make it
/// quit as from its menu.
pub(crate) fn exit_message() -> u32 {
    // SAFETY: the name is a valid NUL-terminated constant.
    unsafe { RegisterWindowMessageW(w!("BetterVolume.Exit")) }
}

/// What the shell needs to start.
#[derive(Clone, Debug)]
pub struct ShellConfig {
    /// Tooltip of the tray icon (already translated).
    pub tooltip: String,
    /// Name of the application, as screen readers announce its menu.
    pub app_name: String,
    /// Whether the run has a tray icon. Without one (the installer and the uninstaller),
    /// only the settings window is used, and the message window is not the one other
    /// launches look for.
    pub tray: bool,
}

/// A primary action on the tray icon, to hand to [`Shell::toggle_flyout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayActivation {
    /// Message time (milliseconds, wrapping) of the action.
    pub(crate) time: u32,
    /// From the keyboard rather than the mouse.
    pub(crate) keyboard: bool,
}

/// Something the user or Windows did, reported to the application's handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Primary action on the tray icon (left click, or keyboard activation): FR-013
    /// wants the flyout toggled ([`Shell::toggle_flyout`]).
    TrayPrimary(TrayActivation),
    /// Middle click on the tray icon.
    TrayMiddle,
    /// The mouse wheel turned while the pointer was over the tray icon (FR-015), by this
    /// many whole notches (positive = up).
    TrayWheel {
        /// Whole notches, positive away from the user.
        notches: i32,
    },
    /// The context menu of the tray icon was requested (right click or keyboard), at
    /// these screen coordinates.
    TrayMenu {
        /// Horizontal screen coordinate, in physical pixels.
        x: i32,
        /// Vertical screen coordinate, in physical pixels.
        y: i32,
    },
    /// The application was launched again while already running (FR-001).
    ActivateRequested,
    /// The installer or the uninstaller asked the application to quit.
    ExitRequested,
    /// A background source called [`Waker::wake`]: drain it.
    Wake,
    /// The computer resumed from sleep or hibernation (NFR-05).
    Resumed,
    /// The computer is about to sleep or hibernate: little time is left (Windows gives
    /// about two seconds) to say goodbye to the network.
    Suspending,
    /// The user asked for something in the flyout.
    Flyout(FlyoutAction),
    /// The flyout was closed (by any means): what it showed no longer needs watching.
    FlyoutClosed,
    /// The user asked for something in the settings window.
    Settings(SettingsAction),
    /// The settings window came to the foreground (what it shows from outside the
    /// application, such as the autostart entry, may have changed meanwhile).
    SettingsActivated,
    /// The settings window was closed, at this place.
    SettingsClosed(Option<crate::settings::WindowPlace>),
    /// What became of the last request to hide or show the Windows volume icon
    /// ([`Shell::set_volume_icon_hidden`]), or of hiding it again in a re-created taskbar.
    VolumeIcon(crate::taskbar::VolumeIconOutcome),
    /// The taskbar crashed shortly after the Windows volume icon was hidden: the shell
    /// stopped hiding it (the new taskbar shows it), and the option must be turned off and
    /// the user told.
    VolumeIconIncident,
    /// The timer of [`Shell::set_update_timer`] fired: an automatic update check may be
    /// due (FR-132).
    UpdateDue,
}

/// Handle given to the application's handler to act on the shell.
pub struct Shell {
    pub(crate) window: HWND,
    pub(crate) taskbar_created: u32,
    pub(crate) activate: u32,
    pub(crate) exit_request: u32,
    /// The run has a tray icon (see [`ShellConfig::tray`]).
    pub(crate) has_tray: bool,
    pub(crate) tooltip: RefCell<String>,
    pub(crate) icon_added: Cell<bool>,
    pub(crate) icon_retries_left: Cell<u32>,
    /// Looks made for Windows' record of our icon (0: not started, `u32::MAX`: done).
    pub(crate) promote_tries: Cell<u32>,
    pub(crate) quitting: Cell<bool>,
    pub(crate) waker: Waker,
    /// Windows appearance preferences, as last read.
    pub(crate) appearance: Cell<ThemeInputs>,
    /// Shared graphics factories and device, kept once a popup was shown.
    graphics: RefCell<Option<Rc<Graphics>>>,
    /// The last attempt to create graphics objects failed (logged once).
    graphics_failed: Cell<bool>,
    pub(crate) tray: RefCell<TrayState>,
    pub(crate) menu_popup: RefCell<Option<Rc<Popup>>>,
    pub(crate) menu_session: RefCell<Option<crate::menu::Session>>,
    pub(crate) flyout_popup: RefCell<Option<Rc<Popup>>>,
    /// The flyout's view and interaction state while it is shown.
    pub(crate) flyout_state: RefCell<crate::flyout::FlyoutState>,
    pub(crate) tooltip_popup: RefCell<Option<Rc<Popup>>>,
    /// The bubble above the taskbar (FR-105), created on first use, and its state.
    pub(crate) bubble_popup: RefCell<Option<Rc<Popup>>>,
    pub(crate) bubble_state: RefCell<crate::bubble::BubbleState>,
    /// Name of the application (screen readers).
    pub(crate) app_name: RefCell<String>,
    pub(crate) flyout_toggle: RefCell<FlyoutToggle>,
    /// The flyout is being hidden (its hiding sends messages that must not hide it again).
    pub(crate) flyout_closing: Cell<bool>,
    /// Foreground watch while the flyout is shown without the focus.
    pub(crate) foreground_hook: Cell<Option<windows::Win32::UI::Accessibility::HWINEVENTHOOK>>,
    /// The settings window, while it is open.
    pub(crate) settings_state: RefCell<crate::settings::SettingsState>,
    /// Hiding the Windows volume icon (FR-018).
    pub(crate) volume_icon: crate::taskbar::VolumeIconState,
    /// The message the taskbar helper reports its outcomes with.
    pub(crate) volume_icon_status: u32,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").field("window", &self.window).finish_non_exhaustive()
    }
}

impl Shell {
    fn new(window: HWND, has_tray: bool) -> Shell {
        Shell {
            window,
            // SAFETY: the name is a valid NUL-terminated constant.
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            activate: activate_message(),
            exit_request: exit_message(),
            has_tray,
            tooltip: RefCell::new(String::new()),
            icon_added: Cell::new(false),
            icon_retries_left: Cell::new(0),
            promote_tries: Cell::new(0),
            quitting: Cell::new(false),
            waker: Waker::new(window),
            appearance: Cell::new(ThemeInputs::default()),
            graphics: RefCell::new(None),
            graphics_failed: Cell::new(false),
            tray: RefCell::new(TrayState::default()),
            menu_popup: RefCell::new(None),
            menu_session: RefCell::new(None),
            flyout_popup: RefCell::new(None),
            flyout_state: RefCell::new(crate::flyout::FlyoutState::default()),
            tooltip_popup: RefCell::new(None),
            bubble_popup: RefCell::new(None),
            bubble_state: RefCell::new(crate::bubble::BubbleState::default()),
            app_name: RefCell::new(String::new()),
            flyout_toggle: RefCell::new(FlyoutToggle::default()),
            flyout_closing: Cell::new(false),
            foreground_hook: Cell::new(None),
            settings_state: RefCell::new(crate::settings::SettingsState::default()),
            volume_icon: crate::taskbar::VolumeIconState::default(),
            volume_icon_status: crate::taskbar::status_message(),
        }
    }

    /// A handle that wakes this UI thread from any thread (see [`Waker`]).
    pub fn waker(&self) -> Waker {
        self.waker.clone()
    }

    /// Ends the application: removes the tray icon and makes [`run`] return. The handler
    /// receives no further event, including those already queued.
    pub fn quit(&self) {
        self.quitting.set(true);
        // SAFETY: `window` is our own window, on the thread that created it (`Shell` is
        // neither `Send` nor `Sync`). Destroying it twice only makes the second call fail.
        let _ = unsafe { DestroyWindow(self.window) };
    }

    /// The theme of the shell surfaces for the current Windows appearance.
    pub(crate) fn theme(&self) -> Theme {
        Theme::shell(&self.appearance.get())
    }

    /// The shared graphics objects, created on first use. A creation that fails is
    /// retried on the next need (a display change may have made it fail once).
    pub(crate) fn graphics(&self) -> Option<Rc<Graphics>> {
        if let Some(graphics) = self.graphics.borrow().as_ref() {
            return Some(graphics.clone());
        }
        let graphics = Rc::new(self.new_graphics()?);
        *self.graphics.borrow_mut() = Some(graphics.clone());
        Some(graphics)
    }

    fn new_graphics(&self) -> Option<Graphics> {
        match Graphics::new(DeviceKind::Software) {
            Ok(graphics) => {
                self.graphics_failed.set(false);
                Some(graphics)
            }
            Err(error) => {
                // Logged once per failure streak: the tray icon asks on every change.
                if !self.graphics_failed.replace(true) {
                    crate::log::error(&format!("graphics unavailable: {error}"));
                }
                None
            }
        }
    }

    /// Drops the shared graphics objects and every popup surface built on them: the next
    /// drawing rebuilds them (recovery after a lost device or a display change, NFR-05).
    pub(crate) fn reset_graphics(&self) {
        let popups = [
            self.menu_popup.borrow().clone(),
            self.flyout_popup.borrow().clone(),
            self.tooltip_popup.borrow().clone(),
            self.bubble_popup.borrow().clone(),
        ];
        for popup in popups.into_iter().flatten() {
            popup.drop_surface();
        }
        if let Ok(mut state) = self.flyout_state.try_borrow_mut() {
            state.forget_frame();
        }
        self.drop_settings_surface();
        let old = self.graphics.borrow_mut().take();
        drop(old);
    }

    /// Sizes and paints a popup before it is shown. A drawing failure (lost device,
    /// display change) rebuilds the graphics objects and tries once more. Returns the
    /// graphics objects that drew it, or `None` (logged) if the popup cannot be drawn.
    pub(crate) fn paint_popup(
        &self,
        popup: &Popup,
        what: &str,
        size: (i32, i32),
        scale: f32,
        text_scale: f32,
        paint: &dyn Fn(&mut bv_ui_kit::backend::Canvas<'_>),
    ) -> Option<Rc<Graphics>> {
        for attempt in 0..2 {
            let graphics = self.graphics()?;
            match popup.paint(&graphics, size, scale, text_scale, paint) {
                Ok(()) => return Some(graphics),
                Err(error) => {
                    crate::log::error(&format!(
                        "{what} not drawn (attempt {}): {error}",
                        attempt + 1
                    ));
                    self.reset_graphics();
                }
            }
        }
        None
    }

    /// Runs `draw` with graphics objects for an in-memory image (the tray icon). Until a
    /// popup has needed the shared objects, temporary ones are made and released at once,
    /// so a flyout never opened costs nothing at idle (footprint budget).
    pub(crate) fn with_image_graphics<R>(&self, draw: impl FnOnce(&Graphics) -> R) -> Option<R> {
        let shared = self.graphics.borrow().clone();
        match shared {
            Some(graphics) => Some(draw(&graphics)),
            None => self.new_graphics().map(|graphics| draw(&graphics)),
        }
    }

    /// Re-reads the Windows appearance; when it changed, redraws what shows it.
    pub(crate) fn refresh_appearance(&self) {
        let inputs = appearance::read();
        if inputs == self.appearance.replace(inputs) {
            return;
        }
        self.render_tray_icon();
        self.restyle_flyout();
        self.restyle_settings();
        self.restyle_bubble();
    }
}

/// The application's event handler.
type Handler<'a> = dyn FnMut(&Shell, Event) + 'a;

/// What the window procedures reach through the message window's user data.
pub(crate) struct Host<'a> {
    pub(crate) shell: Shell,
    handler: RefCell<&'a mut Handler<'a>>,
    /// Events that arrived while the handler was already running.
    pending: RefCell<VecDeque<Event>>,
}

impl Host<'_> {
    /// Delivers `event` to the handler, never re-entering it and never losing an event.
    ///
    /// The handler may run a nested message loop (the context menu), during which this
    /// function is called again from a window procedure. Such events are queued and
    /// delivered in order once the running handler call returns. Once the application is
    /// quitting, events are discarded: nothing may act on a shell that is going away.
    pub(crate) fn dispatch(&self, event: Event) {
        if self.shell.quitting.get() {
            return;
        }
        {
            let mut pending = self.pending.borrow_mut();
            // One queued wake-up already drains every source: a burst arriving while the
            // handler is busy (context menu open) is delivered once.
            if !(event == Event::Wake && pending.contains(&Event::Wake)) {
                pending.push_back(event);
            }
        }
        let Ok(mut handler) = self.handler.try_borrow_mut() else { return };
        loop {
            if self.shell.quitting.get() {
                self.pending.borrow_mut().clear();
                break;
            }
            // The queue borrow ends with this statement, before the handler runs.
            let next = self.pending.borrow_mut().pop_front();
            let Some(next) = next else { break };
            handler(&self.shell, next);
        }
    }
}

/// The host behind a message window, if the window is ours and running.
///
/// # Safety
/// Must be called on the UI thread, with a window whose user data is either 0 or the
/// pointer stored by [`run`] (the message window).
pub(crate) unsafe fn host_of<'a>(window: HWND) -> Option<&'a Host<'a>> {
    // SAFETY: per the contract, the user data is 0 or a live `Host` (cleared in
    // WM_DESTROY before the host goes away). Only shared references are made.
    unsafe { (GetWindowLongPtrW(window, GWLP_USERDATA) as *const Host).as_ref() }
}

/// Creates the hidden window and the tray icon, then runs the message loop on the calling
/// thread until [`Shell::quit`] is called or the window is closed.
///
/// `init` runs once, on this thread, as soon as the window exists and before the tray
/// icon is added: it can take a [`Shell::waker`] for background sources and set the first
/// tooltip and icon. It returns the handler, which then receives every [`Event`]. `init`
/// must not call [`Shell::quit`] nor run a message loop (no menu, no flyout): the window
/// does not know the handler yet. To give up at start, return a handler and quit on the
/// first [`Event::Wake`], which is always delivered right after `init`.
///
/// Returns an error if the window cannot be created or the message loop itself fails.
pub fn run<H: FnMut(&Shell, Event)>(
    config: ShellConfig,
    init: impl FnOnce(&Shell) -> H,
) -> Result<(), ShellError> {
    // Must happen before any window exists. Fails harmlessly if already set.
    // SAFETY: plain call with a constant argument.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };

    // SAFETY: a null module name returns the handle of the current executable.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    let class_name = if config.tray { WINDOW_CLASS } else { WINDOW_ONLY_CLASS };
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class_name,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and its strings are constants.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(windows::core::Error::from_thread().into());
    }
    popup::register_classes(Some(popup_proc))?;
    crate::settings::register_class()?;

    // A real (never shown) top-level window rather than a message-only one: only
    // top-level windows receive the "TaskbarCreated" and setting-change broadcasts.
    // SAFETY: the class was registered above; all other arguments are plain values.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR::null(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }?;

    let shell = Shell::new(window, config.tray);
    *shell.tooltip.borrow_mut() = config.tooltip;
    *shell.app_name.borrow_mut() = config.app_name;
    shell.appearance.set(appearance::read());
    // An instance running with administrator rights must still hear a normal second
    // launch, the installer, the taskbar restarting, its own tray icon and the taskbar
    // helper (Windows filters messages from less privileged processes otherwise).
    for message in [
        shell.activate,
        shell.exit_request,
        shell.taskbar_created,
        WM_TRAY,
        shell.volume_icon_status,
    ] {
        if message != 0 {
            // SAFETY: our own window; no filter-status structure requested.
            let _ = unsafe { ChangeWindowMessageFilterEx(window, message, MSGFLT_ALLOW, None) };
        }
    }

    // No message reaches a handler before it exists: the window procedure only forwards
    // to the default procedure until the user data is set below.
    let mut handler = init(&shell);
    let host =
        Host { shell, handler: RefCell::new(&mut handler), pending: RefCell::new(VecDeque::new()) };
    // SAFETY: `host` outlives the window: the pointer is cleared in WM_DESTROY, and the
    // window is destroyed (below at the latest) before `host` goes out of scope.
    unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, &raw const host as isize) };

    // A wake-up posted while `init` ran may have reached the window before it knew the
    // handler (then only the default procedure saw it, and the waker would stay "pending"
    // for good). Release it and deliver one wake-up: draining an empty source is harmless.
    host.shell.waker.acknowledge();
    host.dispatch(Event::Wake);

    // The handler may have quit on that first wake-up (window already destroyed).
    if !host.shell.quitting.get() && host.shell.has_tray {
        host.shell.add_icon();
    }

    let mut message = MSG::default();
    let outcome = loop {
        // SAFETY: `message` is a valid structure for the calls to fill and read.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        match status {
            0 => break Ok(()),
            -1 => {
                let error = ShellError::from(windows::core::Error::from_thread());
                crate::log::error(&format!("message loop failed: {error}"));
                break Err(error);
            }
            // SAFETY: as above.
            _ => unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    };

    // Normally already destroyed (that is what ended the loop); this covers the error path
    // so that no window is left pointing at `host`.
    // SAFETY: our own window, on its own thread; fails harmlessly if already destroyed.
    let _ = unsafe { DestroyWindow(window) };
    outcome
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: this is the message window, whose user data is 0 or the host (see `run`).
    let Some(host) = (unsafe { host_of(window) }) else {
        // SAFETY: forwarding the unmodified message to the default procedure.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    let shell = &host.shell;

    match message {
        WM_TRAY => {
            let notification = (lparam.0 & 0xFFFF) as u32;
            if notification == WM_MOUSEMOVE {
                shell.start_hover();
            }
            // SAFETY: plain query about the message being processed.
            let time = unsafe { GetMessageTime() } as u32;
            if let Some(event) = tray_event(wparam, lparam, time) {
                host.dispatch(event);
            }
            LRESULT(0)
        }
        WM_INPUT => {
            if let Some(notches) = shell.wheel_input(lparam) {
                host.dispatch(Event::TrayWheel { notches });
            }
            // SAFETY: the default procedure must see raw input to release its data.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_WAKE => {
            // Before the handler drains its sources, so that later news posts again.
            shell.waker.acknowledge();
            host.dispatch(Event::Wake);
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if let Some(event) = power_event(wparam) {
                host.dispatch(event);
            }
            // TRUE: the broadcast is accepted (the return value is otherwise ignored).
            LRESULT(1)
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
            shell.refresh_appearance();
            // SAFETY: forwarding the unmodified message to the default procedure.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_DESTROY => {
            // Also covers a close requested from outside (WM_CLOSE), not only `quit`.
            shell.quitting.set(true);
            shell.waker.close();
            shell.stop_hover();
            shell.stop_watching_foreground();
            shell.remove_icon();
            // Guard rail: the Windows volume icon comes back when BetterVolume quits.
            shell.restore_volume_icon_on_quit();
            shell.destroy_popups();
            // SAFETY: our own window; clearing the pointer stops any later message from
            // reaching `host`.
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        crate::flyout::WM_FLYOUT_CLOSED => {
            host.dispatch(Event::FlyoutClosed);
            LRESULT(0)
        }
        crate::flyout::WM_FOREGROUND_CHANGED => {
            shell.foreground_changed(HWND(wparam.0 as *mut core::ffi::c_void));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_ICON_RETRY => {
            shell.retry_add_icon();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_PROMOTE => {
            shell.try_promote();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_HOVER => {
            shell.check_hover();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == crate::taskbar::TIMER_VOLUME_ICON => {
            shell.volume_icon_timer();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == crate::update::TIMER_UPDATE => {
            shell.update_timer_fired();
            host.dispatch(Event::UpdateDue);
            LRESULT(0)
        }
        // A registration that failed returns 0, which must never match a real message.
        _ if shell.has_tray && shell.taskbar_created != 0 && message == shell.taskbar_created => {
            // Explorer (re)started, the taskbar was rebuilt, or the scaling changed: make
            // sure our icon is there, at the right size (NFR-05).
            shell.add_icon();
            // The Windows volume icon is hidden again in the new taskbar, unless the old
            // one crashed right after it was hidden (guard rail).
            if shell.volume_icon_taskbar_created() {
                host.dispatch(Event::VolumeIconIncident);
            }
            LRESULT(0)
        }
        _ if shell.volume_icon_status != 0 && message == shell.volume_icon_status => {
            if let Some(outcome) = shell.volume_icon_outcome(wparam, lparam) {
                host.dispatch(Event::VolumeIcon(outcome));
            }
            LRESULT(0)
        }
        _ if shell.exit_request != 0 && message == shell.exit_request => {
            host.dispatch(Event::ExitRequested);
            LRESULT(0)
        }
        _ if shell.activate != 0 && message == shell.activate => {
            host.dispatch(Event::ActivateRequested);
            LRESULT(0)
        }
        // SAFETY: forwarding the unmodified message to the default procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Window procedure of the popups (flyout, context menu).
extern "system" fn popup_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: a popup's user data is 0 (during creation) or the message window's handle
    // (set right after creation); that window's user data is 0 or the host.
    let host = unsafe {
        let owner = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut core::ffi::c_void);
        if owner.is_invalid() {
            None
        } else {
            host_of(owner)
        }
    };
    match message {
        // Content comes from composition only: nothing to paint the classic way.
        WM_PAINT => {
            // SAFETY: our own window; validates the whole client area.
            let _ = unsafe { ValidateRect(Some(window), None) };
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        // Popups size themselves for the monitor they open on.
        WM_DPICHANGED => return LRESULT(0),
        // Alt+F4 or a close request from another program: a popup is only ever hidden,
        // never destroyed (the default handling would destroy it for good).
        WM_CLOSE => {
            if let Some(host) = host {
                host.shell.close_popup(window);
            }
            return LRESULT(0);
        }
        _ => {}
    }
    if let Some(host) = host {
        let shell = &host.shell;
        let owner = shell.window;
        if message == WM_GETOBJECT {
            // Screen readers (FR-024).
            let source: Option<Rc<dyn crate::uia::AccessSource>> = if shell.is_menu_window(window) {
                Some(Rc::new(crate::menu::MenuAccess { owner }))
            } else if shell.is_flyout_window(window) {
                Some(Rc::new(crate::flyout::FlyoutAccess { owner }))
            } else {
                None
            };
            if let Some(result) =
                source.and_then(|s| crate::uia::on_get_object(window, wparam, lparam, s))
            {
                return result;
            }
        }
        let handled = if shell.is_menu_window(window) {
            shell.menu_message(message, wparam, lparam)
        } else if shell.is_flyout_window(window) {
            shell.flyout_message(message, wparam, lparam, &|event| host.dispatch(event))
        } else if shell.is_bubble_window(window) {
            shell.bubble_message(message, wparam, lparam)
        } else {
            None
        };
        if let Some(result) = handled {
            return result;
        }
    }
    // SAFETY: forwarding the unmodified message to the default procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Decodes a power broadcast: the resume that Windows always sends after sleep or
/// hibernation (the one sent on user input may never come), and the suspension.
fn power_event(wparam: WPARAM) -> Option<Event> {
    match wparam.0 as u32 {
        PBT_APMRESUMEAUTOMATIC => Some(Event::Resumed),
        PBT_APMSUSPEND => Some(Event::Suspending),
        _ => None,
    }
}

/// Decodes a tray callback (NOTIFYICON_VERSION_4 layout: the low word of `lparam` is the
/// notification, `wparam` holds the anchor position as two signed 16-bit values).
fn tray_event(wparam: WPARAM, lparam: LPARAM, time: u32) -> Option<Event> {
    let notification = (lparam.0 & 0xFFFF) as u32;
    match notification {
        NIN_SELECT => Some(Event::TrayPrimary(TrayActivation { time, keyboard: false })),
        NIN_KEYSELECT => Some(Event::TrayPrimary(TrayActivation { time, keyboard: true })),
        WM_MBUTTONUP => Some(Event::TrayMiddle),
        WM_CONTEXTMENU => Some(Event::TrayMenu {
            x: i32::from((wparam.0 & 0xFFFF) as u16 as i16),
            y: i32::from(((wparam.0 >> 16) & 0xFFFF) as u16 as i16),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Shell::NOTIFYICONDATAW;

    fn callback(notification: u32, x: i16, y: i16) -> (WPARAM, LPARAM) {
        let position = (x as u16 as usize) | ((y as u16 as usize) << 16);
        (
            WPARAM(position),
            LPARAM(((crate::tray::TRAY_ICON_ID as isize) << 16) | notification as isize),
        )
    }

    fn host<'a>(handler: &'a mut Handler<'a>) -> Host<'a> {
        Host {
            shell: Shell::new(HWND::default(), true),
            handler: RefCell::new(handler),
            pending: RefCell::new(VecDeque::new()),
        }
    }

    #[test]
    fn clicks_are_decoded() {
        let (w, l) = callback(NIN_SELECT, 10, 20);
        assert_eq!(
            tray_event(w, l, 7),
            Some(Event::TrayPrimary(TrayActivation { time: 7, keyboard: false }))
        );
        let (w, l) = callback(NIN_KEYSELECT, 10, 20);
        assert_eq!(
            tray_event(w, l, 9),
            Some(Event::TrayPrimary(TrayActivation { time: 9, keyboard: true }))
        );
        let (w, l) = callback(WM_MBUTTONUP, 10, 20);
        assert_eq!(tray_event(w, l, 0), Some(Event::TrayMiddle));
    }

    #[test]
    fn menu_position_keeps_negative_coordinates() {
        // Monitors left of / above the primary one have negative coordinates.
        let (w, l) = callback(WM_CONTEXTMENU, -1920, 1040);
        assert_eq!(tray_event(w, l, 0), Some(Event::TrayMenu { x: -1920, y: 1040 }));
        let (w, l) = callback(WM_CONTEXTMENU, 3000, -200);
        assert_eq!(tray_event(w, l, 0), Some(Event::TrayMenu { x: 3000, y: -200 }));
    }

    #[test]
    fn events_raised_while_the_handler_runs_are_queued_not_lost() {
        let log = RefCell::new(Vec::new());
        let depth = Cell::new(0u32);
        let host_ptr: Cell<*const Host> = Cell::new(std::ptr::null());
        let mut handler = |_: &Shell, event: Event| {
            depth.set(depth.get() + 1);
            assert_eq!(depth.get(), 1, "the handler must never be re-entered");
            log.borrow_mut().push(event.clone());
            if event == Event::TrayMiddle {
                // What a nested message loop (context menu) does: more events come in
                // while this call is still running.
                // SAFETY: the pointer is set below to a `Host` that outlives every call.
                let host = unsafe { &*host_ptr.get() };
                host.dispatch(Event::ActivateRequested);
                host.dispatch(Event::Resumed);
                assert_eq!(log.borrow().len(), 1, "queued events wait for the handler to return");
            }
            depth.set(depth.get() - 1);
        };
        let host = host(&mut handler);
        host_ptr.set(&raw const host);

        host.dispatch(Event::TrayMiddle);

        // Once quitting, nothing reaches the handler any more.
        host.shell.quitting.set(true);
        host.dispatch(Event::TrayMiddle);
        assert!(host.pending.borrow().is_empty());
        drop(host);
        assert_eq!(
            log.into_inner(),
            vec![Event::TrayMiddle, Event::ActivateRequested, Event::Resumed]
        );
    }

    #[test]
    fn wake_ups_queued_while_the_handler_runs_are_delivered_once() {
        let log = RefCell::new(Vec::new());
        let host_ptr: Cell<*const Host> = Cell::new(std::ptr::null());
        let mut handler = |_: &Shell, event: Event| {
            log.borrow_mut().push(event.clone());
            if event == Event::TrayMiddle {
                // SAFETY: the pointer is set below to a `Host` that outlives every call.
                let host = unsafe { &*host_ptr.get() };
                for _ in 0..5 {
                    host.dispatch(Event::Wake);
                }
                host.dispatch(Event::Resumed);
                host.dispatch(Event::Wake);
            }
        };
        let host = host(&mut handler);
        host_ptr.set(&raw const host);
        host.dispatch(Event::TrayMiddle);
        drop(host);
        assert_eq!(log.into_inner(), vec![Event::TrayMiddle, Event::Wake, Event::Resumed]);
    }

    #[test]
    fn resume_is_reported_once() {
        use windows::Win32::UI::WindowsAndMessaging::PBT_APMRESUMESUSPEND;
        assert_eq!(power_event(WPARAM(PBT_APMRESUMEAUTOMATIC as usize)), Some(Event::Resumed));
        assert_eq!(power_event(WPARAM(PBT_APMRESUMESUSPEND as usize)), None);
        assert_eq!(power_event(WPARAM(PBT_APMSUSPEND as usize)), Some(Event::Suspending));
    }

    #[test]
    fn the_tooltip_limit_matches_the_windows_buffer() {
        assert_eq!(crate::TOOLTIP_MAX_UNITS, NOTIFYICONDATAW::default().szTip.len() - 1);
    }

    #[test]
    fn other_notifications_are_ignored() {
        use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_RBUTTONUP};
        for notification in [WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_RBUTTONUP] {
            let (w, l) = callback(notification, 1, 1);
            assert_eq!(tray_event(w, l, 0), None);
        }
    }
}
