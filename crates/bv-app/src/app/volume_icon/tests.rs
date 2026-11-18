//! Hiding the Windows volume icon: the switch, the first-run question, the outcomes and
//! the guard-rail incident.

use bv_audio::fake::FakeEngine;
use bv_core::i18n::Language;
use bv_core::settings::Settings;
use bv_shell::VolumeIconOutcome;
use bv_ui_kit::controls::settings::{Block, SettingsAction, SettingsContent};

use super::super::settings_window::{Page, SettingsEffect};
use super::super::App;

fn app(settings: Settings) -> App<FakeEngine> {
    App::new(FakeEngine::new(), settings, Language::ENGLISH)
}

/// Settings of an installation that passed its first launch before the question existed.
fn older_installation() -> Settings {
    let mut settings = Settings::default();
    settings.general.first_run_done = true;
    settings
}

fn switch_of(content: &SettingsContent) -> &bv_ui_kit::controls::settings::Card {
    content.cards().find(|c| c.title == "Hide the Windows volume icon").expect("the switch")
}

fn is_on(content: &SettingsContent) -> bool {
    matches!(switch_of(content).control, bv_ui_kit::controls::settings::CardControl::Toggle { on, .. } if on)
}

fn toggle(app: &mut App<FakeEngine>, on: bool) -> SettingsEffect {
    let key = switch_of(&app.settings_content()).key;
    app.settings_action(SettingsAction::Toggle(key, on))
}

#[test]
fn the_switch_is_off_by_default_and_says_the_risks() {
    let mut app = app(Settings::default());
    app.settings_opening(false);
    let content = app.settings_content();
    assert!(!is_on(&content), "off by default");
    let help = switch_of(&content).description.clone().unwrap_or_default();
    assert!(help.contains("inserts itself into the taskbar"), "{help}");
    assert!(help.contains("antivirus"), "{help}");
}

#[test]
fn turning_the_switch_on_and_off_asks_the_shell_and_is_saved() {
    let mut app = app(older_installation());
    app.settings_opening(false);
    assert_eq!(toggle(&mut app, true), SettingsEffect::HideVolumeIcon(true));
    assert!(app.take_dirty());
    assert!(app.volume_icon_hidden());
    assert!(app.settings().general.volume_icon_question_done, "choosing answers the question");
    assert!(is_on(&app.settings_content()));
    assert_eq!(toggle(&mut app, false), SettingsEffect::HideVolumeIcon(false));
    assert!(!app.volume_icon_hidden());
}

#[test]
fn a_new_installation_opens_the_window_with_the_question() {
    let mut app = app(Settings::default());
    assert!(app.start_window_pending());
    app.start_window_opening();
    app.settings_opening(false);
    app.start_window_shown(true);
    let question = app.settings_content().question.expect("asked at the first start");
    assert_eq!(question.title, "Hide the Windows volume icon?");
    assert_eq!(question.primary, "Hide the Windows icon");
    assert_eq!(question.secondary, "Keep both");
    // "Hide the Windows icon".
    let effect = app.settings_action(SettingsAction::QuestionAnswer(true));
    assert_eq!(effect, SettingsEffect::HideVolumeIcon(true));
    assert!(app.volume_icon_hidden());
    assert_eq!(app.settings_content().question, None);
    assert!(!app.start_window_pending(), "asked once");
}

#[test]
fn keep_both_answers_the_question_for_good() {
    let mut app = app(older_installation());
    assert!(app.start_window_pending(), "an older installation is asked at its next start");
    app.start_window_opening();
    app.settings_opening(false);
    app.start_window_shown(true);
    assert!(app.settings_content().question.is_some());
    // "Keep both", or the question closed any other way (the view answers false).
    assert_eq!(app.settings_action(SettingsAction::QuestionAnswer(false)), SettingsEffect::Refresh);
    assert!(app.take_dirty());
    assert!(!app.volume_icon_hidden());
    assert_eq!(app.settings_content().question, None);
    assert!(!app.start_window_pending());
}

