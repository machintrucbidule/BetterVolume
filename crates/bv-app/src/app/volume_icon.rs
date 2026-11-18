//! Hiding the Windows volume icon (FR-018, FR-079): what the
//! application knows and decides; the shell does the work (`bv_shell`, `taskbar.rs`).
//!
//! - The switch on the General page turns it on or off ([`SettingsEffect::HideVolumeIcon`]).
//! - The first-run question is shown over the settings window when it opens by
//!   itself at start, for a new installation or one older than the question; it is asked
//!   once.
//! - When the helper cannot find the icon (unknown taskbar layout) or cannot be loaded,
//!   the switch stays on and its description says so; the shell tries again at
//!   each start and each taskbar restart.
//! - After a guard-rail incident the option is off and a notice under the switch says why,
//!   until the settings window closes or the option is turned on again.

use bv_audio::AudioEngine;
use bv_core::i18n::Msg;
use bv_shell::VolumeIconOutcome;
use bv_ui_kit::controls::settings::{Notice, Question, Severity};

use super::settings_window::{Page, SettingsEffect};
use super::App;

/// What the application knows about hiding the Windows icon (not saved).
#[derive(Debug, Default)]
pub(super) struct VolumeIconSession {
    /// The icon cannot be hidden on this Windows (last outcome while wanted).
    pub(super) unavailable: bool,
    /// The guard rail turned the option off: the notice shows.
    pub(super) incident: bool,
    /// The first-run question is shown.
    pub(super) asking: bool,
}

impl<E: AudioEngine> App<E> {
    /// Whether the settings window must open by itself at start: the very first launch
    /// (FR-071), or an installation that never answered the question about the Windows
    /// volume icon.
    pub fn start_window_pending(&self) -> bool {
        self.first_run_pending() || self.volume_icon_question_pending()
    }

    /// The first-run question was never answered (an option already on needs none).
    fn volume_icon_question_pending(&self) -> bool {
        let g = &self.settings.general;
        !g.volume_icon_question_done && !g.hide_windows_volume_icon
    }

    /// The settings window is about to open by itself at start: the question comes with
    /// it when it is still to be asked.
    pub fn start_window_opening(&mut self) {
        self.volume_icon.asking = self.volume_icon_question_pending();
    }

    /// The settings window opened by itself at start did show (or not: nothing is
    /// recorded then, and it opens again at the next start).
    pub fn start_window_shown(&mut self, shown: bool) {
        if shown {
            self.first_run_done();
        } else {
            self.volume_icon.asking = false;
        }
    }

    /// Whether the user wants the Windows volume icon hidden.
    pub fn volume_icon_hidden(&self) -> bool {
        self.settings.general.hide_windows_volume_icon
    }

    /// What became of hiding (or showing) the Windows icon. Returns whether the settings
    /// window shows something new.
    pub fn volume_icon_outcome(&mut self, outcome: VolumeIconOutcome) -> bool {
        let unavailable = match outcome {
            VolumeIconOutcome::Hidden => false,
            VolumeIconOutcome::NotFound | VolumeIconOutcome::Failed => self.volume_icon_hidden(),
            VolumeIconOutcome::Restored => return false,
        };
        std::mem::replace(&mut self.volume_icon.unavailable, unavailable) != unavailable
    }

    /// The taskbar crashed right after the icon was hidden: the option is turned
    /// off, and the settings window (the caller opens it) shows why on the General page.
    pub fn volume_icon_incident(&mut self) {
        if self.settings.general.hide_windows_volume_icon {
            self.settings.general.hide_windows_volume_icon = false;
            self.dirty = true;
        }
        self.volume_icon = VolumeIconSession { incident: true, ..Default::default() };
        self.settings_window.page = Page::General;
    }

    /// The settings window closed: the notice and the question go with it.
    pub fn volume_icon_window_closed(&mut self) {
        self.volume_icon.incident = false;
        self.volume_icon.asking = false;
    }

    /// The switch of the General page (FR-079), or the question's primary button.
    pub(super) fn set_volume_icon_hidden(&mut self, on: bool) -> SettingsEffect {
        let g = &mut self.settings.general;
        if g.hide_windows_volume_icon != on || !g.volume_icon_question_done {
            g.hide_windows_volume_icon = on;
            // Choosing in the settings answers the question too.
            g.volume_icon_question_done = true;
            self.dirty = true;
        }
        self.volume_icon = VolumeIconSession::default();
        SettingsEffect::HideVolumeIcon(on)
    }

    /// The answer to the first-run question: `true` = hide the Windows icon.
    pub(super) fn volume_icon_answer(&mut self, hide: bool) -> SettingsEffect {
        self.volume_icon.asking = false;
        if hide {
            return self.set_volume_icon_hidden(true);
        }
        if !self.settings.general.volume_icon_question_done {
            self.settings.general.volume_icon_question_done = true;
            self.dirty = true;
        }
        SettingsEffect::Refresh
    }

    /// The question to show over the settings window, while it is being asked.
    pub(super) fn volume_icon_question(&self) -> Option<Question> {
        let text = |msg| self.language.text(msg).to_owned();
        self.volume_icon.asking.then(|| Question {
            title: text(Msg::QuestionHideIconTitle),
            text: text(Msg::QuestionHideIconText),
            primary: text(Msg::QuestionHideIconYes),
            secondary: text(Msg::QuestionHideIconNo),
        })
    }

    /// The description under the switch: the risks (always), after the reason it
    /// does not work when that is the case.
    pub(super) fn volume_icon_help(&self) -> String {
        let risks = self.language.text(Msg::SettingsHideIconHelp);
        if self.volume_icon.unavailable && self.volume_icon_hidden() {
            format!("{} {risks}", self.language.text(Msg::SettingsHideIconUnavailable))
        } else {
            risks.to_owned()
        }
    }

    /// The notice under the switch after an incident.
    pub(super) fn volume_icon_notice(
        &self,
        key: bv_ui_kit::controls::settings::SettingKey,
    ) -> Option<Notice> {
        let text = |msg| self.language.text(msg).to_owned();
        self.volume_icon.incident.then(|| Notice {
            key,
            severity: Severity::Caution,
            title: text(Msg::SettingsHideIconIncidentTitle),
            message: text(Msg::SettingsHideIconIncident),
            action: None,
        })
    }
}

#[cfg(test)]
mod tests;
