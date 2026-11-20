//! Windows screens BetterVolume opens: the sound screens of the context menu (FR-100),
//! Windows' "Startup apps" page (Store version, FR-074).

use std::os::windows::ffi::OsStrExt;

use windows::core::{w, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// A Windows screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemPage {
    /// Settings > System > Sound.
    SoundSettings,
    /// Settings > System > Sound > Volume mixer.
    VolumeMixer,
    /// The classic Sound control panel.
    ClassicSoundPanel,
    /// Settings > Apps > Startup.
    StartupApps,
}

impl SystemPage {
    /// What to launch, and its parameters.
    fn target(self) -> (PCWSTR, PCWSTR) {
        match self {
            SystemPage::SoundSettings => (w!("ms-settings:sound"), PCWSTR::null()),
            SystemPage::VolumeMixer => (w!("ms-settings:apps-volume"), PCWSTR::null()),
            SystemPage::ClassicSoundPanel => (w!("control.exe"), w!("mmsys.cpl")),
            SystemPage::StartupApps => (w!("ms-settings:startupapps"), PCWSTR::null()),
        }
    }

    /// Opens the screen. Returns `false` if Windows refused (the failure is logged).
    pub fn open(self) -> bool {
        let (file, parameters) = self.target();
        // SAFETY: constant NUL-terminated strings; no owner window.
        let result = unsafe {
            ShellExecuteW(None, w!("open"), file, parameters, PCWSTR::null(), SW_SHOWNORMAL)
        };
        // Values above 32 mean success (documented ShellExecute convention).
        let ok = result.0 as isize > 32;
        if !ok {
            crate::log::error(&format!("cannot open {self:?}: code {}", result.0 as isize));
        }
        ok
    }
}

/// Opens a folder in File Explorer (FR-095: the log folder). Returns `false` if Windows
/// refused (logged, without the path: it may hold the user name).
pub fn open_folder(path: &std::path::Path) -> bool {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated buffers that outlive the call; no owner window.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let ok = result.0 as isize > 32;
    if !ok {
        crate::log::error(&format!("cannot open a folder: code {}", result.0 as isize));
    }
    ok
}
