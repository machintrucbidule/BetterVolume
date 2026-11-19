//! The settings window host (FR-070): a standard, resizable top-level window with the
//! Windows 11 "Mica" material, drawn by the design system's `SettingsView`.
//!
//! The window exists only while it is open (lazy, ARCHITECTURE §3): closing it destroys
//! it and frees its drawing surface. While open it has its taskbar button; its
//! last place is reported to the application when it closes.

use std::path::{Path, PathBuf};

use bv_ui_kit::access::AccessAction;
use bv_ui_kit::backend::{Graphics, WindowSurface};
use bv_ui_kit::controls::flyout::{GraphicsMeasure, TextMeasure};
use bv_ui_kit::controls::settings::{
    Block, Response, RowPicture, SettingsAction, SettingsContent, SettingsKey, SettingsView,
};
use bv_ui_kit::controls::tray::{self, TrayIcon};
use bv_ui_kit::icons::VolumeLevel;
use bv_ui_kit::tokens::settings as t;
use bv_ui_kit::{Point, Size, Theme, ThemeMode};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_MAINWINDOW, DWMSBT_NONE,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Gdi::{
    MonitorFromPoint, MonitorFromRect, ScreenToClient, ValidateRect, MONITOR_DEFAULTTONULL,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_BACK,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F4, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetWindowLongPtrW,
    GetWindowPlacement, IsIconic, IsWindow, LoadCursorW, RegisterClassW, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos, SetWindowTextW,
    ShowWindow, CW_USEDEFAULT, GWLP_USERDATA, ICON_BIG, ICON_SMALL, IDC_ARROW, MINMAXINFO,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWNORMAL,
    WA_INACTIVE, WINDOWPLACEMENT, WM_ACTIVATE, WM_CAPTURECHANGED, WM_CHAR, WM_CLOSE, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_GETOBJECT, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SETICON, WM_SIZE,
    WM_SYSKEYDOWN, WNDCLASSW, WPF_RESTORETOMAXIMIZED, WS_EX_APPWINDOW, WS_EX_NOREDIRECTIONBITMAP,
    WS_OVERLAPPEDWINDOW,
};

use crate::appicon::IconCache;
use crate::host::{host_of, Event, Host, Shell};
use crate::trayicon::{self, OwnedIcon};
use crate::uia::{self, AccessSource};

/// Class of the settings window. Also how tools find it.
pub(crate) const SETTINGS_CLASS: PCWSTR = w!("BetterVolume.Settings");

/// `WHEEL_DELTA` of `winuser.h`: one notch of a standard wheel.
const WHEEL_DELTA: i32 = 120;

/// Where a window is: its restored (not maximised) rectangle in Windows workspace
/// coordinates (physical pixels), and whether it is maximised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowPlace {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Maximised.
    pub maximized: bool,
}

/// The open settings window.
pub(crate) struct Open {
    window: HWND,
    surface: Option<WindowSurface>,
    view: SettingsView,
    scale: f32,
    tracking_leave: bool,
    /// First half of a character outside the basic plane typed in two `WM_CHAR`s.
    surrogate: Option<u16>,
    /// The window was given its first place (and shown).
    placed: bool,
    wheel_rest: i32,
    _icons: Vec<OwnedIcon>,
    /// Application icons of the rows (FR-096), dropped with the window.
    app_icons: IconCache,
}

/// State of the settings window, kept by the shell.
#[derive(Default)]
pub(crate) struct SettingsState {
    pub(crate) open: Option<Open>,
}

