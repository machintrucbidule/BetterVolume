//! The installer and the uninstaller (FR-130, FR-131).
//!
//! Both are this executable, in a window of their own: the setup file (the program
//! followed by the taskbar helper library and a trailer, `bv_core::package`) opens the
//! installer; the installed program started with `--uninstall` (by Windows' installed-apps
//! list) opens the uninstaller. The window is the settings window at its compact size,
//! without navigation, built from the design system's cards, notices and buttons, so it
//! looks like the rest of the application. The slow part (ending the running copy,
//! writing files, reaching Home Assistant) runs on a thread of its own.
//!
//! [`Setup`] holds what the window shows and decides what each click does; it is pure and
//! unit-tested. [`run_setup`] and [`run_uninstall`] wire it to the shell and do the work.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use bv_core::i18n::{Language, Msg};
use bv_core::package::Version;
use bv_shell::{Event, InstallError, Installed, Package, ShellConfig};
use bv_ui_kit::controls::settings::{
    Block, Button, Card, CardControl, DeviceTexts, Notice, PageKey, SettingKey, SettingsAction,
    SettingsContent, Severity,
};

/// The option card (start with Windows, or keep the settings).
const OPTION: SettingKey = SettingKey(1);
/// The main button (install, update, uninstall, close).
const PRIMARY: SettingKey = SettingKey(2);
/// The cancel button.
const CANCEL: SettingKey = SettingKey(3);
/// The notice (progress, failure, outcome).
const NOTICE: SettingKey = SettingKey(4);
/// The installation folder card.
const FOLDER: SettingKey = SettingKey(5);
/// The notice of a folder that cannot be used.
const FOLDER_NOTICE: SettingKey = SettingKey(6);

/// Name of the sub-folder BetterVolume goes into, inside the folder the user picks.
const FOLDER_NAME: &str = "BetterVolume";

/// How long the running copy is given to quit before it is ended.
const QUIT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the uninstaller tries to reach Home Assistant.
const HA_TIMEOUT: Duration = Duration::from_secs(8);

/// What the window is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Installing this version; `installed` is the version already installed, if any.
    Install {
        /// The version already installed.
        installed: Option<String>,
    },
    /// Uninstalling.
    Uninstall,
}

/// Why the work failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The running copy could not be ended.
    StillRunning,
    /// The files could not be written.
    Files,
    /// The Start-menu or installed-apps entry could not be written.
    Entries,
    /// The setup file is damaged (its parts cannot be read).
    Damaged,
}

impl From<InstallError> for Failure {
    fn from(error: InstallError) -> Failure {
        match error {
            InstallError::Folder | InstallError::Files => Failure::Files,
            InstallError::Entries => Failure::Entries,
        }
    }
}

/// Where the window stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the user's choice.
    Ready,
    /// The work runs.
    Working,
    /// The work failed; the user may try again.
    Failed(Failure),
    /// Installed, but the installed program could not be started.
    NotStarted,
    /// Uninstalled; `ha_kept` when Home Assistant could not be told.
    Uninstalled {
        /// This PC is still listed in Home Assistant.
        ha_kept: bool,
    },
}

/// What a request asks of the runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Nothing more than showing the new content.
    Refresh,
    /// Start the work with the current choice.
    Start,
    /// Close the window (and end the process once the work, if any, is over).
    Close,
    /// Open Windows' folder picker, then report with [`Setup::folder_chosen`].
    ChooseFolder,
}

/// The window's state.
#[derive(Clone, Debug)]
pub struct Setup {
    mode: Mode,
    phase: Phase,
    language: Language,
    version: String,
    /// Install: start with Windows. Uninstall: keep the settings.
    option: bool,
    /// Install: where BetterVolume goes.
    folder: Option<PathBuf>,
    /// The last folder picked could not be used.
    folder_refused: bool,
}

impl Setup {
    /// A window for `mode`, in `language`, for this `version`. Start with Windows is on by
    /// default and the settings are kept by default.
    pub fn new(mode: Mode, language: Language, version: &str, option: bool) -> Setup {
        Setup {
            mode,
            phase: Phase::Ready,
            language,
            version: version.to_owned(),
            option,
            folder: None,
            folder_refused: false,
        }
    }

    /// Sets the installation folder shown (the usual one, or the existing installation's).
    pub fn set_folder(&mut self, folder: PathBuf) {
        self.folder = Some(folder);
    }

    /// Where BetterVolume will be installed.
    pub fn folder(&self) -> Option<&std::path::Path> {
        self.folder.as_deref()
    }

