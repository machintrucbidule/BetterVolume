//! In-taskbar helper (FR-018, ARCHITECTURE section 7).
//!
//! A standalone dynamic library that BetterVolume asks Windows to load into the taskbar
//! process through the **documented** XAML Diagnostics tree-automation-provider (TAP)
//! entry point `InitializeXamlDiagnosticsEx` (the same supported hook published taskbar
//! customisation tools use). Once loaded it reads the taskbar's own XAML visual tree,
//! finds the system tray's volume element and collapses it, and restores it when asked.
//! It never hooks functions and needs no administrator rights.
//!
//! Guard rails (section 7) live here and in the caller:
//! - It touches the volume element only, and does nothing if that element is not found
//!   (an unknown Windows build simply keeps the Windows icon).
//! - The caller (`bv-shell`) decides when to load it, re-applies after a taskbar restart,
//!   restores the icon on exit, and auto-disables the option if the taskbar dies right
//!   after a load. None of that policy lives here.
//!
//! The command is carried in the TAP initialisation string (`GetInitializationData`):
//! `"hide <owner pid> <owner window>"` collapses the volume icon, `"restore ..."` makes it
//! visible again. Each load creates a fresh TAP instance that reads the string once, follows
//! the tree for a few seconds, acts, reports to the owner window and leaves. While
//! the icon is hidden, the owner process is watched: when it ends, the icon is restored.
//!
//! The release profile aborts on panic, and a panic here would end the taskbar: the code
//! of this crate never unwraps, indexes or otherwise panics on data it receives.

// This crate is the application's one in-process-into-the-taskbar component; the COM
// interop it needs is unavoidably unsafe. The unsafe-code policy allows it
// here as in bv-shell and bv-audio, and the gate forbids re-allowing it anywhere else.
// COM method names follow the Windows declarations (xamlOM.h), hence not snake case.
#![allow(non_snake_case)]

mod com;
mod load;
mod tap;
mod tree;
mod watch;
mod xaml;

pub use tap::status;
pub use tree::{is_volume_glyph, Command, Instruction};

use windows::core::{Interface, GUID, HRESULT};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_POINTER, S_FALSE, S_OK};
use windows::Win32::System::Com::IClassFactory;

/// CLSID of this TAP, named by `bv-shell` when it calls `InitializeXamlDiagnosticsEx`.
/// A private, fixed identifier for this library; it is not registered system-wide.
/// `{6D8C3F2A-9E41-4B7D-B2A6-5C1E0F38A7C4}`
pub const CLSID_BV_TASKBAR_HELPER: GUID = GUID::from_u128(0x6d8c3f2a_9e41_4b7d_b2a6_5c1e0f38a7c4);

/// COM class-object factory entry point. The XAML Diagnostics host calls this to obtain a
/// class factory for our TAP CLSID, then asks the factory to create the TAP instance.
///
/// # Safety
/// `rclsid`, `riid` and `ppv` come from the COM runtime; `ppv` must be a valid
/// out-pointer. The signature matches the `DllGetClassObject` contract.
#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    // SAFETY: the COM runtime passes valid pointers for a DllGetClassObject call.
    unsafe {
        if ppv.is_null() {
            return E_POINTER;
        }
        *ppv = core::ptr::null_mut();
        com::trace("DllGetClassObject called");
        if rclsid.is_null() || *rclsid != CLSID_BV_TASKBAR_HELPER {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = tap::Factory.into();
        factory.query(riid, ppv)
    }
}

/// COM unload query. Never: the owner watch and the threads of a connection run code of
/// this library at any time (the XAML host never unloads a TAP library anyway).
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    let _ = com::outstanding();
    S_FALSE
}

/// `S_OK`, shared by the COM entry points and the callback.
#[inline]
fn hr_ok() -> HRESULT {
    S_OK
}
