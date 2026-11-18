//! Hiding the Windows volume icon (FR-018, ARCHITECTURE §7).
//!
//! The work inside the taskbar is done by the `bv-taskbar-helper` library, which Windows
//! loads into the taskbar process through the documented XAML Diagnostics entry point
//! (`InitializeXamlDiagnosticsEx`). This module is the outside half:
//!
//! - **Loading** happens in a short-lived child process (this executable started with
//!   [`LOADER_ARGUMENT`], see [`run_loader`]), so the application itself never loads the
//!   XAML library (footprint) and a failing load cannot disturb it. The child copies the
//!   helper library, named after its content, to the data folder and has that copy loaded:
//!   the taskbar keeps a loaded library open until it restarts, and the copy keeps the
//!   installed file free for updates.
//! - **Outcomes** come back as a registered window message from the helper ("hidden",
//!   "restored", "not found", "failed"), or from the thread that waited for a child that
//!   failed; the shell turns them into [`Event::VolumeIcon`](crate::Event::VolumeIcon).
//! - **Guard rails** (§7): off unless asked; restored on quit (and, by the helper itself,
//!   whenever this process ends in any way); loaded again when the taskbar is re-created;
//!   when the taskbar process crashes shortly after a load, the shell gives up and reports
//!   [`Event::VolumeIconIncident`](crate::Event::VolumeIconIncident).

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{w, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_FOUND, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, KillTimer, PostMessageW, RegisterWindowMessageW,
    SetTimer,
};

use crate::host::Shell;
use crate::install::HELPER_FILE;

/// First argument of the executable when it runs as the loader child.
pub const LOADER_ARGUMENT: &str = "--taskbar-helper";

/// The loader's helper argument meaning "the payload of this executable" (the portable
/// file carries the library inside).
const EMBEDDED_HELPER: &str = "embedded";
/// Prefix of its copies in the data folder (followed by a content hash).
const COPY_PREFIX: &str = "bv_taskbar_helper-";
/// Sub-folder of the data folder holding the copies.
const COPY_FOLDER: &str = "taskbar-helper";

/// CLSID of the helper's tree-automation provider (`bv-taskbar-helper`,
/// `CLSID_BV_TASKBAR_HELPER`; the executable does not link that library).
const HELPER_CLSID: GUID = GUID::from_u128(0x6d8c3f2a_9e41_4b7d_b2a6_5c1e0f38a7c4);

/// Outcome codes of the registered message (mirror of `bv_taskbar_helper::status`).
const STATUS_HIDDEN: usize = 1;
const STATUS_RESTORED: usize = 2;
const STATUS_NOT_FOUND: usize = 3;
const STATUS_FAILED: usize = 4;

/// Timer that loads the helper a little after the taskbar was (re-)created.
pub(crate) const TIMER_VOLUME_ICON: usize = 6;
/// Delay before loading into a taskbar that was just created: its icons are being built.
const RELOAD_DELAY_MS: u32 = 3000;
/// Attempts after a failed load (the taskbar may not be ready yet), each after the delay.
const RETRIES: u32 = 2;
/// A taskbar that crashes within this time after the helper was loaded is an incident.
const INCIDENT_WINDOW: Duration = Duration::from_secs(30);

/// What became of a request to hide or restore the Windows volume icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeIconOutcome {
    /// The Windows volume icon is hidden.
    Hidden,
    /// The Windows volume icon is shown again.
    Restored,
    /// The taskbar has no volume icon this version of BetterVolume recognises (unknown
    /// layout after a Windows update): nothing was changed.
    NotFound,
    /// The helper could not be loaded or could not follow the taskbar: nothing was
    /// changed.
    Failed,
}

/// The registered message carrying outcomes ("BetterVolume.TaskbarHelper").
pub(crate) fn status_message() -> u32 {
    // SAFETY: a constant NUL-terminated name.
    unsafe { RegisterWindowMessageW(w!("BetterVolume.TaskbarHelper")) }
}

/// Decodes an outcome message.
pub(crate) fn outcome(wparam: WPARAM) -> Option<VolumeIconOutcome> {
    match wparam.0 {
        STATUS_HIDDEN => Some(VolumeIconOutcome::Hidden),
        STATUS_RESTORED => Some(VolumeIconOutcome::Restored),
        STATUS_NOT_FOUND => Some(VolumeIconOutcome::NotFound),
        STATUS_FAILED => Some(VolumeIconOutcome::Failed),
        _ => None,
    }
}

