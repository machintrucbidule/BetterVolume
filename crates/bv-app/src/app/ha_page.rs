//! The Home Assistant page of the settings window (FR-090 to FR-092).
//!
//! The enable switch applies at once; the server form (address, port, encryption, user
//! name, password, PC name, discovery prefix) is a draft until "Save" (FR-070). "Test
//! connection" tries the draft in the background. A certificate Windows does not trust
//! can be trusted for this server (its fingerprint is saved with the form). Closing the
//! window with an unsaved draft asks first (the view's close question).

use std::sync::mpsc::Receiver;

use bv_audio::AudioEngine;
use bv_core::encoding::hex;
use bv_core::i18n::Msg;
use bv_core::settings::{valid_discovery_prefix, HomeAssistant, HA_TEXT_MAX};
use bv_mqtt::{Problem, Refusal};
use bv_ui_kit::controls::settings::{
    Block, Button, Card, CardControl, CloseQuestion, Notice, SettingKey, Severity, TextField,
};
use bv_ui_kit::icons::Icon;

use super::ha::{connection_options, Farewell, Status};
use super::App;

/// The keys of the page's controls (above the General page's, below the device rows').
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HaKey {
    Enabled = 100,
    Host,
    Port,
    Tls,
    User,
    Password,
    PcName,
    Prefix,
    Test,
    Save,
    Notice,
}

impl HaKey {
    const ALL: [HaKey; 11] = [
        HaKey::Enabled,
        HaKey::Host,
        HaKey::Port,
        HaKey::Tls,
        HaKey::User,
        HaKey::Password,
        HaKey::PcName,
        HaKey::Prefix,
        HaKey::Test,
        HaKey::Save,
        HaKey::Notice,
    ];

    pub(super) fn key(self) -> SettingKey {
        SettingKey(self as u64)
    }

    pub(super) fn from_key(key: SettingKey) -> Option<HaKey> {
        HaKey::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// The server form as typed (applied on "Save" only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct HaForm {
    host: String,
    port: String,
    tls: bool,
    username: String,
    /// A new password typed; `None` = keep the stored one (never shown back, FR-092).
    password: Option<String>,
    pc_name: String,
    prefix: String,
    /// The certificate trusted for this server, hex.
    trusted: Option<String>,
}

impl HaForm {
    pub(super) fn of(ha: &HomeAssistant) -> HaForm {
        HaForm {
            host: ha.host.clone(),
            port: ha.port.to_string(),
            tls: ha.tls,
            username: ha.username.clone(),
            password: None,
            pc_name: ha.pc_name.clone().unwrap_or_default(),
            prefix: ha.discovery_prefix.clone(),
            trusted: ha.trusted_certificate.clone(),
        }
    }
}

/// What the page reports below the switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    /// A test runs.
    Testing,
    /// The last test's result.
    Tested(Result<(), Problem>),
    /// "Save" refused the form: title and detail.
    Invalid(Msg, Msg),
}

/// The page's state while the settings window is open.
#[derive(Default)]
pub(super) struct HaPage {
    pub(super) form: Option<HaForm>,
    pub(super) outcome: Option<Outcome>,
    pub(super) test: Option<Receiver<Result<(), Problem>>>,
}

/// Title and detail of a connection problem, and whether it can be solved by trusting the
/// certificate.
fn problem_texts(problem: &Problem) -> (Msg, Msg) {
    match problem {
        Problem::Unreachable => (Msg::SettingsHaUnreachable, Msg::SettingsHaUnreachableDetail),
        Problem::Refused(Refusal::Credentials) => {
            (Msg::SettingsHaCredentials, Msg::SettingsHaCredentialsDetail)
        }
        Problem::Refused(Refusal::NotAuthorized) => {
            (Msg::SettingsHaNotAuthorized, Msg::SettingsHaNotAuthorizedDetail)
        }
        Problem::Refused(Refusal::Other(_)) => {
            (Msg::SettingsHaRefused, Msg::SettingsHaRefusedDetail)
        }
        Problem::CertificateUnknown(_) => {
            (Msg::SettingsHaCertUnknown, Msg::SettingsHaCertUnknownDetail)
        }
        Problem::CertificateChanged(_) => {
            (Msg::SettingsHaCertChanged, Msg::SettingsHaCertChangedDetail)
        }
        Problem::Encryption => (Msg::SettingsHaEncryption, Msg::SettingsHaEncryptionDetail),
        Problem::Protocol => (Msg::SettingsHaProtocol, Msg::SettingsHaProtocolDetail),
        Problem::Lost => (Msg::SettingsHaLost, Msg::SettingsHaLostDetail),
    }
}

