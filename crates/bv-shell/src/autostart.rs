//! "Start with Windows" (FR-074): the per-user `Run` entry of this
//! executable, read and written directly (it is the source of truth: the installer, the
//! user and Windows' own "Startup apps" page can change it outside the application).
//!
//! Windows' "Startup apps" page (and Task Manager) disable an entry without removing it,
//! by a flag under `StartupApproved\Run`: an entry is on only if it exists, launches this
//! executable, and is not disabled there. Turning it on clears that flag.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
    RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APPROVED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
const NAME: PCWSTR = w!("BetterVolume");

fn this_executable() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// The command written in the entry: the quoted path of the executable.
fn command(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// The executable an entry launches (quotes and arguments removed).
fn launched(command: &str) -> &str {
    let command = command.trim();
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => command.split(' ').next().unwrap_or(""),
    }
}

/// Whether the `StartupApproved` flag disables the entry (first byte odd: 3 or 7).
fn disabled_by_flag(flag: &[u8]) -> bool {
    flag.first().is_some_and(|b| b & 1 == 1)
}

fn read_text(key: PCWSTR, name: PCWSTR) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `size` bytes; constant NUL-terminated names.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = (size as usize / 2).min(buffer.len());
    let text = &buffer[..units];
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    Some(String::from_utf16_lossy(&text[..end]))
}

fn read_flag(name: PCWSTR) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 64];
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer holds `size` bytes; constant NUL-terminated names.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED,
            name,
            RRF_RT_REG_BINARY,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| {
        buffer.truncate(size as usize);
        buffer
    })
}

/// Whether BetterVolume starts with Windows (this executable, not disabled by Windows).
pub fn autostart_enabled() -> bool {
    this_executable().is_some_and(|exe| autostart_enabled_for(&exe))
}

/// Whether the entry launches `exe` and is not disabled by Windows.
pub fn autostart_enabled_for(exe: &Path) -> bool {
    launches(exe) && !read_flag(NAME).is_some_and(|flag| disabled_by_flag(&flag))
}

/// Whether the entry exists and launches `exe`.
fn launches(exe: &Path) -> bool {
    let Some(entry) = read_text(RUN, NAME) else { return false };
    launched(&entry).eq_ignore_ascii_case(&exe.to_string_lossy())
}

/// Turns starting with Windows on or off. Returns whether the change was made (a failure
/// is logged; the caller re-reads [`autostart_enabled`]).
pub fn set_autostart(on: bool) -> bool {
    this_executable().is_some_and(|exe| set_for(&exe, on))
}

/// Removes the entry if it launches `exe` (uninstall, FR-131); an entry of another copy
/// (a portable file) is left alone. Returns whether no entry launches `exe` any more.
pub(crate) fn remove_for(exe: &Path) -> bool {
    !launches(exe) || disable()
}

/// Makes the entry launch `exe` (`on`), or removes it.
pub(crate) fn set_for(exe: &Path, on: bool) -> bool {
    let done = if on { enable(exe) } else { disable() };
    if !done {
        crate::log::error(&format!(
            "cannot turn start with Windows {}",
            if on { "on" } else { "off" }
        ));
    }
    done
}

fn enable(exe: &Path) -> bool {
    let text: Vec<u16> =
        std::ffi::OsStr::new(&command(exe)).encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: `text` is a NUL-terminated buffer of the given byte size; constant names.
    let written = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            REG_SZ.0,
            Some(text.as_ptr().cast()),
            (text.len() * 2) as u32,
        )
    };
    if written.is_err() {
        return false;
    }
    // A flag left by Windows' "Startup apps" page would keep the entry disabled.
    delete(APPROVED)
}

fn disable() -> bool {
    delete(RUN) & delete(APPROVED)
}

/// Deletes our value under `key`; a value that does not exist counts as deleted.
fn delete(key: PCWSTR) -> bool {
    // SAFETY: constant NUL-terminated names.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, NAME) };
    status.is_ok() || status == ERROR_FILE_NOT_FOUND
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launched_executable_is_read_from_the_command() {
        assert_eq!(
            launched("\"C:\\Apps\\Better Volume\\BetterVolume.exe\" --quiet"),
            "C:\\Apps\\Better Volume\\BetterVolume.exe"
        );
        assert_eq!(launched("C:\\Apps\\BetterVolume.exe --quiet"), "C:\\Apps\\BetterVolume.exe");
        assert_eq!(launched("  "), "");
        let exe = Path::new("C:\\Apps\\Better Volume\\BetterVolume.exe");
        assert_eq!(launched(&command(exe)), exe.to_string_lossy());
    }

    #[test]
    fn the_startup_flag_is_decoded() {
        assert!(!disabled_by_flag(&[2, 0, 0, 0]));
        assert!(!disabled_by_flag(&[6]));
        assert!(disabled_by_flag(&[3, 0, 0, 0, 1, 2]));
        assert!(disabled_by_flag(&[7]));
        assert!(!disabled_by_flag(&[]));
    }
}
