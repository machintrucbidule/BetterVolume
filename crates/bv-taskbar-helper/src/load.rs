//! Loading the helper into the taskbar process from inside it, used only by the watch on
//! the BetterVolume process (`crate::watch`) to restore the icon once BetterVolume is gone. BetterVolume
//! itself loads the helper from outside, the same way (`bv-shell`, `taskbar.rs`).

use windows::core::{s, w, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GetModuleHandleW, GetProcAddress,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Threading::GetCurrentProcessId;

use crate::tree::wide;

/// `InitializeXamlDiagnosticsEx`, exported by `Windows.UI.Xaml.dll`.
type InitFn = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;

/// Endpoints tried: each XAML core of the taskbar registers "VisualDiagConnection<N>";
/// `ERROR_NOT_FOUND` means no core has that number, so the next one is tried.
const MAX_ENDPOINTS: u32 = 16;

/// Load this library again into the current (taskbar) process with "restore".
pub fn restore_from_inside() {
    // SAFETY: plain module queries; the XAML library is loaded in the taskbar process.
    let init = unsafe {
        let Ok(xaml) = GetModuleHandleW(w!("Windows.UI.Xaml.dll")) else { return };
        match GetProcAddress(xaml, s!("InitializeXamlDiagnosticsEx")) {
            // SAFETY: the export has the InitializeXamlDiagnosticsEx signature.
            Some(proc) => std::mem::transmute::<unsafe extern "system" fn() -> isize, InitFn>(proc),
            None => return,
        }
    };
    let Some(path) = own_path() else { return };
    let path = wide(&path);
    let data = wide("restore 0 0");
    for n in 1..=MAX_ENDPOINTS {
        let endpoint = wide(&format!("VisualDiagConnection{n}"));
        // SAFETY: valid NUL-terminated strings for the duration of the call.
        let hr = unsafe {
            init(
                PCWSTR(endpoint.as_ptr()),
                GetCurrentProcessId(),
                PCWSTR::null(),
                PCWSTR(path.as_ptr()),
                crate::CLSID_BV_TASKBAR_HELPER,
                PCWSTR(data.as_ptr()),
            )
        };
        crate::com::trace(&format!("self restore endpoint {n} -> 0x{:08X}", hr.0 as u32));
        if hr != HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            return;
        }
    }
}

/// Full path of this library.
fn own_path() -> Option<String> {
    let mut module = HMODULE::default();
    // SAFETY: the address of one of our own functions identifies our module; no
    // reference is taken.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(restore_from_inside as *const () as *const u16),
            &mut module,
        )
        .ok()?;
    }
    let mut buffer = vec![0u16; 1024];
    // SAFETY: a valid module and buffer.
    let len = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    (len > 0 && len < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..len]))
}
