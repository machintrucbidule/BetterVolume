//! Feasibility probe (ARCHITECTURE section 7): ask Windows to load the built
//! helper library into the taskbar through the documented XAML Diagnostics entry point and
//! run one instruction, then report what the call returned.
//!
//! Manual diagnostic only, run on a development machine by hand; not part
//! of the shipped application (that path lives in `bv-shell`). Usage:
//! `cargo run -p bv-taskbar-helper --features trace --example proof -- hide|restore|watch`.
//! "watch" hides with this probe as the owner: the icon must come back when it exits.

// A standalone diagnostic tool may report to its console (the application itself may not).
#![allow(clippy::print_stderr, clippy::print_stdout)]

use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, HMODULE, HWND};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId};

use bv_taskbar_helper::CLSID_BV_TASKBAR_HELPER;

/// `InitializeXamlDiagnosticsEx`, exported by `Windows.UI.Xaml.dll`.
type InitFn = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    let command = std::env::args().nth(1).unwrap_or_else(|| "hide".into());
    let instruction = match command.as_str() {
        "hide" => "hide 0 0".to_owned(),
        "restore" => "restore 0 0".to_owned(),
        "watch" => format!("hide {} 0", std::process::id()),
        _ => {
            eprintln!("usage: proof hide|restore|watch");
            std::process::exit(2);
        }
    };

    let class = wide("Shell_TrayWnd");
    // SAFETY: a valid class name; a null window name matches by class only.
    let tray: HWND =
        unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) }.unwrap_or_default();
    if tray.0.is_null() {
        eprintln!("taskbar window not found");
        std::process::exit(1);
    }
    let mut pid = 0u32;
    // SAFETY: valid window handle and out-pointer.
    unsafe { GetWindowThreadProcessId(tray, Some(&mut pid)) };
    eprintln!("taskbar process id: {pid}");

    let exe = std::env::current_exe().expect("current exe");
    let dir = exe.parent().expect("exe dir");
    let dll = [dir.join("bv_taskbar_helper.dll"), dir.join("../bv_taskbar_helper.dll")]
        .into_iter()
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            eprintln!("helper library not found near {}", dir.display());
            std::process::exit(1);
        });
    // Load a copy: the taskbar keeps the library it loaded open until it restarts, and the
    // build must stay able to replace its own output.
    let copy =
        std::env::temp_dir().join(format!("bv_taskbar_helper-proof-{}.dll", std::process::id()));
    std::fs::copy(&dll, &copy).expect("copy helper");
    let dll_wide = wide(&copy.to_string_lossy());

    let module = wide("Windows.UI.Xaml.dll");
    // SAFETY: a valid library name.
    let handle: HMODULE =
        unsafe { LoadLibraryW(PCWSTR(module.as_ptr())) }.expect("load Windows.UI.Xaml.dll");
    let name = b"InitializeXamlDiagnosticsEx\0";
    // SAFETY: a valid module and NUL-terminated export name.
    let proc = unsafe { GetProcAddress(handle, windows::core::PCSTR(name.as_ptr())) };
    let Some(proc) = proc else {
        eprintln!("InitializeXamlDiagnosticsEx not found");
        std::process::exit(1);
    };
    // SAFETY: the export has the InitializeXamlDiagnosticsEx signature.
    let init = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, InitFn>(proc) };

    // No COM registration is needed: the XAML host loads the library from the given path
    // and asks its `DllGetClassObject` for the CLSID (checked live).
    let data = wide(&instruction);
    let mut result = HRESULT(0);
    for n in 1..=16 {
        let endpoint = wide(&format!("VisualDiagConnection{n}"));
        // SAFETY: all pointers are valid NUL-terminated strings for the duration of the call.
        result = unsafe {
            init(
                PCWSTR(endpoint.as_ptr()),
                pid,
                PCWSTR::null(),
                PCWSTR(dll_wide.as_ptr()),
                CLSID_BV_TASKBAR_HELPER,
                PCWSTR(data.as_ptr()),
            )
        };
        eprintln!(
            "InitializeXamlDiagnosticsEx({instruction}, endpoint {n}) -> 0x{:08X}",
            result.0 as u32
        );
        if result != HRESULT::from_win32(ERROR_NOT_FOUND.0) {
            break;
        }
    }
    if result.is_ok() && command == "watch" {
        eprintln!("hidden with this probe as owner; exiting in 5 s (the icon must come back)");
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