/// Registers the window class (once per process).
pub(crate) fn register_class() -> windows::core::Result<()> {
    // SAFETY: a null module name returns the handle of the current executable.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
    let class = WNDCLASSW {
        lpfnWndProc: Some(settings_proc),
        hInstance: instance,
        // SAFETY: loads a shared system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        lpszClassName: SETTINGS_CLASS,
        ..Default::default()
    };
    // SAFETY: `class` is fully initialised and its strings are constants.
    unsafe { RegisterClassW(&class) };
    Ok(())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// # Safety
/// `window` must be a live window of this thread.
unsafe fn set_attribute<T>(window: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) -> bool {
    // SAFETY: per the contract; `value` is a plain value of the documented type.
    unsafe {
        DwmSetWindowAttribute(
            window,
            attribute,
            std::ptr::from_ref(value).cast(),
            size_of::<T>() as u32,
        )
    }
    .is_ok()
}

impl Shell {
    /// The theme of application windows for the current Windows appearance.
    pub(crate) fn app_theme(&self) -> Theme {
        Theme::app(&self.appearance.get())
    }

    /// Whether the settings window is open.
    pub fn settings_visible(&self) -> bool {
        self.settings_window().is_some()
    }

    fn settings_window(&self) -> Option<HWND> {
        let state = self.settings_state.try_borrow().ok()?;
        let window = state.open.as_ref()?.window;
        // SAFETY: plain query on a handle; a stale one answers false.
        unsafe { IsWindow(Some(window)) }.as_bool().then_some(window)
    }

    /// Opens the window of the installer or of the uninstaller: the settings
    /// window at its compact size, centred on the primary screen, with `content` (no
    /// navigation pane). Brings it to the front if it is already open.
    pub fn show_compact_window(&self, content: SettingsContent) {
        self.show_settings(content, Some(default_place(t::COMPACT_SIZE)));
    }

    /// Asks the user for a folder with Windows' folder picker, owned by the open
    /// settings window, starting at `initial` (or its nearest existing parent). `None`
    /// when cancelled or when the picker cannot be shown. Runs a modal loop: call it from
    /// a wake-up, not while the window is handling input.
    pub fn choose_folder(&self, initial: &Path, title: &str) -> Option<PathBuf> {
        use windows::core::HSTRING;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
            FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
        };
        let owner = self.settings_window();
        // SAFETY: plain call; balanced below when it succeeded.
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
        let start = initial.ancestors().find(|p| p.is_dir()).map(Path::to_owned);
        // SAFETY: COM is usable on this thread (initialised above, or already in another
        // mode, which the picker supports); every string outlives its call; the returned
        // path is freed once.
        let chosen = unsafe {
            (|| -> windows::core::Result<PathBuf> {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                let options = dialog.GetOptions()?;
                dialog.SetOptions(
                    options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST,
                )?;
                dialog.SetTitle(&HSTRING::from(title))?;
                if let Some(start) = &start {
                    if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(
                        &HSTRING::from(start.as_os_str()),
                        None,
                    ) {
                        let _ = dialog.SetFolder(&item);
                    }
                }
                dialog.Show(owner)?;
                let item = dialog.GetResult()?;
                let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
                let text = name.to_string();
                CoTaskMemFree(Some(name.0 as *const _));
                Ok(PathBuf::from(text?))
            })()
        };
        if com {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
        chosen.ok()
    }

    /// Opens the settings window with `content`, at `place` when it is still on a screen,
    /// or brings it to the front if it is already open.
    pub fn show_settings(&self, content: SettingsContent, place: Option<WindowPlace>) {
        if let Some(window) = self.settings_window() {
            self.update_settings(content);
            // SAFETY: our own window.
            unsafe {
                if IsIconic(window).as_bool() {
                    let _ = ShowWindow(window, SW_RESTORE);
                }
                let _ = SetForegroundWindow(window);
            }
            return;
        }
        if let Err(error) = self.create_settings(content, place) {
            crate::log::error(&format!(
                "settings window not created: {}",
                crate::ShellError::from(error)
            ));
        }
    }

    fn create_settings(
        &self,
        content: SettingsContent,
        place: Option<WindowPlace>,
    ) -> windows::core::Result<()> {
        // SAFETY: a null module name returns the handle of the current executable.
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }?.into();
        let title = wide(&content.name);
        // Created hidden at a default place, then placed for its monitor's scaling.
        // SAFETY: the class is registered at start; the title buffer outlives the call.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
                SETTINGS_CLASS,
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        // SAFETY: our own window; the value is the message window's handle, read back by
        // the window procedure only.
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, self.window.0 as isize) };

        let theme = self.app_theme();
        let icons = self.window_icons(window);
        // SAFETY: our own window, just created on this thread.
        let scale = unsafe { GetDpiForWindow(window) }.max(96) as f32 / 96.0;
        let measure_graphics = self.graphics();
        let mut app_icons = IconCache::with_limit(APP_ICONS_LIMIT);
        let view = match &measure_graphics {
            Some(graphics) => {
                let measure = GraphicsMeasure { graphics, text_scale: theme.text_scale };
                let (w, h) = t::DEFAULT_SIZE;
                let content = resolve_icons(content, &mut app_icons, scale, graphics);
                SettingsView::new(content, Size::new(w, h), &measure, theme.text_scale)
            }
            None => {
                // SAFETY: our own window.
                let _ = unsafe { DestroyWindow(window) };
                return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
            }
        };
        self.settings_state.borrow_mut().open = Some(Open {
            window,
            surface: None,
            view,
            scale,
            tracking_leave: false,
            surrogate: None,
            placed: false,
            wheel_rest: 0,
            _icons: icons,
            app_icons,
        });
        self.apply_window_theme(window, &theme);

        // Placement: the remembered one if it is still on a screen, else centred on the
        // primary monitor at the default size for that monitor's scaling.
        let placement = place.filter(on_a_screen).unwrap_or_else(|| default_place(t::DEFAULT_SIZE));
        let wp = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            showCmd: if placement.maximized {
                SW_SHOWMAXIMIZED.0 as u32
            } else {
                SW_SHOWNORMAL.0 as u32
            },
            rcNormalPosition: RECT {
                left: placement.x,
                top: placement.y,
                right: placement.x + placement.width,
                bottom: placement.y + placement.height,
            },
            ..Default::default()
        };
        // Shows the window. It sends WM_DPICHANGED / WM_SIZE synchronously (no borrow held).
        // SAFETY: our own window and a valid structure.
        unsafe {
            if SetWindowPlacement(window, &wp).is_err() {
                let _ = ShowWindow(window, SW_SHOWNORMAL);
            }
            let _ = SetForegroundWindow(window);
        }
        if let Some(open) = self.settings_state.borrow_mut().open.as_mut() {
            open.placed = true;
        }
        self.paint_settings();
        Ok(())
    }

    /// Small and large window icons: the application icon of the executable, or
    /// the volume glyph drawn by the design system when it has none (test builds).
    fn window_icons(&self, window: HWND) -> Vec<OwnedIcon> {
        let resource = application_icons(window);
        if !resource.is_empty() {
            return resource;
        }
        let theme = self.theme();
        let icon = TrayIcon { level: Some(VolumeLevel::High), badge: None, large: false };
        let mut icons = Vec::new();
        for (kind, pixels) in [(ICON_SMALL, 16_u32), (ICON_BIG, 32)] {
            let drawn = self.with_image_graphics(|graphics| {
                graphics.rasterize(pixels, pixels, tray::scale_for(pixels), 1.0, |c| {
                    tray::paint(c, &icon, &theme)
                })
            });
            if let Some(Ok(image)) = drawn {
                if let Some(owned) = trayicon::create(&image) {
                    // SAFETY: our own window; the icon outlives the window (kept with it).
                    unsafe {
                        SendMessageW(
                            window,
                            WM_SETICON,
                            Some(WPARAM(kind as usize)),
                            Some(LPARAM(owned.0 .0 as isize)),
                        )
                    };
                    icons.push(owned);
                }
            }
        }
        icons
    }

    fn apply_window_theme(&self, window: HWND, theme: &Theme) {
        let dark = BOOL::from(theme.mode == ThemeMode::Dark);
        let backdrop = if theme.window.mica { DWMSBT_MAINWINDOW } else { DWMSBT_NONE };
        // The material shows through the whole client area (-1: the whole window).
        let margins = if theme.window.mica {
            MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }
        } else {
            MARGINS::default()
        };
        // SAFETY: our own window; values of the documented types.
        unsafe {
            set_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark);
            set_attribute(window, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop);
            let _ = DwmExtendFrameIntoClientArea(window, &margins);
        }
    }

    /// Shows new content in the open settings window (nothing if it is closed).
    pub fn update_settings(&self, content: SettingsContent) {
        // Nothing (not even graphics objects) for a closed window.
        if self.settings_window().is_none() {
            return;
        }
        let Some(graphics) = self.graphics() else { return };
        let title = {
            let Ok(mut state) = self.settings_state.try_borrow_mut() else { return };
            let Some(open) = state.open.as_mut() else { return };
            let text_scale = self.app_theme().text_scale;
            let measure = GraphicsMeasure { graphics: &graphics, text_scale };
            let content = resolve_icons(content, &mut open.app_icons, open.scale, &graphics);
            // Unchanged content (an audio change elsewhere): nothing to lay out or announce.
            if *open.view.content() == content {
                return;
            }
            let renamed = open.view.content().name != content.name;
            let name = content.name.clone();
            open.view.set_content(content, &measure);
            renamed.then_some((open.window, name))
        };
        if let Some((window, name)) = title {
            let text = wide(&name);
            // SAFETY: our own window; the buffer outlives the call (sends WM_SETTEXT).
            let _ = unsafe { SetWindowTextW(window, PCWSTR(text.as_ptr())) };
        }
        self.paint_settings();
        if let Some(window) = self.settings_window() {
            uia::structure_changed(window);
        }
    }

    /// Where the settings window is now, if it is open.
    pub fn settings_place(&self) -> Option<WindowPlace> {
        self.settings_window().and_then(place_of)
    }

    /// Closes the settings window, if it is open (its place is reported by
    /// [`Event::SettingsClosed`]).
    pub fn close_settings(&self) {
        if let Some(window) = self.settings_window() {
            // SAFETY: our own window; no borrow is held (it sends WM_DESTROY to us).
            let _ = unsafe { DestroyWindow(window) };
        }
    }

    /// The Windows appearance changed: the open window follows it.
    pub(crate) fn restyle_settings(&self) {
        let Some(window) = self.settings_window() else { return };
        let theme = self.app_theme();
        self.apply_window_theme(window, &theme);
        if let Some(graphics) = self.graphics() {
            if let Ok(mut state) = self.settings_state.try_borrow_mut() {
                if let Some(open) = state.open.as_mut() {
                    let measure =
                        GraphicsMeasure { graphics: &graphics, text_scale: theme.text_scale };
                    let size = open.view.size();
                    open.view.resize(size, theme.text_scale, &measure);
                }
            }
        }
        self.paint_settings();
    }

    /// Drops the drawing surface (graphics objects are being rebuilt).
    pub(crate) fn drop_settings_surface(&self) {
        let old = self
            .settings_state
            .try_borrow_mut()
            .ok()
            .and_then(|mut s| s.open.as_mut().and_then(|o| o.surface.take()));
        drop(old);
    }

    /// Paints the open window at its client size. A drawing failure rebuilds the graphics
    /// objects and tries once more.
    fn paint_settings(&self) {
        for attempt in 0..2 {
            let Some(graphics) = self.graphics() else { return };
            let theme = self.app_theme();
            let result = {
                let Ok(mut state) = self.settings_state.try_borrow_mut() else { return };
                let Some(open) = state.open.as_mut() else { return };
                paint_open(open, &graphics, &theme)
            };
            match result {
                Ok(()) => return,
                Err(error) => {
                    crate::log::error(&format!(
                        "settings not drawn (attempt {}): {error}",
                        attempt + 1
                    ));
                    self.reset_graphics();
                }
            }
        }
    }

    /// Handles a message of the settings window. `None`: not handled.
    fn settings_message(
        &self,
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        dispatch: &dyn Fn(Event),
    ) -> Option<LRESULT> {
        let point = |lparam: LPARAM| {
            let x = i32::from((lparam.0 & 0xFFFF) as u16 as i16);
            let y = i32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16);
            (x, y)
        };
        let to_dip = |(x, y): (i32, i32)| {
            let scale = self
                .settings_state
                .try_borrow()
                .ok()
                .and_then(|s| s.open.as_ref().map(|o| o.scale));
            let scale = scale.unwrap_or(1.0).max(0.1);
            Point::new(x as f32 / scale, y as f32 / scale)
        };
        let response = match message {
            WM_SIZE => {
                // Minimised: nothing to paint.
                if wparam.0 == 1 {
                    return Some(LRESULT(0));
                }
                self.layout_to_client(window);
                self.paint_settings();
                return Some(LRESULT(0));
            }
            WM_DPICHANGED => {
                let dpi = (wparam.0 & 0xFFFF) as u32;
                if let Ok(mut state) = self.settings_state.try_borrow_mut() {
                    if let Some(open) = state.open.as_mut() {
                        // The surface keeps its composition target and is resized at
                        // the new scale by the WM_SIZE below.
                        open.scale = dpi.max(96) as f32 / 96.0;
                        // Application icons at the new size.
                        if let Some(graphics) = self.graphics() {
                            let text_scale = self.app_theme().text_scale;
                            let measure = GraphicsMeasure { graphics: &graphics, text_scale };
                            let content = open.view.content().clone();
                            let content =
                                resolve_icons(content, &mut open.app_icons, open.scale, &graphics);
                            if *open.view.content() != content {
                                open.view.set_content(content, &measure);
                            }
                        }
                        // Being placed on a monitor of another scaling: the remembered
                        // place (physical pixels) wins over Windows' suggestion.
                        if !open.placed {
                            drop(state);
                            self.layout_to_client(window);
                            self.paint_settings();
                            return Some(LRESULT(0));
                        }
                    }
                }
                // SAFETY: Windows passes the suggested rectangle in `lparam`.
                let suggested = unsafe { *(lparam.0 as *const RECT) };
                // SAFETY: our own window; sends WM_SIZE (no borrow held).
                let _ = unsafe {
                    SetWindowPos(
                        window,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
                self.layout_to_client(window);
                self.paint_settings();
                return Some(LRESULT(0));
            }
            WM_GETMINMAXINFO => {
                // SAFETY: our own window; plain query.
                let dpi = unsafe { GetDpiForWindow(window) }.max(96);
                let scale = dpi as f32 / 96.0;
                let (w, h) = t::MIN_SIZE;
                let mut rect =
                    RECT { left: 0, top: 0, right: (w * scale) as i32, bottom: (h * scale) as i32 };
                // SAFETY: valid rectangle; plain computation.
                let _ = unsafe {
                    AdjustWindowRectExForDpi(
                        &mut rect,
                        WS_OVERLAPPEDWINDOW,
                        false,
                        WS_EX_APPWINDOW,
                        dpi,
                    )
                };
                // SAFETY: Windows passes a valid MINMAXINFO in `lparam`.
                let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                info.ptMinTrackSize =
                    POINT { x: rect.right - rect.left, y: rect.bottom - rect.top };
                return Some(LRESULT(0));
            }
            WM_ACTIVATE => {
                if (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE {
                    dispatch(Event::SettingsActivated);
                }
                return None;
            }
            WM_KILLFOCUS => self.with_settings(|view, _| view.focus_lost())?,
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let alt = message == WM_SYSKEYDOWN;
                let editing = self.settings_editing();
                match settings_input(wparam.0 as u16, modifiers(), alt, editing)? {
                    Input::Key(key) => self.with_settings(|view, m| view.key(key, m))?,
                    Input::Paste => {
                        let text = crate::clipboard::text(window)?;
                        self.with_settings(|view, m| view.paste(&text, m))?
                    }
                }
            }
            WM_CHAR => {
                // Typed characters (keyboard or input method) go to a rename in progress.
                if !self.settings_editing() {
                    if let Ok(mut state) = self.settings_state.try_borrow_mut() {
                        if let Some(open) = state.open.as_mut() {
                            open.surrogate = None;
                        }
                    }
                    return None;
                }
                let unit = wparam.0 as u16;
                let text = {
                    let mut state = self.settings_state.try_borrow_mut().ok()?;
                    let open = state.open.as_mut()?;
                    match (open.surrogate.take(), unit) {
                        (None, 0xD800..=0xDBFF) => {
                            open.surrogate = Some(unit);
                            return Some(LRESULT(0));
                        }
                        (Some(high), 0xDC00..=0xDFFF) => String::from_utf16_lossy(&[high, unit]),
                        // Half of a pair without its other half: nothing to type.
                        (_, 0xD800..=0xDFFF) => return Some(LRESULT(0)),
                        (_, unit) => String::from_utf16_lossy(&[unit]),
                    }
                };
                if text.chars().all(char::is_control) {
                    return Some(LRESULT(0));
                }
                self.with_settings(|view, m| view.text_input(&text, m))?
            }
            WM_MOUSEMOVE => {
                self.track_settings_leave(window);
                let p = to_dip(point(lparam));
                self.with_settings(|view, m| view.pointer_move(p, m))?
            }
            WM_MOUSELEAVE => {
                if let Ok(mut state) = self.settings_state.try_borrow_mut() {
                    if let Some(open) = state.open.as_mut() {
                        open.tracking_leave = false;
                    }
                }
                self.with_settings(|view, _| view.pointer_leave())?
            }
            WM_LBUTTONDOWN => {
                let p = to_dip(point(lparam));
                self.with_settings(|view, m| view.press(p, m))?
            }
            WM_LBUTTONUP => {
                let p = to_dip(point(lparam));
                self.with_settings(|view, m| view.release(p, m))?
            }
            WM_CAPTURECHANGED => {
                if HWND(lparam.0 as *mut core::ffi::c_void) == window {
                    return Some(LRESULT(0));
                }
                self.with_settings(|view, _| view.cancel_pointer())?
            }
            WM_MOUSEWHEEL => {
                let delta = i32::from((wparam.0 >> 16) as u16 as i16);
                let (x, y) = point(lparam);
                let mut screen = POINT { x, y };
                // SAFETY: our own window and a valid point.
                let _ = unsafe { ScreenToClient(window, &mut screen) };
                let p = to_dip((screen.x, screen.y));
                let notches = {
                    let mut state = self.settings_state.try_borrow_mut().ok()?;
                    let open = state.open.as_mut()?;
                    let total = open.wheel_rest + delta;
                    open.wheel_rest = total % WHEEL_DELTA;
                    total / WHEEL_DELTA
                };
                if notches == 0 {
                    return Some(LRESULT(0));
                }
                self.with_settings(|view, m| view.wheel(p, notches, m))?
            }
            _ => return None,
        };
        self.apply_settings_response(window, response, dispatch);
        Some(LRESULT(0))
    }

    /// Lays the view out at the window's client size.
    fn layout_to_client(&self, window: HWND) {
        let mut rect = RECT::default();
        // SAFETY: our own window and a valid rectangle.
        if unsafe { GetClientRect(window, &mut rect) }.is_err() {
            return;
        }
        let Some(graphics) = self.graphics() else { return };
        let theme = self.app_theme();
        let Ok(mut state) = self.settings_state.try_borrow_mut() else { return };
        let Some(open) = state.open.as_mut() else { return };
        let size = Size::new(
            (rect.right - rect.left).max(1) as f32 / open.scale,
            (rect.bottom - rect.top).max(1) as f32 / open.scale,
        );
        let measure = GraphicsMeasure { graphics: &graphics, text_scale: theme.text_scale };
        open.view.resize(size, theme.text_scale, &measure);
    }

    /// Runs `f` on the view with a text measure, if the view exists and nothing else uses
    /// it right now.
    fn with_settings(
        &self,
        f: impl FnOnce(&mut SettingsView, &dyn TextMeasure) -> Response,
    ) -> Option<Response> {
        let graphics = self.graphics()?;
        let measure =
            GraphicsMeasure { graphics: &graphics, text_scale: self.app_theme().text_scale };
        let mut state = self.settings_state.try_borrow_mut().ok()?;
        state.open.as_mut().map(|open| f(&mut open.view, &measure))
    }

    /// Whether a device is being renamed in the settings window.
    fn settings_editing(&self) -> bool {
        self.settings_state
            .try_borrow()
            .ok()
            .is_some_and(|s| s.open.as_ref().is_some_and(|o| o.view.editing()))
    }

    fn track_settings_leave(&self, window: HWND) {
        let Ok(mut state) = self.settings_state.try_borrow_mut() else { return };
        let Some(open) = state.open.as_mut() else { return };
        if open.tracking_leave {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: window,
            dwHoverTime: 0,
        };
        // SAFETY: valid structure for our own window.
        open.tracking_leave = unsafe { TrackMouseEvent(&mut track) }.is_ok();
    }

    /// Carries out what the view asked for after an input.
    fn apply_settings_response(&self, window: HWND, response: Response, dispatch: &dyn Fn(Event)) {
        match response.capture {
            // SAFETY: our own window; the capture ends on release.
            Some(true) => unsafe {
                SetCapture(window);
            },
            // SAFETY: plain call.
            Some(false) => unsafe {
                let _ = ReleaseCapture();
            },
            None => {}
        }
        if response.redraw {
            self.paint_settings();
        }
        if response.focus_moved {
            let focused = self
                .settings_state
                .try_borrow()
                .ok()
                .and_then(|s| s.open.as_ref().and_then(|o| o.view.focused_access_id()));
            uia::focus_changed(window, focused);
        }
        if let Some(text) = &response.clipboard {
            crate::clipboard::set_text(window, text);
        }
        if let Some(action) = response.action {
            let moved = matches!(action, SettingsAction::MoveDevice(..));
            dispatch(Event::Settings(action));
            if moved {
                // The row moved under the focus: announce it again at its new place.
                let focused = self
                    .settings_state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.open.as_ref().and_then(|o| o.view.focused_access_id()));
                uia::focus_changed(window, focused);
            }
        }
    }

    /// The window is being destroyed: report where it was and forget it.
    fn settings_destroyed(&self, window: HWND, dispatch: &dyn Fn(Event)) {
        let place = place_of(window);
        uia::disconnect(window);
        let old = self.settings_state.try_borrow_mut().ok().and_then(|mut s| s.open.take());
        // A window destroyed before it was ever placed (creation failed) has no place to
        // remember, and was never open.
        let shown = old.as_ref().is_some_and(|o| o.placed);
        drop(old);
        if shown {
            dispatch(Event::SettingsClosed(place));
        }
    }
}

