//! Updates: the daily check, the flyout line, the About page, the download and the
//! hand-over to the new version (FR-132).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use bv_audio::fake::FakeEngine;
use bv_core::i18n::Language;
use bv_core::package::Kind;
use bv_core::settings::Settings;
use bv_core::update::{DAY, FIRST_DELAY, RETRY_DELAY};
use bv_ui_kit::controls::bubble::BubbleContent;
use bv_ui_kit::controls::flyout::FlyoutAction;
use bv_ui_kit::controls::settings::{Card, CardControl, SettingsAction, SettingsContent};

use super::super::settings_window::Page;
use super::super::App;
use super::{Download, UpdateEffect, UpdateKey, UpdatePlatform};

const SITE: &str = "http://127.0.0.1:9";
const NOW: u64 = 1_800_000_000;

/// What the fake platform was asked, and the means to answer.
#[derive(Default)]
struct Calls {
    fetches: Vec<Sender<Result<String, ()>>>,
    downloads: Vec<(String, Kind, Sender<Download>)>,
}

fn app_with(settings: Settings, file: Option<Kind>) -> (App<FakeEngine>, Rc<RefCell<Calls>>) {
    let mut app = App::new(FakeEngine::new(), settings, Language::ENGLISH);
    let calls = Rc::new(RefCell::new(Calls::default()));
    let (fetch_calls, download_calls) = (calls.clone(), calls.clone());
    app.set_update_platform(UpdatePlatform {
        fetch: Box::new(move || {
            let (tx, rx) = mpsc::channel();
            fetch_calls.borrow_mut().fetches.push(tx);
            rx
        }),
        download: Box::new(move |asset, version, kind| {
            let (tx, rx) = mpsc::channel();
            download_calls.borrow_mut().downloads.push((
                format!("{}|{version}", asset.url),
                kind,
                tx,
            ));
            rx
        }),
        site: SITE.into(),
        file,
        format_time: Box::new(|t| format!("t{t}")),
    });
    (app, calls)
}

fn app() -> (App<FakeEngine>, Rc<RefCell<Calls>>) {
    app_with(Settings::default(), Some(Kind::Setup))
}

