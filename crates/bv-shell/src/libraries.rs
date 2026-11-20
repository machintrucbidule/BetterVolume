//! Where Windows looks for the libraries this process loads.
//!
//! By default Windows looks first in the folder of the executable. A setup or portable
//! file is often started from the downloads folder, where any other download may have
//! left a file named like a system library: it would then run inside this process.
//! Every library this program uses is a Windows library from System32, so the search is
//! restricted to System32 for the whole process, before anything else is loaded. The
//! libraries the executable imports beyond the ones Windows always takes from System32
//! (its "known" libraries) are delay-loaded (`bv-app/build.rs`), so they too are found
//! through this rule; the C runtime is linked into the executable (`.cargo/config.toml`).

use windows::Win32::System::LibraryLoader::{
    SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

/// Restricts the library search of this process to System32. Call it first thing in
/// `main`. Returns whether Windows accepted it (it always does on Windows 10 and 11).
pub fn restrict_library_search() -> bool {
    // SAFETY: plain call with a valid flag; it only changes the process's search rule.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_beside_the_executable_is_not_found_by_name() {
        assert!(restrict_library_search());
        // A genuine library copied beside this test executable under a name of its own:
        // the default search would find it there; the restricted one must not.
        let folder = std::env::current_exe().unwrap().parent().unwrap().to_owned();
        let name = format!("bv-planted-{}.dll", std::process::id());
        let planted = folder.join(&name);
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        std::fs::copy(format!(r"{system}\System32\version.dll"), &planted).unwrap();
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated name; a module that loads is freed right after.
        let loaded = unsafe {
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::PCWSTR(
                wide.as_ptr(),
            ))
        };
        if let Ok(module) = loaded {
            // SAFETY: the module loaded above, freed once.
            let _ = unsafe { windows::Win32::Foundation::FreeLibrary(module) };
        }
        let _ = std::fs::remove_file(&planted);
        assert!(loaded.is_err(), "a library beside the executable was loaded");
    }

    #[test]
    fn the_search_rule_is_accepted_and_system_libraries_still_load() {
        assert!(restrict_library_search());
        // A library outside the known ones, by name: found in System32.
        // SAFETY: a constant NUL-terminated name; the module is freed right after.
        let module = unsafe {
            windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("dwmapi.dll"))
        }
        .expect("dwmapi.dll from System32");
        // SAFETY: the module loaded above, freed once.
        let _ = unsafe { windows::Win32::Foundation::FreeLibrary(module) };
    }
}