/// Paints the view into the window's surface, sizing it to the view.
fn paint_open(
    open: &mut Open,
    graphics: &Graphics,
    theme: &Theme,
) -> Result<(), bv_ui_kit::backend::GraphicsError> {
    let (w, h) = open.view.size().to_pixels(open.scale);
    match open.surface.as_mut() {
        Some(surface) => surface.resize(graphics, w, h, open.scale)?,
        None => {
            open.surface =
                Some(WindowSurface::new(graphics, open.window.0 as isize, w, h, open.scale)?)
        }
    }
    let view = &open.view;
    let measure = GraphicsMeasure { graphics, text_scale: theme.text_scale };
    match open.surface.as_mut() {
        Some(surface) => {
            surface.render(graphics, theme.text_scale, |canvas| view.paint(canvas, theme, &measure))
        }
        None => Ok(()),
    }
}

/// The window's current place.
fn place_of(window: HWND) -> Option<WindowPlace> {
    let mut wp =
        WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    // SAFETY: our own window and a valid structure.
    unsafe { GetWindowPlacement(window, &mut wp) }.ok()?;
    let r = wp.rcNormalPosition;
    // Minimised from the maximised state: it comes back maximised.
    let maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32
        || (wp.showCmd == SW_SHOWMINIMIZED.0 as u32 && wp.flags.contains(WPF_RESTORETOMAXIMIZED));
    Some(WindowPlace {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
        maximized,
    })
}

