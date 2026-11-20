//! BetterVolume executable.
//!
//! Wires the other crates together and owns the application state. Contains no drawing
//! code, no colour or metric literal, no `unsafe` and no hard-coded user-visible text
//! (`docs/ARCHITECTURE.md` §4).
//!
//! Starts as a single instance with the saved settings (FR-125, FR-126);
//! the tray icon shows the volume step of the current output and its tooltip the device
//! and volume, both live; a middle click toggles mute, the wheel over the icon changes
//! the volume; a left click toggles the flyout (device tiles, application volumes, master
//! volume, microphone view, all live); the context menu opens the Windows sound screens,
//! the settings window (also opened by the flyout's gear button and, once, on the very
//! first launch) or quits. Every settings change applies at once and is saved. A bubble
//! above the taskbar announces every change of the current device and shows the volume
//! after a wheel turn or a middle click. The Home Assistant link (when on) publishes the
//! state over MQTT and applies the commands it receives. When asked (General page, or the
//! question shown once at start), the Windows volume icon is hidden through the taskbar
//! helper; started with `--taskbar-helper`, the executable only loads that helper into
//! the taskbar and exits (`bv_shell::run_loader`).
//!
//! The same executable is the installer when it is the setup file, the
//! uninstaller when started with `--uninstall` ([`setup`]), and the portable version when
//! it is the portable file (settings beside it). The installer and the uninstaller
//! make it quit through [`Event::ExitRequested`].
//!
//! Once a day it asks the release service for a newer version (`updater`,
//! `app::update`); on the user's click, the installed copy starts the downloaded setup
//! file with `--update`, a portable copy replaces its own file and restarts with
//! `--replaced <pid>`.
//!
//! Run from its Microsoft Store package (FR-134), it keeps its settings in the package's
//! folder (copying once those of a GitHub copy found on this PC), leaves updates to the
//! Store and starts with Windows through the package's startup task.
#![forbid(unsafe_code)]
#![windows_subsystem = "windows"]

mod app;
mod setup;
mod store;
mod updater;

