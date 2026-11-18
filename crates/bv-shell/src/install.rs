//! Installing BetterVolume for the current user, and removing it (FR-130, FR-131).
//!
//! An installation is: the program and the taskbar helper library in
//! `%LOCALAPPDATA%\Programs\BetterVolume`, a Start-menu entry (no desktop shortcut),
//! an entry in Windows' installed-apps list (whose "Uninstall" launches the
//! installed program with [`UNINSTALL_ARGUMENT`]) and, when chosen, the "start with
//! Windows" entry. Everything is per user: no administrator rights.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::{Interface, GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, FOLDERID_UserProgramFiles, IShellLinkW, SHGetKnownFolderPath, ShellLink,
    KF_FLAG_DONT_VERIFY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, PostMessageW,
};

use crate::host::{exit_message, WINDOW_CLASS};

/// Argument of the installed program that opens the uninstaller (Windows' installed-apps
/// list runs it).
pub const UNINSTALL_ARGUMENT: &str = "--uninstall";

/// File name of the installed program.
pub const PROGRAM_FILE: &str = "BetterVolume.exe";

/// File name of the taskbar helper library beside it (`bv-taskbar-helper`).
pub const HELPER_FILE: &str = "bv_taskbar_helper.dll";

/// Name of the installation folder, of the Start-menu entry and of the installed-apps
/// entry.
const NAME: &str = "BetterVolume";

/// The installed-apps entry (per user).
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\BetterVolume";

/// What is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// The installation folder.
    pub folder: PathBuf,
    /// The installed version, as the installed-apps entry says.
    pub version: String,
}

impl Installed {
    /// The installed program.
    pub fn program(&self) -> PathBuf {
        self.folder.join(PROGRAM_FILE)
    }
}

/// Why an installation could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The installation folder could not be found or created.
    Folder,
    /// The files could not be written (a copy of the program may still be running).
    Files,
    /// The Start-menu entry or the installed-apps entry could not be written.
    Entries,
}

/// The installation folder: `%LOCALAPPDATA%\Programs\BetterVolume`.
pub fn install_folder() -> Option<PathBuf> {
    known_folder(&FOLDERID_UserProgramFiles).map(|f| f.join(NAME))
}

/// The Start-menu entry.
fn shortcut_path() -> Option<PathBuf> {
    known_folder(&FOLDERID_Programs).map(|f| f.join(format!("{NAME}.lnk")))
}

/// The path of a known folder, whether or not it exists yet: on a new Windows account
/// `%LOCALAPPDATA%\Programs` is only created by the first per-user installation, and the
/// installation creates the folders it needs.
fn known_folder(id: &GUID) -> Option<PathBuf> {
    // SAFETY: a valid identifier; the returned string is freed below.
    let folder = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }.ok()?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { folder.to_string() }.ok();
    // SAFETY: allocated by the shell with the task allocator.
    unsafe { CoTaskMemFree(Some(folder.0 as *const _)) };
    text.filter(|t| !t.is_empty()).map(PathBuf::from)
}

/// The current installation, if the installed-apps entry exists and its program is there.
pub fn installed() -> Option<Installed> {
    let folder = PathBuf::from(crate::registry::read_text(UNINSTALL_KEY, "InstallLocation")?);
    let version = crate::registry::read_text(UNINSTALL_KEY, "DisplayVersion").unwrap_or_default();
    let installed = Installed { folder, version };
    installed.program().is_file().then_some(installed)
}

/// Whether this executable is the installed program.
pub fn running_installed() -> bool {
    let (Ok(exe), Some(installed)) = (std::env::current_exe(), installed()) else {
        return false;
    };
    same_path(&exe, &installed.program())
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    canonical(a).to_string_lossy().eq_ignore_ascii_case(&canonical(b).to_string_lossy())
}

