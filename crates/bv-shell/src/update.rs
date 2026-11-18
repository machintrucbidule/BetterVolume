//! What updates need from Windows (FR-132): the timer of the daily check,
//! opening a release page, starting the downloaded setup file or the replaced portable
//! file, and waiting for the copy being replaced to end.

use std::path::Path;
use std::time::Duration;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer, SW_SHOWNORMAL};

use crate::host::Shell;

/// Timer of the next automatic update check (on the message window).
pub(crate) const TIMER_UPDATE: usize = 7;

/// Argument of the setup file started by the running copy to update it: it
/// installs at once, without asking again.
pub const UPDATE_ARGUMENT: &str = "--update";

/// Argument of a portable copy started by the copy it replaced, followed by that
/// copy's process identifier: it waits for that process to end before it starts.
pub const REPLACED_ARGUMENT: &str = "--replaced";

/// Longest wait [`SetTimer`] takes, in milliseconds (USER_TIMER_MAXIMUM).
const TIMER_MAX_MS: u64 = 0x7FFF_FFFF;

impl Shell {
    /// Reports [`Event::UpdateDue`](crate::Event::UpdateDue) once, after `delay` (replaces
    /// a timer already set). Windows may deliver it late (sleep): the application checks
    /// the clock again.
    pub fn set_update_timer(&self, delay: Duration) {
        let ms = u32::try_from(delay.as_millis().min(u128::from(TIMER_MAX_MS))).unwrap_or(u32::MAX);
        // SAFETY: our own window, on its thread; the same identifier replaces the timer.
        unsafe { SetTimer(Some(self.window), TIMER_UPDATE, ms.max(1), None) };
    }

    /// Stops the timer of [`set_update_timer`](Self::set_update_timer).
    pub fn stop_update_timer(&self) {
        // SAFETY: our own window, on its thread; an absent timer only makes the call fail.
        let _ = unsafe { KillTimer(Some(self.window), TIMER_UPDATE) };
    }

    /// The timer fired: it is one-shot.
    pub(crate) fn update_timer_fired(&self) {
        self.stop_update_timer();
    }
}

/// Opens a web page in the user's browser (a release page). Only `https://`
/// addresses, and `http://` on the loopback address (the local imitation of the release
/// service used by tests), are opened.
pub fn open_web_page(url: &str) -> bool {
    let loopback = ["http://127.0.0.1:", "http://localhost:", "http://127.0.0.1/"]
        .iter()
        .any(|p| url.starts_with(p));
    if !(url.starts_with("https://") || loopback) || url.contains(['"', ' ', '\r', '\n']) {
        crate::log::error("refused to open a web page: not an https address");
        return false;
    }
    // SAFETY: NUL-terminated buffers that outlive the call; no owner window.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let ok = result.0 as isize > 32;
    if !ok {
        crate::log::error(&format!("cannot open a web page: code {}", result.0 as isize));
    }
    ok
}

/// Starts `exe` with `args`, in its own folder, without waiting for it.
pub fn launch_with(exe: &Path, args: &[String]) -> bool {
    let folder = exe.parent().map_or_else(std::env::temp_dir, Path::to_owned);
    std::process::Command::new(exe).args(args).current_dir(folder).spawn().is_ok()
}

/// Waits until the process `pid` has ended, for at most `timeout`. Returns whether it has
/// (a process that cannot be found has).
pub fn wait_for_process(pid: u32, timeout: Duration) -> bool {
    // SAFETY: plain call; a failure means the process is gone (or not ours to wait for).
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return true;
    };
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: a handle opened above with the right to wait, closed right after.
    let ended = unsafe { WaitForSingleObject(process, ms) } == WAIT_OBJECT_0;
    // SAFETY: our own handle, closed once.
    let _ = unsafe { CloseHandle(process) };
    ended
}

/// A time (Unix seconds) as the user reads it: the local date and time, in the regional
/// format chosen in Windows ("18/11/2026 14:02"). Empty if Windows cannot format it.
pub fn local_time_text(unix: u64) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::Globalization::{
        GetDateFormatEx, GetTimeFormatEx, DATE_SHORTDATE, TIME_NOSECONDS,
    };
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    // 100-nanosecond intervals since 1601-01-01.
    let ticks = unix.saturating_add(11_644_473_600).saturating_mul(10_000_000);
    let file = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: plain conversions between structures owned here; the current time zone.
    let converted = unsafe {
        FileTimeToSystemTime(&file, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
    };
    if !converted {
        return String::new();
    }
    let mut date = [0u16; 80];
    let mut time = [0u16; 80];
    // SAFETY: the user's default locale (null name), buffers owned here with their length.
    let (d, t) = unsafe {
        (
            GetDateFormatEx(
                PCWSTR::null(),
                DATE_SHORTDATE,
                Some(&local),
                PCWSTR::null(),
                Some(&mut date),
                PCWSTR::null(),
            ),
            GetTimeFormatEx(
                PCWSTR::null(),
                TIME_NOSECONDS,
                Some(&local),
                PCWSTR::null(),
                Some(&mut time),
            ),
        )
    };
    // The lengths count the terminating NUL.
    let text = |buffer: &[u16], len: i32| {
        String::from_utf16_lossy(&buffer[..usize::try_from(len - 1).unwrap_or(0).min(buffer.len())])
    };
    if d <= 0 || t <= 0 {
        return String::new();
    }
    format!("{} {}", text(&date, d), text(&time, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_hold_the_year_and_the_minutes() {
        // 2026-11-18 12:34:56 UTC: whatever the zone and the format, the year shows.
        let text = local_time_text(1_795_005_296);
        assert!(text.contains("2026") || text.contains("26"), "{text}");
        assert!(text.contains(':') || text.contains('.') || text.contains('h'), "{text}");
    }

    #[test]
    fn waiting_for_a_process_that_ended_or_never_was() {
        let mut child =
            std::process::Command::new("cmd").args(["/c", "exit"]).spawn().expect("cmd starts");
        let pid = child.id();
        let _ = child.wait();
        assert!(wait_for_process(pid, Duration::from_millis(100)));
        // Our own process does not end while we wait.
        assert!(!wait_for_process(std::process::id(), Duration::from_millis(50)));
    }

    #[test]
    fn only_https_or_loopback_pages_are_opened() {
        // Refused before reaching Windows: nothing opens during the test.
        assert!(!open_web_page("file:///C:/Windows/system32/calc.exe"));
        assert!(!open_web_page("http://example.com/x"));
        assert!(!open_web_page("https://github.com/a\" b"));
        assert!(!open_web_page("calc.exe"));
    }
}
