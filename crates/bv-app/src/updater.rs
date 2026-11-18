//! The update work that touches the network and the files (FR-132), on
//! threads of its own; the decisions are `app::update`'s and `bv_core::update`'s.
//!
//! Where releases are looked for: the repository the release build was made from
//! (`BETTERVOLUME_REPOSITORY`, set by the release automation from the GitHub repository it
//! runs in), at GitHub. A copy built without it (on a development PC) is not updated. The
//! tools and the tests point a copy at a local imitation of the release service with the
//! environment variable `BETTERVOLUME_UPDATE_SERVER` (loopback addresses only; `off`
//! turns updates off, for the footprint measurement).

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use bv_core::package::{Kind, Version};
use bv_core::update::{self, Asset};
use bv_shell::Waker;

use crate::app::{Download, UpdatePlatform};

/// The repository releases come from, written into release builds.
const REPOSITORY: Option<&str> = option_env!("BETTERVOLUME_REPOSITORY");

/// Repository name used with a local imitation when the build names none.
const TEST_REPOSITORY: &str = "local/BetterVolume";

/// Environment variable pointing a copy at a local imitation of the release service.
const SERVER_VARIABLE: &str = "BETTERVOLUME_UPDATE_SERVER";

/// Name of the folder (in the temporary folder) the setup file of an update goes into.
const DOWNLOAD_FOLDER: &str = "BetterVolume-update";

/// Where releases are looked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    /// The release service.
    pub api: String,
    /// Where release pages and files may be.
    pub site: String,
    /// `owner/name`.
    pub repository: String,
}

/// Whether `url` is an address on this PC's loopback interface.
fn loopback(url: &str) -> bool {
    ["http://127.0.0.1:", "http://localhost:"].iter().any(|p| url.starts_with(p))
}

/// The project page of the repository this copy was built from (FR-095), if any.
pub fn project_page() -> Option<String> {
    project_page_from(REPOSITORY)
}

fn project_page_from(built: Option<&str>) -> Option<String> {
    built.filter(|r| update::valid_repository(r)).map(|r| format!("{}/{r}", update::SITE_BASE))
}

/// Where this copy looks for releases, if anywhere.
pub fn source() -> Option<Source> {
    source_from(std::env::var(SERVER_VARIABLE).ok().as_deref(), REPOSITORY)
}

fn source_from(server: Option<&str>, built: Option<&str>) -> Option<Source> {
    let built = built.filter(|r| update::valid_repository(r));
    match server.map(str::trim).filter(|s| !s.is_empty()) {
        Some("off") => None,
        Some(server) if loopback(server) => {
            let base = server.trim_end_matches('/').to_owned();
            Some(Source {
                api: base.clone(),
                site: base,
                repository: built.unwrap_or(TEST_REPOSITORY).to_owned(),
            })
        }
        server => {
            if server.is_some() {
                bv_shell::log::error(
                    "update: a release service other than the loopback one is ignored",
                );
            }
            built.map(|repository| Source {
                api: update::API_BASE.to_owned(),
                site: update::SITE_BASE.to_owned(),
                repository: repository.to_owned(),
            })
        }
    }
}

/// The user agent of every request: the program and its version, nothing else.
fn agent() -> String {
    format!("BetterVolume/{}", crate::VERSION)
}

/// The means to check and download for this copy, which updates itself with `file`.
pub fn platform(source: Source, waker: Waker, file: Option<Kind>) -> UpdatePlatform {
    let fetch_source = source.clone();
    let fetch_waker = waker.clone();
    UpdatePlatform {
        fetch: Box::new(move || {
            let (done, result) = mpsc::channel();
            let source = fetch_source.clone();
            let waker = fetch_waker.clone();
            let started =
                std::thread::Builder::new().name("bv-update-check".into()).spawn(move || {
                    let _ = done.send(fetch(&source));
                    waker.wake();
                });
            if started.is_err() {
                bv_shell::log::error("update: cannot start the check");
            }
            result
        }),
        download: Box::new(move |asset, version, kind| {
            let (events, inbox) = mpsc::channel();
            let (asset, version, waker) = (asset.clone(), version.clone(), waker.clone());
            let started =
                std::thread::Builder::new().name("bv-update-download".into()).spawn(move || {
                    let mut report = |percent| {
                        let _ = events.send(Download::Progress(percent));
                        waker.wake();
                    };
                    let outcome = destination(kind, &version).ok_or(()).and_then(|target| {
                        download(&asset, kind, &target, &mut report).map(|()| target)
                    });
                    let _ = events.send(outcome.map_or(Download::Failed, Download::Ready));
                    waker.wake();
                });
            if started.is_err() {
                bv_shell::log::error("update: cannot start the download");
            }
            inbox
        }),
        site: source.site,
        file,
        format_time: Box::new(bv_shell::local_time_text),
    }
}

