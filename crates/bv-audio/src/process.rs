//! What the application list needs to know about a process: its executable and the
//! friendly name of that executable (FR-061: friendly names, not raw process names).

use std::ffi::c_void;
use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::SHLoadIndirectString;

use crate::com::{pcwstr, wide};

/// Full path of the executable of process `pid`, if it can be read (it cannot for some
/// protected processes, or once the process has ended).
pub(crate) fn image_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: plain call; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: `buffer` holds `size` characters; Windows writes at most that many and
    // updates `size` to the length written (without the terminating NUL).
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    };
    // SAFETY: the handle was opened above and is closed exactly once.
    let _ = unsafe { CloseHandle(process) };
    result.ok()?;
    let length = (size as usize).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length])).filter(|p| !p.is_empty())
}

/// The friendly name of an executable: its "file description" (what the Windows volume
/// mixer shows, for example "Google Chrome"), else its file name without extension.
pub(crate) fn friendly_name(path: &str) -> String {
    file_description(path).unwrap_or_else(|| file_stem(path))
}

fn file_description(path: &str) -> Option<String> {
    let path = wide(path);
    // SAFETY: `path` is a NUL-terminated buffer that outlives the call.
    let size = unsafe { GetFileVersionInfoSizeW(pcwstr(&path), None) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    // SAFETY: `block` has the size Windows asked for.
    unsafe { GetFileVersionInfoW(pcwstr(&path), None, size, block.as_mut_ptr().cast()) }.ok()?;

    // Languages of the version resource, as (language, code page) pairs.
    let mut languages = Vec::new();
    if let Some(bytes) = query(&block, "\\VarFileInfo\\Translation", Unit::Bytes) {
        for pair in bytes.as_chunks::<4>().0 {
            languages.push((
                u16::from_le_bytes([pair[0], pair[1]]),
                u16::from_le_bytes([pair[2], pair[3]]),
            ));
        }
    }
    // Common fallbacks: US English with Unicode, then with Windows Latin-1.
    languages.extend([(0x0409, 0x04B0), (0x0409, 0x04E4)]);

    languages.into_iter().find_map(|(language, code_page)| {
        let key = format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription");
        let bytes = query(&block, &key, Unit::Characters)?;
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|&u| u != 0)
            .collect();
        let text = String::from_utf16_lossy(&units).trim().to_owned();
        Some(text).filter(|t| !t.is_empty())
    })
}

/// How Windows measures the length of a version resource value.
#[derive(Clone, Copy)]
enum Unit {
    /// Binary values: bytes.
    Bytes,
    /// Text values: UTF-16 characters.
    Characters,
}

/// A value of a version resource, as bytes borrowed from `block` (never beyond it).
fn query<'a>(block: &'a [u8], key: &str, unit: Unit) -> Option<&'a [u8]> {
    let key = wide(key);
    let mut pointer: *mut c_void = std::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: `block` holds a version resource filled by GetFileVersionInfoW and
    // `key` is NUL-terminated; Windows returns a pointer into `block` and the length of
    // the value. The pointer is only turned into an offset, checked against `block`.
    let found =
        unsafe { VerQueryValueW(block.as_ptr().cast(), pcwstr(&key), &mut pointer, &mut length) };
    if !found.as_bool() || pointer.is_null() || length == 0 {
        return None;
    }
    let start = (pointer as usize).checked_sub(block.as_ptr() as usize)?;
    let bytes = match unit {
        Unit::Bytes => length as usize,
        Unit::Characters => (length as usize).checked_mul(2)?,
    };
    let end = start.checked_add(bytes)?.min(block.len());
    block.get(start..end)
}

/// The executable named inside a Windows session identifier, which looks like
/// `{device}|\Device\HarddiskVolume3\Program Files\App\app.exe%b{instance}`. Used when the
/// process itself cannot be opened (a program running with administrator rights or as a
/// service): the path is then in the kernel's own form, good enough for a name.
pub(crate) fn executable_from_session_identifier(identifier: &str) -> Option<&str> {
    let (_, rest) = identifier.split_once('|')?;
    let path = rest.split_once("%b").map_or(rest, |(path, _)| path).trim();
    Some(path).filter(|p| p.contains('\\') && !p.ends_with('\\'))
}

/// A path in the kernel's form (`\Device\HarddiskVolume3\...`, as session identifiers
/// name executables) made readable by the file functions, through the `\\?\GLOBALROOT`
/// prefix. Other paths are returned unchanged.
pub(crate) fn readable_path(path: &str) -> String {
    if path.starts_with("\\Device\\") {
        format!("\\\\?\\GLOBALROOT{path}")
    } else {
        path.to_owned()
    }
}