/// A release answer with these `(tag, preview)` releases, each with both files.
fn answer(releases: &[(&str, bool)]) -> String {
    let items: Vec<String> = releases
        .iter()
        .map(|(tag, preview)| {
            let version = tag.trim_start_matches('v');
            format!(
                r#"{{"tag_name": "{tag}", "draft": false, "prerelease": {preview},
                "html_url": "{SITE}/o/B/releases/tag/{tag}",
                "assets": [
                  {{"name": "BetterVolume-{version}-setup.exe", "size": 10,
                    "browser_download_url": "{SITE}/o/B/download/{version}/setup"}},
                  {{"name": "BetterVolume-{version}-portable.exe", "size": 10,
                    "browser_download_url": "{SITE}/o/B/download/{version}/portable"}}]}}"#
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Answers the last check with `text` at `now`.
fn reply(
    app: &mut App<FakeEngine>,
    calls: &Rc<RefCell<Calls>>,
    text: Result<String, ()>,
    now: u64,
) {
    let sender = calls.borrow_mut().fetches.pop().expect("a check was started");
    sender.send(text).unwrap();
    assert!(app.on_update_events(now));
}

fn about(app: &mut App<FakeEngine>) -> SettingsContent {
    app.settings_opening(false);
    app.settings_action(SettingsAction::SelectPage(Page::About.key()));
    app.settings_content()
}

fn card(content: &SettingsContent, key: UpdateKey) -> Option<Card> {
    content.card(key.key()).cloned()
}

fn line_text(app: &App<FakeEngine>) -> Option<String> {
    app.flyout_content().update.map(|u| u.text)
}

#[test]
fn a_copy_without_release_service_only_says_so() {
    let mut app = App::new(FakeEngine::new(), Settings::default(), Language::ENGLISH);
    assert_eq!(app.take_update_timer(NOW), None, "no timer at all");
    let content = about(&mut app);
    let status = card(&content, UpdateKey::Status).expect("status card");
    assert_eq!(status.title, "Updates are not available for this copy");
    assert_eq!(status.control, CardControl::None);
    assert!(card(&content, UpdateKey::Automatic).is_none());
    assert!(app.flyout_content().update.is_none());
}

#[test]
fn the_store_version_leaves_updates_to_the_store() {
    let mut app = App::new(FakeEngine::new(), Settings::default(), Language::ENGLISH);
    app.set_updates_by_store();
    assert_eq!(app.take_update_timer(NOW), None, "no check of its own");
    let content = about(&mut app);
    let status = card(&content, UpdateKey::Status).expect("status card");
    assert_eq!(status.title, "Updates come from the Microsoft Store");
    assert_eq!(status.control, CardControl::None);
    assert!(card(&content, UpdateKey::Automatic).is_none());
    assert!(card(&content, UpdateKey::Previews).is_none());
    assert!(app.flyout_content().update.is_none());
}

#[test]
fn checks_a_while_after_start_then_once_a_day() {
    let (mut app, calls) = app();
    assert_eq!(app.take_update_timer(NOW), Some(Some(FIRST_DELAY)));
    assert_eq!(app.take_update_timer(NOW), None, "set once");
    app.update_due(NOW);
    assert_eq!(calls.borrow().fetches.len(), 1);
    // While the check runs, no other starts.
    app.update_due(NOW);
    assert_eq!(calls.borrow().fetches.len(), 1);
    reply(&mut app, &calls, Ok(answer(&[("v0.0.1", false)])), NOW);
    assert_eq!(app.settings().updates.last_check, Some(NOW));
    assert!(app.take_dirty());
    assert_eq!(app.take_update_timer(NOW), Some(Some(DAY)));
    // A timer firing early (sleep, clock) waits again rather than checking.
    app.update_due(NOW + 3_600);
    assert!(calls.borrow().fetches.is_empty());
    assert_eq!(app.take_update_timer(NOW + 3_600), Some(Some(DAY - 3_600)));
    let status = card(&about(&mut app), UpdateKey::Status).unwrap();
    assert_eq!(status.title, "BetterVolume is up to date");
    assert_eq!(status.description.as_deref(), Some(&*format!("Last checked: t{NOW}")));
    assert!(app.flyout_content().update.is_none());
}

#[test]
fn a_newer_version_shows_the_line_and_the_about_cards() {
    let (mut app, calls) = app();
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false), ("v8.0.0", false)])), NOW);
    let line = app.flyout_content().update.expect("the line");
    assert_eq!(line.text, "Version 9.0.0 available");
    assert_eq!(line.action.as_deref(), Some("Install"));
    assert_eq!(line.close.as_deref(), Some("Close"));
    let content = about(&mut app);
    let status = card(&content, UpdateKey::Status).unwrap();
    assert_eq!(status.title, "Version 9.0.0 available");
    assert!(matches!(status.control, CardControl::Button { accent: true, .. }));
    let notes = card(&content, UpdateKey::Notes).expect("what's new");
    assert_eq!(notes.title, "What's new in version 9.0.0");
    // The link opens the release page.
    app.settings_action(SettingsAction::Invoke(UpdateKey::Notes.key()));
    assert_eq!(
        app.take_update_effect(),
        Some(UpdateEffect::OpenPage(format!("{SITE}/o/B/releases/tag/v9.0.0")))
    );
}

#[test]
fn a_closed_line_comes_back_only_for_a_newer_version() {
    let (mut app, calls) = app();
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false)])), NOW);
    app.flyout_action(FlyoutAction::DismissUpdate);
    assert!(app.flyout_content().update.is_none());
    assert_eq!(app.settings().updates.dismissed.as_deref(), Some("9.0.0"));
    // The About page keeps the information and the button.
    assert_eq!(card(&about(&mut app), UpdateKey::Status).unwrap().title, "Version 9.0.0 available");
    app.update_due(NOW + DAY);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false)])), NOW + DAY);
    assert!(app.flyout_content().update.is_none(), "same version: still closed");
    app.update_due(NOW + 2 * DAY);
    reply(&mut app, &calls, Ok(answer(&[("v9.1.0", false)])), NOW + 2 * DAY);
    assert_eq!(line_text(&app).as_deref(), Some("Version 9.1.0 available"));
}

#[test]
fn a_failed_automatic_check_tries_again_in_an_hour() {
    let (mut app, calls) = app();
    app.take_update_timer(NOW);
    app.update_due(NOW);
    reply(&mut app, &calls, Err(()), NOW);
    assert_eq!(app.settings().updates.last_check, None);
    assert_eq!(app.take_update_timer(NOW), Some(Some(RETRY_DELAY)));
    let status = card(&about(&mut app), UpdateKey::Status).unwrap();
    assert_eq!(status.title, "Couldn't check for updates");
    // An answer that is not a list of releases fails the same way.
    app.update_due(NOW + RETRY_DELAY);
    reply(&mut app, &calls, Ok("{\"message\": \"rate limited\"}".into()), NOW + RETRY_DELAY);
    assert_eq!(app.take_update_timer(NOW + RETRY_DELAY), Some(Some(RETRY_DELAY)));
    // Success: back to the daily rhythm.
    app.update_due(NOW + 2 * RETRY_DELAY);
    reply(&mut app, &calls, Ok(answer(&[])), NOW + 2 * RETRY_DELAY);
    assert_eq!(app.take_update_timer(NOW + 2 * RETRY_DELAY), Some(Some(DAY)));
}

