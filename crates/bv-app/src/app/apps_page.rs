//! The Applications page of the settings window (FR-096 to FR-099) and the list it edits.
//!
//! Every application heard playing is remembered in the settings (FR-099, never
//! published nor logged): from the sessions while someone looks at them (the flyout or
//! this page), and from the engine's activity watch otherwise. The page lists them in the
//! flyout's order, System sounds included; each row can be moved, hidden from the flyout
//! or forgotten (not System sounds). "Sort alphabetically" asks first, then puts the list
//! back in its automatic order.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use bv_audio::AudioEngine;
use bv_core::apps::{self, AppKey};
use bv_core::i18n::Msg;
use bv_ui_kit::controls::settings::{
    Block, Button, DeviceRow, Question, RowPicture, SettingKey, SettingsAction,
};

use super::App;

/// The "Sort alphabetically" button.
const SORT: SettingKey = SettingKey(0x31_0000);

/// The programs running now, by lower-case executable file name, sorted (the shell reads
/// them; tests give a fixed list).
pub type ProcessList = fn() -> Vec<String>;

impl<E: AudioEngine> App<E> {
    /// Sets how the Applications page learns which programs run (none by default).
    pub fn set_process_list(&mut self, list: ProcessList) {
        self.settings_window.processes = Some(list);
    }

    /// Remembers the applications heard (FR-096): those the engine identified while the
    /// sessions were not watched, then every application of the sessions watched now.
    /// Returns whether the list changed.
    pub(super) fn learn_apps(&mut self) -> bool {
        let mut changed = false;
        for heard in self.engine.take_heard_apps() {
            changed |= self.settings.apps.learn(&heard.app, &heard.name, heard.path.as_deref());
        }
        for row in apps::rows(self.engine.sessions()) {
            changed |= self.settings.apps.learn(row.app, row.name, row.path);
        }
        self.dirty |= changed;
        changed
    }

    /// The Applications page: help line, the sort button, then every remembered
    /// application in order.
    pub(super) fn app_blocks(&self) -> Vec<Block> {
        let text = |msg| self.language.text(msg).to_owned();
        let running = self.settings_window.processes.map(|list| list()).unwrap_or_default();
        let mut blocks = vec![
            Block::Text(text(Msg::SettingsAppsHelp)),
            Block::Buttons(vec![Button {
                key: SORT,
                label: text(Msg::SettingsAppsSort),
                accent: false,
                enabled: true,
            }]),
        ];
        for (key, entry) in
            app_keys(&self.settings.apps.entries).into_iter().zip(&self.settings.apps.entries)
        {
            let row = match entry.app.file_name() {
                Some(file) => {
                    let state = text(if running.binary_search(&file.to_owned()).is_ok() {
                        Msg::SettingsAppRunning
                    } else {
                        Msg::SettingsAppClosed
                    });
                    let source = (!entry.path.trim().is_empty()).then(|| entry.path.clone());
                    DeviceRow {
                        key,
                        list: 0,
                        picture: RowPicture::App { pixels: None, source },
                        name: entry.label().to_owned(),
                        detail: self
                            .language
                            .format(Msg::SettingsAppDetail, &[("file", file), ("state", &state)]),
                        hidden: entry.hidden,
                        renamable: false,
                        forgettable: true,
                    }
                }
                None => DeviceRow {
                    key,
                    list: 0,
                    picture: RowPicture::SystemSounds,
                    name: text(Msg::FlyoutSystemSounds),
                    detail: text(Msg::SettingsSystemSoundsDetail),
                    hidden: entry.hidden,
                    renamable: false,
                    forgettable: false,
                },
            };
            blocks.push(Block::Device(row));
        }
        blocks
    }

    /// The application a row key designates.
    fn app_of(&self, key: SettingKey) -> Option<AppKey> {
        let entries = &self.settings.apps.entries;
        app_keys(entries)
            .into_iter()
            .zip(entries)
            .find(|(k, _)| *k == key)
            .map(|(_, e)| e.app.clone())
    }

