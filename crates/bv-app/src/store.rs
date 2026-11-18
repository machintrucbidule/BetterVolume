//! Reading and writing the settings file (FR-125, FR-126).
//!
//! The file lives in the application's folder of the user profile, or beside the portable
//! file (FR-130). Writes are atomic: the new text goes to a temporary
//! file, is flushed to disk, then replaces the old file in one rename, so a crash or a
//! power cut leaves either the old or the new settings, never half a file. A file that
//! cannot be used is renamed aside (`settings.bad.toml`) and the defaults are used; the
//! application always starts.

use std::path::{Path, PathBuf};

use bv_core::settings::{file, Settings};

/// Name of the settings file.
const FILE: &str = "settings.toml";
/// Where an unusable file is kept (the previous one is replaced).
const ASIDE: &str = "settings.bad.toml";
/// Temporary file of an atomic write.
const TEMPORARY: &str = "settings.toml.tmp";

/// The settings file of one folder.
#[derive(Debug)]
pub struct Store {
    folder: PathBuf,
    /// The last save failed (logged once per streak of failures).
    failing: bool,
    /// The file exists but could not be read (locked, no access): it is never
    /// overwritten during this run, so a temporary problem cannot lose the settings.
    blocked: bool,
}

/// What [`Store::load`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Loaded {
    /// No file yet: defaults (first launch).
    Missing,
    /// The file was read.
    Read,
    /// The file could not be used; it was kept aside and the defaults are used.
    SetAside,
}

impl Store {
    /// The settings file in `folder`.
    pub fn new(folder: &Path) -> Store {
        Store { folder: folder.to_owned(), failing: false, blocked: false }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    /// Reads the settings. Never fails: problems are logged and give the defaults.
    pub fn load(&mut self) -> (Settings, Loaded) {
        let text = match std::fs::read(self.path(FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Settings::default(), Loaded::Missing);
            }
            Err(error) => {
                // Unreadable (locked, no access): neither set aside nor overwritten, it may
                // be fine at the next start.
                bv_shell::log::error(&format!(
                    "settings unreadable ({:?}); defaults used, nothing saved this run",
                    error.kind()
                ));
                self.blocked = true;
                return (Settings::default(), Loaded::SetAside);
            }
        };
        let parsed = String::from_utf8(text)
            .map_err(|_| "not UTF-8 text".to_owned())
            .and_then(|text| file::read(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(loaded) => {
                for warning in &loaded.warnings {
                    bv_shell::log::info(&format!("settings: {warning}"));
                }
                (loaded.settings, Loaded::Read)
            }
            Err(problem) => {
                let kept = std::fs::rename(self.path(FILE), self.path(ASIDE)).is_ok();
                // Not kept aside: never overwritten either (it may still be recovered).
                self.blocked |= !kept;
                bv_shell::log::error(&format!(
                    "settings file unusable ({problem}); {}defaults used",
                    if kept { "kept aside as settings.bad.toml, " } else { "" }
                ));
                (Settings::default(), Loaded::SetAside)
            }
        }
    }

    /// Writes the settings atomically. Returns whether they were written.
    pub fn save(&mut self, settings: &Settings) -> bool {
        if self.blocked {
            return false;
        }
        let result = self.write(&file::write(settings));
        match &result {
            Ok(()) => self.failing = false,
            Err(error) => {
                if !std::mem::replace(&mut self.failing, true) {
                    bv_shell::log::error(&format!("settings not saved ({:?})", error.kind()));
                }
            }
        }
        result.is_ok()
    }

    fn write(&self, text: &str) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(&self.folder)?;
        let temporary = self.path(TEMPORARY);
        {
            let mut out = std::fs::File::create(&temporary)?;
            out.write_all(text.as_bytes())?;
            out.sync_all()?;
        }
        // Replaces the old file in one step (MoveFileEx with "replace existing").
        std::fs::rename(&temporary, self.path(FILE)).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system temporary folder, removed at the end.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Folder {
            let path = std::env::temp_dir().join(format!("bv-store-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            Folder(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn settings_survive_a_restart() {
        let folder = Folder::new("restart");
        let mut store = Store::new(&folder.0);
        assert_eq!(store.load(), (Settings::default(), Loaded::Missing));
        let mut settings = Settings::default();
        settings.general.wheel_step = 9;
        settings.general.first_run_done = true;
        assert!(store.save(&settings));
        assert!(!folder.0.join(TEMPORARY).exists(), "no temporary file left");
        assert_eq!(Store::new(&folder.0).load(), (settings.clone(), Loaded::Read));
        // A second save replaces the file.
        settings.general.wheel_step = 3;
        assert!(store.save(&settings));
        assert_eq!(store.load().0.general.wheel_step, 3);
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_and_defaults_are_used() {
        let folder = Folder::new("corrupt");
        std::fs::create_dir_all(&folder.0).unwrap();
        std::fs::write(folder.0.join(FILE), b"version = 1\n[general\n").unwrap();
        let mut store = Store::new(&folder.0);
        assert_eq!(store.load(), (Settings::default(), Loaded::SetAside));
        assert!(!folder.0.join(FILE).exists());
        assert_eq!(std::fs::read(folder.0.join(ASIDE)).unwrap(), b"version = 1\n[general\n");
        // Not text at all, or written by a newer version: the same.
        std::fs::write(folder.0.join(FILE), [0xFF, 0xFE, 0x00]).unwrap();
        assert_eq!(store.load().1, Loaded::SetAside);
        std::fs::write(folder.0.join(FILE), b"version = 99\n").unwrap();
        assert_eq!(store.load().1, Loaded::SetAside);
        assert_eq!(
            std::fs::read(folder.0.join(ASIDE)).unwrap(),
            b"version = 99\n",
            "the last bad file is kept"
        );
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let folder = Folder::new("unreadable");
        // A folder where the file should be: it exists but cannot be read as a file.
        std::fs::create_dir_all(folder.0.join(FILE)).unwrap();
        let mut store = Store::new(&folder.0);
        assert_eq!(store.load(), (Settings::default(), Loaded::SetAside));
        assert!(!store.save(&Settings::default()), "nothing is written this run");
        assert!(folder.0.join(FILE).is_dir(), "left as it was");
    }

    #[test]
    fn a_failed_save_reports_it_and_keeps_the_old_file() {
        let folder = Folder::new("blocked");
        let mut store = Store::new(&folder.0);
        assert!(store.save(&Settings::default()));
        // A folder in place of the temporary file makes the write fail.
        std::fs::create_dir_all(folder.0.join(TEMPORARY)).unwrap();
        let mut changed = Settings::default();
        changed.general.wheel_step = 5;
        assert!(!store.save(&changed));
        assert_eq!(store.load(), (Settings::default(), Loaded::Read));
    }
}
