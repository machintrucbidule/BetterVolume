//! BetterVolume Windows plumbing.
//!
//! Everything that talks to the Windows shell lives here behind a small safe API, so the
//! executable crate contains no `unsafe` and no Win32 details: the hidden message window
//! and its message loop, the tray icon (drawn by the design system) and its wheel
//! capture, the context menu and flyout popups, the Windows appearance preferences, the
//! single-instance guard, the OS language preferences, the log file, the Windows sound
//! screens, the settings window host, the "start with Windows" entry, the installation for the
//! current user ([`install`]) and this executable read as a release file ([`Package`]), the protection of
//! secrets for the Windows user ([`secret`]), media keys and the [`Waker`]
//! that lets background threads wake the UI thread, and hiding the Windows volume icon
//! through the taskbar helper ([`Shell::set_volume_icon_hidden`], [`run_loader`]).
//!
//! `unsafe` is allowed in this crate (Win32 interop); every block carries a `SAFETY:`
//! comment.

mod appearance;
mod appicon;
mod autostart;
mod bubble;
mod clipboard;
mod flyout;
mod host;
pub mod http;
mod input;
mod install;
mod instance;
mod libraries;
mod locale;
pub mod log;
mod media;
mod menu;
mod package;
mod packaged;
mod placement;
mod popup;
mod processes;
mod registry;
mod screen;
pub mod secret;
mod settings;
mod system;
mod taskbar;
mod tray;
mod trayicon;
mod uia;
mod update;
mod waker;
mod wide;

pub use appicon::app_icon;
pub use autostart::{autostart_enabled, autostart_enabled_for, set_autostart};
pub use host::{run, Event, Shell, ShellConfig, TrayActivation};
pub use install::{
    end_running_instance, folder_usable, install, install_folder, installed, launch,
    remove_entries, remove_files_after_exit, running_installed, InstallError, Installed,
    HELPER_FILE, PROGRAM_FILE, UNINSTALL_ARGUMENT,
};
pub use instance::SingleInstance;
pub use libraries::restrict_library_search;
pub use locale::os_display_languages;
pub use media::MediaKey;
pub use menu::MenuLine;
pub use package::Package;
pub use packaged::{is_packaged, package_data_folder, package_family};
pub use processes::running_executables;
pub use settings::WindowPlace;
pub use system::{open_folder, SystemPage};
pub use taskbar::{remove_unused_helper_copies, run_loader, VolumeIconOutcome, LOADER_ARGUMENT};
pub use tray::{run_promoter, show_icon_on_taskbar, PROMOTER_ARGUMENT};
pub use update::{
    launch_with, local_time_text, open_web_page, wait_for_process, REPLACED_ARGUMENT,
    UPDATE_ARGUMENT,
};
pub use waker::Waker;

/// Longest tray tooltip Windows shows, in UTF-16 units (its buffer holds 128 including the
/// terminating NUL). Longer texts are cut by [`Shell::set_tooltip`]; shorten them first to
/// choose what is lost.
pub const TOOLTIP_MAX_UNITS: usize = 127;

/// A Windows API failure that prevented the shell from starting or running.
#[derive(Debug)]
pub struct ShellError(windows::core::Error);

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The code, not the system text: the log stays in English.
        write!(f, "Windows error 0x{:08X}", self.0.code().0 as u32)
    }
}

impl std::error::Error for ShellError {}

impl From<windows::core::Error> for ShellError {
    fn from(error: windows::core::Error) -> Self {
        Self(error)
    }
}