    /// Applies an action of the Applications page. Returns `false` if it is not one.
    pub(super) fn app_action(&mut self, action: &SettingsAction) -> bool {
        match action {
            SettingsAction::Invoke(key) if *key == SORT => {
                self.settings_window.sorting = true;
            }
            SettingsAction::MoveDevice(key, to) => {
                let Some(app) = self.app_of(*key) else { return false };
                self.dirty |= self.settings.apps.move_to(&app, *to);
            }
            SettingsAction::SetHidden(key, hidden) => {
                let Some(app) = self.app_of(*key) else { return false };
                self.dirty |= self.settings.apps.set_hidden(&app, *hidden);
            }
            SettingsAction::Forget(key) => {
                let Some(app) = self.app_of(*key) else { return false };
                self.dirty |= self.settings.apps.forget(&app);
                // Still playing (its session shows, the page watches them): new again at
                // once, at the place of a new application (FR-096).
                self.learn_apps();
            }
            _ => return false,
        }
        true
    }

    /// The confirmation of "Sort alphabetically", while it is asked.
    pub(super) fn apps_question(&self) -> Option<Question> {
        let text = |msg| self.language.text(msg).to_owned();
        let shown = self.settings_window.sorting && self.applications_page_shown();
        shown.then(|| Question {
            title: text(Msg::SettingsAppsSortQuestion),
            text: text(Msg::SettingsAppsSortText),
            primary: text(Msg::SettingsAppsSortConfirm),
            secondary: text(Msg::SettingsCancel),
        })
    }

    /// The answer to the sort confirmation.
    pub(super) fn apps_answer(&mut self, sort: bool) {
        self.settings_window.sorting = false;
        if sort {
            self.dirty |= self.settings.apps.sort_alphabetically();
        }
    }
}

