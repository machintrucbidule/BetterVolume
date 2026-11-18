//! Updates (FR-132): what the release service says,
//! which version is offered, and when to ask again.
//!
//! The release service is GitHub's list of the project's releases
//! (`GET <api>/repos/<owner>/<name>/releases`), whose answer is a JSON array of releases
//! with their tag, their pre-release flag, their page and their files. Each release holds
//! the two release files, named after the version. Nothing about the user or the PC is
//! sent: the request is the same for everyone.
//!
//! No I/O here: the executable fetches, this module decides.

use crate::json::{self, Value};
use crate::package::{Kind, Version};

/// The release service (GitHub's REST interface).
pub const API_BASE: &str = "https://api.github.com";

/// Where release pages and files live (any address the answer gives must start so).
pub const SITE_BASE: &str = "https://github.com";

/// How many releases are asked for (the newest first): enough to find the newest official
/// version behind a run of previews.
pub const PAGE_SIZE: u32 = 30;

/// Largest answer read from the release service, in bytes (release notes included).
pub const ANSWER_LIMIT: usize = 4 << 20;

/// Largest release file accepted, in bytes.
pub const FILE_LIMIT: u64 = 32 << 20;

/// One day, in seconds: automatic checks are at least this far apart (FR-132).
pub const DAY: u64 = 86_400;

/// Wait after the application starts before an automatic check that is already due: the
/// start stays light, and the network may not be ready right at sign-in.
pub const FIRST_DELAY: u64 = 120;

/// Wait before trying again after an automatic check that failed (no answer).
pub const RETRY_DELAY: u64 = 3_600;

/// One file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// Its address (the service may redirect to where the bytes are).
    pub url: String,
    /// Its size, in bytes.
    pub size: u64,
    /// Its SHA-256 digest, when the service gives one.
    pub digest: Option<[u8; 32]>,
}

/// One published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version (from its tag).
    pub version: Version,
    /// A preview (pre-release) rather than an official version.
    pub preview: bool,
    /// Its page on the project site ("See what's new").
    pub page: String,
    /// The setup file.
    pub setup: Option<Asset>,
    /// The portable file.
    pub portable: Option<Asset>,
}

impl Release {
    /// The file of `kind`.
    pub fn asset(&self, kind: Kind) -> Option<&Asset> {
        match kind {
            Kind::Setup => self.setup.as_ref(),
            Kind::Portable => self.portable.as_ref(),
        }
    }
}

/// Name of the file of `kind` of `version` in a release.
pub fn file_name(kind: Kind, version: &Version) -> String {
    match kind {
        Kind::Setup => format!("BetterVolume-{version}-setup.exe"),
        Kind::Portable => format!("BetterVolume-{version}-portable.exe"),
    }
}

/// Whether `repository` is an `owner/name` pair as the service accepts it (letters,
/// digits, `-`, `_` and `.`).
pub fn valid_repository(repository: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p.len() <= 100
            && !p.starts_with('.')
            && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    matches!(repository.split_once('/'), Some((owner, name)) if part(owner) && part(name))
}

/// The address of the list of releases of `repository` at the service `api`.
pub fn releases_url(api: &str, repository: &str) -> String {
    format!("{}/repos/{repository}/releases?per_page={PAGE_SIZE}", api.trim_end_matches('/'))
}

/// Whether `url` lies under `base` (same scheme and host, a path below it): addresses
/// read from the answer are followed or opened only then.
pub fn under(url: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    url.len() > base.len()
        && url.is_char_boundary(base.len())
        && url[..base.len()].eq_ignore_ascii_case(base)
        && url[base.len()..].starts_with('/')
        && !url.contains(['\\', ' ', '"', '\'', '<', '>', '\r', '\n'])
}

/// Reads the answer of the release service: every published release whose tag is a
/// version, with its files; drafts and entries that cannot be read are left out. Addresses
/// must lie under `site` (the project site, or the local imitation in tests). `None` when
/// the answer is not a list of releases.
pub fn releases(answer: &str, site: &str) -> Option<Vec<Release>> {
    let value = json::parse(answer)?;
    let items = value.as_array()?;
    Some(items.iter().filter_map(|item| release(item, site)).collect())
}