/// The certificate a problem offers to trust.
fn certificate_of(problem: &Problem) -> Option<[u8; 32]> {
    match problem {
        Problem::CertificateUnknown(f) | Problem::CertificateChanged(f) => Some(*f),
        _ => None,
    }
}

impl<E: AudioEngine> App<E> {
    /// The draft form (the saved settings until something is typed).
    fn ha_form(&self) -> HaForm {
        self.settings_window
            .ha
            .form
            .clone()
            .unwrap_or_else(|| HaForm::of(&self.settings.home_assistant))
    }

    /// Whether the form holds unsaved changes.
    pub(super) fn ha_unsaved(&self) -> bool {
        self.settings_window
            .ha
            .form
            .as_ref()
            .is_some_and(|f| *f != HaForm::of(&self.settings.home_assistant))
    }

    /// The close question, while the form holds unsaved changes.
    pub(super) fn close_question(&self) -> Option<CloseQuestion> {
        let text = |msg| self.language.text(msg).to_owned();
        self.ha_unsaved().then(|| CloseQuestion {
            title: text(Msg::SettingsUnsavedTitle),
            text: text(Msg::SettingsUnsavedText),
            save: text(Msg::SettingsUnsavedSave),
            discard: text(Msg::SettingsUnsavedDiscard),
            cancel: text(Msg::SettingsCancel),
        })
    }

    fn ha_card(
        &self,
        key: HaKey,
        icon: Icon,
        title: Msg,
        help: Msg,
        control: CardControl,
    ) -> Block {
        Block::Card(Card {
            key: key.key(),
            icon: Some(icon),
            title: self.language.text(title).to_owned(),
            description: Some(self.language.text(help).to_owned()),
            control,
            enabled: true,
        })
    }

    fn ha_field(
        &self,
        value: &str,
        placeholder: &str,
        password: bool,
        digits: bool,
    ) -> CardControl {
        CardControl::TextField(TextField {
            value: value.to_owned(),
            placeholder: placeholder.to_owned(),
            password,
            digits_only: digits,
            max_chars: if digits { 5 } else { HA_TEXT_MAX },
        })
    }

    /// The notice of the page: a refused form, the last test, or the link's problem.
    fn ha_notice(&self) -> Option<Notice> {
        let text = |msg| self.language.text(msg).to_owned();
        let notice = |severity, title: Msg, message: Msg, action: bool| Notice {
            key: HaKey::Notice.key(),
            severity,
            title: text(title),
            message: text(message),
            action: action.then(|| text(Msg::SettingsHaTrust)),
        };
        let problem = |p: &Problem| {
            let (title, detail) = problem_texts(p);
            let severity = if certificate_of(p).is_some() || *p == Problem::Lost {
                Severity::Caution
            } else {
                Severity::Critical
            };
            notice(severity, title, detail, certificate_of(p).is_some())
        };
        match &self.settings_window.ha.outcome {
            Some(Outcome::Invalid(title, detail)) => {
                Some(notice(Severity::Critical, *title, *detail, false))
            }
            Some(Outcome::Testing) => Some(Notice {
                message: String::new(),
                ..notice(Severity::Info, Msg::SettingsHaTesting, Msg::SettingsHaTesting, false)
            }),
            Some(Outcome::Tested(Ok(()))) => Some(notice(
                Severity::Success,
                Msg::SettingsHaTestOk,
                Msg::SettingsHaTestOkDetail,
                false,
            )),
            Some(Outcome::Tested(Err(p))) => Some(problem(p)),
            None => match self.ha_status() {
                Status::Failed(p) => Some(problem(&p)),
                _ => None,
            },
        }
    }