#[test]
fn check_now_works_with_the_automatic_check_off() {
    let (mut app, calls) = app();
    app.take_update_timer(NOW);
    let content = about(&mut app);
    let automatic = card(&content, UpdateKey::Automatic).expect("the switch is on the About page");
    assert_eq!(automatic.title, "Check for updates automatically");
    app.settings_action(SettingsAction::Toggle(UpdateKey::Automatic.key(), false));
    assert!(!app.settings().general.check_updates);
    assert_eq!(app.take_update_timer(NOW), Some(None), "the timer stops");
    app.update_due(NOW);
    assert!(calls.borrow().fetches.is_empty());
    // The button still checks.
    app.settings_action(SettingsAction::Invoke(UpdateKey::Status.key()));
    let checking = card(&app.settings_content(), UpdateKey::Status).unwrap();
    assert_eq!(checking.title, "Checking for updates…");
    assert!(!checking.enabled);
    reply(&mut app, &calls, Err(()), NOW);
    assert_eq!(app.take_update_timer(NOW), Some(None), "still off");
    app.settings_action(SettingsAction::Toggle(UpdateKey::Automatic.key(), true));
    assert_eq!(app.take_update_timer(NOW), Some(Some(FIRST_DELAY)));
}

#[test]
fn previews_are_offered_only_when_wanted() {
    let (mut app, calls) = app();
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0-beta.1", true)])), NOW);
    assert!(app.flyout_content().update.is_none(), "previews are off by default");
    let previews = card(&about(&mut app), UpdateKey::Previews).expect("the switch");
    assert_eq!(previews.title, "Receive preview versions");
    app.settings_action(SettingsAction::Toggle(UpdateKey::Previews.key(), true));
    assert!(app.settings().updates.previews);
    assert_eq!(line_text(&app).as_deref(), Some("Version 9.0.0-beta.1 available"));
}

#[test]
fn installing_downloads_then_hands_over_to_the_setup_file() {
    let (mut app, calls) = app();
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false)])), NOW);
    app.flyout_action(FlyoutAction::InstallUpdate);
    let (what, kind, sender) = calls.borrow_mut().downloads.pop().expect("a download");
    assert_eq!(what, format!("{SITE}/o/B/download/9.0.0/setup|9.0.0"));
    assert_eq!(kind, Kind::Setup);
    sender.send(Download::Progress(40)).unwrap();
    assert!(app.on_update_events(NOW));
    let line = app.flyout_content().update.unwrap();
    assert_eq!(line.text, "Downloading version 9.0.0… 40%");
    assert_eq!((line.action, line.close), (None, None));
    // A second click while it runs starts nothing.
    app.flyout_action(FlyoutAction::InstallUpdate);
    assert!(calls.borrow().downloads.is_empty());
    let file = PathBuf::from(r"C:\temp\BetterVolume-9.0.0-setup.exe");
    sender.send(Download::Ready(file.clone())).unwrap();
    assert!(app.on_update_events(NOW));
    assert_eq!(app.take_update_effect(), Some(UpdateEffect::Install { kind: Kind::Setup, file }));
    assert_eq!(line_text(&app).as_deref(), Some("Installing version 9.0.0…"));
    // The setup file could not be started: the user can try again.
    app.update_launch_failed();
    let line = app.flyout_content().update.unwrap();
    assert_eq!(line.text, "The update could not be installed");
    assert_eq!(line.action.as_deref(), Some("Try again"));
    app.flyout_action(FlyoutAction::InstallUpdate);
    assert_eq!(calls.borrow().downloads.len(), 1, "tried again");
}

#[test]
fn a_failed_download_can_be_tried_again() {
    let (mut app, calls) = app_with(Settings::default(), Some(Kind::Portable));
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false)])), NOW);
    app.settings_opening(false);
    app.settings_action(SettingsAction::Invoke(UpdateKey::Status.key()));
    let (what, kind, sender) = calls.borrow_mut().downloads.pop().expect("a download");
    assert!(what.contains("/portable|"), "{what}");
    assert_eq!(kind, Kind::Portable);
    drop(sender);
    assert!(app.on_update_events(NOW), "a vanished download counts as failed");
    let status = card(&about(&mut app), UpdateKey::Status).unwrap();
    assert_eq!(status.title, "The update could not be installed");
    assert_eq!(app.take_update_effect(), None);
    app.flyout_action(FlyoutAction::DismissUpdate);
    assert!(app.flyout_content().update.is_none());
}