/// The taskbar process the helper was last loaded into to hide the icon, watched for the
/// incident rule from the moment the load starts (a crash while the helper works in it,
/// before any outcome, counts).
struct Loaded {
    process: HANDLE,
    pid: u32,
    at: Instant,
}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: our own handle.
        let _ = unsafe { CloseHandle(self.process) };
    }
}

/// The shell's side of the feature.
#[derive(Default)]
pub(crate) struct VolumeIconState {
    /// Whether the user wants the Windows icon hidden.
    wanted: Cell<bool>,
    /// The data folder (where the helper copies go); set with the first request.
    folder: RefCell<Option<PathBuf>>,
    /// The taskbar the helper was last loaded into to hide the icon.
    loaded: RefCell<Option<Loaded>>,
    /// Attempts left after a failure.
    retries: Cell<u32>,
    /// Taskbars in a row that ended (any way) shortly after a load.
    quick_ends: Cell<u32>,
}

/// Opens the taskbar process `pid` for the incident rule (waiting and exit-code rights).
fn watch_taskbar(pid: u32) -> Option<Loaded> {
    if pid == 0 {
        return None;
    }
    // SAFETY: plain call; the handle is closed when the `Loaded` is dropped.
    let process =
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .ok()?;
    Some(Loaded { process, pid, at: Instant::now() })
}

/// The process of the current taskbar, if there is one.
fn taskbar_pid() -> u32 {
    // SAFETY: a constant class name; a null window name matches by class only.
    let Ok(tray) = (unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }) else {
        return 0;
    };
    let mut pid = 0u32;
    // SAFETY: a valid window handle and out-pointer.
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    pid
}

/// The incident rule: the taskbar ended shortly after the helper was loaded
/// into it, with a crash code, or for the second time in a row.
pub(crate) fn is_incident(ended_soon: bool, code: u32, quick_ends: u32) -> bool {
    ended_soon && (is_crash(code) || quick_ends >= 2)
}

/// Whether an exit code is that of a crash (an NTSTATUS error), not of a process ended on
/// purpose (Task Manager, `taskkill`, an Explorer restart: 0, 1 or -1).
pub(crate) fn is_crash(code: u32) -> bool {
    code & 0xC000_0000 == 0xC000_0000 && code != u32::MAX
}

impl Shell {
    /// Hides (`true`) or shows again (`false`) the Windows volume icon (FR-018). The
    /// outcome comes later as [`Event::VolumeIcon`](crate::Event::VolumeIcon). `folder`
    /// is the data folder, where the helper library is copied before being loaded.
    pub fn set_volume_icon_hidden(&self, hidden: bool, folder: &Path) {
        let state = &self.volume_icon;
        *state.folder.borrow_mut() = Some(folder.to_path_buf());
        let was = state.wanted.replace(hidden);
        if !hidden && !was {
            return;
        }
        state.retries.set(RETRIES);
        self.cancel_volume_icon_timer();
        self.start_loader(hidden);
    }

    /// The application is quitting: show the Windows icon again (the helper would also do
    /// it by itself once this process is gone).
    pub(crate) fn restore_volume_icon_on_quit(&self) {
        if self.volume_icon.wanted.replace(false) {
            self.start_loader(false);
        }
    }

    /// The taskbar was (re-)created. If the previous taskbar crashed shortly after the
    /// helper was loaded into it, give up (incident): returns `true` and the option
    /// must be turned off. Otherwise, when the icon is wanted hidden, load the helper into
    /// the new taskbar a little later.
    pub(crate) fn volume_icon_taskbar_created(&self) -> bool {
        let state = &self.volume_icon;
        if !state.wanted.get() {
            return false;
        }
        let loaded = state.loaded.borrow_mut().take();
        if let Some(loaded) = loaded {
            // SAFETY: a process handle we own (SYNCHRONIZE and query rights).
            let ended = unsafe { WaitForSingleObject(loaded.process, 0) } == WAIT_OBJECT_0;
            if ended {
                let mut code = 0u32;
                // SAFETY: as above; a valid out-pointer.
                let _ = unsafe { GetExitCodeProcess(loaded.process, &mut code) };
                let soon = loaded.at.elapsed() < INCIDENT_WINDOW;
                let quick_ends = if soon { state.quick_ends.get() + 1 } else { 0 };
                state.quick_ends.set(quick_ends);
                if is_incident(soon, code, quick_ends) {
                    crate::log::error(&format!(
                        "taskbar ended (0x{code:08X}) shortly after the volume icon helper was loaded: option turned off"
                    ));
                    state.wanted.set(false);
                    state.quick_ends.set(0);
                    return true;
                }
            } else {
                // The same taskbar (scaling change): keep watching it.
                if loaded.at.elapsed() >= INCIDENT_WINDOW {
                    state.quick_ends.set(0);
                }
                *state.loaded.borrow_mut() = Some(loaded);
            }
        }
        state.retries.set(RETRIES);
        self.schedule_volume_icon_load();
        false
    }

