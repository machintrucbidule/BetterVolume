//! Updates (FR-080, FR-095, FR-132): the daily check, the
//! flyout line, the About page cards, the download and what follows it.
//!
//! The work that touches the network or files is the [`UpdatePlatform`]'s: `main.rs`
//! gives one that runs it on threads of its own, tests give one fed by hand. Its answers
//! arrive through channels, drained on every wake-up ([`App::on_update_events`]). What the
//! caller must do next is taken after every event: the timer of the next automatic check
//! ([`App::take_update_timer`]) and starting the new version or opening a release page
//! ([`App::take_update_effect`]).

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use bv_audio::AudioEngine;
use bv_core::i18n::Msg;
use bv_core::package::{Kind, Version};
use bv_core::update::{self, Asset, Release};
use bv_ui_kit::controls::bubble::BubbleContent;
use bv_ui_kit::controls::flyout::UpdateLine;
use bv_ui_kit::controls::settings::{Block, Card, CardControl, SettingKey};
use bv_ui_kit::icons::Icon;

use super::App;

/// What a download reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Download {
    /// This much of the file arrived, in percent.
    Progress(u8),
    /// The whole file arrived and is the one announced (size, digest, kind), here.
    Ready(PathBuf),
    /// No file (no answer, broken connection, damaged file, nowhere to write it).
    Failed,
}

/// Starts downloading the file of a kind of a release of a version.
pub type StartDownload = dyn Fn(&Asset, &Version, Kind) -> Receiver<Download>;

/// How the network and file work is done (threads in `main.rs`, fakes in tests).
pub struct UpdatePlatform {
    /// Starts fetching the list of releases; its text (or a failure) comes back once.
    pub fetch: Box<dyn Fn() -> Receiver<Result<String, ()>>>,
    /// Starts downloading the file of `kind` of a release of `version`.
    pub download: Box<StartDownload>,
    /// Where release pages and files may be (the project site, or the local imitation).
    pub site: String,
    /// The release file that updates this copy: the setup file for the installed copy,
    /// the portable file for a portable copy whose folder can be written. `None`
    /// (development build, read-only folder): "Install" opens the release page.
    pub file: Option<Kind>,
    /// A time (Unix seconds) as the user reads it (local date and time).
    pub format_time: Box<dyn Fn(u64) -> String>,
}

/// What the caller must do to install a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateEffect {
    /// Start the downloaded file of `kind` (setup file in update mode, or the replaced
    /// portable file), then let it end this copy; report a failure with
    /// [`App::update_launch_failed`].
    Install {
        /// Which file it is.
        kind: Kind,
        /// Where it was downloaded.
        file: PathBuf,
    },
    /// Open this release page in the browser.
    OpenPage(String),
}

/// Card keys of the About page's update cards (distinct from the other cards' keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum UpdateKey {
    Status,
    Notes,
    Automatic,
    Previews,
}

impl UpdateKey {
    const ALL: [UpdateKey; 4] =
        [UpdateKey::Status, UpdateKey::Notes, UpdateKey::Automatic, UpdateKey::Previews];

    pub(super) fn key(self) -> SettingKey {
        SettingKey(0x30_0000 + self as u64)
    }