#[test]
fn a_copy_that_cannot_replace_itself_opens_the_release_page() {
    let (mut app, calls) = app_with(Settings::default(), None);
    app.update_due(NOW);
    reply(&mut app, &calls, Ok(answer(&[("v9.0.0", false)])), NOW);
    app.flyout_action(FlyoutAction::InstallUpdate);
    assert!(calls.borrow().downloads.is_empty());
    assert_eq!(
        app.take_update_effect(),
        Some(UpdateEffect::OpenPage(format!("{SITE}/o/B/releases/tag/v9.0.0")))
    );
}

#[test]
fn the_first_start_of_a_newer_version_says_it_was_updated() {
    let mut first = App::new(FakeEngine::new(), Settings::default(), Language::ENGLISH);
    assert_eq!(first.updated_bubble(), None, "a first installation is not an update");
    assert_eq!(first.settings().updates.last_version.as_deref(), Some(crate::VERSION));
    assert_eq!(first.updated_bubble(), None);
    let mut settings = Settings::default();
    settings.updates.last_version = Some("0.0.1-alpha".into());
    let mut app = App::new(FakeEngine::new(), settings, Language::ENGLISH);
    match app.updated_bubble() {
        Some(BubbleContent::Notice { title, detail, .. }) => {
            assert_eq!(title, "BetterVolume updated");
            assert_eq!(detail, format!("Version {}", crate::VERSION));
        }
        other => panic!("{other:?}"),
    }
    assert!(app.take_dirty());
    assert_eq!(app.updated_bubble(), None, "once");
    // An older version starting (a reinstall of an older file) says nothing.
    let mut settings = Settings::default();
    settings.updates.last_version = Some("99.0.0".into());
    let mut older = App::new(FakeEngine::new(), settings, Language::ENGLISH);
    assert_eq!(older.updated_bubble(), None);
}

#[test]
fn a_timer_firing_during_a_failed_manual_check_is_not_lost() {
    let (mut app, calls) = app();
    app.take_update_timer(NOW);
    app.settings_opening(false);
    app.settings_action(SettingsAction::Invoke(UpdateKey::Status.key()));
    // The daily timer fires while the manual check runs: nothing new starts.
    app.update_due(NOW + 100);
    assert_eq!(calls.borrow().fetches.len(), 1);
    reply(&mut app, &calls, Err(()), NOW + 101);
    // The manual failure keeps the rhythm: the next automatic check is still planned.
    assert_eq!(app.take_update_timer(NOW + 101), Some(Some(FIRST_DELAY)));
}

/// Offers 9.0.0 and hands its downloaded setup file over (the effect is taken).
fn handed_over(app: &mut App<FakeEngine>, calls: &Rc<RefCell<Calls>>) {
    app.update_due(NOW);
    reply(app, calls, Ok(answer(&[("v9.0.0", false)])), NOW);
    app.flyout_action(FlyoutAction::InstallUpdate);
    let (_, _, sender) = calls.borrow_mut().downloads.pop().expect("a download");
    sender.send(Download::Ready(PathBuf::from("setup.exe"))).unwrap();
    app.on_update_events(NOW);
    assert!(app.take_update_effect().is_some());
}

#[test]
fn a_setup_that_never_ends_this_copy_counts_as_failed() {
    let (mut app, calls) = app();
    handed_over(&mut app, &calls);
    assert_eq!(app.take_update_timer(NOW), Some(Some(super::INSTALL_WAIT)));
    // The timer fires too early (clock): still installing, the timer is set again.
    app.update_due(NOW + 10);
    assert_eq!(line_text(&app).as_deref(), Some("Installing version 9.0.0…"));
    assert!(app.take_update_timer(NOW + 10).is_some());
    app.update_due(NOW + super::INSTALL_WAIT);
    let line = app.flyout_content().update.unwrap();
    assert_eq!(line.text, "The update could not be installed");
    assert_eq!(line.action.as_deref(), Some("Try again"));
    // Back to the daily rhythm.
    assert_eq!(
        app.take_update_timer(NOW + super::INSTALL_WAIT),
        Some(Some(DAY - super::INSTALL_WAIT))
    );
}

#[test]
fn an_old_failure_does_not_stick_to_a_newer_answer() {
    let (mut app, calls) = app();
    handed_over(&mut app, &calls);
    app.update_launch_failed();
    assert_eq!(line_text(&app).as_deref(), Some("The update could not be installed"));
    app.update_due(NOW + DAY);
    reply(&mut app, &calls, Ok(answer(&[("v9.1.0", false)])), NOW + DAY);
    assert_eq!(line_text(&app).as_deref(), Some("Version 9.1.0 available"));
    app.update_launch_failed();
    app.settings_action(SettingsAction::Toggle(UpdateKey::Previews.key(), true));
    assert_eq!(line_text(&app).as_deref(), Some("Version 9.1.0 available"));
}
