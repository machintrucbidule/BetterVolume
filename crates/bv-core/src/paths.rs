//! File paths given by other programs.
//!
//! Each application that plays sound names its own icon and may name its display text
//! through a file (`@file,-id`). Opening a file on another computer (`\\server\share\…`)
//! makes Windows connect to it with the user's sign-in, which tells that computer who the
//! user is. Such paths are never opened: only paths on this PC are.

/// Whether `path` names a file on another computer: a network path (`\\server\…`, in any
/// mix of `\` and `/`), its long form (`\\?\UNC\…`), or any other `\\?\` or `\\.\` form
/// that is not a local drive (`\\?\C:\…`), a local volume (`\\?\Volume{…}\…`) or a local
/// disk (`\\?\GLOBALROOT\Device\HarddiskVolume…`). Paths of this PC (`C:\…`, relative
/// names, `%Variable%\…`) are not.
pub fn is_remote(path: &str) -> bool {
    let path = path.trim();
    let slash = |c: char| c == '\\' || c == '/';
    let mut chars = path.chars();
    if !(chars.next().is_some_and(slash) && chars.next().is_some_and(slash)) {
        return false;
    }
    let rest = &path[2..];
    // `\\?\` and `\\.\` (Win32 namespaces): only the local forms are allowed.
    let namespace = rest.len() >= 2
        && matches!(rest.as_bytes()[0], b'?' | b'.')
        && slash(char::from(rest.as_bytes()[1]));
    if !namespace {
        return true;
    }
    let inner = rest[2..].replace('/', "\\");
    let lower = inner.to_ascii_lowercase();
    let drive = {
        let b = inner.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
    };
    let local = drive
        || lower.starts_with("volume{")
        || lower.starts_with("globalroot\\device\\harddiskvolume");
    !local || lower.contains("\\..\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_of_this_pc_are_not_remote() {
        for path in [
            r"C:\Program Files\Player\player.exe",
            r"c:/apps/x.ico",
            r"%SystemRoot%\System32\AudioSrv.Dll",
            "player.exe",
            "",
            r"\\?\C:\Apps\player.exe",
            r"\\?\Volume{0b1c2d3e-0000-0000-0000-100000000000}\Apps\p.exe",
            r"\\?\GLOBALROOT\Device\HarddiskVolume3\Apps\player.exe",
            r"\\.\GLOBALROOT\Device\HarddiskVolume3\Apps\player.exe",
            r"  D:\x.exe",
        ] {
            assert!(!is_remote(path), "{path}");
        }
    }

    #[test]
    fn network_paths_are_remote() {
        for path in [
            r"\\server\share\icon.ico",
            "//server/share/icon.ico",
            r"\/server\share\icon.ico",
            r"  \\server\share\x.dll",
            r"\\?\UNC\server\share\icon.ico",
            r"\\.\UNC\server\share\icon.ico",
            r"\\?\GLOBALROOT\Device\Mup\server\share\x.exe",
            r"\\?\GLOBALROOT\Device\LanmanRedirector\server\share\x.exe",
            r"\\?\GLOBALROOT\Device\HarddiskVolume3\..\Mup\server\x.exe",
            r"\\?\pipe\x",
            r"\\?",
            r"\\",
            r"\\192.168.1.2\share\a.ico",
        ] {
            assert!(is_remote(path), "{path}");
        }
    }
}