    /// An outcome arrived (helper message, or failed child). Returns the outcome to report,
    /// or `None` when it is stale or a retry is under way.
    pub(crate) fn volume_icon_outcome(
        &self,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<VolumeIconOutcome> {
        let state = &self.volume_icon;
        let outcome = outcome(wparam)?;
        match outcome {
            VolumeIconOutcome::Hidden => {
                if !state.wanted.get() {
                    // Hidden after the user changed their mind: undo.
                    self.start_loader(false);
                    return None;
                }
                // Normally the taskbar watched since the load started; the window of the
                // incident rule then runs from now.
                let pid = u32::try_from(lparam.0).unwrap_or(0);
                let mut loaded = state.loaded.borrow_mut();
                match loaded.as_mut() {
                    Some(watched) if watched.pid == pid => watched.at = Instant::now(),
                    _ => *loaded = watch_taskbar(pid),
                }
            }
            VolumeIconOutcome::Restored => {
                *state.loaded.borrow_mut() = None;
                if state.wanted.get() {
                    // Restored by an older request after the user asked again.
                    self.start_loader(true);
                    return None;
                }
            }
            // A failed load is tried again, to hide as to restore (a restore that keeps
            // failing leaves the icon to the helper's owner watch: it comes back at the
            // latest when BetterVolume ends).
            VolumeIconOutcome::Failed if state.retries.get() > 0 => {
                state.retries.set(state.retries.get() - 1);
                self.schedule_volume_icon_load();
                return None;
            }
            VolumeIconOutcome::NotFound | VolumeIconOutcome::Failed => {}
        }
        Some(outcome)
    }

    /// The reload timer fired: load again for what is wanted now.
    pub(crate) fn volume_icon_timer(&self) {
        self.cancel_volume_icon_timer();
        self.start_loader(self.volume_icon.wanted.get());
    }

    fn schedule_volume_icon_load(&self) {
        // SAFETY: our own window; a one-shot use (killed when it fires).
        unsafe { SetTimer(Some(self.window), TIMER_VOLUME_ICON, RELOAD_DELAY_MS, None) };
    }

    fn cancel_volume_icon_timer(&self) {
        // SAFETY: our own window; killing a timer that does not run only fails.
        let _ = unsafe { KillTimer(Some(self.window), TIMER_VOLUME_ICON) };
    }

    /// Starts the loader child; a thread waits for it and reports a failure as an outcome
    /// message (a success is reported by the helper itself).
    fn start_loader(&self, hide: bool) {
        let Some(folder) = self.volume_icon.folder.borrow().clone() else { return };
        if hide {
            // Watched from now: a crash while the helper works in it counts.
            let pid = taskbar_pid();
            let mut loaded = self.volume_icon.loaded.borrow_mut();
            match loaded.as_mut() {
                Some(watched) if watched.pid == pid => {
                    // That taskbar outlived its last load: earlier quick ends are history.
                    if watched.at.elapsed() >= INCIDENT_WINDOW {
                        self.volume_icon.quick_ends.set(0);
                    }
                    watched.at = Instant::now();
                }
                _ => *loaded = watch_taskbar(pid),
            }
        }
        let window = self.window.0 as isize;
        let spawned = (|| {
            let exe = std::env::current_exe().ok()?;
            // The library beside the executable, or inside it (portable file).
            let helper = match crate::package::Package::own() {
                Some(package) if package.kind == bv_core::package::Kind::Portable => {
                    std::ffi::OsString::from(EMBEDDED_HELPER)
                }
                _ => exe.parent()?.join(HELPER_FILE).into_os_string(),
            };
            std::process::Command::new(&exe)
                .arg(LOADER_ARGUMENT)
                .arg(if hide { "hide" } else { "restore" })
                .arg(std::process::id().to_string())
                .arg(window.to_string())
                .arg(&helper)
                .arg(folder.join(COPY_FOLDER))
                .spawn()
                .ok()
        })();
        let Some(mut child) = spawned else {
            crate::log::error("volume icon helper: loader not started");
            post_outcome(window, STATUS_FAILED);
            return;
        };
        let _ = std::thread::Builder::new().name("bv-taskbar-loader".into()).spawn(move || {
            let ok = child.wait().is_ok_and(|status| status.success());
            if !ok {
                post_outcome(window, STATUS_FAILED);
            }
        });
    }
}

/// Posts an outcome to the message window (from any thread).
fn post_outcome(window: isize, status: usize) {
    let message = status_message();
    if message != 0 {
        // SAFETY: posting to a window that may be gone only fails.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(window as *mut core::ffi::c_void)),
                message,
                WPARAM(status),
                LPARAM(0),
            )
        };
    }
}