fn release(item: &Value, site: &str) -> Option<Release> {
    if item.get("draft").and_then(Value::as_bool) != Some(false) {
        return None;
    }
    let version = Version::parse(item.get("tag_name")?.as_str()?)?;
    let flagged = item.get("prerelease").and_then(Value::as_bool).unwrap_or(true);
    let page = item.get("html_url")?.as_str()?;
    if !under(page, site) {
        return None;
    }
    let assets = item.get("assets").and_then(Value::as_array).unwrap_or_default();
    let asset = |kind| {
        let name = file_name(kind, &version);
        assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .and_then(|a| self::asset(a, site))
    };
    Some(Release {
        preview: flagged || version.is_pre_release(),
        page: page.to_owned(),
        setup: asset(Kind::Setup),
        portable: asset(Kind::Portable),
        version,
    })
}

fn asset(item: &Value, site: &str) -> Option<Asset> {
    let url = item.get("browser_download_url")?.as_str()?;
    let size = item.get("size")?.as_u64()?;
    if !under(url, site) || size == 0 || size > FILE_LIMIT {
        return None;
    }
    // "sha256:<64 hex digits>" (absent from releases made before the service gave it).
    let digest = match item.get("digest").and_then(Value::as_str) {
        Some(text) => {
            let hex = text.strip_prefix("sha256:")?;
            let bytes = crate::encoding::from_hex(hex)?;
            Some(<[u8; 32]>::try_from(bytes.as_slice()).ok()?)
        }
        None => None,
    };
    Some(Asset { url: url.to_owned(), size, digest })
}

/// The version to offer to a copy running `running`: the newest release newer
/// than it, previews included only when `previews` is on.
pub fn offer<'a>(
    releases: &'a [Release],
    running: &Version,
    previews: bool,
) -> Option<&'a Release> {
    releases
        .iter()
        .filter(|r| r.version > *running && (previews || !r.preview))
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// Whether the flyout line is shown for `available`: once closed for a version, it
/// comes back only for an even newer one.
pub fn line_shown(available: &Version, dismissed: Option<&str>) -> bool {
    dismissed.and_then(Version::parse).is_none_or(|closed| *available > closed)
}

/// Whether a downloaded file is the one the release announced: its size and, when
/// announced, its digest.
pub fn file_matches(asset: &Asset, bytes: &[u8]) -> bool {
    bytes.len() as u64 == asset.size
        && asset.digest.is_none_or(|digest| crate::encoding::sha256(bytes) == digest)
}

/// Shortest wait of the check timer, in seconds; a timer firing this close to its time
/// counts as on time (Windows may deliver it a little early or late).
pub const MIN_WAIT: u64 = 60;

/// Whether an automatic check is due at `now`, given the time of the last successful one
/// (a day later, or a clock moved back by more than a day).
pub fn due(last_check: Option<u64>, now: u64) -> bool {
    last_check.is_none_or(|last| {
        now.saturating_add(MIN_WAIT) >= last.saturating_add(DAY) || last > now.saturating_add(DAY)
    })
}