/// Fetches the list of releases.
fn fetch(source: &Source) -> Result<String, ()> {
    let url = update::releases_url(&source.api, &source.repository);
    let headers =
        [("Accept", "application/vnd.github+json"), ("X-GitHub-Api-Version", "2022-11-28")];
    match bv_shell::http::get(&url, &agent(), &headers, update::ANSWER_LIMIT, &mut |_, _| {}) {
        Ok(body) => String::from_utf8(body).map_err(|_| ()),
        Err(error) => {
            bv_shell::log::error(&format!("update: check failed ({error})"));
            Err(())
        }
    }
}

/// Where the downloaded file of `kind` goes: the temporary folder for the setup file, beside
/// this executable for the portable one (it is then renamed into place, on the same drive).
fn destination(kind: Kind, version: &Version) -> Option<PathBuf> {
    match kind {
        Kind::Setup => {
            Some(std::env::temp_dir().join(DOWNLOAD_FOLDER).join(update::file_name(kind, version)))
        }
        Kind::Portable => std::env::current_exe().ok().map(|exe| sibling(&exe, "new")),
    }
}

/// `file` with `extension` added to its whole name (`a.exe` → `a.exe.new`).
fn sibling(file: &Path, extension: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Downloads, checks and writes the file; reports progress on the way.
fn download(
    asset: &Asset,
    kind: Kind,
    target: &Path,
    report: &mut dyn FnMut(u8),
) -> Result<(), ()> {
    let mut shown = 0u8;
    let size = asset.size.max(1);
    let mut progress = |received: u64, _: Option<u64>| {
        let percent = u8::try_from((received.min(size) * 100) / size).unwrap_or(100);
        if percent != shown {
            shown = percent;
            report(percent);
        }
    };
    let limit = usize::try_from(asset.size).map_err(|_| ())?;
    let bytes =
        bv_shell::http::get(&asset.url, &agent(), &[], limit, &mut progress).map_err(|error| {
            bv_shell::log::error(&format!("update: download failed ({error})"));
        })?;
    if !update::file_matches(asset, &bytes) {
        bv_shell::log::error("update: the downloaded file is not the one announced");
        return Err(());
    }
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder).map_err(|_| ())?;
    }
    let partial = sibling(target, "partial");
    let written = std::fs::write(&partial, &bytes).and_then(|()| std::fs::rename(&partial, target));
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
        bv_shell::log::error("update: the downloaded file cannot be written");
        return Err(());
    }
    // The file must be a release file of the expected kind.
    if bv_shell::Package::of(target).map(|p| p.kind) != Some(kind) {
        let _ = std::fs::remove_file(target);
        bv_shell::log::error("update: the downloaded file is not a release file of that kind");
        return Err(());
    }
    Ok(())
}

/// Puts the downloaded portable file `new` in place of this executable: this one
/// is renamed aside (a running program can be renamed, not overwritten) and removed by
/// the new copy once this one has ended. Returns the executable's path.
pub fn swap_portable(new: &Path) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let old = sibling(&exe, "old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).ok()?;
    if std::fs::rename(new, &exe).is_err() {
        let _ = std::fs::rename(&old, &exe);
        return None;
    }
    Some(exe)
}

/// Undoes [`swap_portable`] when the new copy could not be started.
pub fn unswap_portable(exe: &Path, new: &Path) {
    let old = sibling(exe, "old");
    if std::fs::rename(exe, new).is_ok() {
        let _ = std::fs::rename(&old, exe);
    }
}