/// `InitializeXamlDiagnosticsEx`, exported by `Windows.UI.Xaml.dll`.
type InitFn = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;

/// Endpoints tried: each XAML core of the taskbar registers "VisualDiagConnection<N>";
/// `ERROR_NOT_FOUND` means no core has that number, so the next one is tried.
const MAX_ENDPOINTS: u32 = 16;

/// Why the loader child failed (its exit code).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LoaderError {
    Arguments = 2,
    NoTaskbar = 3,
    Copy = 4,
    Xaml = 5,
    Load = 6,
}

/// The loader child: `<exe> --taskbar-helper <hide|restore> <owner pid> <owner window>
/// <helper library> <copy folder>`. Copies the library, has the taskbar load the copy
/// with the instruction, and returns the process exit code (0 = loaded).
pub fn run_loader(args: &[String]) -> i32 {
    match load(args) {
        Ok(()) => 0,
        Err(error) => {
            crate::log::error(&format!("volume icon helper not loaded: {error:?}"));
            error as i32
        }
    }
}

fn load(args: &[String]) -> Result<(), LoaderError> {
    let [command, owner, window, helper, folder] = args else {
        return Err(LoaderError::Arguments);
    };
    if !matches!(command.as_str(), "hide" | "restore")
        || owner.parse::<u32>().is_err()
        || window.parse::<isize>().is_err()
    {
        return Err(LoaderError::Arguments);
    }
    let bytes = if helper == EMBEDDED_HELPER {
        crate::package::Package::own().and_then(|p| p.payload().ok())
    } else {
        std::fs::read(helper).ok()
    };
    let copy =
        bytes.and_then(|bytes| copy_helper(&bytes, Path::new(folder))).ok_or(LoaderError::Copy)?;

    // SAFETY: a constant class name; a null window name matches by class only.
    let tray = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) }
        .map_err(|_| LoaderError::NoTaskbar)?;
    let mut pid = 0u32;
    // SAFETY: a valid window handle and out-pointer.
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    if pid == 0 {
        return Err(LoaderError::NoTaskbar);
    }

    // SAFETY: a system library by name, from System32 only.
    let xaml =
        unsafe { LoadLibraryExW(w!("Windows.UI.Xaml.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|_| LoaderError::Xaml)?;
    // SAFETY: a valid module and a NUL-terminated export name.
    let proc = unsafe { GetProcAddress(xaml, windows::core::s!("InitializeXamlDiagnosticsEx")) }
        .ok_or(LoaderError::Xaml)?;
    // SAFETY: the export has the InitializeXamlDiagnosticsEx signature.
    let init = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, InitFn>(proc) };

    let path = wide(&copy.to_string_lossy());
    let data = wide(&format!("{command} {owner} {window}"));
    for n in 1..=MAX_ENDPOINTS {
        let endpoint = wide(&format!("VisualDiagConnection{n}"));
        // SAFETY: valid NUL-terminated strings for the duration of the call.
        let hr = unsafe {
            init(
                PCWSTR(endpoint.as_ptr()),
                pid,
                PCWSTR::null(),
                PCWSTR(path.as_ptr()),
                HELPER_CLSID,
                PCWSTR(data.as_ptr()),
            )
        };
        if hr.is_ok() {
            return Ok(());
        }
        if hr != HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            crate::log::error(&format!("volume icon helper: load refused (0x{:08X})", hr.0 as u32));
            return Err(LoaderError::Load);
        }
    }
    Err(LoaderError::Load)
}