/// Whether a remembered place still shows on a monitor (its title bar at least).
fn on_a_screen(place: &WindowPlace) -> bool {
    if place.width <= 0 || place.height <= 0 {
        return false;
    }
    let top_band =
        RECT { left: place.x, top: place.y, right: place.x + place.width, bottom: place.y + 32 };
    // SAFETY: plain query with a valid rectangle.
    !unsafe { MonitorFromRect(&top_band, MONITOR_DEFAULTTONULL) }.is_invalid()
}

/// The default place: the default size for the primary monitor's scaling, centred on it.
/// Resource number of the application icon in the executable (`bv-app/build.rs`).
const APP_ICON_RESOURCE: usize = 1;

/// The application icon of the executable at the small and large window-icon sizes of
/// `window`'s scaling, set on the window. Empty when the executable has none.
fn application_icons(window: HWND) -> Vec<OwnedIcon> {
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON,
    };
    // SAFETY: a null module name returns the handle of the current executable.
    let Ok(module) = (unsafe { GetModuleHandleW(None) }) else { return Vec::new() };
    // SAFETY: our own window; plain query.
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let mut icons = Vec::new();
    for (kind, metric) in [(ICON_SMALL, SM_CXSMICON), (ICON_BIG, SM_CXICON)] {
        // SAFETY: plain query.
        let pixels = unsafe { windows::Win32::UI::HiDpi::GetSystemMetricsForDpi(metric, dpi) };
        // SAFETY: a resource number passed as the name (MAKEINTRESOURCE); the icon is
        // owned (not shared) and destroyed by `OwnedIcon`.
        let loaded = unsafe {
            LoadImageW(
                Some(module.into()),
                PCWSTR(APP_ICON_RESOURCE as *const u16),
                IMAGE_ICON,
                pixels,
                pixels,
                LR_DEFAULTCOLOR,
            )
        };
        let Ok(handle) = loaded else { return Vec::new() };
        let icon = OwnedIcon(HICON(handle.0));
        // SAFETY: our own window; the icon outlives the window (kept with it).
        unsafe {
            SendMessageW(
                window,
                WM_SETICON,
                Some(WPARAM(kind as usize)),
                Some(LPARAM(icon.0 .0 as isize)),
            )
        };
        icons.push(icon);
    }
    icons
}

