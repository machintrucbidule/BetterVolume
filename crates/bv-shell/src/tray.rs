//! The tray icon: image, tooltip, presence in the taskbar, and wheel capture (FR-010 to
//! FR-015).
//!
//! Wheel capture (`docs/ARCHITECTURE.md` §6): Windows does not send wheel events to
//! tray icons. When the icon reports the pointer over it, raw mouse input is turned on
//! for the hidden window and a 100 ms timer checks whether the pointer is still on the
//! icon; wheel turns while it is are reported, and when it leaves, both are turned off.
//! Nothing runs while the pointer is elsewhere.

use bv_ui_kit::controls::tray::{self, TrayIcon};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEMOUSE,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
    NOTIFYICON_VERSION_4, NOTIFY_ICON_MESSAGE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, LoadIconW, SetTimer, IDI_APPLICATION, RI_MOUSE_WHEEL,
};

use crate::host::{Shell, TIMER_HOVER, TIMER_ICON_RETRY, TIMER_PROMOTE, WM_TRAY};
use crate::input::WheelAccumulator;
use crate::trayicon::{self, OwnedIcon};
use crate::wide::copy_truncated;

/// Identifier of our (single) tray icon within the message window.
pub(crate) const TRAY_ICON_ID: u32 = 1;

/// Delay between two attempts to add the tray icon.
const ICON_RETRY_INTERVAL_MS: u32 = 2000;
/// Attempts before giving up on the timer (one minute); the "TaskbarCreated" broadcast
/// still triggers a new attempt afterwards. Keeps the retry transient: no endless polling.
const ICON_RETRY_MAX_ATTEMPTS: u32 = 30;
/// How often the pointer position is checked while it hovers the icon.
const HOVER_INTERVAL_MS: u32 = 100;
/// Looks for Windows' record of a new icon: every 3 s, five times at most.
const PROMOTE_INTERVAL_MS: u32 = 3000;
const PROMOTE_MAX_TRIES: u32 = 5;

/// Image and wheel state of the tray icon.
#[derive(Debug, Default)]
pub(crate) struct TrayState {
    /// What the application wants shown (`None` until it says).
    wanted: Option<TrayIcon>,
    /// The rendered image (`None`: stock icon, before the first render or if rendering
    /// failed).
    image: Option<OwnedIcon>,
    hovering: bool,
    wheel: WheelAccumulator,
}

impl Shell {
    /// Changes the tooltip of the tray icon.
    pub fn set_tooltip(&self, tooltip: &str) {
        tooltip.clone_into(&mut self.tooltip.borrow_mut());
        if self.icon_added.get() {
            self.notify_icon(NIM_MODIFY);
        }
    }

    /// Changes the tray icon image (volume step, style). Drawn by the design system at
    /// the taskbar's scaling and in the taskbar's colours; redrawn by the shell itself
    /// when those change.
    pub fn set_tray_icon(&self, icon: TrayIcon) {
        if self.tray.borrow().wanted == Some(icon) && self.tray.borrow().image.is_some() {
            return;
        }
        self.tray.borrow_mut().wanted = Some(icon);
        self.render_tray_icon();
    }

    /// Renders the wanted icon for the current scaling and theme, and shows it.
    pub(crate) fn render_tray_icon(&self) {
        let Some(icon) = self.tray.borrow().wanted else { return };
        let pixels = crate::screen::tray_icon_pixels();
        let theme = self.theme();
        let drawn = self.with_image_graphics(|graphics| {
            graphics.rasterize(pixels, pixels, tray::scale_for(pixels), 1.0, |c| {
                tray::paint(c, &icon, &theme)
            })
        });
        let image = match drawn {
            Some(Ok(pixels)) => trayicon::create(&pixels),
            Some(Err(error)) => {
                crate::log::error(&format!("tray icon not drawn: {error}"));
                None
            }
            None => None,
        };
        let Some(image) = image else { return };
        // The old image is destroyed only once the taskbar holds its copy of the new one.
        let old = self.tray.borrow_mut().image.replace(image);
        if self.icon_added.get() {
            self.notify_icon(NIM_MODIFY);
        }
        drop(old);
    }

    /// Once per run, as soon as the icon is in place, asks Windows to show it on the
    /// taskbar if Windows holds no preference for this copy yet. Windows writes its record
    /// of a new icon a moment after it appears: a missing record is looked for again a few
    /// times (one-shot timer), then given up.
    fn promote_first_start(&self) {
        if self.promote_tries.get() == 0 {
            self.promote_tries.set(1);
            self.try_promote();
        }
    }