/// Whether BetterVolume can be installed in `folder` without administrator rights:
/// an absolute path the uninstaller can name safely, in which (or, while it does
/// not exist yet, in whose nearest existing parent) this user can create a file.
pub fn folder_usable(folder: &Path) -> bool {
    let text = folder.to_string_lossy();
    // An existing file of that name cannot become the folder.
    if !folder.is_absolute() || text.contains(['"', '%']) || folder.is_file() {
        return false;
    }
    let Some(existing) = folder.ancestors().find(|p| p.is_dir()) else { return false };
    let probe = existing.join(format!("BetterVolume.{}.probe", std::process::id()));
    match std::fs::File::create_new(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Installs (or updates) BetterVolume in `folder`: writes the program and the helper
/// library, the Start-menu entry and the installed-apps entry, and sets the "start with
/// Windows" entry as asked. Any running copy must have been ended first
/// ([`end_running_instance`]). Returns the installed program.
pub fn install(
    folder: &Path,
    program: &[u8],
    helper: &[u8],
    version: &str,
    autostart: bool,
) -> Result<PathBuf, InstallError> {
    std::fs::create_dir_all(folder).map_err(|_| InstallError::Folder)?;
    let exe = folder.join(PROGRAM_FILE);
    // One wait for both files.
    let deadline = Instant::now() + IN_USE_WAIT;
    write_replacing(&folder.join(HELPER_FILE), helper, deadline)
        .map_err(|_| InstallError::Files)?;
    write_replacing(&exe, program, deadline).map_err(|_| InstallError::Files)?;

    let _com = ComScope::new();
    let shortcut = shortcut_path().ok_or(InstallError::Entries)?;
    create_shortcut(&shortcut, &exe, folder).map_err(|_| InstallError::Entries)?;
    let size_kb = (program.len() + helper.len()).div_ceil(1024);
    write_uninstall_entry(folder, &exe, version, size_kb).map_err(|_| InstallError::Entries)?;
    // A failure here is logged; the user can still turn it on in the settings window.
    // Turned off: only an entry of this program goes (a portable copy's stays).
    if autostart {
        crate::autostart::set_for(&exe, true);
    } else {
        crate::autostart::remove_for(&exe);
    }
    crate::log::info(&format!("installed version {version}"));
    Ok(exe)
}

/// How long files in use may keep the installation waiting: the program that just quit
/// may still run for a moment from it (the helper restoring the Windows volume icon takes
/// up to about ten seconds), or an antivirus may be reading it.
const IN_USE_WAIT: Duration = Duration::from_secs(20);

/// Windows errors of a file in use: access denied (a running program's file), sharing
/// and lock violations.
const IN_USE_ERRORS: [i32; 3] = [5, 32, 33];

/// Whether `error` says a file is in use (worth waiting for).
fn in_use(error: &std::io::Error) -> bool {
    error.raw_os_error().is_some_and(|code| IN_USE_ERRORS.contains(&code))
}

/// Writes `bytes` beside `path`, then renames the new file into place (a half-written
/// program is never left behind), trying again while the old file is in use, until
/// `deadline`. Any other failure is reported at once.
fn write_replacing(path: &Path, bytes: &[u8], deadline: Instant) -> std::io::Result<()> {
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    let mut written = std::fs::write(&partial, bytes);
    let mut waited = false;
    if written.is_ok() {
        written = loop {
            match std::fs::rename(&partial, path) {
                Ok(()) => break Ok(()),
                Err(error) if Instant::now() >= deadline || !in_use(&error) => break Err(error),
                Err(_) => {
                    waited = true;
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        };
    }
    if waited {
        let outcome = if written.is_ok() { "free again" } else { "still in use, giving up" };
        crate::log::info(&format!("install: a file to replace was in use; {outcome}"));
    }
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    written
}

/// COM for the calling thread, for as long as it lives.
struct ComScope(bool);

impl ComScope {
    fn new() -> ComScope {
        // SAFETY: plain call; balanced in `drop` when it succeeded.
        ComScope(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn create_shortcut(shortcut: &Path, exe: &Path, folder: &Path) -> windows::core::Result<()> {
    if let Some(parent) = shortcut.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // SAFETY: COM is initialised on this thread (`ComScope`); the strings outlive the calls.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
        link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0)?;
        let file: IPersistFile = link.cast()?;
        file.Save(&HSTRING::from(shortcut.as_os_str()), true)
    }
}

fn write_uninstall_entry(
    folder: &Path,
    exe: &Path,
    version: &str,
    size_kb: usize,
) -> windows::core::Result<()> {
    let exe_text = exe.to_string_lossy();
    let texts = [
        ("DisplayName", NAME.to_owned()),
        ("DisplayVersion", version.to_owned()),
        ("Publisher", NAME.to_owned()),
        ("DisplayIcon", format!("{exe_text},0")),
        ("InstallLocation", folder.to_string_lossy().into_owned()),
        ("UninstallString", format!("\"{exe_text}\" {UNINSTALL_ARGUMENT}")),
    ];
    let numbers = [
        ("NoModify", 1u32),
        ("NoRepair", 1),
        ("EstimatedSize", u32::try_from(size_kb).unwrap_or(u32::MAX)),
    ];
    let mut key = HKEY::default();
    // SAFETY: a NUL-terminated key name and a valid out-pointer; the key is closed below.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(UNINSTALL_KEY),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    }
    .ok()?;
    let mut result = Ok(());
    for (name, value) in texts {
        let data: Vec<u8> =
            value.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        // SAFETY: an open key; the data is a NUL-terminated UTF-16 string.
        let status =
            unsafe { RegSetValueExW(key, &HSTRING::from(name), None, REG_SZ, Some(&data)) };
        if result.is_ok() {
            result = status.ok();
        }
    }
    for (name, value) in numbers {
        // SAFETY: an open key; four bytes of data.
        let status = unsafe {
            RegSetValueExW(key, &HSTRING::from(name), None, REG_DWORD, Some(&value.to_le_bytes()))
        };
        if result.is_ok() {
            result = status.ok();
        }
    }
    // SAFETY: the key opened above, closed once.
    let _ = unsafe { RegCloseKey(key) };
    result
}

/// Removes what the installation registered: the "start with Windows" entry when it
/// launches the installed program, the Start-menu entry and the installed-apps entry
/// (FR-131). The files go once the uninstaller has exited ([`remove_files_after_exit`]).
pub fn remove_entries(installed: &Installed) {
    crate::autostart::remove_for(&installed.program());
    if let Some(shortcut) = shortcut_path() {
        let _ = std::fs::remove_file(shortcut);
    }
    // SAFETY: a NUL-terminated key name.
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(UNINSTALL_KEY)) };
    if status.is_err() && status != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
        crate::log::error("uninstall: the installed-apps entry could not be removed");
    }
}

/// Deletes the installed files, then the folder if it is empty, a moment after this
/// process ended (a running program cannot delete itself). Only the two known files are
/// deleted, never anything else the folder may hold.
pub fn remove_files_after_exit(installed: &Installed) {
    let folder = installed.folder.to_string_lossy().into_owned();
    // Refuse anything the command interpreter would read as syntax even inside quotes (a
    // quote ends the path, `%` expands a variable): the files then simply stay.
    if folder.is_empty() || folder.contains(['"', '%']) {
        crate::log::error("uninstall: the installation folder cannot be removed automatically");
        return;
    }
    let system =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
    // Every two seconds (`ping`), for at most about twenty, the two files are deleted; once
    // both are gone, `rd` removes the folder if it is empty and the command ends. The wait
    // covers this process ending and the short helper that restores the Windows volume
    // icon, which runs from the program file for a moment after BetterVolume quit.
    let program = format!("{folder}\\{PROGRAM_FILE}");
    let helper = format!("{folder}\\{HELPER_FILE}");
    let command = format!(
        "/d /c for /l %i in (1,1,10) do @(ping -n 3 127.0.0.1 >nul & del /f /q \"{program}\" \"{helper}\" 2>nul & if not exist \"{program}\" if not exist \"{helper}\" (rd \"{folder}\" 2>nul & exit))"
    );
    let started = std::process::Command::new(system.join("System32").join("cmd.exe"))
        .raw_arg(command)
        .current_dir(std::env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn();
    if started.is_err() {
        crate::log::error("uninstall: the files could not be scheduled for removal");
    }
}

/// Starts `exe` (the installed program, after an installation).
pub fn launch(exe: &Path) -> bool {
    let folder = exe.parent().map_or_else(std::env::temp_dir, Path::to_owned);
    std::process::Command::new(exe).current_dir(folder).spawn().is_ok()
}

/// Asks the running BetterVolume (installed or portable) to quit as from its menu, and
/// waits for it to end; one that does not end within `timeout` is ended by force (the
/// taskbar helper then restores the Windows volume icon by itself). Returns
/// whether no instance runs any more.
pub fn end_running_instance(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    // SAFETY: the class name is a valid NUL-terminated constant.
    let Ok(window) = (unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()) }) else {
        return true;
    };
    let mut pid = 0u32;
    // SAFETY: valid out pointer; a stale handle only makes the call fail.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    // SAFETY: plain call; the handle is closed below.
    let process = (pid != 0)
        .then(|| unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) }.ok())
        .flatten();
    let message = exit_message();
    if message != 0 {
        // SAFETY: posting to a window that may be gone only fails.
        let _ = unsafe { PostMessageW(Some(window), message, WPARAM(0), LPARAM(0)) };
    }
    let ended = match process {
        Some(process) => {
            let ended = wait(process, deadline.saturating_duration_since(Instant::now())) || {
                crate::log::error("install: the running copy did not quit, ending it");
                // SAFETY: a handle opened with the terminate right.
                let _ = unsafe { TerminateProcess(process, 1) };
                wait(process, Duration::from_secs(5))
            };
            // SAFETY: opened above, closed once.
            let _ = unsafe { CloseHandle(process) };
            ended
        }
        // Not allowed to watch it (it runs with more rights): watch its window instead.
        None => loop {
            if !instance_window_exists() {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(100));
        },
    };
    ended && !instance_window_exists()
}