    /// The Home Assistant page (FR-090, mockup scene 5).
    pub(super) fn ha_blocks(&self) -> Vec<Block> {
        let text = |msg| self.language.text(msg).to_owned();
        let ha = &self.settings.home_assistant;
        let form = self.ha_form();
        let status = match self.ha_status() {
            Status::Off => Msg::SettingsHaStatusOff,
            Status::NotConfigured => Msg::SettingsHaStatusNotConfigured,
            Status::Connecting => Msg::SettingsHaStatusConnecting,
            Status::Connected => Msg::SettingsHaStatusConnected,
            Status::Failed(_) => Msg::SettingsHaStatusFailed,
        };
        let toggle = |on| CardControl::Toggle {
            on,
            on_label: text(Msg::SettingsOn),
            off_label: text(Msg::SettingsOff),
        };
        let mut blocks = vec![self.ha_card(
            HaKey::Enabled,
            Icon::Home,
            Msg::SettingsHaLink,
            status,
            toggle(ha.enabled),
        )];
        blocks.extend(self.ha_notice().map(Block::Notice));
        let password_hint =
            if ha.password.is_some() { text(Msg::SettingsHaPasswordSaved) } else { String::new() };
        let computer = self.ha.computer_name();
        blocks.extend([
            Block::Header(text(Msg::SettingsHaServer)),
            self.ha_card(
                HaKey::Host,
                Icon::Globe,
                Msg::SettingsHaHost,
                Msg::SettingsHaHostHelp,
                self.ha_field(&form.host, "", false, false),
            ),
            self.ha_card(
                HaKey::Port,
                Icon::Ethernet,
                Msg::SettingsHaPort,
                Msg::SettingsHaPortHelp,
                self.ha_field(&form.port, "", false, true),
            ),
            self.ha_card(
                HaKey::Tls,
                Icon::Lock,
                Msg::SettingsHaTls,
                Msg::SettingsHaTlsHelp,
                toggle(form.tls),
            ),
            self.ha_card(
                HaKey::User,
                Icon::Contact,
                Msg::SettingsHaUser,
                Msg::SettingsHaUserHelp,
                self.ha_field(&form.username, "", false, false),
            ),
            self.ha_card(
                HaKey::Password,
                Icon::Key,
                Msg::SettingsHaPassword,
                Msg::SettingsHaPasswordHelp,
                self.ha_field(form.password.as_deref().unwrap_or(""), &password_hint, true, false),
            ),
            Block::Header(text(Msg::SettingsHaThisPc)),
            self.ha_card(
                HaKey::PcName,
                Icon::Pc,
                Msg::SettingsHaPcName,
                Msg::SettingsHaPcNameHelp,
                self.ha_field(&form.pc_name, &computer, false, false),
            ),
            self.ha_card(
                HaKey::Prefix,
                Icon::Tag,
                Msg::SettingsHaPrefix,
                Msg::SettingsHaPrefixHelp,
                self.ha_field(&form.prefix, "", false, false),
            ),
            Block::Buttons(vec![
                Button {
                    key: HaKey::Test.key(),
                    label: text(Msg::SettingsHaTest),
                    accent: true,
                    enabled: !form.host.trim().is_empty()
                        && self.settings_window.ha.outcome != Some(Outcome::Testing),
                },
                Button {
                    key: HaKey::Save.key(),
                    label: text(Msg::SettingsHaSave),
                    accent: false,
                    enabled: self.ha_unsaved(),
                },
            ]),
        ]);
        blocks
    }

    fn form_mut(&mut self) -> &mut HaForm {
        let saved = HaForm::of(&self.settings.home_assistant);
        let page = &mut self.settings_window.ha;
        // A refused form's message goes once the user changes something.
        if matches!(page.outcome, Some(Outcome::Invalid(..))) {
            page.outcome = None;
        }
        page.form.get_or_insert(saved)
    }

    /// The switch of the page: applies at once (FR-070). Off removes the PC from Home
    /// Assistant (FR-116).
    pub(super) fn ha_set_enabled(&mut self, on: bool) {
        if self.settings.home_assistant.enabled == on {
            return;
        }
        self.settings.home_assistant.enabled = on;
        self.dirty = true;
        self.settings_window.ha.outcome = None;
        if on {
            self.ha_start();
        } else {
            self.ha_stop(Farewell::Remove);
        }
    }

    /// The encryption switch of the form (a draft, like the fields).
    pub(super) fn ha_set_tls(&mut self, on: bool) {
        self.form_mut().tls = on;
    }

    /// A field of the form changed.
    pub(super) fn ha_set_text(&mut self, key: HaKey, value: String) {
        let form = self.form_mut();
        match key {
            HaKey::Host => form.host = value,
            HaKey::Port => form.port = value,
            HaKey::User => form.username = value,
            HaKey::Password => form.password = (!value.is_empty()).then_some(value),
            HaKey::PcName => form.pc_name = value,
            HaKey::Prefix => form.prefix = value,
            _ => {}
        }
    }

    /// The settings the form describes, or why it is refused.
    fn ha_from_form(&self, form: &HaForm) -> Result<HomeAssistant, (Msg, Msg)> {
        let saved = &self.settings.home_assistant;
        let port = form
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or((Msg::SettingsHaBadPort, Msg::SettingsHaBadPortDetail))?;
        let prefix = form.prefix.trim().to_owned();
        if !valid_discovery_prefix(&prefix) {
            return Err((Msg::SettingsHaBadPrefix, Msg::SettingsHaBadPrefixDetail));
        }
        let host = form.host.trim().to_owned();
        let server_changed = host != saved.host || port != saved.port || form.tls != saved.tls;
        // A certificate trusted for another server is not kept.
        let trusted = if server_changed && form.trusted == saved.trusted_certificate {
            None
        } else {
            form.trusted.clone()
        };
        let pc_name = form.pc_name.trim();
        Ok(HomeAssistant {
            host,
            port,
            tls: form.tls,
            username: form.username.trim().to_owned(),
            pc_name: (!pc_name.is_empty()).then(|| pc_name.to_owned()),
            discovery_prefix: prefix,
            trusted_certificate: trusted,
            ..saved.clone()
        })
    }

