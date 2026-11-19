//! Which programs run now, by executable file name: the Applications page says whether
//! each remembered application is running (read when the page is shown or refreshed,
//! never watched).

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// The executable file names of the running processes, in lower case, sorted, each once.
/// Empty if Windows refuses the list.
pub fn running_executables() -> Vec<String> {
    let mut names = Vec::new();
    // SAFETY: plain call; the snapshot handle is closed below.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return names;
    };
    let mut entry =
        PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    // SAFETY: `entry` is a valid structure with its size set, as both calls require.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while more {
        let length = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..length]).to_lowercase();
        if !name.is_empty() {
            names.push(name);
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: the snapshot was opened above and is closed exactly once.
    let _ = unsafe { CloseHandle(snapshot) };
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_program_is_listed_by_its_file_name() {
        let me = std::env::current_exe().unwrap();
        let file = me.file_name().unwrap().to_string_lossy().to_lowercase();
        let running = running_executables();
        assert!(running.binary_search(&file).is_ok(), "{file} not in {running:?}");
        assert!(running.iter().all(|n| *n == n.to_lowercase()));
    }
}
