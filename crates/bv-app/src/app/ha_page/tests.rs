//! The Home Assistant page: the switch, the draft form, "Save", "Test connection",
//! trusting a certificate and the close question.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use bv_audio::fake::FakeEngine;
use bv_core::ha::Message;
use bv_core::i18n::Language;
use bv_core::settings::Settings;
use bv_mqtt::{Event, Options};
use bv_ui_kit::controls::settings::{SettingsAction, SettingsContent};

use super::super::ha::{Link, Platform};
use super::super::settings_window::{Page, SettingsEffect};
use super::*;

#[derive(Default)]
struct Wire {
    connects: Vec<Options>,
    tests: Vec<(Options, Sender<Result<(), Problem>>)>,
    farewells: Vec<Vec<Message>>,
    pending: Vec<Event>,
}

struct FakeLink(Rc<RefCell<Wire>>);

impl Link for FakeLink {
    fn publish(&mut self, _: Message) {}

    fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.0.borrow_mut().pending)
    }

    fn stop(self: Box<Self>, farewell: Vec<Message>) {
        self.0.borrow_mut().farewells.push(farewell);
    }
}

fn app(settings: Settings) -> (App<FakeEngine>, Rc<RefCell<Wire>>) {
    let wire = Rc::new(RefCell::new(Wire::default()));
    let mut app = App::new(FakeEngine::new(), settings, Language::ENGLISH);
    let (connect, test) = (Rc::clone(&wire), Rc::clone(&wire));
    app.set_ha_platform(Platform {
        connect: Box::new(move |options| {
            connect.borrow_mut().connects.push(options);
            Box::new(FakeLink(Rc::clone(&connect)))
        }),
        unprotect: Box::new(|stored| stored.strip_prefix("enc:").map(str::to_owned)),
        protect: Box::new(|password| Some(format!("enc:{password}"))),
        test: Box::new(move |options| {
            let (done, result) = mpsc::channel();
            test.borrow_mut().tests.push((options, done));
            result
        }),
        computer_name: "DESKTOP-1".into(),
    });
    app.settings_opening(false);
    app.settings_action(SettingsAction::SelectPage(Page::HomeAssistant.key()));
    app.take_dirty();
    (app, wire)
}

fn configured() -> Settings {
    let mut settings = Settings::default();
    let ha = &mut settings.home_assistant;
    ha.enabled = true;
    ha.host = "broker.lan".into();
    ha.username = "user".into();
    ha.password = Some("enc:old".into());
    settings
}

fn act(app: &mut App<FakeEngine>, action: SettingsAction) -> SettingsEffect {
    app.settings_action(action)
}

fn text(app: &mut App<FakeEngine>, key: HaKey, value: &str) {
    act(app, SettingsAction::SetText(key.key(), value.into()));
}

fn field(content: &SettingsContent, key: HaKey) -> TextField {
    content.field(key.key()).cloned().expect("field shown")
}

fn button_enabled(content: &SettingsContent, key: HaKey) -> bool {
    content.buttons().find(|b| b.key == key.key()).expect("button shown").enabled
}

fn notice(content: &SettingsContent) -> Option<Notice> {
    content.notices().next().cloned()
}

#[test]
fn the_page_shows_the_saved_settings_and_the_status() {
    let (app, _) = app(configured());
    let content = app.settings_content();
    assert_eq!(content.nav.len(), 5, "General, Devices, Applications, Home Assistant, About");
    assert_eq!(content.page, Page::HomeAssistant.key());
    let switch = content.card(HaKey::Enabled.key()).unwrap();
    assert!(matches!(switch.control, CardControl::Toggle { on: true, .. }));
    assert_eq!(switch.description.as_deref(), Some("Connecting…"));
    assert_eq!(field(&content, HaKey::Host).value, "broker.lan");
    assert_eq!(field(&content, HaKey::Port).value, "1883");
    assert!(field(&content, HaKey::Port).digits_only);
    let password = field(&content, HaKey::Password);
    assert!(password.password && password.value.is_empty(), "never shown back (FR-092)");
    assert_eq!(password.placeholder, "Saved, type to replace it");
    assert_eq!(field(&content, HaKey::PcName).placeholder, "DESKTOP-1", "the default name");
    assert_eq!(field(&content, HaKey::Prefix).value, "homeassistant");
    assert!(button_enabled(&content, HaKey::Test));
    assert!(!button_enabled(&content, HaKey::Save), "nothing to save");
    assert_eq!(content.close_question, None);
}

#[test]
fn the_switch_applies_at_once() {
    let (mut app, wire) = app(configured());
    assert_eq!(wire.borrow().connects.len(), 1);
    act(&mut app, SettingsAction::Toggle(HaKey::Enabled.key(), false));
    assert!(!app.settings().home_assistant.enabled);
    assert!(app.take_dirty());
    let farewell = wire.borrow_mut().farewells.pop().expect("stopped");
    assert!(farewell.iter().all(|m| m.payload.is_empty()), "entities removed (FR-116)");
    assert_eq!(app.ha_status(), Status::Off);
    act(&mut app, SettingsAction::Toggle(HaKey::Enabled.key(), true));
    assert_eq!(wire.borrow().connects.len(), 2, "started again");
}

