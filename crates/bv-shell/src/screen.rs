//! Screen queries: taskbar, monitors, scaling, pointer.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetAncestor, GetClassNameW, GetCursorPos, GetTopWindow, GetWindow, GetWindowLongW,
    IsWindowVisible, WindowFromPoint, GA_ROOT, GWL_EXSTYLE, GW_HWNDNEXT, SM_CXSMICON,
    WS_EX_LAYOUTRTL, WS_EX_TOPMOST,
};

use crate::placement::{taskbar_edge, usable_area, Edge, ScreenRect};

/// Class of the Windows taskbar on the primary monitor (the one holding the tray).
const TASKBAR_CLASS: PCWSTR = w!("Shell_TrayWnd");

/// Everything needed to place a popup on one monitor.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    /// The whole monitor: popups are cut to it while they move.
    pub monitor: ScreenRect,
    /// The area popups may use.
    pub area: ScreenRect,
    /// Edge the taskbar is docked to (bottom when it is on another monitor).
    pub edge: Edge,
    /// Scale of the monitor (1.0 = 96 DPI).
    pub scale: f32,
    /// Right-to-left taskbar (tray at the left end).
    pub rtl: bool,
    /// The taskbar window, when found (popups rise from behind it).
    pub taskbar: Option<HWND>,
}

fn rect(r: RECT) -> ScreenRect {
    ScreenRect::new(r.left, r.top, r.right, r.bottom)
}

pub(crate) fn taskbar_window() -> Option<HWND> {
    // SAFETY: the class name is a valid constant.
    unsafe { FindWindowW(TASKBAR_CLASS, PCWSTR::null()) }.ok().filter(|h| !h.is_invalid())
}

fn taskbar_rect() -> Option<ScreenRect> {
    let mut data = APPBARDATA { cbSize: size_of::<APPBARDATA>() as u32, ..Default::default() };
    // SAFETY: `data` is a valid structure with its size set.
    let ok = unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut data) };
    (ok != 0).then(|| rect(data.rc))
}

fn layout_of(monitor: HMONITOR) -> Layout {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `info` is a valid structure with its size set; a stale monitor fails.
    let found = unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool();
    let (monitor_rect, work) = if found {
        (rect(info.rcMonitor), rect(info.rcWork))
    } else {
        let fallback = ScreenRect::new(0, 0, 1024, 768);
        (fallback, fallback)
    };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    // SAFETY: valid out pointers; on failure the defaults stay.
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let taskbar = taskbar_window();
    let bar = taskbar_rect().filter(|bar| {
        // The taskbar belongs to this monitor if its centre lies on it.
        let (cx, cy) = ((bar.left + bar.right) / 2, (bar.top + bar.bottom) / 2);
        cx >= monitor_rect.left
            && cx < monitor_rect.right
            && cy >= monitor_rect.top
            && cy < monitor_rect.bottom
    });
    let edge = bar.map_or(Edge::Bottom, |bar| taskbar_edge(&bar, &monitor_rect));
    let rtl = taskbar.is_some_and(|h| {
        // SAFETY: reading a style of a window of another process is allowed.
        let style = unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32;
        style & WS_EX_LAYOUTRTL.0 != 0
    });
    Layout {
        monitor: monitor_rect,
        area: usable_area(&work, bar.as_ref(), edge),
        edge,
        scale: dpi_x.max(1) as f32 / 96.0,
        rtl,
        taskbar,
    }
}

/// Layout of the monitor holding the taskbar (and the tray): where the flyout opens.
pub(crate) fn taskbar_layout() -> Layout {
    let monitor = match taskbar_window() {
        // SAFETY: plain call; a stale window falls back to the primary monitor.
        Some(h) => unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY) },
        // SAFETY: plain call with a constant point.
        None => unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) },
    };
    layout_of(monitor)
}

/// Layout of the monitor under a screen point: where a menu opened there goes.
pub(crate) fn layout_at(x: i32, y: i32) -> Layout {
    // SAFETY: plain call.
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    layout_of(monitor)
}