    /// One look at Windows' record of our icon (see [`Shell::promote_first_start`]).
    pub(crate) fn try_promote(&self) {
        let tries = self.promote_tries.get();
        match promote(true) {
            Promotion::NoRecord if tries < PROMOTE_MAX_TRIES => {
                self.promote_tries.set(tries + 1);
                // SAFETY: our own window; a timer with the same identifier is replaced.
                unsafe { SetTimer(Some(self.window), TIMER_PROMOTE, PROMOTE_INTERVAL_MS, None) };
            }
            outcome => {
                self.promote_tries.set(u32::MAX);
                // SAFETY: our own window; fails harmlessly when no timer runs.
                let _ = unsafe { KillTimer(Some(self.window), TIMER_PROMOTE) };
                if outcome == Promotion::Promoted {
                    crate::log::info("tray icon: shown on the taskbar (first start of this copy)");
                }
            }
        }
    }

    /// Puts the tray icon in place (first start, or after a "TaskbarCreated" broadcast).
    pub(crate) fn add_icon(&self) {
        // The broadcast also comes when the scaling changed: the image follows.
        self.render_tray_icon();
        // "TaskbarCreated" is also broadcast when the icon still exists: adding then
        // fails but modifying succeeds, and the icon must keep counting as present.
        let present = self.notify_icon(NIM_ADD) || self.notify_icon(NIM_MODIFY);
        self.icon_added.set(present);
        if present {
            self.set_icon_version();
            self.stop_icon_retry();
            self.promote_first_start();
        } else {
            // The taskbar is not ready (very early start at logon, Explorer restarting).
            self.icon_retries_left.set(ICON_RETRY_MAX_ATTEMPTS);
            // SAFETY: our own window; a timer with the same identifier is simply replaced.
            if unsafe {
                SetTimer(Some(self.window), TIMER_ICON_RETRY, ICON_RETRY_INTERVAL_MS, None)
            } == 0
            {
                crate::log::error("tray icon: cannot start the retry timer");
            }
        }
    }

    /// One retry tick: tries again, and stops the timer on success or after the last attempt.
    pub(crate) fn retry_add_icon(&self) {
        let present = self.notify_icon(NIM_ADD) || self.notify_icon(NIM_MODIFY);
        self.icon_added.set(present);
        if present {
            self.set_icon_version();
            self.promote_first_start();
        }
        let (left, stop) = retry_outcome(self.icon_retries_left.get(), present);
        self.icon_retries_left.set(left);
        if stop {
            if !present {
                crate::log::error("tray icon: taskbar still unavailable after one minute");
            }
            self.stop_icon_retry();
        }
    }

    fn set_icon_version(&self) {
        if !self.notify_icon(NIM_SETVERSION) {
            crate::log::error("tray icon: modern callback version refused");
        }
    }

    fn stop_icon_retry(&self) {
        self.icon_retries_left.set(0);
        // SAFETY: our own window; fails harmlessly when the timer is not running.
        let _ = unsafe { KillTimer(Some(self.window), TIMER_ICON_RETRY) };
    }

    pub(crate) fn remove_icon(&self) {
        self.stop_icon_retry();
        if self.icon_added.replace(false) {
            self.notify_icon(NIM_DELETE);
        }
    }