/// Row keys of the remembered applications, in list order: stable while the application
/// is remembered (a hash of its key, so the window keeps its focus across updates), below
/// 2^24 as the design system requires, apart from the device rows and the cards.
fn app_keys(entries: &[bv_core::apps::KnownApp]) -> Vec<SettingKey> {
    const BASE: u64 = 0x80_0000;
    const MASK: u64 = 0x7F_FFFF;
    let mut used = HashSet::new();
    entries
        .iter()
        .map(|entry| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            entry.app.hash(&mut hasher);
            let mut slot = hasher.finish() & MASK;
            // Two applications hashing alike: the later one takes the next slot.
            loop {
                let key = BASE | slot;
                if used.insert(key) {
                    break SettingKey(key);
                }
                slot = (slot + 1) & MASK;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::settings_window::Page;
    use bv_audio::fake::FakeEngine;
    use bv_audio::{DeviceInfo, DeviceState, Flow, HeardApp, Stream};
    use bv_core::i18n::Language;
    use bv_core::settings::Settings;
    use bv_ui_kit::controls::settings::SettingsContent;

    fn running() -> Vec<String> {
        vec!["chat.exe".into(), "player.exe".into()]
    }

    fn heard(file: &str, name: &str) -> HeardApp {
        let path = format!(r"C:\Program Files\{name}\{file}");
        HeardApp { app: AppKey::executable(&path), name: name.into(), path: Some(path) }
    }

    fn stream(file: &str, name: &str) -> Stream {
        let path = format!(r"C:\Apps\{file}");
        Stream {
            app: AppKey::executable(&path),
            process: 7,
            name: name.into(),
            icon: Some(path.clone()),
            path: Some(path),
            volume: 0.5,
            muted: false,
            heard_here: true,
        }
    }

    /// An application with an output, two applications heard while the flyout was closed,
    /// a third one playing now; the settings window closed.
    fn closed_app() -> App<FakeEngine> {
        let speakers = DeviceInfo {
            id: "spk".into(),
            windows_name: "Speakers".into(),
            state: DeviceState::Active,
            ..Default::default()
        };
        let mut engine = FakeEngine::new().with_device(Flow::Output, speakers);
        engine.streams = vec![
            stream("player.exe", "Player"),
            stream("chat.exe", "Chat"),
            stream("browser.exe", "Browser"),
            Stream { app: AppKey::System, name: String::new(), ..stream("x", "") },
        ];
        let mut app = App::new(engine, Settings::default(), Language::ENGLISH);
        app.set_process_list(running);
        app.engine.heard_apps = vec![heard("Player.exe", "Player"), heard("chat.exe", "Chat")];
        app.on_audio_signals();
        app
    }

    /// The same, with the settings window open on the Applications page.
    fn app() -> App<FakeEngine> {
        let mut app = closed_app();
        app.settings_opening(false);
        app.settings_action(SettingsAction::SelectPage(Page::Applications.key()));
        app
    }

    fn rows(content: &SettingsContent) -> Vec<(String, String, bool)> {
        content.rows().map(|r| (r.name.clone(), r.detail.clone(), r.hidden)).collect()
    }

    fn key(app: &App<FakeEngine>, name: &str) -> SettingKey {
        app.settings_content().rows().find(|r| r.name == name).expect("row shown").key
    }

    /// The application rows of the flyout.
    fn flyout_names(app: &mut App<FakeEngine>) -> Vec<String> {
        app.flyout_opening();
        let content = app.flyout_content();
        app.flyout_closed();
        content.apps.unwrap().rows.iter().map(|r| r.label.clone()).collect()
    }

    #[test]
    fn applications_heard_while_the_flyout_was_closed_are_remembered() {
        let mut closed = closed_app();
        assert!(closed.take_dirty(), "the settings file gets them");
        let entry = closed.settings.apps.get(&AppKey::executable("player.exe")).unwrap();
        assert_eq!(entry.path, r"C:\Program Files\Player\Player.exe");
        // Not seen yet: the browser plays now but was not identified.
        assert!(closed.settings.apps.get(&AppKey::executable("browser.exe")).is_none());
        // The page shows the sessions, so the browser playing now is learnt too.
        let app = app();
        assert!(app.engine.watching_sessions, "the page watches the sessions");
        let content = app.settings_content();
        assert_eq!(content.title, "Applications");
        assert_eq!(
            rows(&content),
            [
                ("Browser".into(), "browser.exe · Closed".into(), false),
                ("Chat".into(), "chat.exe · In use".into(), false),
                ("Player".into(), "player.exe · In use".into(), false),
                (
                    "System sounds".into(),
                    "The sounds of Windows: notifications, alerts".into(),
                    false
                ),
            ]
        );
        // The last place the executable was seen in gives its icon.
        let player = content.rows().find(|r| r.name == "Player").unwrap();
        assert_eq!(
            player.picture,
            RowPicture::App { pixels: None, source: Some(r"C:\Apps\player.exe".into()) }
        );
        assert!(player.forgettable && !player.renamable);
        let system = content.rows().find(|r| r.name == "System sounds").unwrap();
        assert_eq!(system.picture, RowPicture::SystemSounds);
        assert!(!system.forgettable);
        assert_eq!(content.devices.forget_question, "Forget this application?");
        assert!(matches!(content.blocks[1], Block::Buttons(_)), "the sort button above the list");
    }

    #[test]
    fn an_application_the_opening_flyout_shows_is_remembered_at_once() {
        let mut app = closed_app();
        app.take_dirty();
        // Playing before the flyout opens, never identified by the engine (a program it
        // may not open, such as one started by a service): the flyout reads its session.
        assert!(app.settings.apps.get(&AppKey::executable("browser.exe")).is_none());
        app.flyout_opening();
        assert!(app.settings.apps.get(&AppKey::executable("browser.exe")).is_some());
        assert!(app.take_dirty(), "saved without waiting for another audio event");
        // Learnt during an audio event: reported as a change, so the open page refreshes.
        app.engine.heard_apps = vec![heard("audio.exe", "Audacity")];
        assert!(app.on_audio_signals().sessions);
        assert!(!app.on_audio_signals().any(), "nothing new: nothing to refresh");
    }

    #[test]
    fn the_page_watches_sessions_only_while_it_shows() {
        let mut app = app();
        assert!(app.engine.watching_sessions);
        // Sent to the General page (the volume icon incident): watch and question go.
        app.settings_action(SettingsAction::Invoke(SORT));
        app.settings_show_general();
        assert!(!app.engine.watching_sessions);
        assert!(app.settings_content().question.is_none());
        app.settings_action(SettingsAction::SelectPage(Page::Applications.key()));
        assert!(app.engine.watching_sessions);
        app.settings_action(SettingsAction::SelectPage(Page::General.key()));
        assert!(!app.engine.watching_sessions);
        app.settings_action(SettingsAction::SelectPage(Page::Applications.key()));
        assert!(app.engine.watching_sessions);
        app.settings_closed();
        assert!(!app.engine.watching_sessions);
    }

    #[test]
    fn moving_and_hiding_reach_the_flyout() {
        let mut app = app();
        app.on_audio_signals();
        app.settings_closed();
        assert_eq!(flyout_names(&mut app), ["Browser", "Chat", "Player", "System sounds"]);
        app.take_dirty();
        app.settings_action(SettingsAction::MoveDevice(key(&app, "System sounds"), 0));
        app.settings_action(SettingsAction::MoveDevice(key(&app, "Player"), 1));
        app.settings_action(SettingsAction::SetHidden(key(&app, "Chat"), true));
        assert!(app.take_dirty());
        assert!(app.settings.apps.custom_order);
        let shown: Vec<(String, bool)> =
            app.settings_content().rows().map(|r| (r.name.clone(), r.hidden)).collect();
        assert_eq!(
            shown,
            [
                ("System sounds".into(), false),
                ("Player".into(), false),
                ("Browser".into(), false),
                ("Chat".into(), true),
            ]
        );
        assert_eq!(flyout_names(&mut app), ["System sounds", "Player", "Browser"]);
        // A new application now goes at the end.
        app.engine.heard_apps = vec![heard("audio.exe", "Audacity")];
        app.on_audio_signals();
        let last = app.settings_content().rows().last().unwrap().name.clone();
        assert_eq!(last, "Audacity");
    }

    #[test]
    fn forgetting_removes_an_application_but_never_system_sounds() {
        let mut app = app();
        // Not playing (heard earlier, closed since): gone from the page.
        app.engine.streams.retain(|s| s.name != "Chat");
        app.on_audio_signals();
        app.settings_action(SettingsAction::Forget(key(&app, "Chat")));
        assert!(app.settings_content().rows().all(|r| r.name != "Chat"));
        // Playing now, hidden and moved first: back at once, new (visible, at the end
        // place of a new application), without waiting for another audio event.
        app.settings_action(SettingsAction::SetHidden(key(&app, "Player"), true));
        app.settings_action(SettingsAction::MoveDevice(key(&app, "Player"), 0));
        app.settings_action(SettingsAction::Forget(key(&app, "Player")));
        let rows: Vec<(String, bool)> =
            app.settings_content().rows().map(|r| (r.name.clone(), r.hidden)).collect();
        assert_eq!(
            rows,
            [("Browser".into(), false), ("Player".into(), false), ("System sounds".into(), false)]
        );
        app.settings_action(SettingsAction::Forget(key(&app, "System sounds")));
        assert!(app.settings_content().rows().any(|r| r.name == "System sounds"));
    }

    #[test]
    fn sorting_asks_first() {
        let mut app = app();
        app.settings_action(SettingsAction::MoveDevice(key(&app, "System sounds"), 0));
        app.take_dirty();
        app.settings_action(SettingsAction::Invoke(SORT));
        let question = app.settings_content().question.expect("confirmation shown");
        assert_eq!(question.title, "Sort alphabetically?");
        assert_eq!((question.primary.as_str(), question.secondary.as_str()), ("Sort", "Cancel"));
        // Cancelled: nothing changes.
        app.settings_action(SettingsAction::QuestionAnswer(false));
        assert!(app.settings_content().question.is_none());
        assert!(!app.take_dirty());
        assert_eq!(app.settings_content().rows().next().unwrap().name, "System sounds");
        // Confirmed: alphabetical again, automatic mode.
        app.settings_action(SettingsAction::Invoke(SORT));
        app.settings_action(SettingsAction::QuestionAnswer(true));
        assert!(app.take_dirty());
        assert!(!app.settings.apps.custom_order);
        assert_eq!(app.settings_content().rows().last().unwrap().name, "System sounds");
        assert!(!app.settings.general.volume_icon_question_done, "the other question untouched");
    }

    #[test]
    fn device_rows_are_not_mistaken_for_applications() {
        let mut app = app();
        app.settings_action(SettingsAction::SelectPage(Page::Devices.key()));
        let speakers = app.settings_content().rows().next().unwrap().key;
        app.settings_action(SettingsAction::SetHidden(speakers, true));
        assert!(app.settings.devices.outputs[0].hidden);
        assert!(app.settings.apps.entries.iter().all(|e| !e.hidden));
        assert_eq!(app.settings_content().devices.forget_question, "Forget this device?");
    }
}