    /// Whether the folder can still be changed: a first installation only (an existing
    /// one keeps its folder).
    fn folder_changeable(&self) -> bool {
        matches!(self.mode, Mode::Install { installed: None })
    }

    /// Title of Windows' folder picker.
    pub fn picker_title(&self) -> String {
        self.text(Msg::SetupFolderPick)
    }

    /// The user picked `chosen` in the folder picker: BetterVolume goes into its
    /// "BetterVolume" sub-folder (unless it is one already), if `usable` accepts it.
    pub fn folder_chosen(
        &mut self,
        chosen: &std::path::Path,
        usable: &dyn Fn(&std::path::Path) -> bool,
    ) -> Effect {
        let target = target_folder(chosen);
        self.folder_refused = !usable(&target);
        if !self.folder_refused {
            self.folder = Some(target);
        }
        Effect::Refresh
    }

    /// Where the window stands.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The user's choice (start with Windows, or keep the settings).
    pub fn option(&self) -> bool {
        self.option
    }

    fn text(&self, msg: Msg) -> String {
        self.language.text(msg).to_owned()
    }

    /// Whether this installs a newer version over an installed one.
    fn updating(&self) -> bool {
        match &self.mode {
            Mode::Install { installed: Some(installed) } => {
                match (Version::parse(installed), Version::parse(&self.version)) {
                    (Some(old), Some(new)) => new > old,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// What the window shows.
    pub fn content(&self) -> SettingsContent {
        let install = matches!(self.mode, Mode::Install { .. });
        let (window, title) = match &self.mode {
            Mode::Install { .. } if self.updating() => (Msg::SetupWindow, Msg::SetupUpdateTitle),
            Mode::Install { .. } => (Msg::SetupWindow, Msg::SetupInstallTitle),
            Mode::Uninstall => (Msg::UninstallWindow, Msg::UninstallTitle),
        };
        let intro = match &self.mode {
            Mode::Install { installed: Some(installed) } if *installed == self.version => {
                self.language.format(Msg::SetupReinstallText, &[("version", &self.version)])
            }
            Mode::Install { installed: Some(installed) } => self.language.format(
                Msg::SetupUpdateText,
                &[("installed", installed), ("version", &self.version)],
            ),
            Mode::Install { installed: None } => {
                self.language.format(Msg::SetupInstallText, &[("version", &self.version)])
            }
            Mode::Uninstall => self.text(Msg::UninstallText),
        };
        let working = self.phase == Phase::Working;
        let mut blocks = Vec::new();
        let finished = matches!(self.phase, Phase::NotStarted | Phase::Uninstalled { .. });
        if !finished {
            blocks.push(Block::Text(intro));
            let (label, help) = if install {
                (Msg::SetupAutostart, Msg::SetupAutostartHelp)
            } else {
                (Msg::UninstallKeep, Msg::UninstallKeepHelp)
            };
            blocks.push(Block::Card(Card {
                key: OPTION,
                icon: None,
                title: self.text(label),
                description: Some(self.text(help)),
                control: CardControl::Toggle {
                    on: self.option,
                    on_label: self.text(Msg::SettingsOn),
                    off_label: self.text(Msg::SettingsOff),
                },
                enabled: !working,
            }));
            if let (true, Some(folder)) = (install, &self.folder) {
                blocks.push(Block::Card(Card {
                    key: FOLDER,
                    icon: None,
                    title: self.text(Msg::SetupFolder),
                    description: Some(folder.display().to_string()),
                    control: if self.folder_changeable() {
                        CardControl::Button {
                            label: self.text(Msg::SetupFolderChange),
                            accent: false,
                        }
                    } else {
                        CardControl::None
                    },
                    enabled: !working,
                }));
            }
        }
        let notice = |severity, title: Msg, message: String| {
            Block::Notice(Notice {
                key: NOTICE,
                severity,
                title: self.text(title),
                message,
                action: None,
            })
        };
        if self.folder_refused {
            blocks.push(Block::Notice(Notice {
                key: FOLDER_NOTICE,
                severity: Severity::Caution,
                title: self.text(Msg::SetupFolderRefused),
                message: self.text(Msg::SetupFolderRefusedText),
                action: None,
            }));
        }
        match self.phase {
            Phase::Ready => {}
            Phase::Working => {
                let (title, text) = if install {
                    (Msg::SetupWorking, Msg::SetupWorkingText)
                } else {
                    (Msg::UninstallWorking, Msg::UninstallWorkingText)
                };
                blocks.push(notice(Severity::Info, title, self.text(text)));
            }
            Phase::Failed(failure) => {
                let title = if install { Msg::SetupFailed } else { Msg::UninstallFailed };
                let text = match failure {
                    Failure::StillRunning => Msg::SetupStillRunning,
                    Failure::Files => Msg::SetupFilesFailed,
                    Failure::Entries => Msg::SetupEntriesFailed,
                    Failure::Damaged => Msg::SetupDamaged,
                };
                blocks.push(notice(Severity::Critical, title, self.text(text)));
            }
            Phase::NotStarted => blocks.push(notice(
                Severity::Caution,
                Msg::SetupNotStarted,
                self.text(Msg::SetupNotStartedText),
            )),
            Phase::Uninstalled { ha_kept: false } => blocks.push(notice(
                Severity::Success,
                Msg::UninstallDone,
                self.text(Msg::UninstallDoneText),
            )),
            Phase::Uninstalled { ha_kept: true } => blocks.push(notice(
                Severity::Caution,
                Msg::UninstallHaKept,
                self.text(Msg::UninstallHaKeptText),
            )),
        }
        let button = |key, label: Msg, accent, enabled| Button {
            key,
            label: self.text(label),
            accent,
            enabled,
        };
        let buttons = if finished {
            vec![button(PRIMARY, Msg::SetupClose, true, true)]
        } else {
            let primary = match (&self.mode, self.phase) {
                (Mode::Install { .. }, Phase::Failed(Failure::Damaged)) => None,
                (_, Phase::Failed(_)) => Some(Msg::SetupRetry),
                (Mode::Install { .. }, _) if self.updating() => Some(Msg::SetupUpdate),
                (Mode::Install { .. }, _) => Some(Msg::SetupInstall),
                (Mode::Uninstall, _) => Some(Msg::UninstallUninstall),
            };
            let mut buttons: Vec<Button> =
                primary.map(|label| button(PRIMARY, label, true, !working)).into_iter().collect();
            buttons.push(button(CANCEL, Msg::SetupCancel, false, !working));
            buttons
        };
        blocks.push(Block::Buttons(buttons));
        SettingsContent {
            name: self.text(window),
            nav_name: String::new(),
            nav: Vec::new(),
            page: PageKey(1),
            title: self.text(title),
            blocks,
            devices: DeviceTexts::default(),
            close_question: None,
            question: None,
            reveal: None,
        }
    }

    /// Applies a request of the window.
    pub fn action(&mut self, action: &SettingsAction) -> Effect {
        let can_choose = matches!(self.phase, Phase::Ready | Phase::Failed(_));
        match action {
            SettingsAction::Toggle(OPTION, on) if can_choose => {
                self.option = *on;
                Effect::Refresh
            }
            SettingsAction::Invoke(FOLDER) if can_choose && self.folder_changeable() => {
                // A new choice starts: an earlier refusal no longer applies.
                self.folder_refused = false;
                Effect::ChooseFolder
            }
            SettingsAction::Invoke(PRIMARY) if can_choose => {
                if self.phase == Phase::Failed(Failure::Damaged) {
                    return Effect::Refresh;
                }
                self.folder_refused = false;
                self.phase = Phase::Working;
                Effect::Start
            }
            SettingsAction::Invoke(PRIMARY)
                if matches!(self.phase, Phase::NotStarted | Phase::Uninstalled { .. }) =>
            {
                Effect::Close
            }
            SettingsAction::Invoke(CANCEL) if self.phase != Phase::Working => Effect::Close,
            _ => Effect::Refresh,
        }
    }

    /// The work ended. For an installation `Ok(true)` means the installed program
    /// started (the window then closes); for an uninstallation it means Home Assistant
    /// was told (or had nothing to be told).
    pub fn finished(&mut self, outcome: Result<bool, Failure>) -> Effect {
        match (&self.mode, outcome) {
            (_, Err(failure)) => {
                self.phase = Phase::Failed(failure);
                Effect::Refresh
            }
            (Mode::Install { .. }, Ok(true)) => Effect::Close,
            (Mode::Install { .. }, Ok(false)) => {
                self.phase = Phase::NotStarted;
                Effect::Refresh
            }
            (Mode::Uninstall, Ok(told)) => {
                self.phase = Phase::Uninstalled { ha_kept: !told };
                Effect::Refresh
            }
        }
    }

    /// Whether the uninstallation was done (the files can go once the process ends).
    pub fn uninstalled(&self) -> bool {
        matches!(self.phase, Phase::Uninstalled { .. })
    }
}

/// The language of the window: the one chosen in the settings, else Windows' own.
fn language(chosen: Option<&str>) -> Language {
    Language::resolve(chosen, &bv_shell::os_display_languages())
}

/// Runs the window until it is closed and the work, if started, is over. `work` runs on
/// a thread of its own with the user's choice; `done` runs at the very end.
fn run_window(
    mut setup: Setup,
    auto_start: bool,
    work: impl Fn(bool, Option<PathBuf>) -> Result<bool, Failure> + Send + Sync + 'static,
    done: impl FnOnce(&Setup) + 'static,
) -> ExitCode {
    let work = std::sync::Arc::new(work);
    let config = ShellConfig {
        tooltip: String::new(),
        app_name: String::new(),
        tray: false,
        text_locale: setup.language.locale_name(),
    };
    let outcome = bv_shell::run(config, move |shell| {
        let waker = shell.waker();
        let (results, inbox) = mpsc::channel::<Result<bool, Failure>>();
        let mut opened = false;
        let mut picking = false;
        let mut closed = false;
        let mut done = Some(done);
        move |shell, event| {
            let mut finish = |shell: &bv_shell::Shell, setup: &Setup| {
                if let Some(done) = done.take() {
                    done(setup);
                }
                shell.quit();
            };
            // Runs the work on a thread of its own with the user's choice.
            let start = |shell: &bv_shell::Shell, setup: &mut Setup| {
                shell.update_settings(setup.content());
                let (work, results, waker) = (work.clone(), results.clone(), waker.clone());
                let option = setup.option();
                let folder = setup.folder().map(std::path::Path::to_owned);
                let started =
                    std::thread::Builder::new().name("bv-setup".into()).spawn(move || {
                        let _ = results.send(work(option, folder));
                        waker.wake();
                    });
                if started.is_err() {
                    setup.finished(Err(Failure::Files));
                    shell.update_settings(setup.content());
                }
            };
            match event {
                Event::Wake if !opened => {
                    opened = true;
                    shell.show_compact_window(setup.content());
                    // No window (graphics failure): nothing can be asked, give up.
                    if !shell.settings_visible() {
                        closed = true;
                        finish(shell, &setup);
                    }
                    // An update the user already asked for starts at once.
                    else if auto_start
                        && setup.action(&SettingsAction::Invoke(PRIMARY)) == Effect::Start
                    {
                        start(shell, &mut setup);
                    }
                }
                // The picker runs from a wake-up: never while the window handles input.
                Event::Wake if std::mem::take(&mut picking) => {
                    let initial = setup.folder().map(std::path::Path::to_owned).unwrap_or_default();
                    if let Some(chosen) = shell.choose_folder(&initial, &setup.picker_title()) {
                        setup.folder_chosen(&chosen, &bv_shell::folder_usable);
                        shell.update_settings(setup.content());
                    }
                    // A work result may have arrived meanwhile.
                    waker.wake();
                }
                Event::Wake => {
                    if let Ok(outcome) = inbox.try_recv() {
                        let effect = setup.finished(outcome);
                        if closed || effect == Effect::Close {
                            shell.close_settings();
                            finish(shell, &setup);
                        } else {
                            shell.update_settings(setup.content());
                        }
                    }
                }
                Event::Settings(action) => match setup.action(&action) {
                    Effect::Refresh => shell.update_settings(setup.content()),
                    Effect::Close => shell.close_settings(),
                    Effect::ChooseFolder => {
                        picking = true;
                        waker.wake();
                    }
                    Effect::Start => start(shell, &mut setup),
                },
                Event::SettingsClosed(_) => {
                    closed = true;
                    // While the work runs, the process waits for it (never half done).
                    if setup.phase() != Phase::Working {
                        finish(shell, &setup);
                    }
                }
                _ => {}
            }
        }
    });
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            bv_shell::log::error(&format!("setup window stopped: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// The installer: this executable is the setup file `package`.
/// With `update` (started by the running copy), an existing installation is
/// updated at once, without asking again.
pub fn run_setup(package: Package, update: bool) -> ExitCode {
    let installed = bv_shell::installed();
    // Choices of an existing installation are kept (its language, start with Windows).
    // No folder is created before the user installs (no log for a cancelled setup); an
    // existing settings file that cannot be used is set aside, as at any start (FR-126).
    let chosen = crate::local_folder()
        .filter(|folder| folder.is_dir())
        .map(|folder| crate::store::Store::new(&folder).load().0);
    let language = language(chosen.as_ref().and_then(|s| s.general.language.as_deref()));
    let autostart =
        installed.as_ref().is_none_or(|i| bv_shell::autostart_enabled_for(&i.program()));
    let version = crate::VERSION;
    let previous = installed.as_ref().map(Installed::program);
    // An existing installation keeps its folder; a first one starts from the usual one.
    let folder = installed.as_ref().map(|i| i.folder.clone()).or_else(bv_shell::install_folder);
    let auto_start = update && installed.is_some();
    let mode = Mode::Install { installed: installed.map(|i| i.version) };
    let mut setup = Setup::new(mode, language, version, autostart);
    if let Some(folder) = folder {
        setup.set_folder(folder);
    }
    run_window(
        setup,
        auto_start,
        move |autostart, folder| {
            if let Some(folder) = crate::local_folder() {
                bv_shell::log::init(&folder);
            }
            let program = package.program().map_err(|_| Failure::Damaged)?;
            let helper = package.payload().map_err(|_| Failure::Damaged)?;
            if !program.starts_with(b"MZ") || !helper.starts_with(b"MZ") {
                return Err(Failure::Damaged);
            }
            if !bv_shell::end_running_instance(QUIT_TIMEOUT) {
                return Err(Failure::StillRunning);
            }
            let folder = folder.ok_or(Failure::Files)?;
            match bv_shell::install(&folder, &program, &helper, version, autostart) {
                Ok(exe) => Ok(bv_shell::launch(&exe)),
                Err(error) => {
                    // The copy that was running is not left stopped: the installed program
                    // (unchanged, or already replaced) starts again. A portable copy that was
                    // ended is not known here and stays stopped.
                    if let Some(previous) = previous.as_ref().filter(|p| p.is_file()) {
                        bv_shell::launch(previous);
                    }
                    Err(error.into())
                }
            }
        },
        |_| {},
    )
}

/// The uninstaller: this executable is the installed program, started by Windows'
/// installed-apps list.
pub fn run_uninstall() -> ExitCode {
    let local = crate::local_folder();
    if let Some(folder) = &local {
        bv_shell::log::init(folder);
    }
    // The installation the entry describes, else the folder of this executable when it
    // is the usual installation folder (an entry already removed).
    let target = bv_shell::installed().or_else(|| {
        let exe = std::env::current_exe().ok()?;
        let folder = exe.parent()?.to_owned();
        let usual = bv_shell::install_folder()?;
        same_folder(&folder, &usual).then_some(Installed { folder, version: String::new() })
    });
    let settings = local.as_ref().map(|folder| crate::store::Store::new(folder).load().0);
    let language = language(settings.as_ref().and_then(|s| s.general.language.as_deref()));
    let setup = Setup::new(Mode::Uninstall, language, crate::VERSION, true);
    let removed = target.clone();
    run_window(
        setup,
        false,
        move |keep, _| {
            if !bv_shell::end_running_instance(QUIT_TIMEOUT) {
                return Err(Failure::StillRunning);
            }
            let mut told = true;
            if !keep {
                if let Some(folder) = &local {
                    // Read now: the application may have changed them before it quit.
                    let (settings, _) = crate::store::Store::new(folder).load();
                    told = crate::app::ha::remove_at_uninstall(
                        &settings.home_assistant,
                        &crate::unprotect,
                        HA_TIMEOUT,
                    );
                    delete_data(folder);
                }
            }
            if let Some(target) = &target {
                bv_shell::remove_entries(target);
            }
            bv_shell::log::info("uninstalled");
            Ok(told)
        },
        move |setup| {
            if setup.uninstalled() {
                if let Some(target) = &removed {
                    bv_shell::remove_files_after_exit(target);
                }
            }
        },
    )
}

/// The folder BetterVolume goes into when the user picks `chosen`: its "BetterVolume"
/// sub-folder, unless `chosen` is already named so.
fn target_folder(chosen: &std::path::Path) -> PathBuf {
    let named = chosen
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(FOLDER_NAME));
    if named {
        chosen.to_owned()
    } else {
        chosen.join(FOLDER_NAME)
    }
}

fn same_folder(a: &std::path::Path, b: &std::path::Path) -> bool {
    let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_owned());
    canonical(a).to_string_lossy().eq_ignore_ascii_case(&canonical(b).to_string_lossy())
}

/// Deletes the data folder (settings, log, copies of the helper library). Copies still
/// loaded by the taskbar cannot be deleted until it restarts: they stay, harmless.
fn delete_data(folder: &PathBuf) {
    bv_shell::log::info("uninstall: deleting the settings");
    // The log goes too: close it first.
    bv_shell::log::close();
    let _ = std::fs::remove_dir_all(folder);
}

#[cfg(test)]
mod tests;