/// Copies the helper library (`bytes`) to `folder`, named after its content, unless that copy is
/// already there; removes older copies that are no longer in use (a copy loaded in the
/// taskbar stays until the taskbar restarts). Returns the copy's path.
fn copy_helper(bytes: &[u8], folder: &Path) -> Option<PathBuf> {
    let name = format!("{COPY_PREFIX}{:016x}.dll", fnv1a(bytes));
    let copy = folder.join(&name);
    std::fs::create_dir_all(folder).ok()?;
    let complete = |path: &Path| std::fs::read(path).ok().as_deref() == Some(bytes);
    if !complete(&copy) {
        // Written beside, then renamed: a half-written library is never loaded. The
        // temporary name is this process's own (two loaders may run at once).
        let partial = folder.join(format!("{name}.{}.partial", std::process::id()));
        let written =
            std::fs::write(&partial, bytes).and_then(|()| std::fs::rename(&partial, &copy));
        if written.is_err() {
            let _ = std::fs::remove_file(&partial);
            // Another loader may have put the same copy in place meanwhile.
            if !complete(&copy) {
                return None;
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            // Older copies only: a temporary file belongs to a loader still writing it.
            if file.starts_with(COPY_PREFIX) && file.ends_with(".dll") && file != name {
                // Fails while the taskbar holds it: it goes at a later load.
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Some(copy)
}

/// FNV-1a 64-bit hash: a content name for the copies (not a security measure).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crashes_are_told_from_processes_ended_on_purpose() {
        assert!(is_crash(0xC000_0005)); // access violation
        assert!(is_crash(0xC000_0409)); // stack buffer overrun (fail fast)
        assert!(!is_crash(0));
        assert!(!is_crash(1)); // taskkill, Task Manager
        assert!(!is_crash(u32::MAX)); // TerminateProcess(-1), PowerShell Stop-Process
        assert!(!is_crash(0x8000_0003));
    }

    #[test]
    fn an_incident_is_a_crash_or_a_second_quick_end_soon_after_a_load() {
        assert!(is_incident(true, 0xC000_0005, 1));
        assert!(!is_incident(true, 1, 1), "one restart on purpose");
        assert!(is_incident(true, 1, 2), "a second one in a row");
        assert!(!is_incident(false, 0xC000_0005, 1), "long after the load");
        assert!(!is_incident(false, 1, 0));
    }

    #[test]
    fn outcomes_decode() {
        assert_eq!(outcome(WPARAM(1)), Some(VolumeIconOutcome::Hidden));
        assert_eq!(outcome(WPARAM(2)), Some(VolumeIconOutcome::Restored));
        assert_eq!(outcome(WPARAM(3)), Some(VolumeIconOutcome::NotFound));
        assert_eq!(outcome(WPARAM(4)), Some(VolumeIconOutcome::Failed));
        assert_eq!(outcome(WPARAM(0)), None);
        assert_eq!(outcome(WPARAM(5)), None);
    }

    #[test]
    fn the_loader_refuses_bad_arguments() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(load(&args(&[])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["show", "1", "2", "a", "b"])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["hide", "x", "2", "a", "b"])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["hide", "1", "2", "a"])), Err(LoaderError::Arguments));
    }

    #[test]
    fn copies_are_named_after_their_content_and_old_ones_go() {
        let base = std::env::temp_dir().join(format!("bv-helper-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("copies");
        std::fs::create_dir_all(&base).expect("folder");
        let first = copy_helper(b"first", &folder).expect("copied");
        assert_eq!(std::fs::read(&first).expect("read"), b"first");
        // Same content: same copy.
        assert_eq!(copy_helper(b"first", &folder), Some(first.clone()));

        let second = copy_helper(b"second", &folder).expect("copied");
        assert_ne!(first, second);
        assert!(!first.exists(), "the unused older copy is removed");
        assert!(second.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_hash_is_stable() {
        // Known FNV-1a 64 values.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn the_clsid_matches_the_helper() {
        assert_eq!(format!("{HELPER_CLSID:?}"), "6D8C3F2A-9E41-4B7D-B2A6-5C1E0F38A7C4");
    }
}
