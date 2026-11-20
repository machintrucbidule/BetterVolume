//! The Microsoft Store version (FR-134): this executable running from its Store package.
//!
//! A packaged process sees its writes under `%LOCALAPPDATA%` and `HKEY_CURRENT_USER`
//! redirected to a private place of the package, invisible to other programs. So the
//! Store version keeps its settings in the package's own folder (removed by Windows at
//! uninstall), starts with Windows through the package's startup task (a `Run` entry it
//! wrote would be seen by nobody), and has the taskbar load the helper library straight
//! from the package folder (a copy in the data folder would be invisible to the taskbar).

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::core::{HSTRING, PWSTR};
use windows::ApplicationModel::{StartupTask, StartupTaskState};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;

/// The startup task declared in the package manifest (`packaging/store`).
const STARTUP_TASK: &str = "BetterVolume";

/// The family name of this process's package, or `None` when it does not run from a
/// package (installed or portable copy, development build). Read once.
pub fn package_family() -> Option<&'static str> {
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY.get_or_init(read_family).as_deref()
}

/// Whether this is the Store version.
pub fn is_packaged() -> bool {
    package_family().is_some()
}

fn read_family() -> Option<String> {
    // Family names are at most 64 characters ("PACKAGE_FAMILY_NAME_MAX_LENGTH").
    let mut buffer = [0u16; 128];
    let mut len = buffer.len() as u32;
    // SAFETY: `len` holds the buffer's size in characters; the buffer outlives the call.
    let status = unsafe { GetCurrentPackageFamilyName(&mut len, Some(PWSTR(buffer.as_mut_ptr()))) };
    if status != ERROR_SUCCESS {
        // APPMODEL_ERROR_NO_PACKAGE: not packaged.
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end])).filter(|f| !f.is_empty())
}

/// The package's own data folder, `%LOCALAPPDATA%\Packages\<family>\LocalState`, for the
/// settings and the log of the Store version; `None` when not packaged.
pub fn package_data_folder() -> Option<PathBuf> {
    let family = package_family()?;
    let base = std::env::var_os("LOCALAPPDATA").filter(|b| !b.is_empty())?;
    Some(PathBuf::from(base).join("Packages").join(family).join("LocalState"))
}

fn startup_task() -> Option<StartupTask> {
    let task = StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).and_then(|op| op.join());
    match task {
        Ok(task) => Some(task),
        Err(error) => {
            crate::log::error(&format!("startup task unavailable: {error}"));
            None
        }
    }
}

fn is_on(state: StartupTaskState) -> bool {
    state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
}

/// Runs `work` on a short-lived thread of its own and waits for it. The startup task's
/// calls are asynchronous Windows operations: waiting for one on the UI thread (a
/// single-threaded COM apartment that is not pumping messages meanwhile) could wait
/// forever if its completion needed that thread.
fn off_ui_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    std::thread::Builder::new().name("bv-startup-task".into()).spawn(work).ok()?.join().ok()
}

/// Whether the package's startup task is on (FR-074, Store version).
pub(crate) fn autostart_enabled() -> bool {
    off_ui_thread(|| startup_task().and_then(|task| task.State().ok()).is_some_and(is_on))
        .unwrap_or(false)
}

/// Turns the startup task on or off. Windows never turns on a task the user turned off in
/// its "Startup apps" page: then that page is opened, for the user to do it there.
/// Returns whether the task is now as asked.
pub(crate) fn set_autostart(on: bool) -> bool {
    let state = off_ui_thread(move || {
        let task = startup_task()?;
        if !on {
            // Read again: a task an administrator keeps on stays on.
            return Some(task.Disable().and_then(|()| task.State()));
        }
        Some(task.RequestEnableAsync().and_then(|op| op.join()))
    })
    .flatten();
    match state {
        Some(Ok(state)) => {
            if on && state == StartupTaskState::DisabledByUser {
                crate::system::SystemPage::StartupApps.open();
            }
            is_on(state) == on
        }
        Some(Err(error)) => {
            crate::log::error(&format!("cannot change the startup task: {error}"));
            false
        }
        None => false,
    }
}

/// `PROCESS_CREATION_DESKTOP_APP_BREAKAWAY_ENABLE_PROCESS_TREE`: the processes the new
/// process starts run outside the package's redirection.
const BREAKAWAY_CHILDREN: u32 = 0x01;

/// Starts `exe` with `args` (plain words, no quotes or spaces) so that the processes *it*
/// starts run outside the package's redirection. Windows runs the processes a Store app
/// starts inside its redirection, and offers no way to start one outside directly: only
/// the grandchildren can be. Returns whether the process started.
pub(crate) fn spawn_with_free_children(exe: &std::path::Path, args: &[&str]) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        UpdateProcThreadAttribute, CREATE_NO_WINDOW, EXTENDED_STARTUPINFO_PRESENT,
        LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY, STARTUPINFOEXW,
    };
    if args.iter().any(|a| a.is_empty() || a.contains(['"', ' ', '\t'])) {
        return false;
    }
    let mut line: Vec<u16> =
        format!("\"{}\" {}", exe.display(), args.join(" ")).encode_utf16().chain([0]).collect();
    let mut size = 0usize;
    // SAFETY: the documented size query (no list, one attribute); it reports an error.
    let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
    let mut storage = vec![0u8; size.max(1)];
    let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
    // SAFETY: `storage` holds the size Windows asked for and outlives the list's use.
    if unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }.is_err() {
        return false;
    }
    let policy = BREAKAWAY_CHILDREN;
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: `policy` and `list` live until the list is deleted below; the command line
    // is a writable NUL-terminated buffer; the structures are valid for the call.
    let started = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_DESKTOP_APP_POLICY as usize,
            Some((&raw const policy).cast()),
            std::mem::size_of::<u32>(),
            None,
            None,
        )
        .and_then(|()| {
            let mut startup = STARTUPINFOEXW::default();
            startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = list;
            CreateProcessW(
                PCWSTR::null(),
                Some(windows::core::PWSTR(line.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                None,
                PCWSTR::null(),
                &startup.StartupInfo,
                &mut info,
            )
        })
    };
    // SAFETY: the list initialised above, no longer used.
    unsafe { DeleteProcThreadAttributeList(list) };
    if started.is_err() {
        return false;
    }
    // SAFETY: handles returned by CreateProcessW, closed once.
    unsafe {
        let _ = CloseHandle(info.hThread);
        let _ = CloseHandle(info.hProcess);
    }
    true
}
