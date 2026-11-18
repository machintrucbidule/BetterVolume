use std::path::{Path, PathBuf};

use super::*;

fn english() -> Language {
    Language::ENGLISH
}

fn buttons(content: &SettingsContent) -> Vec<(SettingKey, String, bool)> {
    content.buttons().map(|b| (b.key, b.label.clone(), b.enabled)).collect()
}

fn notice(content: &SettingsContent) -> Option<(Severity, String)> {
    content.notices().next().map(|n| (n.severity, n.title.clone()))
}

fn option_on(content: &SettingsContent) -> Option<bool> {
    match content.card(OPTION).map(|c| &c.control) {
        Some(CardControl::Toggle { on, .. }) => Some(*on),
        _ => None,
    }
}

#[test]
fn a_first_installation_asks_and_installs() {
    let mut setup = Setup::new(Mode::Install { installed: None }, english(), "2.0.0", true);
    let content = setup.content();
    assert_eq!(content.title, "Install BetterVolume");
    assert!(content.nav.is_empty(), "no navigation pane");
    assert_eq!(option_on(&content), Some(true), "start with Windows on by default");
    assert!(matches!(&content.blocks[0], Block::Text(t) if t.contains("2.0.0")));
    assert_eq!(
        buttons(&content),
        [(PRIMARY, "Install".into(), true), (CANCEL, "Cancel".into(), true)]
    );

    assert_eq!(setup.action(&SettingsAction::Toggle(OPTION, false)), Effect::Refresh);
    assert!(!setup.option());
    assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Start);
    assert_eq!(setup.phase(), Phase::Working);
    let content = setup.content();
    assert!(buttons(&content).iter().all(|(_, _, enabled)| !enabled), "nothing while working");
    assert!(!content.card(OPTION).unwrap().enabled);
    assert_eq!(notice(&content).map(|n| n.0), Some(Severity::Info));
    // Neither cancel nor a toggle can interrupt the work.
    assert_eq!(setup.action(&SettingsAction::Invoke(CANCEL)), Effect::Refresh);
    assert_eq!(setup.action(&SettingsAction::Toggle(OPTION, true)), Effect::Refresh);
    assert!(!setup.option());

    // Installed and started: the window closes.
    assert_eq!(setup.finished(Ok(true)), Effect::Close);
}

#[test]
fn an_installed_program_that_did_not_start_is_reported() {
    let mut setup = Setup::new(Mode::Install { installed: None }, english(), "2.0.0", true);
    setup.action(&SettingsAction::Invoke(PRIMARY));
    assert_eq!(setup.finished(Ok(false)), Effect::Refresh);
    let content = setup.content();
    assert_eq!(notice(&content).map(|n| n.0), Some(Severity::Caution));
    assert!(content.card(OPTION).is_none());
    assert_eq!(buttons(&content), [(PRIMARY, "Close".into(), true)]);
    assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Close);
}

#[test]
fn a_newer_version_is_an_update() {
    let setup =
        Setup::new(Mode::Install { installed: Some("2.0.0".into()) }, english(), "2.1.0", true);
    let content = setup.content();
    assert_eq!(content.title, "Update BetterVolume");
    assert!(
        matches!(&content.blocks[0], Block::Text(t) if t.contains("2.0.0") && t.contains("2.1.0"))
    );
    assert_eq!(buttons(&content)[0].1, "Update");
    // The same version (or an older one) installs again.
    for installed in ["2.1.0", "3.0.0", "not a version"] {
        let setup = Setup::new(
            Mode::Install { installed: Some(installed.into()) },
            english(),
            "2.1.0",
            true,
        );
        assert_eq!(setup.content().title, "Install BetterVolume", "{installed}");
        assert_eq!(buttons(&setup.content())[0].1, "Install");
    }
    // The same version says it is installed again.
    let same =
        Setup::new(Mode::Install { installed: Some("2.1.0".into()) }, english(), "2.1.0", true);
    assert!(matches!(&same.content().blocks[0], Block::Text(t) if t.contains("already installed")));
}

#[test]
fn a_failure_can_be_tried_again() {
    let mut setup = Setup::new(Mode::Install { installed: None }, english(), "2.0.0", true);
    setup.action(&SettingsAction::Invoke(PRIMARY));
    for (failure, text) in [
        (Failure::StillRunning, Msg::SetupStillRunning),
        (Failure::Files, Msg::SetupFilesFailed),
        (Failure::Entries, Msg::SetupEntriesFailed),
    ] {
        assert_eq!(setup.finished(Err(failure)), Effect::Refresh);
        let content = setup.content();
        let shown = content.notices().next().unwrap();
        assert_eq!(shown.severity, Severity::Critical);
        assert_eq!(shown.message, english().text(text));
        assert_eq!(buttons(&content)[0], (PRIMARY, "Try again".into(), true));
        // The choice can still change, and the work starts again.
        assert_eq!(setup.action(&SettingsAction::Toggle(OPTION, false)), Effect::Refresh);
        assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Start);
    }
    // Cancel closes once nothing runs.
    setup.finished(Err(Failure::Files));
    assert_eq!(setup.action(&SettingsAction::Invoke(CANCEL)), Effect::Close);
}

#[test]
fn a_damaged_setup_file_cannot_be_tried_again() {
    let mut setup = Setup::new(Mode::Install { installed: None }, english(), "2.0.0", true);
    setup.action(&SettingsAction::Invoke(PRIMARY));
    setup.finished(Err(Failure::Damaged));
    let content = setup.content();
    assert_eq!(buttons(&content), [(CANCEL, "Cancel".into(), true)]);
    assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Refresh);
    assert_eq!(setup.phase(), Phase::Failed(Failure::Damaged));
}