    /// "Save": applies the form and restarts the link if the connection changed. Returns
    /// whether it was saved.
    pub(super) fn ha_save(&mut self) -> bool {
        let form = self.ha_form();
        let mut new = match self.ha_from_form(&form) {
            Ok(new) => new,
            Err((title, detail)) => {
                self.settings_window.ha.outcome = Some(Outcome::Invalid(title, detail));
                return false;
            }
        };
        if let Some(password) = &form.password {
            match self.ha.protect(password) {
                Some(protected) => new.password = Some(protected),
                None => {
                    // Never stored in clear (FR-092): nothing is saved, the user is told.
                    bv_shell::log::error("home assistant: the password cannot be protected");
                    self.settings_window.ha.outcome = Some(Outcome::Invalid(
                        Msg::SettingsHaPasswordFailed,
                        Msg::SettingsHaPasswordFailedDetail,
                    ));
                    return false;
                }
            }
        }
        let old = self.settings.home_assistant.clone();
        self.settings_window.ha.form = None;
        self.settings_window.ha.outcome = None;
        if new == old {
            return true;
        }
        let connection_changed = HomeAssistant { pc_name: None, ..new.clone() }
            != HomeAssistant { pc_name: None, ..old.clone() };
        if connection_changed {
            // Entities under another prefix would stay behind: remove them first.
            let farewell = if new.discovery_prefix != old.discovery_prefix {
                Farewell::Remove
            } else {
                Farewell::Offline
            };
            self.ha_stop(farewell);
        }
        self.settings.home_assistant = new;
        self.dirty = true;
        if connection_changed {
            self.ha_start();
        }
        true
    }

    /// "Don't save": the draft is dropped.
    pub(super) fn ha_discard(&mut self) {
        self.settings_window.ha.form = None;
        self.settings_window.ha.outcome = None;
    }

    /// "Test connection": tries the draft in the background (FR-091).
    pub(super) fn ha_test(&mut self) {
        let form = self.ha_form();
        let settings = match self.ha_from_form(&form) {
            Ok(settings) => settings,
            Err((title, detail)) => {
                self.settings_window.ha.outcome = Some(Outcome::Invalid(title, detail));
                return;
            }
        };
        let password = form.password.clone().or_else(|| {
            let stored = self.settings.home_assistant.password.as_deref()?;
            self.ha.unprotect(stored)
        });
        let mut options = connection_options(&settings, password);
        // Never the link's own identifier: the broker would drop the link for it.
        options.client_id = format!("bettervolume-test-{}", std::process::id());
        if let Some(receiver) = self.ha.run_test(options) {
            self.settings_window.ha.test = Some(receiver);
            self.settings_window.ha.outcome = Some(Outcome::Testing);
        }
    }

    /// The result of a running test, if it arrived. Returns whether the page changed.
    pub(super) fn ha_test_done(&mut self) -> bool {
        let Some(receiver) = &self.settings_window.ha.test else { return false };
        let Ok(result) = receiver.try_recv() else { return false };
        self.settings_window.ha.test = None;
        self.settings_window.ha.outcome = Some(Outcome::Tested(result));
        true
    }

    /// "Trust this server": the certificate the notice shows is trusted, the form
    /// saved with it, and the link restarted.
    pub(super) fn ha_trust(&mut self) {
        let problem = match &self.settings_window.ha.outcome {
            Some(Outcome::Tested(Err(p))) => Some(p.clone()),
            None => match self.ha_status() {
                Status::Failed(p) => Some(p),
                _ => None,
            },
            _ => None,
        };
        let Some(fingerprint) = problem.as_ref().and_then(certificate_of) else { return };
        let fingerprint = hex(&fingerprint);
        if self.settings_window.ha.outcome.is_some() {
            // A test of the typed values: the certificate belongs to the typed server,
            // saved with them.
            self.form_mut().trusted = Some(fingerprint);
            self.ha_save();
            return;
        }
        // The link's own problem: the certificate belongs to the saved server, whatever
        // is typed in the form (one certificate for one server).
        let saved = HaForm::of(&self.settings.home_assistant);
        self.settings.home_assistant.trusted_certificate = Some(fingerprint.clone());
        self.dirty = true;
        if let Some(form) = self.settings_window.ha.form.as_mut() {
            if form.trusted == saved.trusted {
                form.trusted = Some(fingerprint);
            }
        }
        self.ha_stop(Farewell::Offline);
        self.ha_start();
    }
}

#[cfg(test)]
mod tests;