/// This version. The update test tool builds a copy with another number
/// (`BETTERVOLUME_VERSION`, `tools/update-test.ps1`); release builds never set it.
pub(crate) const VERSION: &str = match option_env!("BETTERVOLUME_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The time now, in Unix seconds (update check, FR-132).
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use bv_audio::Engine;
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{Settings, WindowPlacement};
use bv_shell::{Event, MenuLine, Shell, ShellConfig, SingleInstance, SystemPage, WindowPlace};
use bv_ui_kit::icons::Icon;

use crate::app::ha::{Farewell, MqttLink, Platform};
use crate::app::{App, FlyoutEffect, SettingsEffect, UpdateEffect};
use crate::store::{Loaded, Store};
use bv_core::package::Kind;

/// Entries of the tray context menu.
#[derive(Clone, Copy)]
enum MenuAction {
    Open(SystemPage),
    Settings,
    Quit,
}

/// The tray context menu (FR-100).
fn menu(language: Language) -> Vec<MenuLine<MenuAction>> {
    let item = |value, icon, label: Msg, enabled| MenuLine::Item {
        value,
        icon,
        label: language.text(label).to_owned(),
        enabled,
    };
    vec![
        item(
            MenuAction::Open(SystemPage::SoundSettings),
            Icon::SoundSettings,
            Msg::MenuSoundSettings,
            true,
        ),
        item(MenuAction::Open(SystemPage::VolumeMixer), Icon::Mixer, Msg::MenuVolumeMixer, true),
        item(
            MenuAction::Open(SystemPage::ClassicSoundPanel),
            Icon::SoundPanel,
            Msg::MenuSoundPanel,
            true,
        ),
        MenuLine::Separator,
        item(MenuAction::Settings, Icon::Settings, Msg::MenuSettings, true),
        MenuLine::Separator,
        item(MenuAction::Quit, Icon::Quit, Msg::MenuQuit, true),
    ]
}

/// Shows the current state in the flyout, if it is open (nothing is built otherwise).
fn refresh_flyout(shell: &Shell, app: &App<Engine>) {
    if shell.flyout_visible() {
        shell.update_flyout(app.flyout_content());
    }
}

/// Shows the new output volume in the bubble (FR-017), when wanted.
fn show_volume(shell: &Shell, app: &App<Engine>) {
    if let Some(content) = app.volume_bubble(shell.flyout_visible()) {
        shell.show_bubble(content);
    }
}

/// Shows the current state in the settings window, if it is open.
fn refresh_settings(shell: &Shell, app: &App<Engine>) {
    if shell.settings_visible() {
        shell.update_settings(app.settings_content());
    }
}

/// Hides or shows again the Windows volume icon (FR-018); when hiding at the user's
/// request, BetterVolume's own icon is also put on the taskbar.
fn hide_volume_icon(shell: &Shell, on: bool, asked_now: bool) {
    shell.set_volume_icon_hidden(on, &helper_folder());
    if on && asked_now {
        bv_shell::show_icon_on_taskbar();
    }
}

/// Opens the settings window (or brings it forward), reading the autostart entry.
fn open_settings(shell: &Shell, app: &mut App<Engine>) {
    if !shell.settings_visible() {
        app.settings_opening(bv_shell::autostart_enabled());
    }
    let place = app.settings().settings_window.map(|p| WindowPlace {
        x: p.x,
        y: p.y,
        width: p.width,
        height: p.height,
        maximized: p.maximized,
    });
    shell.show_settings(app.settings_content(), place);
}

fn placement(place: WindowPlace) -> WindowPlacement {
    WindowPlacement {
        x: place.x,
        y: place.y,
        width: place.width,
        height: place.height,
        maximized: place.maximized,
    }
}

/// Quits as from the menu: the settings window's place and the settings are saved, Home
/// Assistant is told the PC goes offline, and the Windows volume icon comes back.
fn quit(shell: &Shell, app: &mut App<Engine>, store: &mut Option<Store>) {
    // Where the settings window was is kept before it goes.
    app.set_settings_place(shell.settings_place().map(placement));
    app.ha_stop(Farewell::Offline);
    save(store, app);
    shell.quit();
}

/// Saves the settings if they changed (FR-125: at once, atomic write of a small file).
fn save(store: &mut Option<Store>, app: &mut App<Engine>) {
    if app.take_dirty() {
        if let Some(store) = store.as_mut() {
            store.save(app.settings());
        }
    }
}

/// Does what installing a version needs, or opens its page.
fn apply_update(
    shell: &Shell,
    app: &mut App<Engine>,
    store: &mut Option<Store>,
    effect: UpdateEffect,
) {
    match effect {
        UpdateEffect::OpenPage(url) => {
            shell.hide_flyout();
            bv_shell::open_web_page(&url);
        }
        // The setup file ends this copy itself (exit message) and starts the new one.
        UpdateEffect::Install { kind: Kind::Setup, file } => {
            if !bv_shell::launch_with(&file, &[bv_shell::UPDATE_ARGUMENT.to_owned()]) {
                app.update_launch_failed();
            }
        }
        UpdateEffect::Install { kind: Kind::Portable, file } => {
            let Some(exe) = updater::swap_portable(&file) else {
                app.update_launch_failed();
                return;
            };
            let args = [bv_shell::REPLACED_ARGUMENT.to_owned(), std::process::id().to_string()];
            if bv_shell::launch_with(&exe, &args) {
                bv_shell::log::info("update: the portable file was replaced, restarting");
                quit(shell, app, store);
            } else {
                updater::unswap_portable(&exe, &file);
                app.update_launch_failed();
            }
        }
    }
}

/// Decrypts the stored Home Assistant password (Base64 of a blob protected for the Windows
/// user, FR-092).
fn unprotect(stored: &str) -> Option<String> {
    let blob = bv_core::encoding::from_base64(stored)?;
    String::from_utf8(bv_shell::secret::unprotect(&blob)?).ok()
}

/// Encrypts a Home Assistant password for the settings file (FR-092).
fn protect(password: &str) -> Option<String> {
    bv_shell::secret::protect(password.as_bytes()).map(|blob| bv_core::encoding::base64(&blob))
}

/// The shell's key for a media key asked by Home Assistant.
fn media_key(key: bv_core::ha::MediaKey) -> bv_shell::MediaKey {
    match key {
        bv_core::ha::MediaKey::PlayPause => bv_shell::MediaKey::PlayPause,
        bv_core::ha::MediaKey::Next => bv_shell::MediaKey::Next,
        bv_core::ha::MediaKey::Previous => bv_shell::MediaKey::Previous,
    }
}

/// Environment variable naming another data folder: the measurement tool runs the
/// application with a folder of its own, so it never reads or changes the user's settings.
const DATA_FOLDER_VARIABLE: &str = "BETTERVOLUME_DATA_DIR";

/// The folder the tools chose ([`DATA_FOLDER_VARIABLE`]), if any.
fn override_folder() -> Option<PathBuf> {
    std::env::var_os(DATA_FOLDER_VARIABLE).filter(|f| !f.is_empty()).map(PathBuf::from)
}

/// The user's own BetterVolume folder, `%LOCALAPPDATA%\BetterVolume`: settings and log of
/// the installed version, copies of the taskbar helper for every version.
pub(crate) fn local_folder() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|base| !base.is_empty())
        .map(|base| PathBuf::from(base).join("BetterVolume"))
}

