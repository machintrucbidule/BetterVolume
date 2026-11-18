//! Application volume rows (FR-061, FR-062, FR-064, FR-065).
//!
//! Windows reports audio *sessions* (one or more per application: a browser may open
//! several). The flyout shows one row per application, controlling all its sessions
//! together. This module groups the sessions and orders the rows.

use crate::volume;

/// What identifies an application across its sessions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AppKey {
    /// The Windows "System sounds" session.
    System,
    /// An application, by full executable path in lower case (Windows paths ignore case).
    Executable(String),
    /// A process whose executable path cannot be read (protected process).
    Process(u32),
}

impl AppKey {
    /// The key of an application from its executable path (case is ignored).
    pub fn executable(path: &str) -> Self {
        AppKey::Executable(path.to_lowercase())
    }
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

/// Groups `streams` into one row per application, ordered System sounds first, then by
/// friendly name, alphabetically, ignoring case and accents (FR-064). Rows with the same
/// name keep a stable order (by key), so nothing jumps between two refreshes.
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
        } else {
            rows.push(AppRow {
                app: &stream.app,
                name: &stream.name,
                icon: stream.icon.as_deref(),
                volume: percent,
                muted: stream.muted,
                sessions: 1,
            });
        }
    }
    rows.sort_by(|a, b| {
        b.is_system()
            .cmp(&a.is_system())
            .then_with(|| sort_key(a.name).cmp(&sort_key(b.name)))
            .then_with(|| a.app.cmp(b.app))
    });
    rows
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
        Stream { app, process: 0, name: name.into(), icon: None, volume, muted, heard_here: true }
    }

    fn exe(path: &str) -> AppKey {
        AppKey::executable(path)
    }

    fn names<'a>(rows: &[AppRow<'a>]) -> Vec<&'a str> {
        rows.iter().map(|r| r.name).collect()
    }

    #[test]
    fn system_sounds_come_first_then_alphabetical() {
        let streams = [
            stream(exe("c:\\z.exe"), "Zoom", 0.5, false),
            stream(exe("c:\\b.exe"), "browser", 0.5, false),
            stream(AppKey::System, "", 0.5, false),
            stream(exe("c:\\a.exe"), "Audacity", 0.5, false),
        ];
        let list = rows(&streams);
        assert!(list[0].is_system());
        assert_eq!(names(&list), ["", "Audacity", "browser", "Zoom"]);
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
    fn missing_name_or_icon_is_taken_from_another_stream() {
        let app = exe("c:\\app.exe");
        let mut first = stream(app.clone(), "", 0.5, false);
        first.icon = None;
        let mut second = stream(app, "App", 0.5, false);
        second.icon = Some("c:\\app.exe,0".into());
        let streams = [first, second];
        let list = rows(&streams);
        assert_eq!(list[0].name, "App");
        assert_eq!(list[0].icon, Some("c:\\app.exe,0"));
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
        assert_eq!(names(&list), ["", "Browser"]);
        assert_eq!(list[1].sessions, 1, "only the sessions heard here are counted");
    }

    #[test]
    fn no_stream_gives_no_row() {
        assert!(rows(&[]).is_empty());
    }
}
