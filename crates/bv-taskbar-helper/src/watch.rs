//! Guard rail: while the Windows volume icon is hidden, the BetterVolume process that asked
//! for it is watched from inside the taskbar. When it ends in any way (quit, crash, ended
//! from the Task Manager, uninstalled), the helper loads itself again with "restore", so
//! the Windows icon never stays hidden without BetterVolume's own icon.
//!
//! The wait is a thread-pool wait on the process handle: nothing runs until it ends.

use std::sync::Mutex;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Threading::{
    OpenProcess, RegisterWaitForSingleObject, UnregisterWait, INFINITE, PROCESS_SYNCHRONIZE,
    WT_EXECUTEONLYONCE,
};

/// The current watch: the wait registration and the process handle it waits on.
struct Watch {
    wait: HANDLE,
    process: HANDLE,
}

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for Watch {}

static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

/// Watches `pid` (replacing any previous watch). 0 watches nothing.
pub fn watch(pid: u32) {
    cancel();
    if pid == 0 {
        return;
    }
    // SAFETY: plain call; the handle is closed by `cancel` or after the wait fired.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return;
    };
    let mut wait = HANDLE::default();
    // SAFETY: valid out-pointer and process handle; the callback is a plain function that
    // stays loaded (the library is never unloaded from the taskbar once loaded).
    let registered = unsafe {
        RegisterWaitForSingleObject(
            &mut wait,
            process,
            Some(owner_ended),
            None,
            INFINITE,
            WT_EXECUTEONLYONCE,
        )
    };
    if registered.is_err() {
        // SAFETY: our own handle.
        let _ = unsafe { CloseHandle(process) };
        return;
    }
    if let Ok(mut current) = WATCH.lock() {
        *current = Some(Watch { wait, process });
    }
}

/// Stops watching (the icon was restored on purpose, or a new owner takes over).
pub fn cancel() {
    let old = WATCH.lock().ok().and_then(|mut current| current.take());
    if let Some(old) = old {
        // SAFETY: handles we own. A non-blocking unregistration: a callback already
        // running finds no watch and restores (harmless: the icon is wanted back anyway
        // when the owner is gone).
        unsafe {
            let _ = UnregisterWait(old.wait);
            let _ = CloseHandle(old.process);
        }
    }
}

/// The owner ended: restore the Windows icon from a thread of our own (a new XAML
/// diagnostics connection must not be made on a thread-pool thread that already has one).
unsafe extern "system" fn owner_ended(_context: *mut core::ffi::c_void, _timed_out: bool) {
    crate::com::trace("owner ended: restoring");
    cancel();
    let _ = std::thread::Builder::new()
        .name("bv-taskbar-helper-restore".into())
        .spawn(crate::load::restore_from_inside);
}
