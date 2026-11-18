//! Noticing that the process behind an audio session has ended (FR-063).
//!
//! Windows does not always expire the sessions of a process that exits (in particular one
//! that is killed), so its row would stay listed. Each followed session therefore also
//! waits on its process through the Windows thread pool: no thread of our own, no
//! polling. When the process ends, a "sessions changed" signal makes the UI thread
//! enumerate again, and sessions of ended processes are left out.

use std::ffi::c_void;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    OpenProcess, RegisterWaitForSingleObject, UnregisterWaitEx, WaitForSingleObject, INFINITE,
    PROCESS_SYNCHRONIZE, WT_EXECUTEONLYONCE,
};

use crate::signals::{Signal, Sink};

/// What the thread-pool callback needs.
struct Context {
    sink: Sink,
    generation: u64,
}

/// A wait on one process, cancelled when dropped.
pub(crate) struct ExitWatch {
    process: HANDLE,
    wait: HANDLE,
    context: *mut Context,
}

/// Outcome of [`ExitWatch::start`].
pub(crate) enum Watch {
    /// The process runs and is watched.
    Running(ExitWatch),
    /// No such process any more: a session it left behind must not be listed.
    Gone,
    /// The process exists but may not be watched (protected): its session relies on
    /// Windows expiring it.
    Unwatchable,
}

impl ExitWatch {
    /// Starts waiting for process `pid` to end.
    pub(crate) fn start(pid: u32, sink: Sink, generation: u64) -> Watch {
        if pid == 0 {
            return Watch::Unwatchable;
        }
        // SAFETY: plain call; the handle is closed in `drop` (or below on failure).
        let process = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(process) => process,
            // Windows answers "invalid parameter" for an identifier no process has.
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
                return Watch::Gone
            }
            Err(_) => return Watch::Unwatchable,
        };
        // Already ended (its process object kept alive by someone's handle): no wait may
        // be registered, because its callback would fire at once — even if the wait were
        // cancelled right away — and signal a change that is none.
        // SAFETY: `process` was opened with SYNCHRONIZE; a zero timeout only checks the
        // state.
        if unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
            // SAFETY: opened above, closed once.
            let _ = unsafe { CloseHandle(process) };
            return Watch::Gone;
        }
        let context = Box::into_raw(Box::new(Context { sink, generation }));
        let mut wait = HANDLE::default();
        // SAFETY: `process` is a valid handle; `context` stays valid until `drop` has
        // cancelled the wait and waited for any running callback (UnregisterWaitEx with
        // INVALID_HANDLE_VALUE), and only then frees it.
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                process,
                Some(on_exit),
                Some(context as *const c_void),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        if registered.is_err() {
            // SAFETY: nothing else refers to these: the wait was not registered.
            unsafe {
                drop(Box::from_raw(context));
                let _ = CloseHandle(process);
            }
            return Watch::Unwatchable;
        }
        // If the process ends from now on, the callback signals it: that is a real change.
        Watch::Running(Self { process, wait, context })
    }

    /// Whether the process has already ended.
    pub(crate) fn has_exited(&self) -> bool {
        // SAFETY: `process` is a valid handle opened with SYNCHRONIZE; a zero timeout
        // only checks the state.
        unsafe { WaitForSingleObject(self.process, 0) == WAIT_OBJECT_0 }
    }
}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        // SAFETY: the wait was registered by `start`. INVALID_HANDLE_VALUE makes the call
        // return only once no callback is running any more, so the context can then be
        // freed. This is never called from the callback itself (it runs on the UI thread).
        unsafe {
            let _ = UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE));
            drop(Box::from_raw(self.context));
            let _ = CloseHandle(self.process);
        }
    }
}

/// Called once by the thread pool when the process ends.
unsafe extern "system" fn on_exit(context: *mut c_void, _timed_out: bool) {
    // SAFETY: `context` is the `Context` registered by `ExitWatch::start`, alive until
    // `drop` has made sure no callback runs; only shared access (`Sink` is `Sync`).
    if let Some(context) = unsafe { (context as *const Context).as_ref() } {
        context.sink.send(Signal::Sessions { generation: context.generation });
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_end_of_a_process_is_signalled() {
        // A child that waits for its input: it ends when we kill it.
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let mut child = Command::new(format!("{windows}\\System32\\cmd.exe"))
            .args(["/c", "pause"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("start a child process");
        let (sender, receiver) = mpsc::channel();
        let (woken, wakes) = mpsc::channel();
        let sink = Sink::new(
            sender,
            Arc::new(move || {
                let _ = woken.send(());
            }),
        );
        let Watch::Running(watch) = ExitWatch::start(child.id(), sink, 9) else {
            panic!("the child must be watchable");
        };
        assert!(!watch.has_exited());
        assert!(wakes.recv_timeout(Duration::from_millis(200)).is_err(), "no signal while it runs");

        child.kill().unwrap();
        let _ = child.wait();
        assert!(wakes.recv_timeout(Duration::from_secs(5)).is_ok(), "signalled when it ends");
        assert_eq!(receiver.try_recv(), Ok(Signal::Sessions { generation: 9 }));
        assert!(watch.has_exited());
        drop(watch);
    }

    #[test]
    fn unknown_processes_are_not_watched() {
        let (sender, receiver) = mpsc::channel();
        let sink = Sink::new(sender, Arc::new(|| {}));
        assert!(matches!(ExitWatch::start(0, sink.clone(), 1), Watch::Unwatchable));
        // Process identifiers are multiples of 4: this one never exists.
        assert!(matches!(ExitWatch::start(u32::MAX - 2, sink.clone(), 1), Watch::Gone));
        // An ended process is "gone" too, whether something still holds a handle on it
        // (here: `child`) or not.
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let mut child = Command::new(format!(r"{windows}\System32\cmd.exe"))
            .args(["/c", "exit"])
            .spawn()
            .expect("start a child process");
        let pid = child.id();
        let _ = child.wait();
        for _ in 0..50 {
            assert!(matches!(ExitWatch::start(pid, sink.clone(), 1), Watch::Gone));
        }
        drop(child);
        assert!(matches!(ExitWatch::start(pid, sink, 1), Watch::Gone));
        // No signal for a process that had already ended: that would make the session
        // list re-read itself for nothing, again and again.
        std::thread::sleep(Duration::from_millis(200));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_watch_can_be_dropped_while_the_process_runs() {
        let (sender, receiver) = mpsc::channel();
        let sink = Sink::new(sender, Arc::new(|| {}));
        let Watch::Running(watch) = ExitWatch::start(std::process::id(), sink, 2) else {
            panic!("we can watch ourselves");
        };
        assert!(!watch.has_exited());
        drop(watch);
        assert!(receiver.try_recv().is_err());
    }
}