/// The pointer position in screen pixels.
pub(crate) fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    // SAFETY: valid out pointer.
    unsafe { GetCursorPos(&mut p) }.ok().map(|()| (p.x, p.y))
}

/// Size in pixels of a tray icon at the taskbar's scaling.
pub(crate) fn tray_icon_pixels() -> u32 {
    let dpi = (taskbar_layout().scale * 96.0).round() as u32;
    // SAFETY: plain call.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) };
    u32::try_from(size).ok().filter(|&s| s >= 16).unwrap_or(16)
}

/// Whether the window under a screen point is the taskbar or its hidden-icons panel.
/// `true` when that cannot be told, so nothing is refused on an unusual shell.
pub(crate) fn taskbar_under(x: i32, y: i32) -> bool {
    // SAFETY: plain query.
    let window = unsafe { WindowFromPoint(POINT { x, y }) };
    !window.is_invalid() && taskbar_surface(window).unwrap_or(true)
}

/// Classes of the taskbar surfaces: the main and secondary taskbars and the hidden-icons
/// panel (other windows of the same Explorer process, such as File Explorer, are not).
const TASKBAR_SURFACES: [&str; 3] =
    ["Shell_TrayWnd", "Shell_SecondaryTrayWnd", "TopLevelWindowForOverflowXamlIsland"];

/// Whether a window is part of the taskbar (see [`TASKBAR_SURFACES`]); `None` when the
/// window is unknown.
pub(crate) fn taskbar_surface(window: HWND) -> Option<bool> {
    if window.is_invalid() {
        return None;
    }
    // SAFETY: plain queries; a stale handle only makes them fail.
    let class = unsafe {
        let root = GetAncestor(window, GA_ROOT);
        let mut name = [0u16; 64];
        let len = GetClassNameW(root, &mut name);
        String::from_utf16_lossy(&name[..len.max(0) as usize])
    };
    (!class.is_empty()).then(|| TASKBAR_SURFACES.contains(&class.as_str()))
}

/// Class of the Windows 11 hidden-icons panel (the overflow opened by the arrow next to
/// the tray icons).
const HIDDEN_ICONS_CLASS_NAME: &str = "TopLevelWindowForOverflowXamlIsland";

/// Whether the hidden-icons panel is open: a popup opened from it must appear in place,
/// on top, rather than rise from behind the taskbar (it would pass under the panel).
pub(crate) fn hidden_icons_open() -> bool {
    // The panel is not found by a class search (`FindWindowEx` does not list it), so the
    // z-order is walked from the top, through the always-on-top windows only (the panel
    // is one of them): a few windows, done only when a popup opens.
    // SAFETY: plain queries on window handles; stale ones only make them fail.
    unsafe {
        let mut window = GetTopWindow(None).ok();
        while let Some(current) = window.filter(|h| !h.is_invalid()) {
            let style = GetWindowLongW(current, GWL_EXSTYLE) as u32;
            if style & WS_EX_TOPMOST.0 == 0 {
                return false;
            }
            let mut name = [0u16; 64];
            let len = GetClassNameW(current, &mut name);
            let class = String::from_utf16_lossy(&name[..len.max(0) as usize]);
            if class == HIDDEN_ICONS_CLASS_NAME && IsWindowVisible(current).as_bool() {
                return true;
            }
            window = GetWindow(current, GW_HWNDNEXT).ok();
        }
    }
    false
}

/// Whether a full-screen program (game, video, presentation) is in front. The taskbar is
/// then behind it: a popup placed just below the taskbar would be hidden.
pub(crate) fn fullscreen_in_front() -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    // SAFETY: plain query; a failure counts as "no".
    unsafe { SHQueryUserNotificationState() }.is_ok_and(|state| {
        state == QUNS_BUSY
            || state == QUNS_RUNNING_D3D_FULL_SCREEN
            || state == QUNS_PRESENTATION_MODE
    })
}
