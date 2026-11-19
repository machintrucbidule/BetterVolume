//! Applications: volume rows (FR-061, FR-062, FR-064, FR-065) and the remembered list of
//! the Applications page (FR-096 to FR-099).
//!
//! Windows reports audio *sessions* (one or more per application: a browser may open
//! several). The flyout shows one row per application, controlling all its sessions
//! together. This module groups the sessions, remembers every application heard, and
//! orders and filters the rows by that list.

use crate::volume;

/// What identifies an application across its sessions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppKey {
    /// The Windows "System sounds" session.
    System,
    /// An application, by the file name of its executable in lower case (Windows file
    /// names ignore case), such as `discord.exe`: a program that moves to a new folder at
    /// each update stays the same application, and two programs with the same file name
    /// are one.
    Executable(String),
    /// A process whose executable path cannot be read (protected process).
    Process(u32),
}

impl AppKey {
    /// The key of an application from its executable path (case is ignored; the folder
    /// does not count, only the file name).
    pub fn executable(path: &str) -> Self {
        AppKey::Executable(file_name(path).to_lowercase())
    }

    /// The executable file name of an application, if it has one.
    pub fn file_name(&self) -> Option<&str> {
        match self {
            AppKey::Executable(name) => Some(name),
            _ => None,
        }
    }
}

/// The last part of a Windows path (the whole text if it has no separator).
fn file_name(path: &str) -> &str {
    let path = path.trim().trim_end_matches(['\\', '/']);
    path.rsplit(['\\', '/']).next().filter(|name| !name.is_empty()).unwrap_or(path)
}

/// One audio session, as reported by the engine.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    /// Application the session belongs to.
    pub app: AppKey,
    /// Windows process identifier of the session (0 if unknown).
    pub process: u32,
    /// Friendly name of the application (empty for System sounds: the interface uses its
    /// own translated label, FR-122 does not apply to it).
    pub name: String,
    /// Where the application icon can be loaded from (an executable or icon resource path,
    /// possibly followed by `,<index>`), if known.
    pub icon: Option<String>,
    /// Full path of the application's executable, if it could be read.
    pub path: Option<String>,
    /// Windows volume scalar, 0.0 to 1.0.
    pub volume: f32,
    /// Muted.
    pub muted: bool,
    /// The application plays on this output now, or this output is where it was last
    /// heard. Sessions Windows keeps open without sound on a device where the
    /// application may never play again are not.
    pub heard_here: bool,
}

/// One row of the application list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRow<'a> {
    /// The application (pass it back to the engine to change its volume).
    pub app: &'a AppKey,
    /// Friendly name (empty for System sounds).
    pub name: &'a str,
    /// Icon source of the first session that has one.
    pub icon: Option<&'a str>,
    /// Executable path of the first session that has one.
    pub path: Option<&'a str>,
    /// Volume shown: the loudest session of the application.
    pub volume: u8,
    /// Muted only if every session of the application is muted.
    pub muted: bool,
    /// Number of sessions grouped in this row.
    pub sessions: usize,
}

impl AppRow<'_> {
    /// Whether this is the System sounds row.
    pub fn is_system(&self) -> bool {
        *self.app == AppKey::System
    }
}