fn wait(process: HANDLE, timeout: Duration) -> bool {
    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: a handle opened with the synchronize right.
    let status = unsafe { WaitForSingleObject(process, ms) };
    status == WAIT_OBJECT_0
}

fn instance_window_exists() -> bool {
    // SAFETY: the class name is a valid NUL-terminated constant.
    unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()) }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folders_are_the_per_user_ones() {
        let folder = install_folder().unwrap();
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert_eq!(folder, local.join("Programs").join("BetterVolume"));
        let shortcut = shortcut_path().unwrap();
        assert!(shortcut.ends_with("Start Menu\\Programs\\BetterVolume.lnk"), "{shortcut:?}");
    }

    #[test]
    fn only_writable_absolute_folders_are_usable() {
        let temp = std::env::temp_dir();
        assert!(folder_usable(&temp));
        // A folder that does not exist yet is judged by its nearest existing parent.
        assert!(folder_usable(&temp.join("bv-not-there").join("BetterVolume")));
        assert!(!folder_usable(Path::new("relative\\BetterVolume")));
        assert!(!folder_usable(&temp.join("100%")));
        assert!(!folder_usable(&temp.join("a\"b")));
        assert!(!temp.join("bv-not-there").exists(), "nothing created");
    }

    #[test]
    fn a_file_is_replaced_whole() {
        let folder = std::env::temp_dir().join(format!("bv-install-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("file.exe");
        write_replacing(&path, b"one", Instant::now()).unwrap();
        write_replacing(&path, b"two", Instant::now()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no partial file left");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Holds `path` open without sharing (as a running program holds its file) for `held`.
    fn hold(path: &Path, held: Duration) -> std::thread::JoinHandle<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new().read(true).share_mode(0).open(path).unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(held);
            drop(file);
        })
    }

    #[test]
    fn a_file_in_use_is_replaced_once_free_again() {
        let folder = std::env::temp_dir().join(format!("bv-in-use-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("BetterVolume.exe");
        std::fs::write(&path, b"old").unwrap();
        let holder = hold(&path, Duration::from_millis(600));
        write_replacing(&path, b"new", Instant::now() + Duration::from_secs(5)).expect("waited");
        holder.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // Still in use when the wait is over: refused, the old file kept, nothing left.
        let holder = hold(&path, Duration::from_millis(1500));
        assert!(
            write_replacing(&path, b"newer", Instant::now() + Duration::from_millis(300)).is_err()
        );
        holder.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().collect();
        assert_eq!(names.len(), 1, "no partial file left");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn only_a_file_in_use_is_waited_for() {
        // A running program's file (access denied), sharing and lock violations.
        for code in [5, 32, 33] {
            assert!(in_use(&std::io::Error::from_raw_os_error(code)), "{code}");
        }
        // Not found, path not found, disk full, invalid name: reported at once.
        for code in [2, 3, 112, 123] {
            assert!(!in_use(&std::io::Error::from_raw_os_error(code)), "{code}");
        }
        assert!(!in_use(&std::io::Error::other("no code")));
    }

    #[test]
    fn the_uninstaller_removes_files_still_in_use_once_free() {
        let folder = std::env::temp_dir().join(format!("bv-remove-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let program = folder.join(PROGRAM_FILE);
        std::fs::write(&program, b"program").unwrap();
        std::fs::write(folder.join(HELPER_FILE), b"helper").unwrap();
        // The program file stays in use for a while after the uninstaller ends.
        let holder = hold(&program, Duration::from_secs(5));
        remove_files_after_exit(&Installed { folder: folder.clone(), version: String::new() });
        let start = Instant::now();
        while folder.exists() && start.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(250));
        }
        holder.join().unwrap();
        assert!(!folder.exists(), "folder removed once the file was free");
    }
}