/// Folder of the settings and of the log (FR-125): the tools' folder, else beside the
/// portable file when that folder can be written, else the Store package's own folder
/// (FR-134), else the user's own folder.
fn data_folder(portable: bool) -> Option<PathBuf> {
    override_folder()
        .or_else(|| portable.then(portable_folder).flatten())
        .or_else(bv_shell::package_data_folder)
        .or_else(local_folder)
}

/// The folder of the portable file, if a file can be created there.
fn portable_folder() -> Option<PathBuf> {
    let folder = std::env::current_exe().ok()?.parent()?.to_owned();
    let probe = folder.join(format!("BetterVolume.{}.probe", std::process::id()));
    std::fs::File::create_new(&probe).ok()?;
    let _ = std::fs::remove_file(&probe);
    Some(folder)
}

/// Folder of the copies of the taskbar helper library: never beside a portable file (a
/// removable drive must not hold a library the taskbar has loaded).
fn helper_folder() -> PathBuf {
    override_folder()
        .or_else(bv_shell::package_data_folder)
        .or_else(local_folder)
        .unwrap_or_else(std::env::temp_dir)
}

fn main() -> ExitCode {
    // Before any other library is loaded: Windows libraries come from System32 only, never
    // from the folder of this file (a setup file run from the downloads folder).
    if !bv_shell::restrict_library_search() {
        return ExitCode::FAILURE;
    }
    // The loader child of the taskbar helper: load it, report, exit.
    let args: Vec<String> = std::env::args().collect();
    let package = bv_shell::Package::own();
    let portable = package.as_ref().is_some_and(|p| p.kind == bv_core::package::Kind::Portable);
    if args.get(1).map(String::as_str) == Some(bv_shell::LOADER_ARGUMENT) {
        if let Some(folder) = data_folder(portable) {
            bv_shell::log::init(&folder);
        }
        let code = bv_shell::run_loader(args.get(2..).unwrap_or_default());
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    // The Store version's writer of the taskbar preference of its icon: write, exit.
    if args.get(1).map(String::as_str) == Some(bv_shell::PROMOTER_ARGUMENT) {
        if let Some(folder) = data_folder(portable) {
            bv_shell::log::init(&folder);
        }
        let code = bv_shell::run_promoter(args.get(2..).unwrap_or_default());
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    // The uninstaller and the installer: a window of their own, no tray icon.
    // Only the installed program uninstalls (never a setup or portable file).
    if package.is_none() && args.get(1).map(String::as_str) == Some(bv_shell::UNINSTALL_ARGUMENT) {
        return setup::run_uninstall();
    }
    if let Some(package) = package.filter(|p| p.kind == Kind::Setup) {
        // Started by the running copy to update it, it installs without asking.
        let update = args.get(1).map(String::as_str) == Some(bv_shell::UPDATE_ARGUMENT);
        return setup::run_setup(package, update);
    }
    // Started by the portable copy this file replaced, which is quitting.
    if args.get(1).map(String::as_str) == Some(bv_shell::REPLACED_ARGUMENT) {
        if let Some(pid) = args.get(2).and_then(|p| p.parse().ok()) {
            bv_shell::wait_for_process(pid, std::time::Duration::from_secs(20));
        }
    }
    // FR-001: a second launch hands over to the running instance and exits.
    let Some(_instance) = SingleInstance::acquire() else {
        return ExitCode::SUCCESS;
    };
    let data = data_folder(portable);
    if let Some(folder) = &data {
        bv_shell::log::init(folder);
    }
    // A bug that panics ends the process (release builds abort): leave a trace (FR-005).
    // Only the place in the code: a panic message could carry personal data.
    std::panic::set_hook(Box::new(|info| {
        let place =
            info.location().map_or_else(String::new, |l| format!(" at {}:{}", l.file(), l.line()));
        bv_shell::log::error(&format!("panic{place}"));
    }));

    // FR-125, FR-126: the saved settings, or the defaults (never blocks the start).
    let mut store = data.as_ref().map(|folder| Store::new(folder));
    // FR-134: the Store version's first start takes the settings of a GitHub copy.
    if let (Some(store), true, Some(other)) =
        (&store, bv_shell::is_packaged() && override_folder().is_none(), local_folder())
    {
        store.import_once(&other);
    }
    let (settings, loaded) =
        store.as_mut().map_or_else(|| (Settings::default(), Loaded::Missing), Store::load);
    if loaded == Loaded::SetAside {
        bv_shell::log::info("starting from default settings");
    }
    let os_languages = bv_shell::os_display_languages();
    let language = Language::resolve(settings.general.language.as_deref(), &os_languages);

    let app_name = language.text(Msg::AppName).to_owned();
    let config = ShellConfig {
        tooltip: app_name.clone(),
        app_name,
        tray: true,
        text_locale: language.locale_name(),
    };
    let outcome = bv_shell::run(config, move |shell| {
        // Audio callbacks arrive on Windows threads: they only wake the UI thread.
        let waker = shell.waker();
        let engine = Engine::new(Arc::new(move || waker.wake()));
        let mut app = App::new(engine, settings, language);
        app.set_project_page(updater::project_page());
        app.set_process_list(bv_shell::running_executables);
        app.set_os_languages(os_languages);
        // The Home Assistant link starts only if it is on (FR-118); its threads, like the
        // audio callbacks, only wake the UI thread.
        let ha_waker = shell.waker();
        let test_waker = shell.waker();
        app.set_ha_platform(Platform {
            connect: Box::new(move |options| {
                let waker = ha_waker.clone();
                let client = bv_mqtt::Client::start(options, Arc::new(move || waker.wake()));
                Box::new(MqttLink(Some(client)))
            }),
            unprotect: Box::new(unprotect),
            protect: Box::new(protect),
            test: Box::new(move |options| {
                let (done, result) = std::sync::mpsc::channel();
                let waker = test_waker.clone();
                // A test may wait up to the connection timeout: never on the UI thread.
                let _ = std::thread::Builder::new().name("bv-mqtt-test".into()).spawn(move || {
                    let _ = done.send(bv_mqtt::client::test(&options));
                    waker.wake();
                });
                result
            }),
            computer_name: std::env::var("COMPUTERNAME").unwrap_or_default(),
        });
        // FR-132: updates, for a copy built for them (or pointed at the local imitation).
        // The installed copy updates with the setup file, a portable one with the portable
        // file when its folder can be written; otherwise "Install" opens the release page.
        updater::clean_up();
        if bv_shell::is_packaged() {
            // FR-134: the Store updates its package itself.
            app.set_updates_by_store();
        } else if let Some(source) = updater::source() {
            let file = if portable {
                portable_folder().map(|_| Kind::Portable)
            } else if bv_shell::running_installed() {
                Some(Kind::Setup)
            } else {
                None
            };
            app.set_update_platform(updater::platform(source, shell.waker(), file));
        }
        let mut first_wake = true;
        // FR-018: the Windows volume icon is hidden again at each start.
        if app.volume_icon_hidden() {
            hide_volume_icon(shell, true, false);
        } else {
            // No loader runs, so the copies left from earlier loads are removed here.
            bv_shell::remove_unused_helper_copies(&helper_folder());
        }
        let mut tooltip = app.tooltip();
        shell.set_tooltip(&tooltip);
        shell.set_tray_icon(app.tray_icon());

        // Only touches the taskbar when something really changed.
        let mut refresh_tray = move |shell: &Shell, app: &App<Engine>| {
            let text = app.tooltip();
            if text != tooltip {
                shell.set_tooltip(&text);
                tooltip = text;
            }
            shell.set_tray_icon(app.tray_icon());
        };

        move |shell: &Shell, event| {
            match event {
                Event::Wake => {
                    let changes = app.on_audio_signals();
                    let remote = app.on_ha_events();
                    let updates = app.on_update_events(now());
                    for key in &remote.media {
                        media_key(*key).press();
                    }
                    refresh_tray(shell, &app);
                    if changes.any() || remote.refresh || updates {
                        refresh_flyout(shell, &app);
                        refresh_settings(shell, &app);
                    }
                    // A volume or mute change ordered from Home Assistant.
                    if remote.volume_bubble {
                        show_volume(shell, &app);
                    }
                    // FR-071: the very first launch opens the settings window once
                    // (the first wake-up always comes right after start); so does the
                    // next start of an installation that never answered the question
                    // about the Windows volume icon, shown over it.
                    let first = std::mem::take(&mut first_wake);
                    if first && app.start_window_pending() {
                        app.start_window_opening();
                        open_settings(shell, &mut app);
                        // Only once it really showed: a graphics failure retries next start.
                        app.start_window_shown(shell.settings_visible());
                        refresh_settings(shell, &app);
                    }
                    // The first start of a newer version says so.
                    if let Some(content) = first.then(|| app.updated_bubble()).flatten() {
                        shell.show_bubble(content);
                    }
                }
                Event::Resumed => {
                    app.on_resumed();
                    app.update_resumed();
                    // FR-115: back online after sleep (orders kept while away apply now).
                    app.ha_start();
                    refresh_tray(shell, &app);
                    refresh_flyout(shell, &app);
                }
                // Home Assistant sees the PC unavailable at once, not 45 s later.
                Event::Suspending => app.ha_stop(Farewell::Offline),
                Event::TrayMiddle => {
                    // Even when nothing was toggled, the click may have re-read the devices.
                    app.toggle_output_mute();
                    refresh_tray(shell, &app);
                    refresh_flyout(shell, &app);
                    show_volume(shell, &app);
                }
                Event::TrayWheel { notches } => {
                    app.wheel(notches);
                    refresh_tray(shell, &app);
                    refresh_flyout(shell, &app);
                    // Shown even when the volume is already at an end, like Windows.
                    show_volume(shell, &app);
                }
                Event::TrayPrimary(activation) => {
                    shell.toggle_flyout(activation, || {
                        app.flyout_opening();
                        app.flyout_content()
                    });
                    // Applications seen by the opening flyout join the Applications page.
                    refresh_settings(shell, &app);
                }
                // The installer or the uninstaller needs the files.
                Event::ExitRequested => quit(shell, &mut app, &mut store),
                Event::ActivateRequested => {
                    if !shell.flyout_visible() {
                        app.flyout_opening();
                    }
                    shell.show_flyout(app.flyout_content());
                    refresh_settings(shell, &app);
                }
                Event::Flyout(action) => match app.flyout_action(action) {
                    FlyoutEffect::Refresh => {
                        refresh_tray(shell, &app);
                        refresh_flyout(shell, &app);
                    }
                    FlyoutEffect::OpenSoundSettings => {
                        shell.hide_flyout();
                        SystemPage::SoundSettings.open();
                    }
                    FlyoutEffect::OpenSettings => {
                        shell.hide_flyout();
                        open_settings(shell, &mut app);
                    }
                },
                // Posted when the flyout hid: an opening handled since then wins.
                Event::FlyoutClosed if !shell.flyout_visible() => app.flyout_closed(),
                Event::FlyoutClosed => {}
                Event::TrayMenu { x, y } => match shell.popup_menu(&menu(app.language()), x, y) {
                    Some(MenuAction::Open(page)) => {
                        page.open();
                    }
                    Some(MenuAction::Settings) => open_settings(shell, &mut app),
                    Some(MenuAction::Quit) => quit(shell, &mut app, &mut store),
                    None => {}
                },
                Event::Settings(action) => {
                    let effect = app.settings_action(action);
                    // A language chosen here applies to the fonts before anything redraws.
                    shell.set_text_locale(&app.language().locale_name());
                    match effect {
                        SettingsEffect::Refresh => {}
                        SettingsEffect::SetAutostart(on) => {
                            bv_shell::set_autostart(on);
                            app.set_autostart_state(bv_shell::autostart_enabled());
                        }
                        SettingsEffect::Close => shell.close_settings(),
                        SettingsEffect::OpenLogFolder => {
                            if let Some(folder) = &data {
                                bv_shell::open_folder(folder);
                            }
                        }
                        SettingsEffect::OpenProjectPage => {
                            if let Some(page) = app.project_page() {
                                bv_shell::open_web_page(page);
                            }
                        }
                        SettingsEffect::HideVolumeIcon(on) => hide_volume_icon(shell, on, true),
                    }
                    // Every setting applies at once, everywhere (FR-070, FR-075).
                    refresh_tray(shell, &app);
                    refresh_flyout(shell, &app);
                    refresh_settings(shell, &app);
                }
                Event::SettingsActivated => {
                    // The autostart entry may have been changed outside.
                    if app.set_autostart_state(bv_shell::autostart_enabled()) {
                        refresh_settings(shell, &app);
                    }
                }
                Event::SettingsClosed(place) => {
                    app.set_settings_place(place.map(placement));
                    app.volume_icon_window_closed();
                    app.settings_closed();
                }
                Event::VolumeIcon(outcome) => {
                    if app.volume_icon_outcome(outcome) {
                        refresh_settings(shell, &app);
                    }
                }
                // FR-132: the automatic check may be due.
                Event::UpdateDue => {
                    app.update_due(now());
                    refresh_flyout(shell, &app);
                    refresh_settings(shell, &app);
                }
                // The taskbar crashed right after the icon was hidden.
                Event::VolumeIconIncident => {
                    app.volume_icon_incident();
                    open_settings(shell, &mut app);
                    app.settings_show_general();
                    refresh_settings(shell, &app);
                }
            }
            // FR-105: a default device changed, from wherever.
            if let Some(content) = app.device_bubble(shell.flyout_visible()) {
                shell.show_bubble(content);
            }
            // A flyout that could not be shown (graphics failure) must not keep the
            // application sessions watched.
            if app.flyout_open() && !shell.flyout_visible() {
                app.flyout_closed();
            }
            // FR-113: whatever changed reaches Home Assistant at once.
            app.ha_sync();
            // FR-132: the next automatic check, and installing what the user asked for.
            match app.take_update_timer(now()) {
                Some(Some(seconds)) => shell.set_update_timer(Duration::from_secs(seconds)),
                Some(None) => shell.stop_update_timer(),
                None => {}
            }
            if let Some(effect) = app.take_update_effect() {
                apply_update(shell, &mut app, &mut store, effect);
                refresh_flyout(shell, &app);
                refresh_settings(shell, &app);
            }
            shell.set_text_locale(&app.language().locale_name());
            save(&mut store, &mut app);
        }
    });

    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            bv_shell::log::error(&format!("shell stopped: {error}"));
            ExitCode::FAILURE
        }
    }
}
