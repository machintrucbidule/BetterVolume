//! The local log file (FR-005): technical failures are written there, never shown raw.
//!
//! One plain-text file, `BetterVolume.log`, in a folder chosen by the executable. Lines
//! are `<UTC time> <level> <message>`. When the file passes [`MAX_BYTES`] it is renamed to
//! `BetterVolume.old.log` (replacing the previous one) and a new file starts, so the log
//! never grows without bound. Logging never fails loudly: if the folder cannot be
//! written, messages are dropped.
//!
//! Messages are written in English for developers; they must never contain personal data
//! beyond what is needed to diagnose (no device names, no user paths).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Size at which the log file is rotated.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Name of the current log file.
pub const FILE_NAME: &str = "BetterVolume.log";
/// Name of the previous log file.
pub const OLD_FILE_NAME: &str = "BetterVolume.old.log";

/// Importance of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Something failed; the application carried on.
    Error,
    /// Worth knowing when diagnosing.
    Info,
}

struct Sink {
    folder: PathBuf,
    file: Option<File>,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// Starts logging into `folder` (created if needed). Later calls change the folder.
pub fn init(folder: &Path) {
    let _ = fs::create_dir_all(folder);
    if let Ok(mut sink) = SINK.lock() {
        *sink = Some(Sink { folder: folder.to_owned(), file: None });
    }
}

/// Stops logging and closes the file (before its folder is deleted, at uninstall).
pub fn close() {
    if let Ok(mut sink) = SINK.lock() {
        *sink = None;
    }
}

/// Logs an error.
pub fn error(message: &str) {
    write(Level::Error, message);
}

/// Logs an informative message.
pub fn info(message: &str) {
    write(Level::Info, message);
}

/// Logs a message (dropped when logging was not started or the file cannot be written).
pub fn write(level: Level, message: &str) {
    // `try_lock`: a message written while another one is being written on this thread
    // (a panic inside this function, reported by the panic hook) is dropped instead of
    // waiting for ever on the lock.
    let Ok(mut guard) = SINK.try_lock() else { return };
    let Some(sink) = guard.as_mut() else { return };
    let path = sink.folder.join(FILE_NAME);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size >= MAX_BYTES {
        sink.file = None;
        // If the file can be neither renamed nor removed (locked by another program),
        // the message is dropped rather than letting the file grow without bound.
        if fs::rename(&path, sink.folder.join(OLD_FILE_NAME)).is_err()
            && fs::remove_file(&path).is_err()
        {
            return;
        }
    }
    if sink.file.is_none() {
        sink.file = OpenOptions::new().create(true).append(true).open(&path).ok();
    }
    let Some(file) = sink.file.as_mut() else { return };
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = writeln!(file, "{} {} {}", timestamp(seconds), label(level), one_line(message));
}

fn label(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Info => "INFO ",
    }
}

/// A message on one line: line breaks would make the file ambiguous.
fn one_line(message: &str) -> String {
    message.replace(['\r', '\n'], " ")
}

/// `YYYY-MM-DDTHH:MM:SSZ` for seconds since 1970 (UTC, proleptic Gregorian calendar).
pub fn timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_iso_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(timestamp(1_794_873_599), "2026-11-16T23:59:59Z");
    }

    #[test]
    fn messages_stay_on_one_line() {
        assert_eq!(one_line("a\r\nb\nc"), "a  b c");
    }

    #[test]
    fn writes_and_rotates() {
        let folder = std::env::temp_dir().join(format!("bv-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        init(&folder);
        error("first failure");
        let text = fs::read_to_string(folder.join(FILE_NAME)).unwrap();
        assert!(text.contains("ERROR first failure"), "{text}");

        // Past the limit, the next message starts a new file and keeps the old one.
        fs::write(folder.join(FILE_NAME), vec![b'x'; MAX_BYTES as usize]).unwrap();
        if let Ok(mut sink) = SINK.lock() {
            if let Some(sink) = sink.as_mut() {
                sink.file = None;
            }
        }
        info("after rotation");
        let current = fs::read_to_string(folder.join(FILE_NAME)).unwrap();
        assert!(current.contains("INFO  after rotation"), "{current}");
        // Other tests of this crate log at the same time (one log for the process): a line
        // of theirs may land in the full file just before it is set aside.
        let old = fs::read(folder.join(OLD_FILE_NAME)).unwrap();
        assert!(
            old.len() as u64 >= MAX_BYTES && old[..MAX_BYTES as usize].iter().all(|b| *b == b'x')
        );
        let _ = fs::remove_dir_all(&folder);
    }
}