/// Groups `streams` into one row per application, in the default order (FR-064): by
/// friendly name, alphabetically, ignoring case and accents, System sounds last. Rows
/// with the same name keep a stable order (by key), so nothing jumps between two
/// refreshes. [`AppList::arrange`] then applies the user's order and exclusions.
///
/// The interface always shows a System sounds row (FR-065); it is not invented here when
/// Windows has no such session, because there would be nothing to control. Sessions of
/// applications not heard on this output are left out.
pub fn rows(streams: &[Stream]) -> Vec<AppRow<'_>> {
    let mut rows: Vec<AppRow<'_>> = Vec::new();
    // An application is listed on the output it plays on or was last heard on;
    // System sounds always (FR-065).
    for stream in streams.iter().filter(|s| s.heard_here || s.app == AppKey::System) {
        let percent = volume::to_percent(stream.volume);
        if let Some(row) = rows.iter_mut().find(|r| *r.app == stream.app) {
            row.volume = row.volume.max(percent);
            row.muted &= stream.muted;
            row.sessions += 1;
            if row.name.is_empty() {
                row.name = &stream.name;
            }
            if row.icon.is_none() {
                row.icon = stream.icon.as_deref();
            }
            if row.path.is_none() {
                row.path = stream.path.as_deref();
            }
        } else {
            rows.push(AppRow {
                app: &stream.app,
                name: &stream.name,
                icon: stream.icon.as_deref(),
                path: stream.path.as_deref(),
                volume: percent,
                muted: stream.muted,
                sessions: 1,
            });
        }
    }
    rows.sort_by(|a, b| {
        a.is_system()
            .cmp(&b.is_system())
            .then_with(|| sort_key(a.name).cmp(&sort_key(b.name)))
            .then_with(|| a.app.cmp(b.app))
    });
    rows
}

/// An application heard playing while nobody looked at the sessions (the flyout closed):
/// what the engine could read of it, to remember it (FR-096).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeardApp {
    /// The application.
    pub app: AppKey,
    /// Its friendly name.
    pub name: String,
    /// Full path of its executable.
    pub path: Option<String>,
}

/// An application remembered on the Applications page (FR-096, FR-099).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownApp {
    /// [`AppKey::System`] or [`AppKey::Executable`] (a process without a readable
    /// executable is never remembered: nothing would recognise it again).
    pub app: AppKey,
    /// Friendly name when last heard (empty for System sounds).
    pub name: String,
    /// Full path of the executable when last heard (for its icon); empty if unknown.
    pub path: String,
    /// Excluded from the flyout (FR-097).
    pub hidden: bool,
}

impl KnownApp {
    /// A newly remembered application: visible.
    pub fn new(app: AppKey, name: &str, path: Option<&str>) -> Self {
        Self {
            app,
            name: name.to_owned(),
            path: path.unwrap_or_default().to_owned(),
            hidden: false,
        }
    }

    /// System sounds, visible.
    pub fn system() -> Self {
        Self::new(AppKey::System, "", None)
    }

    /// The name to sort and show: the friendly name, else the executable file name.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() {
            self.app.file_name().unwrap_or_default()
        } else {
            &self.name
        }
    }

    fn is_system(&self) -> bool {
        self.app == AppKey::System
    }
}

/// Every application remembered, in the order of the Applications page and the flyout
/// (FR-064, FR-098).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppList {
    /// The applications, System sounds included, in order.
    pub entries: Vec<KnownApp>,
    /// The user moved a row: the order is theirs. Until then the list stays
    /// alphabetical, System sounds last, and a new application takes its alphabetical
    /// place; afterwards a new application goes at the end, just before System sounds
    /// while System sounds is last.
    pub custom_order: bool,
}

impl Default for AppList {
    fn default() -> Self {
        Self { entries: vec![KnownApp::system()], custom_order: false }
    }
}

impl AppList {
    /// The entry of `app`.
    pub fn get(&self, app: &AppKey) -> Option<&KnownApp> {
        self.entries.iter().find(|e| e.app == *app)
    }

    fn position(&self, app: &AppKey) -> Option<usize> {
        self.entries.iter().position(|e| e.app == *app)
    }

    /// Where an application called `name`, not remembered yet, goes.
    fn place_for(&self, name: &str) -> usize {
        let system_last = self.entries.last().is_some_and(KnownApp::is_system);
        let end = self.entries.len() - usize::from(system_last);
        if self.custom_order {
            return end;
        }
        let key = sort_key(name);
        self.entries[..end].iter().position(|e| sort_key(e.label()) > key).unwrap_or(end)
    }