fn default_place((w, h): (f32, f32)) -> WindowPlace {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    // SAFETY: plain queries; a failure leaves the structure zeroed.
    let (work, dpi) = unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info =
            MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let mut dpi_x = 96;
        let mut dpi_y = 96;
        let _ = windows::Win32::UI::HiDpi::GetDpiForMonitor(
            monitor,
            windows::Win32::UI::HiDpi::MDT_EFFECTIVE_DPI,
            &mut dpi_x,
            &mut dpi_y,
        );
        (info.rcWork, dpi_x.max(96))
    };
    let scale = dpi as f32 / 96.0;
    let mut rect = RECT { left: 0, top: 0, right: (w * scale) as i32, bottom: (h * scale) as i32 };
    // SAFETY: valid rectangle; plain computation.
    let _ = unsafe {
        AdjustWindowRectExForDpi(&mut rect, WS_OVERLAPPEDWINDOW, false, WS_EX_APPWINDOW, dpi)
    };
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    let work_w = work.right - work.left;
    let work_h = work.bottom - work.top;
    let (width, height) = if work_w > 0 && work_h > 0 {
        (width.min(work_w), height.min(work_h))
    } else {
        (width, height)
    };
    WindowPlace {
        x: work.left + (work_w - width).max(0) / 2,
        y: work.top + (work_h - height).max(0) / 2,
        width,
        height,
        maximized: false,
    }
}