#[test]
fn an_unanswered_question_is_asked_again_at_the_next_start() {
    let mut app = app(older_installation());
    app.start_window_opening();
    app.settings_opening(false);
    // The window could not be shown (graphics failure): nothing recorded.
    app.start_window_shown(false);
    assert_eq!(app.settings_content().question, None);
    assert!(app.start_window_pending());
    // Shown, then the application quits without an answer.
    app.start_window_opening();
    app.start_window_shown(true);
    app.volume_icon_window_closed();
    assert!(app.start_window_pending());
}

#[test]
fn an_option_already_on_needs_no_question() {
    let mut settings = older_installation();
    settings.general.hide_windows_volume_icon = true;
    let mut app = app(settings);
    assert!(!app.start_window_pending());
    app.start_window_opening();
    assert_eq!(app.settings_content().question, None);
}

#[test]
fn an_icon_that_cannot_be_hidden_is_said_under_the_switch() {
    let mut app = app(older_installation());
    app.settings_opening(false);
    toggle(&mut app, true);
    assert!(app.volume_icon_outcome(VolumeIconOutcome::NotFound));
    let content = app.settings_content();
    assert!(is_on(&content), "the switch stays on");
    let help = switch_of(&content).description.clone().unwrap_or_default();
    assert!(help.starts_with("The Windows volume icon cannot be hidden"), "{help}");
    assert!(help.contains("antivirus"), "the risks stay: {help}");
    // Found at a later start or taskbar restart.
    assert!(app.volume_icon_outcome(VolumeIconOutcome::Hidden));
    assert!(!app.volume_icon_outcome(VolumeIconOutcome::Hidden), "nothing new");
    let help = switch_of(&app.settings_content()).description.clone().unwrap_or_default();
    assert!(!help.contains("cannot be hidden"), "{help}");
    // A failed load says the same.
    assert!(app.volume_icon_outcome(VolumeIconOutcome::Failed));
    // Once off, nothing to explain.
    toggle(&mut app, false);
    let help = switch_of(&app.settings_content()).description.clone().unwrap_or_default();
    assert!(!help.contains("cannot be hidden"), "{help}");
    assert!(!app.volume_icon_outcome(VolumeIconOutcome::Failed));
}

#[test]
fn an_incident_turns_the_option_off_and_explains_it_next_to_the_switch() {
    let mut settings = older_installation();
    settings.general.hide_windows_volume_icon = true;
    settings.general.volume_icon_question_done = true;
    let mut app = app(settings);
    app.settings_opening(false);
    app.settings_action(SettingsAction::SelectPage(Page::About.key()));
    app.volume_icon_incident();
    assert!(app.take_dirty());
    assert!(!app.volume_icon_hidden(), "turned off");
    let content = app.settings_content();
    assert_eq!(content.page, Page::General.key(), "the General page shows");
    assert!(!is_on(&content));
    let switch = content
        .blocks
        .iter()
        .position(|b| matches!(b, Block::Card(c) if c.title == "Hide the Windows volume icon"));
    let notice = content.blocks.iter().position(|b| matches!(b, Block::Notice(_)));
    assert_eq!(notice, switch.map(|s| s + 1), "the notice right under the switch");
    let Some(Block::Notice(shown)) = notice.map(|n| &content.blocks[n]) else { panic!("notice") };
    assert_eq!(content.reveal, Some(shown.key), "the window scrolls to it");
    // It goes when the window closes, or when the option is turned on again.
    app.volume_icon_window_closed();
    assert!(app.settings_content().notices().next().is_none());
    app.volume_icon_incident();
    toggle(&mut app, true);
    assert!(app.settings_content().notices().next().is_none());
}

#[test]
fn the_texts_exist_in_french() {
    let mut app = App::new(
        FakeEngine::new(),
        Settings::default(),
        Language::from_tag("fr").expect("French ships"),
    );
    app.start_window_opening();
    app.settings_opening(false);
    let content = app.settings_content();
    let question = content.question.expect("asked");
    assert_eq!(question.title, "Masquer l\u{2019}ic\u{f4}ne volume de Windows\u{a0}?");
    assert_eq!(question.secondary, "Garder les deux");
}