/// Seconds to wait, at `now` (Unix time, seconds), before the next automatic check,
/// given the time of the last successful one (FR-132: at most once a day). A clock moved
/// back by more than a day counts as "never checked".
pub fn wait_before_check(last_check: Option<u64>, now: u64) -> u64 {
    let due = match last_check {
        Some(last) if last <= now.saturating_add(DAY) => last.saturating_add(DAY),
        _ => now,
    };
    due.saturating_sub(now).clamp(FIRST_DELAY, DAY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "https://github.com";

    fn v(text: &str) -> Version {
        Version::parse(text).expect("version")
    }

    fn answer() -> String {
        let digest = crate::encoding::hex(&crate::encoding::sha256(b"setup"));
        format!(
            r#"[
  {{"tag_name": "v2.2.0-beta.1", "draft": false, "prerelease": true,
    "html_url": "https://github.com/o/BetterVolume/releases/tag/v2.2.0-beta.1", "assets": []}},
  {{"tag_name": "v2.1.0", "draft": false, "prerelease": false,
    "html_url": "https://github.com/o/BetterVolume/releases/tag/v2.1.0",
    "body": "Notes with \"quotes\" and é",
    "assets": [
      {{"name": "BetterVolume-2.1.0-setup.exe", "size": 5,
        "digest": "sha256:{digest}",
        "browser_download_url": "https://github.com/o/BetterVolume/releases/download/v2.1.0/BetterVolume-2.1.0-setup.exe"}},
      {{"name": "BetterVolume-2.1.0-portable.exe", "size": 7,
        "browser_download_url": "https://github.com/o/BetterVolume/releases/download/v2.1.0/BetterVolume-2.1.0-portable.exe"}},
      {{"name": "notes.txt", "size": 3, "browser_download_url": "https://github.com/x"}}
    ]}},
  {{"tag_name": "v3.0.0", "draft": true, "prerelease": false,
    "html_url": "https://github.com/o/BetterVolume/releases/tag/v3.0.0", "assets": []}},
  {{"tag_name": "nightly", "draft": false, "prerelease": false,
    "html_url": "https://github.com/o/BetterVolume/releases/tag/nightly", "assets": []}},
  {{"tag_name": "v2.0.0", "draft": false, "prerelease": false,
    "html_url": "https://elsewhere.example/v2.0.0", "assets": []}},
  {{"tag_name": "v1.9.0", "draft": false, "prerelease": false,
    "html_url": "https://github.com/o/BetterVolume/releases/tag/v1.9.0", "assets": []}}
]"#
        )
    }

    #[test]
    fn reads_the_published_releases_and_their_files() {
        let found = super::releases(&answer(), SITE).expect("a list");
        let tags: Vec<String> = found.iter().map(|r| r.version.to_string()).collect();
        // The draft, the tag that is no version and the page elsewhere are left out.
        assert_eq!(tags, ["2.2.0-beta.1", "2.1.0", "1.9.0"]);
        assert!(found[0].preview);
        let official = &found[1];
        assert!(!official.preview);
        let setup = official.setup.as_ref().expect("setup file");
        assert_eq!(setup.size, 5);
        assert_eq!(setup.digest, Some(crate::encoding::sha256(b"setup")));
        assert!(setup.url.ends_with("/BetterVolume-2.1.0-setup.exe"));
        let portable = official.asset(Kind::Portable).expect("portable file");
        assert_eq!((portable.size, portable.digest), (7, None));
        assert_eq!(releases(&answer(), "https://another.example"), Some(Vec::new()));
        assert_eq!(releases("{\"message\": \"Not Found\"}", SITE), None);
        assert_eq!(releases("not json", SITE), None);
    }

    #[test]
    fn a_release_flagged_preview_or_named_so_is_a_preview() {
        let text = r#"[{"tag_name": "2.3.0-rc.1", "draft": false, "prerelease": false,
            "html_url": "https://github.com/o/B/releases/tag/2.3.0-rc.1"},
            {"tag_name": "2.3.1", "draft": false,
            "html_url": "https://github.com/o/B/releases/tag/2.3.1"}]"#;
        let found = super::releases(text, SITE).expect("a list");
        assert!(found.iter().all(|r| r.preview), "suffix, or no flag at all");
    }

    #[test]
    fn files_with_a_wrong_address_size_or_digest_are_ignored() {
        let make = |extra: &str| {
            format!(
                r#"[{{"tag_name": "v2.1.0", "draft": false, "prerelease": false,
                "html_url": "https://github.com/o/B/releases/tag/v2.1.0",
                "assets": [{{"name": "BetterVolume-2.1.0-setup.exe", {extra}}}]}}]"#
            )
        };
        let url = r#""browser_download_url": "https://github.com/o/B/x""#;
        for extra in [
            format!(r#""size": 0, {url}"#),
            format!(r#""size": 99999999999, {url}"#),
            r#""size": 5, "browser_download_url": "http://github.com/o/B/x""#.to_owned(),
            format!(r#""size": 5, "digest": "md5:00", {url}"#),
            format!(r#""size": 5, "digest": "sha256:abcd", {url}"#),
        ] {
            let found = releases(&make(&extra), SITE).expect("a list");
            assert_eq!(found[0].setup, None, "{extra}");
        }
    }

    #[test]
    fn offers_the_newest_newer_official_version_unless_previews_are_wanted() {
        let found = super::releases(&answer(), SITE).expect("a list");
        let offered = |running: &str, previews| {
            offer(&found, &v(running), previews).map(|r| r.version.to_string())
        };
        assert_eq!(offered("2.0.0", false).as_deref(), Some("2.1.0"));
        assert_eq!(offered("2.0.0", true).as_deref(), Some("2.2.0-beta.1"));
        assert_eq!(offered("2.1.0", false), None);
        assert_eq!(offered("2.1.0", true).as_deref(), Some("2.2.0-beta.1"));
        // A preview install is offered the next official version.
        assert_eq!(offered("2.1.0-beta.3", false).as_deref(), Some("2.1.0"));
        assert_eq!(offered("2.2.0-beta.1", true), None);
        assert_eq!(offered("9.0.0", true), None);
    }

    #[test]
    fn a_closed_line_comes_back_only_for_a_newer_version() {
        assert!(line_shown(&v("2.1.0"), None));
        assert!(!line_shown(&v("2.1.0"), Some("2.1.0")));
        assert!(line_shown(&v("2.2.0"), Some("2.1.0")));
        assert!(!line_shown(&v("2.0.5"), Some("2.1.0")));
        assert!(line_shown(&v("2.1.0"), Some("damaged")));
    }

    #[test]
    fn a_downloaded_file_must_match_its_size_and_digest() {
        let asset =
            Asset { url: String::new(), size: 5, digest: Some(crate::encoding::sha256(b"setup")) };
        assert!(file_matches(&asset, b"setup"));
        assert!(!file_matches(&asset, b"setuq"));
        assert!(!file_matches(&asset, b"setup!"));
        assert!(file_matches(&Asset { digest: None, ..asset }, b"other"));
    }

    #[test]
    fn checks_at_most_once_a_day() {
        let now = 1_000_000;
        assert_eq!(wait_before_check(None, now), FIRST_DELAY);
        assert_eq!(wait_before_check(Some(now - 2 * DAY), now), FIRST_DELAY);
        assert_eq!(wait_before_check(Some(now - DAY + 600), now), 600);
        assert_eq!(wait_before_check(Some(now - 10), now), DAY - 10);
        assert_eq!(wait_before_check(Some(now), now), DAY);
        // A clock moved back a little keeps the day; by more than a day, it is forgotten.
        assert_eq!(wait_before_check(Some(now + 100), now), DAY);
        assert_eq!(wait_before_check(Some(now + 2 * DAY), now), FIRST_DELAY);
        assert!(due(None, now));
        assert!(due(Some(now - DAY), now));
        assert!(due(Some(now - DAY + 30), now), "a timer a little early counts");
        assert!(!due(Some(now - DAY + 600), now));
        assert!(!due(Some(now + 100), now));
        assert!(due(Some(now + 2 * DAY), now));
    }

    #[test]
    fn addresses_must_lie_under_their_base() {
        assert!(under("https://github.com/o/B/releases/tag/v1", SITE));
        assert!(under("HTTPS://GitHub.com/o", SITE));
        assert!(!under("https://github.com", SITE));
        assert!(!under("https://github.com.evil.example/o", SITE));
        assert!(!under("https://github.comx/o", SITE));
        assert!(!under("http://github.com/o", SITE));
        assert!(!under("https://github.com/o b", SITE));
        assert!(!under("https://github.com/o\"", SITE));
        assert!(under("http://127.0.0.1:8123/o/B", "http://127.0.0.1:8123/"));
        assert!(!under("é", SITE));
    }

    #[test]
    fn repositories_and_addresses() {
        assert!(valid_repository("owner/BetterVolume"));
        assert!(valid_repository("a-b_c.d/e"));
        for bad in ["", "owner", "owner/", "/name", "a/b/c", "a b/c", "a/.git", "a/b?x"] {
            assert!(!valid_repository(bad), "{bad}");
        }
        assert_eq!(
            releases_url("https://api.github.com/", "o/B"),
            "https://api.github.com/repos/o/B/releases?per_page=30"
        );
        assert_eq!(file_name(Kind::Portable, &v("2.0.0")), "BetterVolume-2.0.0-portable.exe");
    }
}