/// Modifier keys held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Modifiers {
    shift: bool,
    ctrl: bool,
    /// Alt held (with Ctrl, this is AltGr: a character, not a shortcut).
    alt: bool,
}

fn modifiers() -> Modifiers {
    // SAFETY: plain queries of the keyboard state of this thread.
    unsafe {
        Modifiers {
            shift: GetKeyState(i32::from(VK_SHIFT.0)) < 0,
            ctrl: GetKeyState(i32::from(VK_CONTROL.0)) < 0,
            alt: GetKeyState(i32::from(VK_MENU.0)) < 0,
        }
    }
}

/// What a key asks of the settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Input {
    Key(SettingsKey),
    /// Ctrl+V: paste the clipboard text into the rename in progress.
    Paste,
}

/// The input for a virtual key (`alt`: held with Alt; `editing`: a rename is in
/// progress, so Ctrl+letters edit text), if the window reacts to it.
fn settings_input(key: u16, held: Modifiers, alt: bool, editing: bool) -> Option<Input> {
    if alt {
        // Only Alt+Up and Alt+Down (move a row, open a list); other Alt combinations
        // (Alt+F4, the window menu) are Windows'.
        return match key {
            k if k == VK_UP.0 => Some(Input::Key(SettingsKey::AltUp)),
            k if k == VK_DOWN.0 => Some(Input::Key(SettingsKey::AltDown)),
            _ => None,
        };
    }
    let select = held.shift;
    if held.ctrl && held.alt {
        // AltGr (Ctrl+Alt) types characters on many layouts: leave it to WM_CHAR.
        return None;
    }
    if held.ctrl {
        return match key {
            0x41 if editing => Some(Input::Key(SettingsKey::SelectAll)),
            0x43 if editing => Some(Input::Key(SettingsKey::Copy)),
            0x58 if editing => Some(Input::Key(SettingsKey::Cut)),
            0x56 if editing => Some(Input::Paste),
            k if k == VK_TAB.0 => None,
            _ => None,
        };
    }
    Some(Input::Key(match key {
        k if k == VK_TAB.0 => SettingsKey::Tab { back: held.shift },
        k if k == VK_LEFT.0 => SettingsKey::Left { select },
        k if k == VK_RIGHT.0 => SettingsKey::Right { select },
        k if k == VK_UP.0 => SettingsKey::Up,
        k if k == VK_DOWN.0 => SettingsKey::Down,
        k if k == VK_PRIOR.0 => SettingsKey::PageUp,
        k if k == VK_NEXT.0 => SettingsKey::PageDown,
        k if k == VK_HOME.0 => SettingsKey::Home { select },
        k if k == VK_END.0 => SettingsKey::End { select },
        k if k == VK_RETURN.0 => SettingsKey::Enter,
        k if k == VK_SPACE.0 => SettingsKey::Space,
        k if k == VK_ESCAPE.0 => SettingsKey::Escape,
        k if k == VK_F4.0 => SettingsKey::Open,
        k if k == VK_BACK.0 => SettingsKey::Backspace,
        k if k == VK_DELETE.0 => SettingsKey::Delete,
        _ => return None,
    }))
}