#[test]
fn the_form_is_a_draft_until_saved() {
    let (mut app, wire) = app(configured());
    text(&mut app, HaKey::Host, "192.0.2.10");
    text(&mut app, HaKey::Password, "new secret");
    act(&mut app, SettingsAction::Toggle(HaKey::Tls.key(), true));
    assert_eq!(app.settings().home_assistant.host, "broker.lan", "not applied yet");
    assert!(!app.take_dirty());
    let content = app.settings_content();
    assert!(button_enabled(&content, HaKey::Save));
    assert!(content.close_question.is_some(), "closing would ask");
    assert_eq!(field(&content, HaKey::Host).value, "192.0.2.10", "the draft is shown");

    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    let ha = &app.settings().home_assistant;
    assert_eq!((ha.host.as_str(), ha.tls), ("192.0.2.10", true));
    assert_eq!(ha.password.as_deref(), Some("enc:new secret"), "stored encrypted");
    assert!(app.take_dirty());
    let wire = wire.borrow();
    assert_eq!(wire.farewells.len(), 1, "the old connection said goodbye");
    let options = wire.connects.last().unwrap();
    assert_eq!(options.host, "192.0.2.10");
    assert_eq!(options.password.as_deref(), Some("new secret"));
    assert!(options.tls.is_some());
    drop(wire);
    let content = app.settings_content();
    assert!(!button_enabled(&content, HaKey::Save));
    assert_eq!(content.close_question, None);
    assert!(field(&content, HaKey::Password).value.is_empty());
}

#[test]
fn a_new_pc_name_needs_no_reconnection() {
    let (mut app, wire) = app(configured());
    text(&mut app, HaKey::PcName, "  Office  ");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.pc_name.as_deref(), Some("Office"));
    assert_eq!(wire.borrow().connects.len(), 1);
    // Emptied: back to the computer name.
    text(&mut app, HaKey::PcName, "");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.pc_name, None);
}

#[test]
fn an_invalid_form_is_refused_with_a_message() {
    let (mut app, _) = app(configured());
    text(&mut app, HaKey::Port, "70000");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.port, 1883);
    let shown = notice(&app.settings_content()).expect("message");
    assert_eq!((shown.severity, shown.title.as_str()), (Severity::Critical, "Invalid port"));
    // Typing again removes the message.
    text(&mut app, HaKey::Port, "1884");
    assert_eq!(notice(&app.settings_content()), None);
    text(&mut app, HaKey::Prefix, "home/#");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(notice(&app.settings_content()).unwrap().title, "Invalid discovery prefix");
    assert_eq!(app.settings().home_assistant.discovery_prefix, "homeassistant");
}

#[test]
fn a_changed_prefix_removes_the_entities_from_the_old_one() {
    let (mut app, wire) = app(configured());
    wire.borrow_mut().pending.push(Event::Connected);
    app.on_ha_events();
    text(&mut app, HaKey::Prefix, "ha");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    let farewell = wire.borrow_mut().farewells.pop().unwrap();
    assert!(farewell.iter().any(|m| m.topic.starts_with("homeassistant/") && m.payload.is_empty()));
}

#[test]
fn testing_uses_the_draft_in_the_background() {
    let (mut app, wire) = app(configured());
    text(&mut app, HaKey::Host, "other.lan");
    act(&mut app, SettingsAction::Invoke(HaKey::Test.key()));
    let content = app.settings_content();
    assert_eq!(notice(&content).unwrap().title, "Testing the connection…");
    assert!(!button_enabled(&content, HaKey::Test), "one test at a time");
    let (options, done) = wire.borrow_mut().tests.pop().expect("test started");
    assert_eq!(options.host, "other.lan");
    assert_eq!(options.password.as_deref(), Some("old"), "the stored password");
    assert!(options.client_id.starts_with("bettervolume-test-"), "never the link's identifier");
    assert!(options.will.is_none() && options.subscriptions.is_empty());

    done.send(Err(Problem::Refused(bv_mqtt::Refusal::Credentials))).unwrap();
    assert!(app.on_ha_events().refresh);
    let shown = notice(&app.settings_content()).unwrap();
    assert_eq!(shown.title, "User name or password refused");
    assert_eq!(shown.action, None);

    act(&mut app, SettingsAction::Invoke(HaKey::Test.key()));
    let (_, done) = wire.borrow_mut().tests.pop().unwrap();
    done.send(Ok(())).unwrap();
    app.on_ha_events();
    assert_eq!(notice(&app.settings_content()).unwrap().severity, Severity::Success);
}