/// Removes what an update left: the replaced portable file beside this one, and the
/// downloaded setup files. Files still in use stay until the next start.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        for extension in ["old", "new", "new.partial"] {
            let _ = std::fs::remove_file(sibling(&exe, extension));
        }
    }
    let folder = std::env::temp_dir().join(DOWNLOAD_FOLDER);
    if folder.is_dir() {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_page_is_the_repository_the_build_names() {
        assert_eq!(
            project_page_from(Some("owner/BetterVolume")).as_deref(),
            Some("https://github.com/owner/BetterVolume")
        );
        assert_eq!(project_page_from(None), None, "a development build has none");
        assert_eq!(project_page_from(Some("not a repository")), None);
    }

    #[test]
    fn releases_are_looked_for_where_the_build_says_or_at_the_local_imitation() {
        assert_eq!(source_from(None, None), None, "a development build is not updated");
        assert_eq!(source_from(None, Some("not a repository")), None);
        let github = source_from(None, Some("owner/BetterVolume")).expect("a source");
        assert_eq!(github.api, "https://api.github.com");
        assert_eq!(github.site, "https://github.com");
        assert_eq!(github.repository, "owner/BetterVolume");
        let local = source_from(Some("http://127.0.0.1:8123/"), None).expect("the imitation");
        assert_eq!(
            (local.api.as_str(), local.site.as_str()),
            ("http://127.0.0.1:8123", "http://127.0.0.1:8123")
        );
        assert_eq!(local.repository, TEST_REPOSITORY);
        assert_eq!(source_from(Some("off"), Some("owner/BetterVolume")), None);
        // Only the loopback interface may replace the release service.
        assert_eq!(
            source_from(Some("https://evil.example"), Some("owner/BetterVolume")),
            Some(github)
        );
        assert_eq!(source_from(Some("http://192.168.1.2:80"), None), None);
    }

    /// Serves `answers` (path → body) on the loopback address, one request each.
    fn serve(answers: Vec<(String, Vec<u8>)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let thread = std::thread::spawn(move || {
            let mut asked = Vec::new();
            for _ in 0..answers.len() {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut header = String::new();
                let mut agent = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    if header.starts_with("User-Agent:") {
                        agent = header.trim().to_owned();
                    }
                    header.clear();
                }
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                asked.push(format!("{path} {agent}"));
                let body = answers.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone());
                let (status, body) = match body {
                    Some(body) => ("200 OK", body),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
            asked
        });
        (base, thread)
    }

    #[test]
    fn checks_and_downloads_through_the_network_stack() {
        let setup = bv_core::package::build(Kind::Setup, b"MZprogram", b"MZhelper");
        let digest = bv_core::encoding::hex(&bv_core::encoding::sha256(&setup));
        let file = "/download/BetterVolume-9.0.0-setup.exe".to_owned();
        let (base, server) = serve(vec![
            ("/repos/local/BetterVolume/releases?per_page=30".into(), b"[]".to_vec()),
            (file.clone(), setup.clone()),
            (file.clone(), setup.clone()),
        ]);
        let source = source_from(Some(&base), None).unwrap();
        assert_eq!(fetch(&source).as_deref(), Ok("[]"));
        let asset = Asset {
            url: format!("{base}{file}"),
            size: setup.len() as u64,
            digest: bv_core::encoding::from_hex(&digest).and_then(|d| d.try_into().ok()),
        };
        let folder = std::env::temp_dir().join(format!("bv-download-{}", std::process::id()));
        let target = folder.join("setup.exe");
        let mut seen = Vec::new();
        assert_eq!(download(&asset, Kind::Setup, &target, &mut |p| seen.push(p)), Ok(()));
        assert_eq!(std::fs::read(&target).unwrap(), setup);
        assert_eq!(seen.last(), Some(&100));
        // The right bytes, but not the kind of file expected: refused and removed.
        assert_eq!(download(&asset, Kind::Portable, &target, &mut |_| {}), Err(()));
        assert!(!target.exists());
        let asked = server.join().unwrap();
        assert!(
            asked
                .iter()
                .all(|a| a.ends_with(&format!("User-Agent: BetterVolume/{}", crate::VERSION))),
            "{asked:?}"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_damaged_download_is_refused() {
        let setup = bv_core::package::build(Kind::Setup, b"MZprogram", b"MZhelper");
        let (base, server) = serve(vec![("/f".into(), setup.clone())]);
        let asset =
            Asset { url: format!("{base}/f"), size: setup.len() as u64, digest: Some([7; 32]) };
        let folder = std::env::temp_dir().join(format!("bv-damaged-{}", std::process::id()));
        let target = folder.join("setup.exe");
        assert_eq!(download(&asset, Kind::Setup, &target, &mut |_| {}), Err(()));
        assert!(!target.exists());
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn sibling_names_keep_the_whole_name() {
        let file = Path::new(r"C:\x\BetterVolume-2.0.0-portable.exe");
        assert_eq!(
            sibling(file, "old"),
            PathBuf::from(r"C:\x\BetterVolume-2.0.0-portable.exe.old")
        );
    }

    #[test]
    fn the_portable_file_is_swapped_and_back() {
        // Works on files in a scratch folder: the swap of the running executable itself is
        // exercised live (tools/update-test.ps1).
        let folder = std::env::temp_dir().join(format!("bv-swap-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("a.exe");
        let new = sibling(&exe, "new");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        let old = sibling(&exe, "old");
        std::fs::rename(&exe, &old).unwrap();
        std::fs::rename(&new, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        unswap_portable(&exe, &new);
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert_eq!(std::fs::read(&new).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
