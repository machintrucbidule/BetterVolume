//! Single instance per Windows session (FR-001).

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
};

use crate::host::{activate_message, WINDOW_CLASS};

/// `Local\` scopes the name to the current Windows session: one instance per session.
const MUTEX_NAME: PCWSTR = w!("Local\\BetterVolume.SingleInstance");

/// Proof that this process is the only running instance. Keep it alive for the whole run;
/// dropping it lets another instance start.
#[derive(Debug)]
pub struct SingleInstance {
    mutex: HANDLE,
}

impl SingleInstance {
    /// Claims the single-instance slot.
    ///
    /// Returns `None` when another instance already runs in this session; that instance
    /// has then been asked to come forward (it receives [`crate::Event::ActivateRequested`])
    /// and the caller should simply exit.
    ///
    /// "Access denied" also means an instance is running: the guard exists but belongs to
    /// an instance started with higher privileges. Only if Windows cannot create the guard
    /// for an unrelated reason does the application start anyway rather than being locked
    /// out.
    pub fn acquire() -> Option<SingleInstance> {
        // SAFETY: the name is a valid NUL-terminated constant; no security attributes.
        let created = unsafe { CreateMutexW(None, false, MUTEX_NAME) };
        match created {
            // SAFETY: reads the calling thread's last-error value. On success the binding
            // returns the handle without making any other call, so the value is still the
            // one set by `CreateMutexW`.
            Ok(mutex) if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS => {
                // SAFETY: `mutex` is a handle we own, closed exactly once.
                let _ = unsafe { CloseHandle(mutex) };
                signal_running_instance();
                None
            }
            Ok(mutex) => Some(SingleInstance { mutex }),
            Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => {
                // Best effort: Windows does not let us message a more privileged window.
                signal_running_instance();
                None
            }
            Err(_) => Some(SingleInstance { mutex: HANDLE::default() }),
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if !self.mutex.is_invalid() && !self.mutex.0.is_null() {
            // SAFETY: the handle was returned by `CreateMutexW` and is closed exactly once.
            let _ = unsafe { CloseHandle(self.mutex) };
        }
    }
}

/// Asks the running instance to come forward. Best effort: if its window does not exist
/// yet (it is still starting), there is nothing to do.
fn signal_running_instance() {
    // SAFETY: the class name is a valid NUL-terminated constant.
    let Ok(window) = (unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()) }) else { return };
    // The running instance opens its flyout and must be allowed to take the foreground
    // (this launch has that right when the user started it): otherwise the flyout would
    // not get the focus and could not close when it loses it.
    let mut process = 0u32;
    // SAFETY: valid out pointer; a stale handle only makes the call fail.
    if unsafe { GetWindowThreadProcessId(window, Some(&mut process)) } != 0 && process != 0 {
        // SAFETY: plain call; fails harmlessly when this process has no foreground right.
        let _ = unsafe { AllowSetForegroundWindow(process) };
    }
    // SAFETY: posting a registered message without pointers to a window, valid or not, is
    // always sound; a stale handle only makes the call fail.
    let _ = unsafe { PostMessageW(Some(window), activate_message(), WPARAM(0), LPARAM(0)) };
}