#[test]
fn an_unknown_certificate_can_be_trusted() {
    let mut settings = configured();
    settings.home_assistant.tls = true;
    let (mut app, wire) = app(settings);
    act(&mut app, SettingsAction::Invoke(HaKey::Test.key()));
    let (_, done) = wire.borrow_mut().tests.pop().unwrap();
    done.send(Err(Problem::CertificateUnknown([0xAB; 32]))).unwrap();
    app.on_ha_events();
    let shown = notice(&app.settings_content()).unwrap();
    assert_eq!(
        (shown.severity, shown.action.as_deref()),
        (Severity::Caution, Some("Trust this server"))
    );

    act(&mut app, SettingsAction::Invoke(HaKey::Notice.key()));
    assert_eq!(
        app.settings().home_assistant.trusted_certificate.as_deref(),
        Some("ab".repeat(32).as_str())
    );
    let options = wire.borrow().connects.last().cloned().unwrap();
    assert_eq!(options.tls.unwrap().trusted, Some([0xAB; 32]), "used at once");

    // Another server: the certificate trusted for the old one goes.
    text(&mut app, HaKey::Host, "elsewhere.lan");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.trusted_certificate, None);
}

#[test]
fn the_link_problem_shows_and_a_changed_certificate_can_be_trusted() {
    let mut settings = configured();
    settings.home_assistant.tls = true;
    let (mut app, wire) = app(settings);
    wire.borrow_mut().pending.push(Event::Disconnected {
        problem: Problem::CertificateChanged([1; 32]),
        retry_in: std::time::Duration::from_secs(1),
    });
    app.on_ha_events();
    let content = app.settings_content();
    assert_eq!(
        content.card(HaKey::Enabled.key()).unwrap().description.as_deref(),
        Some("Not connected: trying again automatically")
    );
    assert_eq!(notice(&content).unwrap().title, "The certificate of the server changed");
    act(&mut app, SettingsAction::Invoke(HaKey::Notice.key()));
    assert_eq!(app.settings().home_assistant.trusted_certificate, Some("01".repeat(32)));
}

#[test]
fn the_close_question_saves_or_drops_the_draft() {
    let (mut app, _) = app(configured());
    text(&mut app, HaKey::User, "someone");
    assert_eq!(act(&mut app, SettingsAction::CloseAnswer(false)), SettingsEffect::Close);
    assert_eq!(app.settings().home_assistant.username, "user");
    assert_eq!(app.settings_content().close_question, None);

    text(&mut app, HaKey::User, "someone");
    assert_eq!(act(&mut app, SettingsAction::CloseAnswer(true)), SettingsEffect::Close);
    assert_eq!(app.settings().home_assistant.username, "someone");

    // A refused form keeps the window open, with its message.
    text(&mut app, HaKey::Port, "0");
    assert_eq!(act(&mut app, SettingsAction::CloseAnswer(true)), SettingsEffect::Refresh);
    assert!(notice(&app.settings_content()).is_some());
    // The draft stays across pages and is dropped when the window opens again.
    act(&mut app, SettingsAction::SelectPage(Page::General.key()));
    assert!(app.settings_content().close_question.is_some());
    app.settings_opening(false);
    assert_eq!(app.settings_content().close_question, None);
}

#[test]
fn a_refused_save_on_closing_shows_the_page_and_its_message() {
    let (mut app, _) = app(configured());
    text(&mut app, HaKey::Port, "0");
    act(&mut app, SettingsAction::SelectPage(Page::General.key()));
    assert_eq!(act(&mut app, SettingsAction::CloseAnswer(true)), SettingsEffect::Refresh);
    let content = app.settings_content();
    assert_eq!(content.page, Page::HomeAssistant.key(), "back where the problem is");
    assert_eq!(notice(&content).unwrap().title, "Invalid port");
}

#[test]
fn the_link_certificate_is_trusted_for_the_saved_server_only() {
    let mut settings = configured();
    settings.home_assistant.tls = true;
    let (mut app, wire) = app(settings);
    text(&mut app, HaKey::Host, "typed.lan");
    wire.borrow_mut().pending.push(Event::Disconnected {
        problem: Problem::CertificateUnknown([2; 32]),
        retry_in: std::time::Duration::from_secs(1),
    });
    app.on_ha_events();
    act(&mut app, SettingsAction::Invoke(HaKey::Notice.key()));
    let ha = &app.settings().home_assistant;
    assert_eq!(ha.host, "broker.lan", "the typed host is still a draft");
    assert_eq!(ha.trusted_certificate, Some("02".repeat(32)));
    assert_eq!(wire.borrow().connects.last().unwrap().host, "broker.lan");
    // Saving the typed host afterwards drops a certificate seen on the other server.
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.host, "typed.lan");
    assert_eq!(app.settings().home_assistant.trusted_certificate, None);
}

#[test]
fn a_password_that_cannot_be_protected_is_not_saved() {
    let (mut app, _) = app(configured());
    app.ha.platform.as_mut().unwrap().protect = Box::new(|_| None);
    text(&mut app, HaKey::Password, "new");
    act(&mut app, SettingsAction::Invoke(HaKey::Save.key()));
    assert_eq!(app.settings().home_assistant.password.as_deref(), Some("enc:old"));
    assert_eq!(notice(&app.settings_content()).unwrap().title, "The password could not be stored");
    assert!(app.settings_content().close_question.is_some(), "the draft is kept");
}