#[test]
fn the_uninstaller_keeps_the_settings_unless_told_otherwise() {
    let mut setup = Setup::new(Mode::Uninstall, english(), "2.0.0", true);
    let content = setup.content();
    assert_eq!(content.title, "Uninstall BetterVolume");
    assert_eq!(content.card(OPTION).unwrap().title, "Keep my settings");
    assert_eq!(option_on(&content), Some(true));
    assert_eq!(buttons(&content)[0].1, "Uninstall");
    assert!(!setup.uninstalled());

    setup.action(&SettingsAction::Toggle(OPTION, false));
    assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Start);
    assert!(!setup.option());

    // Home Assistant was told: success.
    assert_eq!(setup.clone().finished(Ok(true)), Effect::Refresh);
    let mut told = setup.clone();
    told.finished(Ok(true));
    assert_eq!(told.phase(), Phase::Uninstalled { ha_kept: false });
    assert_eq!(notice(&told.content()).map(|n| n.0), Some(Severity::Success));
    assert!(told.uninstalled());

    // Home Assistant could not be reached: the user is told where to finish.
    setup.finished(Ok(false));
    let content = setup.content();
    assert_eq!(notice(&content).map(|n| n.0), Some(Severity::Caution));
    assert!(content.card(OPTION).is_none());
    assert_eq!(buttons(&content), [(PRIMARY, "Close".into(), true)]);
    assert_eq!(setup.action(&SettingsAction::Invoke(PRIMARY)), Effect::Close);
    assert!(setup.uninstalled());
}

#[test]
fn every_text_is_translated() {
    for language in Language::all() {
        for mode in [Mode::Install { installed: Some("1.0.0".into()) }, Mode::Uninstall] {
            let mut setup = Setup::new(mode, language, "2.0.0", true);
            let mut contents = vec![setup.content()];
            setup.action(&SettingsAction::Invoke(PRIMARY));
            contents.push(setup.content());
            setup.finished(Err(Failure::StillRunning));
            contents.push(setup.content());
            setup.action(&SettingsAction::Invoke(PRIMARY));
            setup.finished(Ok(false));
            contents.push(setup.content());
            for content in contents {
                assert!(!content.name.is_empty() && !content.title.is_empty());
                assert!(!content.title.contains('{'), "placeholder left: {}", content.title);
                for block in &content.blocks {
                    if let Block::Text(text) = block {
                        assert!(!text.contains('{'), "placeholder left: {text}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_first_installation_can_choose_its_folder() {
    let mut setup = Setup::new(Mode::Install { installed: None }, english(), "2.0.0", true);
    setup.set_folder(PathBuf::from(r"C:\Users\u\AppData\Local\Programs\BetterVolume"));
    let content = setup.content();
    let card = content.card(FOLDER).unwrap();
    assert_eq!(card.title, "Installation folder");
    assert_eq!(
        card.description.as_deref(),
        Some(r"C:\Users\u\AppData\Local\Programs\BetterVolume")
    );
    assert!(matches!(&card.control, CardControl::Button { label, .. } if label == "Change…"));
    assert!(card.enabled);
    assert_eq!(setup.action(&SettingsAction::Invoke(FOLDER)), Effect::ChooseFolder);

    // BetterVolume goes into its own sub-folder of the folder picked.
    setup.folder_chosen(Path::new(r"D:\Apps"), &|_| true);
    assert_eq!(setup.folder(), Some(Path::new(r"D:\Apps\BetterVolume")));
    // Unless the folder picked is already named so.
    setup.folder_chosen(Path::new(r"E:\betterVolume"), &|_| true);
    assert_eq!(setup.folder(), Some(Path::new(r"E:\betterVolume")));

    // A protected folder is refused: the previous choice stays, a notice says why.
    setup.folder_chosen(Path::new(r"C:\Program Files"), &|_| false);
    assert_eq!(setup.folder(), Some(Path::new(r"E:\betterVolume")));
    let content = setup.content();
    let refused = content.notices().find(|n| n.key == FOLDER_NOTICE).unwrap();
    assert_eq!(refused.severity, Severity::Caution);
    // A good choice clears the notice.
    setup.folder_chosen(Path::new(r"D:\Apps"), &|_| true);
    assert!(setup.content().notices().next().is_none());

    // While working, the folder cannot change.
    setup.action(&SettingsAction::Invoke(PRIMARY));
    assert!(!setup.content().card(FOLDER).unwrap().enabled);
    assert_eq!(setup.action(&SettingsAction::Invoke(FOLDER)), Effect::Refresh);
}

#[test]
fn an_existing_installation_keeps_its_folder() {
    let mut setup =
        Setup::new(Mode::Install { installed: Some("2.0.0".into()) }, english(), "2.1.0", true);
    setup.set_folder(PathBuf::from(r"D:\Apps\BetterVolume"));
    let content = setup.content();
    let card = content.card(FOLDER).unwrap();
    assert_eq!(card.control, CardControl::None, "no Change button for an update");
    assert_eq!(setup.action(&SettingsAction::Invoke(FOLDER)), Effect::Refresh);
    // The uninstaller shows no folder card.
    let uninstall = Setup::new(Mode::Uninstall, english(), "2.1.0", true);
    assert!(uninstall.content().card(FOLDER).is_none());
}