/// The file name of `path` without its extension.
pub(crate) fn file_stem(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Resolves an indirect string such as `@%SystemRoot%\System32\AudioSrv.Dll,-202` (how
/// some applications name their sessions). Other texts are returned unchanged.
pub(crate) fn resolve_indirect(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if !text.starts_with('@') {
        return Some(text.to_owned());
    }
    let source = wide(text);
    let mut buffer = vec![0u16; 512];
    // SAFETY: `source` is NUL-terminated; Windows writes a NUL-terminated string that
    // fits in `buffer`.
    unsafe { SHLoadIndirectString(PCWSTR(source.as_ptr()), &mut buffer, None) }.ok()?;
    let length = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length])).filter(|t| !t.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_own_executable_is_found() {
        let path = image_path(std::process::id()).expect("own image path");
        let expected = std::env::current_exe().unwrap();
        assert!(path.eq_ignore_ascii_case(&expected.to_string_lossy()), "{path}");
        assert_eq!(image_path(0), None);
    }

    #[test]
    fn system_executables_have_a_friendly_name() {
        let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let notepad = format!("{windows}\\System32\\notepad.exe");
        if Path::new(&notepad).exists() {
            let name = friendly_name(&notepad);
            assert!(!name.is_empty() && !name.ends_with(".exe"), "{name}");
        }
        let explorer = format!("{windows}\\explorer.exe");
        assert!(!friendly_name(&explorer).is_empty());
    }

    #[test]
    fn friendly_name_falls_back_to_the_file_name() {
        assert_eq!(friendly_name("C:\\does\\not\\exist\\My Player.exe"), "My Player");
        assert_eq!(friendly_name(""), "");
    }

    #[test]
    fn executables_are_read_from_session_identifiers() {
        let id = "{0.0.0.00000000}.{8c1f}|\\Device\\HarddiskVolume3\\Program Files\\Tool\\tool.exe%b{00000000-0000}";
        let path = executable_from_session_identifier(id);
        assert_eq!(path, Some("\\Device\\HarddiskVolume3\\Program Files\\Tool\\tool.exe"));
        assert_eq!(file_stem(path.unwrap()), "tool");
        assert_eq!(executable_from_session_identifier("{0.0.0.00000000}.{8c1f}|#%b{x}"), None);
        assert_eq!(executable_from_session_identifier("no separator"), None);
        assert_eq!(executable_from_session_identifier("x|\\a\\b.exe"), Some("\\a\\b.exe"));
        assert_eq!(executable_from_session_identifier("x|"), None);
    }

    #[test]
    fn executables_named_by_their_kernel_path_get_their_description() {
        // A sandboxed process cannot be opened: its executable is only known by the kernel
        // path of its session identifier. Built here for a
        // program every Windows has.
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let win32 = format!("{system}\\System32\\notepad.exe");
        let drive: Vec<u16> = win32[..2].encode_utf16().chain([0]).collect();
        let mut target = [0u16; 260];
        // SAFETY: `drive` is NUL-terminated; Windows writes at most `target.len()` units.
        let written = unsafe {
            windows::Win32::Storage::FileSystem::QueryDosDeviceW(
                PCWSTR(drive.as_ptr()),
                Some(&mut target),
            )
        };
        let device = String::from_utf16_lossy(&target[..written as usize])
            .split('\0')
            .next()
            .unwrap_or_default()
            .to_owned();
        assert!(device.starts_with("\\Device\\"), "{device}");
        let kernel = format!("{device}{}", &win32[2..]);
        let expected = friendly_name(&win32);
        assert_ne!(expected, "notepad", "notepad.exe has a description");
        assert_eq!(friendly_name(&readable_path(&kernel)), expected);
        assert_eq!(readable_path("C:\\a.exe"), "C:\\a.exe");
    }

    #[test]
    fn plain_and_indirect_strings_are_resolved() {
        assert_eq!(resolve_indirect("  Player "), Some("Player".to_owned()));
        assert_eq!(resolve_indirect("   "), None);
        assert_eq!(resolve_indirect("@C:\\does\\not\\exist.dll,-1"), None);
        let system_sounds = resolve_indirect("@%SystemRoot%\\System32\\AudioSrv.Dll,-202");
        if let Some(name) = system_sounds {
            assert!(!name.starts_with('@'));
        }
    }
}