/// Window procedure of the settings window.
extern "system" fn settings_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Content comes from composition only: nothing to paint the classic way.
        WM_PAINT => {
            // SAFETY: our own window; validates the whole client area.
            let _ = unsafe { ValidateRect(Some(window), None) };
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        _ => {}
    }
    // SAFETY: the window's user data is 0 (during creation) or the message window's
    // handle; that window's user data is 0 or the host.
    let host: Option<&Host<'_>> = unsafe {
        let owner = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut core::ffi::c_void);
        if owner.is_invalid() {
            None
        } else {
            host_of(owner)
        }
    };
    if let Some(host) = host {
        let shell = &host.shell;
        let dispatch = |event| host.dispatch(event);
        match message {
            WM_CLOSE => {
                // Unsaved changes: the close question opens and the window stays;
                // the application closes it after the answer.
                // A question shown is answered by closing: the response carries it.
                let asked = shell.with_settings(|view, m| view.request_close(m));
                if let Some(response) = asked {
                    let keep_open = response.keep_open;
                    if keep_open || response.action.is_some() {
                        shell.apply_settings_response(window, response, &dispatch);
                    }
                    if keep_open {
                        return LRESULT(0);
                    }
                }
                // The answer may have closed the window already.
                if shell.settings_window() != Some(window) {
                    return LRESULT(0);
                }
                // A rename in progress is kept, as when leaving the box.
                let committed = shell.with_settings(|view, m| view.end_edit(m));
                if let Some(response) = committed.filter(|r| r.action.is_some()) {
                    shell.apply_settings_response(window, response, &dispatch);
                }
                // Destroyed (not hidden): an unused settings window costs nothing.
                // SAFETY: our own window; no borrow is held.
                let _ = unsafe { DestroyWindow(window) };
                return LRESULT(0);
            }
            WM_DESTROY => {
                shell.settings_destroyed(window, &dispatch);
                // SAFETY: our own window: no message reaches the host any more.
                unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
                return LRESULT(0);
            }
            WM_GETOBJECT => {
                let source: std::rc::Rc<dyn AccessSource> =
                    std::rc::Rc::new(SettingsAccess { owner: shell.window });
                if let Some(result) = uia::on_get_object(window, wparam, lparam, source) {
                    return result;
                }
            }
            _ => {
                if let Some(result) =
                    shell.settings_message(window, message, wparam, lparam, &dispatch)
                {
                    return result;
                }
            }
        }
    }
    // SAFETY: forwarding the unmodified message to the default procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Screen-reader access to the settings window: reads the view through the message window.