    /// Remembers `app`, heard playing with this name and executable path (FR-096).
    /// A known one gets its name and path refreshed. Returns `true` if the list changed.
    pub fn learn(&mut self, app: &AppKey, name: &str, path: Option<&str>) -> bool {
        if matches!(app, AppKey::Process(_)) {
            return false;
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.app == *app) {
            let mut changed = false;
            if !name.trim().is_empty() && entry.name != name {
                entry.name = name.to_owned();
                changed = true;
            }
            if let Some(path) = path.filter(|p| !p.trim().is_empty() && *p != entry.path) {
                entry.path = path.to_owned();
                changed = true;
            }
            if changed && !self.custom_order {
                self.sort();
            }
            return changed;
        }
        let entry = KnownApp::new(app.clone(), name, path);
        let at = if entry.is_system() { self.entries.len() } else { self.place_for(entry.label()) };
        self.entries.insert(at, entry);
        true
    }

    /// The rows of the flyout in this list's order, without the excluded applications
    /// (FR-064, FR-097). An application not remembered (a process whose executable
    /// cannot be read) goes where a new one would.
    pub fn arrange<'a>(&self, rows: Vec<AppRow<'a>>) -> Vec<AppRow<'a>> {
        let mut ranked: Vec<(usize, AppRow<'a>)> = rows
            .into_iter()
            .filter(|row| !self.get(row.app).is_some_and(|e| e.hidden))
            .map(|row| {
                // Known rows at even ranks, others just before the place they would take.
                let rank = match self.position(row.app) {
                    Some(at) => 2 * at + 1,
                    None => 2 * self.place_for(row.name),
                };
                (rank, row)
            })
            .collect();
        // Stable: rows of one rank keep their default (alphabetical) order.
        ranked.sort_by_key(|(rank, _)| *rank);
        ranked.into_iter().map(|(_, row)| row).collect()
    }

    /// Moves `app` to position `to` (FR-098): the order becomes the user's. Returns
    /// `true` if anything changed.
    pub fn move_to(&mut self, app: &AppKey, to: usize) -> bool {
        let Some(from) = self.position(app) else { return false };
        let to = to.min(self.entries.len() - 1);
        if from == to {
            return false;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        self.custom_order = true;
        true
    }

    /// Shows or hides `app` in the flyout (FR-097). Returns `true` if it changed.
    pub fn set_hidden(&mut self, app: &AppKey, hidden: bool) -> bool {
        self.entries
            .iter_mut()
            .find(|e| e.app == *app)
            .is_some_and(|e| std::mem::replace(&mut e.hidden, hidden) != hidden)
    }

    /// Forgets `app` (FR-096): System sounds cannot be. Returns `true` if it was removed.
    pub fn forget(&mut self, app: &AppKey) -> bool {
        let before = self.entries.len();
        if *app != AppKey::System {
            self.entries.retain(|e| e.app != *app);
        }
        self.entries.len() != before
    }

    /// Puts the list back in alphabetical order, System sounds last, and back in the
    /// automatic mode (a new application takes its alphabetical place). Returns `true`
    /// if anything changed.
    pub fn sort_alphabetically(&mut self) -> bool {
        let before = self.clone();
        self.custom_order = false;
        self.sort();
        *self != before
    }

    fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            a.is_system()
                .cmp(&b.is_system())
                .then_with(|| sort_key(a.label()).cmp(&sort_key(b.label())))
                .then_with(|| a.app.cmp(&b.app))
        });
    }

    /// Repairs a damaged or hand-edited list: no process entry, executables in lower
    /// case, every application once (its first entry wins), System sounds present (at the
    /// end if it was missing), alphabetical while the order is automatic.
    pub fn normalise(&mut self) {
        let mut kept: Vec<KnownApp> = Vec::with_capacity(self.entries.len() + 1);
        for mut entry in std::mem::take(&mut self.entries) {
            entry.app = match entry.app {
                AppKey::Executable(name) if !file_name(&name).is_empty() => {
                    AppKey::executable(&name)
                }
                AppKey::System => AppKey::System,
                _ => continue,
            };
            if entry.is_system() {
                entry.name.clear();
                entry.path.clear();
            }
            if !kept.iter().any(|k| k.app == entry.app) {
                kept.push(entry);
            }
        }
        if !kept.iter().any(KnownApp::is_system) {
            kept.push(KnownApp::system());
        }
        self.entries = kept;
        if !self.custom_order {
            self.sort();
        }
    }
}