    pub(super) fn from_key(key: SettingKey) -> Option<UpdateKey> {
        UpdateKey::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// Where the update work stands.
#[derive(Default)]
pub(super) struct UpdateSession {
    platform: Option<UpdatePlatform>,
    /// The releases of the last answer.
    releases: Vec<Release>,
    /// A check runs (`true`: asked by the user).
    checking: Option<(Receiver<Result<String, ()>>, bool)>,
    /// The last check got no usable answer.
    check_failed: bool,
    /// A download runs, of this version.
    downloading: Option<(Receiver<Download>, Version)>,
    percent: u8,
    /// The new version was handed over to its file.
    installing: Option<Version>,
    /// When it was handed over (Unix seconds).
    installing_since: Option<u64>,
    /// The last download or hand-over failed.
    install_failed: bool,
    /// The timer of the automatic check must be set again.
    timer_stale: bool,
    /// An automatic check failed: the next one is at this time (Unix seconds).
    retry_at: Option<u64>,
    effect: Option<UpdateEffect>,
}

/// How long the setup file may take to end this copy before the installation counts as
/// failed (it asks at once; installing takes about a second).
const INSTALL_WAIT: u64 = 120;

/// This version.
pub(super) fn running_version() -> Version {
    Version::parse(crate::VERSION).unwrap_or(Version { numbers: [0; 3], pre: Vec::new() })
}

impl<E: AudioEngine> App<E> {
    /// Gives the means to check and download (no platform: this copy is not updated).
    pub fn set_update_platform(&mut self, platform: UpdatePlatform) {
        self.update.platform = Some(platform);
        self.update.timer_stale = true;
    }

    fn previews(&self) -> bool {
        self.settings.updates.previews
    }

    /// The version offered, if any.
    fn offered(&self) -> Option<&Release> {
        update::offer(&self.update.releases, &running_version(), self.previews())
    }

    /// Whether the automatic check is on and possible.
    fn automatic(&self) -> bool {
        self.settings.general.check_updates && self.update.platform.is_some()
    }

    /// The timer of the next automatic check, if it must change: `Some(Some(seconds))`
    /// sets it, `Some(None)` stops it. `now` is the Unix time in seconds.
    pub fn take_update_timer(&mut self, now: u64) -> Option<Option<u64>> {
        if !std::mem::take(&mut self.update.timer_stale) {
            return None;
        }
        if let Some(since) = self.update.installing_since {
            return Some(Some(
                since.saturating_add(INSTALL_WAIT).saturating_sub(now).max(update::MIN_WAIT),
            ));
        }
        Some((self.automatic() && self.update.checking.is_none()).then(
            || match self.update.retry_at {
                Some(at) => at.saturating_sub(now).max(update::MIN_WAIT),
                None => update::wait_before_check(self.settings.updates.last_check, now),
            },
        ))
    }

    /// The computer woke up: the timer may have drifted while it slept.
    pub fn update_resumed(&mut self) {
        self.update.timer_stale = true;
    }

    /// The timer fired: checks if a day went by since the last answer, otherwise waits
    /// again.
    pub fn update_due(&mut self, now: u64) {
        // The new version was handed to its file but this copy is still here: the setup
        // failed or was closed. The user can try again.
        if let (Some(version), Some(since)) =
            (&self.update.installing, self.update.installing_since)
        {
            if now.saturating_add(update::MIN_WAIT) >= since.saturating_add(INSTALL_WAIT) {
                bv_shell::log::error(&format!(
                    "update: version {version} was not installed in time"
                ));
                self.update.installing = None;
                self.update.installing_since = None;
                self.update.install_failed = true;
            }
            // Either way the one-shot timer is set again (the wait, or the daily rhythm).
            self.update.timer_stale = true;
            return;
        }
        if !self.automatic() {
            return;
        }
        let due = match self.update.retry_at {
            Some(at) => now.saturating_add(update::MIN_WAIT) >= at,
            None => update::due(self.settings.updates.last_check, now),
        };
        if due {
            self.start_check(false);
        } else {
            self.update.timer_stale = true;
        }
    }

    fn start_check(&mut self, manual: bool) {
        if self.update.checking.is_some() {
            return;
        }
        let Some(platform) = &self.update.platform else { return };
        self.update.checking = Some(((platform.fetch)(), manual));
        self.update.check_failed = false;
        self.update.retry_at = None;
    }

    /// The release list or the download has news. Returns whether what is shown changed.
    pub fn on_update_events(&mut self, now: u64) -> bool {
        let mut changed = false;
        if let Some((inbox, manual)) = &self.update.checking {
            let manual = *manual;
            let answer = match inbox.try_recv() {
                Ok(answer) => Some(answer),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            };
            if let Some(answer) = answer {
                self.update.checking = None;
                self.check_answered(answer, now, manual);
                changed = true;
            }
        }
        while let Some((inbox, _)) = &self.update.downloading {
            let event = match inbox.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => Download::Failed,
            };
            changed = true;
            match event {
                Download::Progress(percent) => self.update.percent = percent.min(100),
                Download::Ready(file) => {
                    let (_, version) = self.update.downloading.take().expect("downloading");
                    let kind = self.update.platform.as_ref().and_then(|p| p.file);
                    match kind {
                        Some(kind) => {
                            bv_shell::log::info(&format!("update: version {version} downloaded"));
                            self.update.installing = Some(version);
                            self.update.installing_since = Some(now);
                            self.update.timer_stale = true;
                            self.update.effect = Some(UpdateEffect::Install { kind, file });
                        }
                        None => self.update.install_failed = true,
                    }
                }
                Download::Failed => {
                    self.update.downloading = None;
                    self.update.install_failed = true;
                    bv_shell::log::error("update: the download failed");
                }
            }
        }
        changed
    }

    fn check_answered(&mut self, answer: Result<String, ()>, now: u64, manual: bool) {
        let site = self.update.platform.as_ref().map_or("", |p| p.site.as_str());
        match answer.ok().and_then(|text| update::releases(&text, site)) {
            Some(releases) => {
                self.update.releases = releases;
                self.update.check_failed = false;
                // A fresh answer: an earlier failed installation no longer shows.
                self.update.install_failed = false;
                self.settings.updates.last_check = Some(now);
                self.dirty = true;
                self.update.timer_stale = true;
                match self.offered() {
                    Some(r) => {
                        bv_shell::log::info(&format!("update: version {} offered", r.version))
                    }
                    None => bv_shell::log::info("update: up to date"),
                }
            }
            None => {
                self.update.check_failed = true;
                bv_shell::log::error("update: the release service gave no usable answer");
                // An automatic check tries again in an hour; a manual one keeps the daily
                // rhythm (the timer is set again from the last answer: one that fired while
                // the manual check ran must not be lost).
                if !manual && self.automatic() {
                    self.update.retry_at = Some(now.saturating_add(update::RETRY_DELAY));
                }
                self.update.timer_stale = true;
            }
        }
    }

    /// "Install" (flyout line or About page), or "Try again" after a failure.
    pub fn update_install(&mut self) {
        if self.update.downloading.is_some() || self.update.installing.is_some() {
            return;
        }
        let Some(release) = self.offered().cloned() else { return };
        let platform = self.update.platform.as_ref();
        let asset =
            platform.and_then(|p| p.file.and_then(|kind| release.asset(kind).map(|a| (a, kind))));
        match (platform, asset) {
            (Some(platform), Some((asset, kind))) => {
                self.update.downloading = Some((
                    (platform.download)(asset, &release.version, kind),
                    release.version.clone(),
                ));
                self.update.percent = 0;
                self.update.install_failed = false;
            }
            // This copy cannot replace itself: the page offers the files.
            _ => self.update.effect = Some(UpdateEffect::OpenPage(release.page.clone())),
        }
    }

    /// The flyout line was closed: it comes back for a newer version only.
    pub fn update_dismiss(&mut self) {
        if let Some(version) = self.offered().map(|r| r.version.to_string()) {
            self.settings.updates.dismissed = Some(version);
            self.dirty = true;
        }
        self.update.install_failed = false;
    }

    /// The downloaded file could not be started (or the files not swapped).
    pub fn update_launch_failed(&mut self) {
        self.update.installing = None;
        self.update.installing_since = None;
        self.update.install_failed = true;
        bv_shell::log::error("update: the new version could not be started");
    }

    /// What the caller must do now to install, if anything.
    pub fn take_update_effect(&mut self) -> Option<UpdateEffect> {
        self.update.effect.take()
    }

    /// The first start of a newer version says it was updated (once). Remembers
    /// the version that runs.
    pub fn updated_bubble(&mut self) -> Option<BubbleContent> {
        let running = running_version();
        let last = self.settings.updates.last_version.as_deref().and_then(Version::parse);
        if last.as_ref() == Some(&running) {
            return None;
        }
        self.settings.updates.last_version = Some(running.to_string());
        self.dirty = true;
        let newer = last.is_some_and(|last| running > last);
        newer.then(|| BubbleContent::Notice {
            icon: Icon::Update,
            title: self.language.text(Msg::UpdateBubble).to_owned(),
            detail: self
                .language
                .format(Msg::SettingsVersion, &[("version", &running.to_string())]),
        })
    }

    /// The flyout line, when something is offered and not closed, or while the
    /// user's request runs or failed.
    pub(super) fn update_line(&self) -> Option<UpdateLine> {
        let lang = self.language;
        let close = Some(lang.text(Msg::UpdateClose).to_owned());
        if let Some((_, version)) = &self.update.downloading {
            let text = lang.format(
                Msg::UpdateDownloading,
                &[("version", &version.to_string()), ("percent", &self.update.percent.to_string())],
            );
            return Some(UpdateLine { text, action: None, close: None });
        }
        if let Some(version) = &self.update.installing {
            let text = lang.format(Msg::UpdateInstalling, &[("version", &version.to_string())]);
            return Some(UpdateLine { text, action: None, close: None });
        }
        let release = self.offered()?;
        if self.update.install_failed {
            return Some(UpdateLine {
                text: lang.text(Msg::UpdateFailed).to_owned(),
                action: Some(lang.text(Msg::UpdateRetry).to_owned()),
                close,
            });
        }
        if !update::line_shown(&release.version, self.settings.updates.dismissed.as_deref()) {
            return None;
        }
        Some(UpdateLine {
            text: lang.format(Msg::UpdateAvailable, &[("version", &release.version.to_string())]),
            action: Some(lang.text(Msg::UpdateInstall).to_owned()),
            close,
        })
    }

    /// The update cards of the About page (FR-095).
    pub(super) fn update_cards(&self) -> Vec<Block> {
        let lang = self.language;
        let text = |msg| lang.text(msg).to_owned();
        let card =
            |key: UpdateKey, icon, title: String, description: Option<String>, control, enabled| {
                Block::Card(Card {
                    key: key.key(),
                    icon: Some(icon),
                    title,
                    description,
                    control,
                    enabled,
                })
            };
        let button = |label: Msg, accent| CardControl::Button { label: text(label), accent };
        let mut blocks = Vec::new();
        let Some(platform) = &self.update.platform else {
            blocks.push(card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateUnavailable),
                Some(text(Msg::UpdateUnavailableHelp)),
                CardControl::None,
                true,
            ));
            return blocks;
        };
        let last =
            self.settings.updates.last_check.map(|t| {
                lang.format(Msg::UpdateLastCheck, &[("time", &(platform.format_time)(t))])
            });
        let busy = self.update.downloading.is_some() || self.update.installing.is_some();
        let offered = self.offered();
        let status = if let Some((_, version)) = &self.update.downloading {
            let title = lang.format(
                Msg::UpdateDownloading,
                &[("version", &version.to_string()), ("percent", &self.update.percent.to_string())],
            );
            card(
                UpdateKey::Status,
                Icon::Download,
                title,
                None,
                button(Msg::UpdateInstall, true),
                false,
            )
        } else if let Some(version) = &self.update.installing {
            let title = lang.format(Msg::UpdateInstalling, &[("version", &version.to_string())]);
            card(
                UpdateKey::Status,
                Icon::Download,
                title,
                None,
                button(Msg::UpdateInstall, true),
                false,
            )
        } else if let (Some(release), true) = (offered, self.update.install_failed) {
            let _ = release;
            card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateFailed),
                Some(text(Msg::UpdateFailedHelp)),
                button(Msg::UpdateRetry, true),
                true,
            )
        } else if let Some(release) = offered {
            let title =
                lang.format(Msg::UpdateAvailable, &[("version", &release.version.to_string())]);
            card(
                UpdateKey::Status,
                Icon::Download,
                title,
                Some(text(Msg::UpdateAvailableHelp)),
                button(Msg::UpdateInstall, true),
                true,
            )
        } else if self.update.checking.is_some() {
            card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateChecking),
                last,
                button(Msg::UpdateCheckNow, false),
                false,
            )
        } else if self.update.check_failed {
            card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateCheckFailed),
                Some(text(Msg::UpdateCheckFailedHelp)),
                button(Msg::UpdateCheckNow, false),
                true,
            )
        } else if last.is_some() {
            card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateUpToDate),
                last,
                button(Msg::UpdateCheckNow, false),
                true,
            )
        } else {
            card(
                UpdateKey::Status,
                Icon::Update,
                text(Msg::UpdateTitle),
                Some(text(Msg::UpdateNotChecked)),
                button(Msg::UpdateCheckNow, false),
                true,
            )
        };
        blocks.push(status);
        // What changes, on the release page.
        if let Some(release) = offered {
            blocks.push(card(
                UpdateKey::Notes,
                Icon::Document,
                lang.format(Msg::UpdateWhatsNew, &[("version", &release.version.to_string())]),
                Some(text(Msg::UpdateWhatsNewHelp)),
                CardControl::Link { label: text(Msg::UpdateSeeWhatsNew) },
                true,
            ));
        }
        let toggle = |on| CardControl::Toggle {
            on,
            on_label: text(Msg::SettingsOn),
            off_label: text(Msg::SettingsOff),
        };
        blocks.push(card(
            UpdateKey::Automatic,
            Icon::Update,
            text(Msg::UpdateAutomatic),
            Some(text(Msg::UpdateAutomaticHelp)),
            toggle(self.settings.general.check_updates),
            true,
        ));
        blocks.push(card(
            UpdateKey::Previews,
            Icon::Preview,
            text(Msg::UpdatePreviews),
            Some(text(Msg::UpdatePreviewsHelp)),
            toggle(self.previews()),
            !busy,
        ));
        blocks
    }

    /// A toggle or a button of the update cards.
    pub(super) fn update_toggle(&mut self, key: UpdateKey, on: bool) {
        match key {
            UpdateKey::Automatic => {
                self.settings.general.check_updates = on;
                self.dirty = true;
                self.update.timer_stale = true;
            }
            UpdateKey::Previews => {
                self.settings.updates.previews = on;
                self.dirty = true;
                // Another version may be offered now: an earlier failure is not about it.
                self.update.install_failed = false;
            }
            UpdateKey::Status | UpdateKey::Notes => {}
        }
    }

    /// A button or link of the update cards.
    pub(super) fn update_invoke(&mut self, key: UpdateKey) {
        match key {
            UpdateKey::Status if self.offered().is_some() => self.update_install(),
            // Always possible, also with the automatic check off.
            UpdateKey::Status => self.start_check(true),
            UpdateKey::Notes => {
                if let Some(page) = self.offered().map(|r| r.page.clone()) {
                    self.update.effect = Some(UpdateEffect::OpenPage(page));
                }
            }
            UpdateKey::Automatic | UpdateKey::Previews => {}
        }
    }
}

#[cfg(test)]
mod tests;