    fn notify_icon(&self, action: NOTIFY_ICON_MESSAGE) -> bool {
        let image = self.tray.borrow().image.as_ref().map(|i| i.0);
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.window,
            uID: TRAY_ICON_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAY,
            hIcon: match image {
                Some(icon) => icon,
                // Until the first image exists (or if drawing failed): the stock icon.
                // SAFETY: loads a shared system icon; no module handle is involved.
                None => unsafe { LoadIconW(None, IDI_APPLICATION) }.unwrap_or_default(),
            },
            ..Default::default()
        };
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        copy_truncated(&self.tooltip.borrow(), &mut data.szTip);
        // SAFETY: `data` is a fully initialised structure with the right `cbSize`.
        unsafe { Shell_NotifyIconW(action, &data) }.as_bool()
    }

    /// The icon's rectangle on screen, if the taskbar shows it.
    pub(crate) fn icon_rect(&self) -> Option<RECT> {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.window,
            uID: TRAY_ICON_ID,
            ..Default::default()
        };
        // SAFETY: `id` is a valid structure with its size set.
        unsafe { Shell_NotifyIconGetRect(&id) }.ok()
    }

    /// Whether the pointer is on the icon: inside its rectangle, and over a window of the
    /// taskbar (a full-screen window covering the taskbar hides the icon).
    fn pointer_on_icon(&self) -> bool {
        let (Some(r), Some((x, y))) = (self.icon_rect(), crate::screen::cursor()) else {
            return false;
        };
        let inside = x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        inside && crate::screen::taskbar_under(x, y)
    }

    /// The pointer is over the icon: listen to the wheel until it leaves.
    pub(crate) fn start_hover(&self) {
        if self.tray.borrow().hovering {
            return;
        }
        if !set_raw_mouse(self, true) {
            return;
        }
        // SAFETY: our own window.
        if unsafe { SetTimer(Some(self.window), TIMER_HOVER, HOVER_INTERVAL_MS, None) } == 0 {
            set_raw_mouse(self, false);
            crate::log::error("wheel capture: cannot start the hover timer");
            return;
        }
        self.tray.borrow_mut().hovering = true;
    }

    /// Hover timer tick: stops listening once the pointer left the icon.
    pub(crate) fn check_hover(&self) {
        if !self.pointer_on_icon() {
            self.stop_hover();
        }
    }

    pub(crate) fn stop_hover(&self) {
        let mut tray = self.tray.borrow_mut();
        if !tray.hovering {
            return;
        }
        tray.hovering = false;
        tray.wheel.reset();
        drop(tray);
        // SAFETY: our own window; fails harmlessly when the timer is not running.
        let _ = unsafe { KillTimer(Some(self.window), TIMER_HOVER) };
        set_raw_mouse(self, false);
    }

    /// Raw input arrived: whole wheel notches turned over the icon, if any.
    pub(crate) fn wheel_input(&self, lparam: LPARAM) -> Option<i32> {
        if !self.tray.borrow().hovering {
            return None;
        }
        let delta = wheel_delta(lparam)?;
        if !self.pointer_on_icon() {
            return None;
        }
        let notches = self.tray.borrow_mut().wheel.add(delta);
        (notches != 0).then_some(notches)
    }
}

/// After one retry tick with `left` attempts remaining: the attempts left, and whether
/// the retry timer stops (the icon is there, or the last attempt was used).
fn retry_outcome(left: u32, present: bool) -> (u32, bool) {
    let left = left.saturating_sub(1);
    (left, present || left == 0)
}

/// Turns raw mouse input for the hidden window on or off.
fn set_raw_mouse(shell: &Shell, on: bool) -> bool {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01, // generic desktop controls
        usUsage: 0x02,     // mouse
        dwFlags: if on { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
        hwndTarget: if on { shell.window } else { Default::default() },
    };
    // SAFETY: one valid device structure, with its size.
    let ok =
        unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) }.is_ok();
    if !ok {
        crate::log::error(&format!(
            "wheel capture: raw input {} refused",
            if on { "on" } else { "off" }
        ));
    }
    ok
}

/// The vertical wheel delta of a raw mouse input, if it carries one.
fn wheel_delta(lparam: LPARAM) -> Option<i32> {
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    // SAFETY: `raw` is large enough for a mouse input (`size` says so); the handle comes
    // from the WM_INPUT message being processed.
    let read = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut core::ffi::c_void),
            RID_INPUT,
            Some((&raw mut raw).cast()),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == u32::MAX || raw.header.dwType != RIM_TYPEMOUSE.0 {
        return None;
    }
    // SAFETY: the header says this is mouse input, so the mouse member is the valid one.
    let buttons = unsafe { raw.data.mouse.Anonymous.Anonymous };
    if u32::from(buttons.usButtonFlags) & RI_MOUSE_WHEEL == 0 {
        return None;
    }
    Some(i32::from(buttons.usButtonData as i16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_stop_on_success_or_after_the_last_attempt() {
        assert_eq!(
            retry_outcome(ICON_RETRY_MAX_ATTEMPTS, false),
            (ICON_RETRY_MAX_ATTEMPTS - 1, false)
        );
        assert_eq!(retry_outcome(5, true), (4, true));
        assert_eq!(retry_outcome(1, false), (0, true));
        // A stray tick after the timer stopped changes nothing and stops again.
        assert_eq!(retry_outcome(0, false), (0, true));
        // Thirty failing ticks end the retries: a transient timer, never endless.
        let mut left = ICON_RETRY_MAX_ATTEMPTS;
        let mut ticks = 0;
        loop {
            ticks += 1;
            let (next, stop) = retry_outcome(left, false);
            left = next;
            if stop {
                break;
            }
        }
        assert_eq!(ticks, ICON_RETRY_MAX_ATTEMPTS);
    }
}

/// Asks Windows to show this application's tray icon directly on the taskbar rather than
/// behind the hidden-icons arrow (the Windows "other system tray icons" preference),
/// for this executable only. Called once, when the user turns on hiding the Windows volume
/// icon: afterwards the user's own choice in Windows is respected. Does nothing when
/// Windows has no record of the icon yet.
///
/// Windows keeps one key per icon under `HKCU\Control Panel\NotifyIconSettings`, holding
/// the executable path (absolute, or starting with a known-folder identifier), the icon
/// identifier and the `IsPromoted` preference; the taskbar follows changes to it.
pub fn show_icon_on_taskbar() {
    promote(false);
}

/// What [`promote`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Promotion {
    /// Windows has no record of our icon yet (it writes it shortly after the icon appears).
    NoRecord,
    /// The record already holds a preference (only when undecided ones were asked for).
    Decided,
    /// The icon was asked to show on the taskbar.
    Promoted,
}

