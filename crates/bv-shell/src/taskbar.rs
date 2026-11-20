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

/// Prefix of its copies in the data folder (followed by a content hash).
const COPY_PREFIX: &str = "bv_taskbar_helper-";
/// Sub-folder of the data folder holding the copies.
const COPY_FOLDER: &str = "taskbar-helper";
/// A copy changed less than this ago is in a loader's hands and is never removed.
const COPY_FRESH: Duration = Duration::from_secs(60);

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
    /// Loads started whose outcome has not arrived yet: each load reports once (the
    /// helper, or the thread waiting for a child that failed). An outcome nobody asked
    /// for (another program posting the message) is ignored.
    pending: Cell<u32>,
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

/// Whether an outcome sent by process `from` (0: none, a failed load reported by this
/// process) answers a load this process started: one is awaited (`pending`), and it comes
/// from the taskbar (`taskbar` is asked only then).
fn outcome_expected(pending: u32, from: u32, taskbar: impl FnOnce() -> u32) -> bool {
    pending > 0 && (from == 0 || from == taskbar())
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

    /// The taskbar was laid out again without being re-created (its position, its size or
    /// auto-hide changed): Windows then builds a new, visible volume icon. When the icon
    /// is wanted hidden, hide it again a little later (repeated changes push the load back,
    /// so it runs once the taskbar has settled).
    pub(crate) fn volume_icon_layout_changed(&self) {
        let state = &self.volume_icon;
        if !state.wanted.get() {
            return;
        }
        state.retries.set(RETRIES);
        self.schedule_volume_icon_load();
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
        // Only an outcome of a load this process started counts, and only from the
        // taskbar (or, for a failed load, from this process: no process id).
        let from = u32::try_from(lparam.0).unwrap_or(0);
        if !outcome_expected(state.pending.get(), from, taskbar_pid) {
            crate::log::info("volume icon helper: an outcome nobody asked for was ignored");
            return None;
        }
        state.pending.set(state.pending.get() - 1);
        match outcome {
            VolumeIconOutcome::Hidden => {
                if !state.wanted.get() {
                    // Hidden after the user changed their mind: undo.
                    self.start_loader(false);
                    return None;
                }
                // Normally the taskbar watched since the load started; the window of the
                // incident rule then runs from now.
                let mut loaded = state.loaded.borrow_mut();
                match loaded.as_mut() {
                    Some(watched) if watched.pid == from => watched.at = Instant::now(),
                    _ => *loaded = watch_taskbar(from),
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
        // Every path below ends in exactly one outcome message.
        self.volume_icon.pending.set(self.volume_icon.pending.get().saturating_add(1));
        let spawned = (|| {
            let exe = std::env::current_exe().ok()?;
            std::process::Command::new(&exe)
                .arg(LOADER_ARGUMENT)
                .arg(if hide { "hide" } else { "restore" })
                .arg(std::process::id().to_string())
                .arg(window.to_string())
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
/// <copy folder>`. Copies the library, has the taskbar load the copy with the
/// instruction, and returns the process exit code (0 = loaded). The library is always this
/// executable's own ([`own_helper`]), never one named on the command line: the loader can
/// not be used to have the taskbar load any other library.
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
    let [command, owner, window, folder] = args else {
        return Err(LoaderError::Arguments);
    };
    if !matches!(command.as_str(), "hide" | "restore")
        || owner.parse::<u32>().is_err()
        || window.parse::<isize>().is_err()
    {
        return Err(LoaderError::Arguments);
    }
    let copy = own_helper()
        .and_then(|bytes| copy_helper(&bytes, Path::new(folder)))
        .ok_or(LoaderError::Copy)?;

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

/// The bytes of this executable's own helper library: inside the file for the portable
/// file, beside it otherwise (the installed program and development builds).
fn own_helper() -> Option<Vec<u8>> {
    match crate::package::Package::own() {
        Some(package) if package.kind == bv_core::package::Kind::Portable => package.payload().ok(),
        _ => std::fs::read(std::env::current_exe().ok()?.parent()?.join(HELPER_FILE)).ok(),
    }
}

/// Copies the helper library (`bytes`) to `folder`, named after its content, unless that copy is
/// already there; removes older copies that are no longer in use ([`remove_copies`]).
/// Returns the copy's path.
fn copy_helper(bytes: &[u8], folder: &Path) -> Option<PathBuf> {
    let name = format!("{COPY_PREFIX}{:016x}.dll", fnv1a(bytes));
    let copy = folder.join(&name);
    std::fs::create_dir_all(folder).ok()?;
    // An existing copy is marked as just used first, so another process tidying the folder
    // leaves it (the taskbar may not hold it yet). Fails harmlessly while the taskbar
    // holds it, or when there is no copy yet.
    let _ = std::fs::OpenOptions::new()
        .write(true)
        .open(&copy)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()));
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
    remove_copies(folder, Some(&name), COPY_FRESH);
    Some(copy)
}

/// Removes the copies of the helper library the taskbar no longer holds, and their folder
/// once empty. Called at start while the Windows volume icon is not to be hidden (no
/// loader runs then, so nothing else would remove them). `folder` is the data folder.
pub fn remove_unused_helper_copies(folder: &Path) {
    let folder = folder.join(COPY_FOLDER);
    remove_copies(&folder, None, COPY_FRESH);
    // Fails while a copy is still there (held by the taskbar): it goes at a later start.
    let _ = std::fs::remove_dir(&folder);
}

/// Removes the copies in `folder` except `keep` and those changed less than `fresh` ago (just
/// written or reused by a loader that has not had the taskbar load it yet, perhaps of
/// another copy of BetterVolume sharing the folder). A temporary file is removed on the same
/// terms: a fresh one belongs to a loader still writing it, an old one to a loader that
/// ended halfway. A copy the taskbar holds cannot be deleted: it goes once the taskbar has
/// restarted.
fn remove_copies(folder: &Path, keep: Option<&str>, fresh: Duration) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        let ours = file.ends_with(".dll") || file.ends_with(".partial");
        if !file.starts_with(COPY_PREFIX) || !ours || Some(&*file) == keep {
            continue;
        }
        let age = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok());
        // Unknown age (clock moved back): treated as fresh, kept for a later pass.
        if age.is_some_and(|age| age >= fresh) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
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
    fn only_awaited_outcomes_from_the_taskbar_count() {
        // Nothing awaited: ignored, whoever sent it.
        assert!(!outcome_expected(0, 0, || 7));
        assert!(!outcome_expected(0, 7, || 7));
        // Awaited: from the taskbar, or a failed load reported by this process.
        assert!(outcome_expected(1, 7, || 7));
        assert!(outcome_expected(2, 0, || 7));
        // From any other process: ignored.
        assert!(!outcome_expected(1, 8, || 7));
        assert!(!outcome_expected(1, 8, || 0));
    }

    #[test]
    fn the_loader_refuses_bad_arguments() {
        let args = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(load(&args(&[])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["show", "1", "2", "b"])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["hide", "x", "2", "b"])), Err(LoaderError::Arguments));
        assert_eq!(load(&args(&["hide", "1", "2"])), Err(LoaderError::Arguments));
        // A library named on the command line is refused: only the five-part form of
        // older versions had one, and it is no longer understood.
        assert_eq!(
            load(&args(&["hide", "1", "2", r"C:\elsewhere\other.dll", "b"])),
            Err(LoaderError::Arguments)
        );
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
        assert!(first.exists(), "a copy used less than a minute ago stays");
        set_age(&first, COPY_FRESH * 2);
        assert_eq!(copy_helper(b"second", &folder), Some(second.clone()));
        assert!(!first.exists(), "the unused older copy is removed");
        assert!(second.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Dates the last change of `path` back by `age`.
    fn set_age(path: &Path, age: Duration) {
        let file = std::fs::OpenOptions::new().write(true).open(path).expect("open");
        file.set_modified(std::time::SystemTime::now() - age).expect("dated");
    }

    #[test]
    fn a_reused_copy_is_marked_as_just_used() {
        let base = std::env::temp_dir().join(format!("bv-helper-reuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let copy = copy_helper(b"same", &base).expect("copied");
        set_age(&copy, COPY_FRESH * 2);
        copy_helper(b"same", &base).expect("reused");
        remove_copies(&base, None, COPY_FRESH);
        assert!(copy.exists(), "a reused copy is not removed before the taskbar loads it");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unused_copies_go_at_start_and_then_their_folder() {
        let base = std::env::temp_dir().join(format!("bv-helper-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join(COPY_FOLDER);
        std::fs::create_dir_all(&folder).expect("folder");
        let old = folder.join(format!("{COPY_PREFIX}0000000000000001.dll"));
        let fresh = folder.join(format!("{COPY_PREFIX}0000000000000002.dll"));
        let partial = folder.join(format!("{COPY_PREFIX}0000000000000003.dll.7.partial"));
        let stale = folder.join(format!("{COPY_PREFIX}0000000000000005.dll.8.partial"));
        let other = folder.join("notes.txt");
        for path in [&old, &fresh, &partial, &stale, &other] {
            std::fs::write(path, b"x").expect("written");
        }
        for path in [&old, &stale, &other] {
            set_age(path, COPY_FRESH * 2);
        }
        remove_unused_helper_copies(&base);
        assert!(!old.exists(), "an old copy goes");
        assert!(!stale.exists(), "a temporary file of a loader that ended halfway goes");
        assert!(fresh.exists(), "a copy a loader just wrote stays");
        assert!(partial.exists(), "a temporary file still being written stays");
        assert!(other.exists(), "nothing else is touched");

        for path in [&fresh, &partial, &other] {
            std::fs::remove_file(path).expect("removed");
        }
        let only = folder.join(format!("{COPY_PREFIX}0000000000000004.dll"));
        std::fs::write(&only, b"x").expect("written");
        set_age(&only, COPY_FRESH * 2);
        remove_unused_helper_copies(&base);
        assert!(!folder.exists(), "the empty folder goes too");
        // Nothing there at all: nothing happens.
        remove_unused_helper_copies(&base);
        assert!(base.exists());
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