/// Alphabetical sort key: lower case, common Latin accents and ligatures folded, so that
/// "Éditeur" sorts with the E and not after Z.
fn sort_key(name: &str) -> String {
    let mut key = String::with_capacity(name.len());
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => key.push('a'),
            'ç' | 'ć' | 'č' => key.push('c'),
            'ď' | 'đ' => key.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => key.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => key.push('i'),
            'ł' | 'ľ' => key.push('l'),
            'ñ' | 'ń' | 'ň' => key.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => key.push('o'),
            'ř' => key.push('r'),
            'ś' | 'š' | 'ş' => key.push('s'),
            'ť' | 'ţ' => key.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => key.push('u'),
            'ý' | 'ÿ' => key.push('y'),
            'ź' | 'ż' | 'ž' => key.push('z'),
            'æ' => key.push_str("ae"),
            'œ' => key.push_str("oe"),
            'ß' => key.push_str("ss"),
            other => key.push(other),
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(app: AppKey, name: &str, volume: f32, muted: bool) -> Stream {
        Stream {
            app,
            process: 0,
            name: name.into(),
            icon: None,
            path: None,
            volume,
            muted,
            heard_here: true,
        }
    }

    fn exe(path: &str) -> AppKey {
        AppKey::executable(path)
    }

    fn names<'a>(rows: &[AppRow<'a>]) -> Vec<&'a str> {
        rows.iter().map(|r| r.name).collect()
    }

    #[test]
    fn alphabetical_by_default_with_system_sounds_last() {
        let streams = [
            stream(exe("c:\\z.exe"), "Zoom", 0.5, false),
            stream(exe("c:\\b.exe"), "browser", 0.5, false),
            stream(AppKey::System, "", 0.5, false),
            stream(exe("c:\\a.exe"), "Audacity", 0.5, false),
        ];
        let list = rows(&streams);
        assert!(list[3].is_system());
        assert_eq!(names(&list), ["Audacity", "browser", "Zoom", ""]);
    }

    #[test]
    fn accents_and_case_do_not_break_alphabetical_order() {
        let streams = [
            stream(exe("1"), "Zèbre", 0.5, false),
            stream(exe("2"), "Éditeur", 0.5, false),
            stream(exe("3"), "écoute", 0.5, false),
            stream(exe("4"), "Œuvre", 0.5, false),
            stream(exe("5"), "Dolby", 0.5, false),
        ];
        assert_eq!(names(&rows(&streams)), ["Dolby", "écoute", "Éditeur", "Œuvre", "Zèbre"]);
    }

    #[test]
    fn an_application_is_its_executable_file_name() {
        assert_eq!(
            exe(r"C:\Users\Someone\AppData\Local\Discord\app-1.0.9001\Discord.exe"),
            AppKey::Executable("discord.exe".into())
        );
        assert_eq!(
            exe(r"C:\Users\Someone\AppData\Local\Discord\app-1.0.9002\discord.EXE"),
            exe(r"D:\Other\Discord.exe")
        );
        assert_eq!(exe(r"\Device\HarddiskVolume3\Apps\Player.exe"), exe("player.exe"));
        assert_eq!(exe("c:/apps/player.exe"), exe("player.exe"));
        assert_eq!(exe(r"c:\apps\player.exe").file_name(), Some("player.exe"));
        assert_eq!(AppKey::System.file_name(), None);
    }

    #[test]
    fn streams_of_one_application_make_one_row() {
        let streams = [
            stream(exe("C:\\Apps\\Browser.exe"), "Browser", 0.30, false),
            stream(exe("c:\\apps\\browser.EXE"), "Browser", 0.80, false),
            stream(exe("c:\\apps\\player.exe"), "Player", 0.5, false),
        ];
        let list = rows(&streams);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Browser");
        assert_eq!(list[0].sessions, 2);
        assert_eq!(list[0].volume, 80, "the loudest session is shown");
        assert_eq!(list[1].sessions, 1);
    }

    #[test]
    fn a_group_is_muted_only_if_every_stream_is() {
        let app = exe("c:\\app.exe");
        let partly =
            [stream(app.clone(), "App", 0.5, true), stream(app.clone(), "App", 0.5, false)];
        assert!(!rows(&partly)[0].muted);
        let fully = [stream(app.clone(), "App", 0.5, true), stream(app, "App", 0.5, true)];
        assert!(rows(&fully)[0].muted);
    }

    #[test]
    fn missing_name_icon_or_path_is_taken_from_another_stream() {
        let app = exe("c:\\app.exe");
        let first = stream(app.clone(), "", 0.5, false);
        let mut second = stream(app, "App", 0.5, false);
        second.icon = Some("c:\\app.exe,0".into());
        second.path = Some("c:\\app.exe".into());
        let streams = [first, second];
        let list = rows(&streams);
        assert_eq!(list[0].name, "App");
        assert_eq!(list[0].icon, Some("c:\\app.exe,0"));
        assert_eq!(list[0].path, Some("c:\\app.exe"));
    }

    #[test]
    fn same_names_keep_a_stable_order() {
        let streams =
            [stream(AppKey::Process(9), "Same", 0.5, false), stream(exe("b"), "Same", 0.5, false)];
        let reversed = [streams[1].clone(), streams[0].clone()];
        let apps = |s: &[Stream]| rows(s).iter().map(|r| r.app.clone()).collect::<Vec<_>>();
        assert_eq!(apps(&streams), apps(&reversed));
    }

    #[test]
    fn volume_is_shown_in_percent() {
        let streams = [stream(exe("a"), "A", 0.426, false), stream(exe("b"), "B", 1.2, false)];
        let list = rows(&streams);
        assert_eq!(list[0].volume, 43);
        assert_eq!(list[1].volume, 100);
    }

    #[test]
    fn applications_not_heard_here_are_left_out() {
        // A session kept open without sound where the application was not last
        // heard (a launcher that opened this output once) gives no row; System sounds stay.
        let mut idle = stream(exe("c:\\launcher.exe"), "Launcher", 0.5, false);
        idle.heard_here = false;
        let mut system = stream(AppKey::System, "", 0.5, false);
        system.heard_here = false;
        let browser = stream(exe("c:\\browser.exe"), "Browser", 0.5, false);
        // One application: a row as soon as one of its sessions was heard here.
        let mut browser_old = browser.clone();
        browser_old.heard_here = false;
        let streams = [idle, system, browser_old, browser];
        let list = rows(&streams);
        assert_eq!(names(&list), ["Browser", ""]);
        assert_eq!(list[0].sessions, 1, "only the sessions heard here are counted");
    }

    #[test]
    fn no_stream_gives_no_row() {
        assert!(rows(&[]).is_empty());
    }

    // ----- The remembered list -----

    fn labels(list: &AppList) -> Vec<&str> {
        list.entries.iter().map(|e| if e.is_system() { "<system>" } else { e.label() }).collect()
    }

    fn learnt(names: &[&str]) -> AppList {
        let mut list = AppList::default();
        for name in names {
            let path = format!("C:\\Apps\\{name}.exe");
            assert!(list.learn(&exe(&path), name, Some(&path)));
        }
        list
    }

    #[test]
    fn the_list_starts_with_system_sounds_only() {
        let list = AppList::default();
        assert_eq!(labels(&list), ["<system>"]);
        assert!(!list.custom_order);
    }

    #[test]
    fn until_the_user_orders_new_applications_take_their_alphabetical_place() {
        let list = learnt(&["Zoom", "Browser", "Éditeur", "audacity"]);
        assert_eq!(labels(&list), ["audacity", "Browser", "Éditeur", "Zoom", "<system>"]);
        assert_eq!(list.get(&exe("zoom.exe")).unwrap().path, "C:\\Apps\\Zoom.exe");
    }

    #[test]
    fn once_ordered_new_applications_go_at_the_end_before_system_sounds_if_last() {
        let mut list = learnt(&["Browser", "Zoom"]);
        assert!(list.move_to(&exe("zoom.exe"), 0));
        assert!(list.custom_order);
        list.learn(&exe("a.exe"), "Audacity", None);
        assert_eq!(labels(&list), ["Zoom", "Browser", "Audacity", "<system>"]);
        // System sounds moved up: a new application then goes at the very end.
        assert!(list.move_to(&AppKey::System, 0));
        list.learn(&exe("c.exe"), "Calculator", None);
        assert_eq!(labels(&list), ["<system>", "Zoom", "Browser", "Audacity", "Calculator"]);
    }

    #[test]
    fn learning_a_known_application_refreshes_it_without_moving_it_once_ordered() {
        let mut list = learnt(&["Browser", "Zoom"]);
        list.move_to(&exe("zoom.exe"), 0);
        assert!(!list.learn(&exe(r"C:\Apps\Zoom.exe"), "Zoom", Some(r"C:\Apps\Zoom.exe")));
        assert!(list.learn(&exe(r"D:\New\Zoom.exe"), "Zoom Workplace", Some(r"D:\New\Zoom.exe")));
        assert_eq!(labels(&list), ["Zoom Workplace", "Browser", "<system>"]);
        assert_eq!(list.get(&exe("zoom.exe")).unwrap().path, r"D:\New\Zoom.exe");
        // An empty name or path does not erase the remembered one.
        assert!(!list.learn(&exe("zoom.exe"), "", None));
        assert_eq!(list.get(&exe("zoom.exe")).unwrap().name, "Zoom Workplace");
    }

    #[test]
    fn a_renamed_application_moves_to_its_new_alphabetical_place_in_automatic_mode() {
        let mut list = learnt(&["Browser", "Player"]);
        list.learn(&exe("player.exe"), "Audio player", None);
        assert_eq!(labels(&list), ["Audio player", "Browser", "<system>"]);
    }

    #[test]
    fn protected_processes_are_not_remembered() {
        let mut list = AppList::default();
        assert!(!list.learn(&AppKey::Process(42), "Hidden", None));
        assert_eq!(list.entries.len(), 1);
    }

    #[test]
    fn the_flyout_follows_the_list_and_leaves_out_hidden_applications() {
        let mut list = learnt(&["Browser", "Player", "Zoom"]);
        list.move_to(&AppKey::System, 0);
        list.move_to(&exe("zoom.exe"), 1);
        list.set_hidden(&exe("player.exe"), true);
        let streams = [
            stream(exe(r"c:\browser.exe"), "Browser", 0.5, false),
            stream(exe(r"c:\player.exe"), "Player", 0.5, false),
            stream(exe(r"c:\zoom.exe"), "Zoom", 0.5, false),
            stream(AppKey::System, "", 0.5, false),
        ];
        let arranged = list.arrange(rows(&streams));
        assert_eq!(names(&arranged), ["", "Zoom", "Browser"]);
        // System sounds can be hidden too.
        list.set_hidden(&AppKey::System, true);
        assert_eq!(names(&list.arrange(rows(&streams))), ["Zoom", "Browser"]);
    }

    #[test]
    fn applications_not_remembered_go_where_a_new_one_would() {
        // Automatic order: at their alphabetical place.
        let list = learnt(&["Browser", "Zoom"]);
        let streams = [
            stream(exe("zoom.exe"), "Zoom", 0.5, false),
            stream(AppKey::Process(7), "Mixer", 0.5, false),
            stream(exe("browser.exe"), "Browser", 0.5, false),
            stream(AppKey::System, "", 0.5, false),
        ];
        assert_eq!(names(&list.arrange(rows(&streams))), ["Browser", "Mixer", "Zoom", ""]);
        // The user's order: at the end, before System sounds while it is last.
        let mut ordered = list.clone();
        ordered.move_to(&exe("zoom.exe"), 0);
        assert_eq!(names(&ordered.arrange(rows(&streams))), ["Zoom", "Browser", "Mixer", ""]);
    }

    #[test]
    fn hiding_and_showing_report_changes() {
        let mut list = learnt(&["Browser"]);
        assert!(list.set_hidden(&exe("browser.exe"), true));
        assert!(!list.set_hidden(&exe("browser.exe"), true));
        assert!(list.get(&exe("browser.exe")).unwrap().hidden);
        assert!(list.set_hidden(&exe("browser.exe"), false));
        assert!(!list.set_hidden(&exe("unknown.exe"), true));
    }

    #[test]
    fn any_application_but_system_sounds_can_be_forgotten() {
        let mut list = learnt(&["Browser", "Zoom"]);
        assert!(list.forget(&exe("browser.exe")));
        assert!(!list.forget(&exe("browser.exe")));
        assert!(!list.forget(&AppKey::System));
        assert_eq!(labels(&list), ["Zoom", "<system>"]);
        // Heard again: a new, visible application.
        list.set_hidden(&exe("zoom.exe"), true);
        list.forget(&exe("zoom.exe"));
        list.learn(&exe("zoom.exe"), "Zoom", None);
        assert!(!list.get(&exe("zoom.exe")).unwrap().hidden);
    }

    #[test]
    fn moving_keeps_every_entry_and_ignores_unknown_ones() {
        let mut list = learnt(&["A", "B", "C"]);
        assert!(!list.move_to(&exe("x.exe"), 0));
        assert!(!list.move_to(&exe("a.exe"), 0), "already there");
        assert!(!list.custom_order);
        assert!(list.move_to(&exe("a.exe"), 99));
        assert_eq!(labels(&list), ["B", "C", "<system>", "A"]);
    }

    #[test]
    fn sorting_puts_the_list_back_in_automatic_alphabetical_order() {
        let mut list = learnt(&["Browser", "Zoom", "Audacity"]);
        list.move_to(&AppKey::System, 0);
        list.move_to(&exe("zoom.exe"), 1);
        list.set_hidden(&exe("browser.exe"), true);
        assert!(list.sort_alphabetically());
        assert_eq!(labels(&list), ["Audacity", "Browser", "Zoom", "<system>"]);
        assert!(!list.custom_order);
        assert!(list.get(&exe("browser.exe")).unwrap().hidden, "visibility is kept");
        assert!(!list.sort_alphabetically(), "already sorted");
        list.learn(&exe("c.exe"), "Calculator", None);
        assert_eq!(labels(&list), ["Audacity", "Browser", "Calculator", "Zoom", "<system>"]);
    }

    #[test]
    fn an_application_without_a_name_sorts_by_its_file_name() {
        let list = {
            let mut list = AppList::default();
            list.learn(&exe("zeta.exe"), "", None);
            list.learn(&exe("alpha.exe"), "Mixer", None);
            list
        };
        assert_eq!(labels(&list), ["Mixer", "zeta.exe", "<system>"]);
    }

    #[test]
    fn a_damaged_list_is_repaired() {
        let mut list = AppList {
            entries: vec![
                KnownApp::new(AppKey::Process(3), "Process", None),
                KnownApp::new(AppKey::Executable(r"C:\Apps\Zoom.EXE".into()), "Zoom", None),
                KnownApp { hidden: true, ..KnownApp::new(exe("zoom.exe"), "Zoom again", None) },
                KnownApp::new(AppKey::Executable(String::new()), "Nothing", None),
                KnownApp::new(exe("a.exe"), "Audacity", None),
            ],
            custom_order: true,
        };
        list.normalise();
        assert_eq!(labels(&list), ["Zoom", "Audacity", "<system>"]);
        assert_eq!(list.entries[0].app, exe("zoom.exe"));
        assert!(!list.entries[0].hidden, "the first entry wins");
        // In automatic mode the repaired list is alphabetical.
        list.custom_order = false;
        list.normalise();
        assert_eq!(labels(&list), ["Audacity", "Zoom", "<system>"]);
    }
}