/// Sets `IsPromoted` on Windows' record of our icon; with `only_undecided`, only when the
/// record holds no preference yet (the first start of this copy).
pub(crate) fn promote(only_undecided: bool) -> Promotion {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, REG_DWORD,
    };
    let Ok(exe) = std::env::current_exe() else { return Promotion::NoRecord };
    let exe = exe.to_string_lossy().to_string();
    const SETTINGS: &str = "Control Panel\\NotifyIconSettings";
    let mut key = HKEY::default();
    // SAFETY: a constant path and a valid out-pointer; the key is closed below.
    if unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(SETTINGS), None, KEY_READ, &mut key)
    }
    .is_err()
    {
        return Promotion::NoRecord;
    }
    let mut outcome = Promotion::NoRecord;
    let mut index = 0u32;
    loop {
        let mut name = [0u16; 128];
        let mut len = name.len() as u32;
        // SAFETY: an open key, a buffer of `len` units.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(windows::core::PWSTR(name.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status.is_err() {
            break;
        }
        index += 1;
        let len = (len as usize).min(name.len());
        let sub = format!("{SETTINGS}\\{}", String::from_utf16_lossy(&name[..len]));
        let path = crate::registry::read_text(&sub, "ExecutablePath")
            .and_then(|p| resolve_known_folder(&p));
        let uid = crate::registry::read_dword(&sub, "UID");
        if path.is_some_and(|p| p.eq_ignore_ascii_case(&exe)) && uid == Some(TRAY_ICON_ID) {
            if only_undecided && crate::registry::read_dword(&sub, "IsPromoted").is_some() {
                if outcome == Promotion::NoRecord {
                    outcome = Promotion::Decided;
                }
                continue;
            }
            outcome = Promotion::Promoted;
            let one: u32 = 1;
            // SAFETY: a valid key path and a 4-byte value.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(sub.as_str()),
                    windows::core::w!("IsPromoted"),
                    REG_DWORD.0,
                    Some((&raw const one).cast()),
                    4,
                )
            };
            if status.is_err() {
                crate::log::error("tray icon: taskbar preference not written");
            }
        }
    }
    // SAFETY: the key opened above.
    let _ = unsafe { RegCloseKey(key) };
    outcome
}

/// An executable path as Windows records it for tray icons: absolute, or a known-folder
/// identifier in braces followed by the rest of the path.
fn resolve_known_folder(path: &str) -> Option<String> {
    let Some(rest) = path.strip_prefix('{') else { return Some(path.to_owned()) };
    let (id, tail) = rest.split_once('}')?;
    let id = windows::core::GUID::try_from(id).ok()?;
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    // SAFETY: a valid identifier; the returned string is freed below.
    let folder = unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) }.ok()?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { folder.to_string() }.ok();
    // SAFETY: allocated by the shell with the task allocator.
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(folder.0 as *const _)) };
    Some(format!("{}{tail}", text?))
}

#[cfg(test)]
mod promote_tests {
    use super::resolve_known_folder;

    #[test]
    fn absolute_paths_are_kept_and_known_folders_resolved() {
        assert_eq!(resolve_known_folder("C:\\Apps\\a.exe").as_deref(), Some("C:\\Apps\\a.exe"));
        // FOLDERID_Windows.
        let windows = resolve_known_folder("{F38BF404-1D43-42F2-9305-67DE0B28FC23}\\explorer.exe")
            .expect("resolved");
        assert!(windows.to_ascii_lowercase().ends_with("\\windows\\explorer.exe"), "{windows}");
        assert_eq!(resolve_known_folder("{not a guid}\\x.exe"), None);
        assert_eq!(resolve_known_folder("{unclosed"), None);
    }
}