pub(crate) struct SettingsAccess {
    pub(crate) owner: HWND,
}

impl SettingsAccess {
    fn with_host<R>(&self, f: impl FnOnce(&Host<'_>) -> Option<R>) -> Option<R> {
        // SAFETY: UI Automation calls the providers on the UI thread (COM threading), where
        // the message window's user data is the host or 0.
        let host = unsafe { host_of(self.owner) }?;
        f(host)
    }
}

impl AccessSource for SettingsAccess {
    fn tree(&self) -> Option<bv_ui_kit::access::AccessNode> {
        self.with_host(|host| {
            let state = host.shell.settings_state.try_borrow().ok()?;
            let graphics = host.shell.graphics()?;
            let text_scale = host.shell.app_theme().text_scale;
            let measure = GraphicsMeasure { graphics: &graphics, text_scale };
            state.open.as_ref().map(|o| o.view.access_tree(&measure))
        })
    }

    fn scale(&self) -> f32 {
        self.with_host(|host| {
            let state = host.shell.settings_state.try_borrow().ok()?;
            state.open.as_ref().map(|o| o.scale)
        })
        .unwrap_or(1.0)
    }

    fn act(&self, id: u64, action: AccessAction) -> bool {
        self.with_host(|host| {
            let shell = &host.shell;
            let window = shell.settings_window()?;
            let response = shell.with_settings(|view, m| view.access_action(id, action, m))?;
            let acted = response != Response::default();
            shell.apply_settings_response(window, response, &|event| host.dispatch(event));
            Some(acted)
        })
        .unwrap_or(false)
    }

    fn focused(&self) -> Option<u64> {
        self.with_host(|host| {
            let state = host.shell.settings_state.try_borrow().ok()?;
            state.open.as_ref().and_then(|o| o.view.focused_access_id())
        })
    }
}

/// Most application icons kept while the window is open (a long Applications page).
const APP_ICONS_LIMIT: usize = 512;

/// Loads the application icons of the rows at the size they are drawn (FR-096); rows
/// keep a generic picture when an icon cannot be loaded.
fn resolve_icons(
    mut content: SettingsContent,
    icons: &mut IconCache,
    scale: f32,
    graphics: &Graphics,
) -> SettingsContent {
    let (size, _) = t::ROW_APP_ICON;
    let pixels = (size * scale).round().max(1.0) as u32;
    for block in &mut content.blocks {
        if let Block::Device(row) = block {
            if let RowPicture::App { pixels: loaded, source: Some(source) } = &mut row.picture {
                *loaded = icons.get(source, pixels, graphics);
            }
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_settings_inputs() {
        let none = Modifiers::default();
        let shift = Modifiers { shift: true, ..none };
        let ctrl = Modifiers { ctrl: true, ..none };
        let key = |k| Some(Input::Key(k));
        assert_eq!(
            settings_input(VK_TAB.0, shift, false, false),
            key(SettingsKey::Tab { back: true })
        );
        assert_eq!(settings_input(VK_SPACE.0, none, false, false), key(SettingsKey::Space));
        assert_eq!(settings_input(VK_F4.0, none, false, false), key(SettingsKey::Open));
        assert_eq!(settings_input(VK_DOWN.0, none, true, false), key(SettingsKey::AltDown));
        assert_eq!(settings_input(VK_UP.0, none, true, false), key(SettingsKey::AltUp));
        assert_eq!(
            settings_input(VK_LEFT.0, shift, false, true),
            key(SettingsKey::Left { select: true })
        );
        // Alt+F4 stays Windows': it closes the window.
        assert_eq!(settings_input(VK_F4.0, none, true, false), None);
        // Letters are typed text (WM_CHAR), not keys.
        assert_eq!(settings_input(0x41, none, false, true), None);
        // Clipboard shortcuts only while renaming.
        assert_eq!(settings_input(0x56, ctrl, false, true), Some(Input::Paste));
        assert_eq!(settings_input(0x43, ctrl, false, true), key(SettingsKey::Copy));
        assert_eq!(settings_input(0x41, ctrl, false, false), None);
        // AltGr+letter types a character on some layouts: never a shortcut.
        let altgr = Modifiers { ctrl: true, alt: true, ..none };
        assert_eq!(settings_input(0x43, altgr, false, true), None);
    }

    #[test]
    fn a_place_off_every_screen_is_not_used() {
        assert!(!on_a_screen(&WindowPlace {
            x: -100_000,
            y: -100_000,
            width: 800,
            height: 600,
            maximized: false
        }));
        assert!(!on_a_screen(&WindowPlace { width: 0, ..Default::default() }));
    }
}
