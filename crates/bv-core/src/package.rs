//! The two files of a release (FR-130) and version numbers.
//!
//! Both files are the executable itself followed by a payload (the taskbar helper
//! library) and a fixed-size trailer saying which file it is. Windows ignores bytes after
//! the last section of an executable, so the file still runs as the program it starts
//! with. The setup file installs: it writes its program part as `BetterVolume.exe` and its
//! payload beside it. The portable file runs in place and writes its payload out only when
//! the taskbar helper is needed. An executable without a trailer is an installed copy or a
//! development build.
//!
//! Layout: `[program][payload][trailer]`, the trailer being, in order: the kind (one
//! byte), seven zero bytes, the program length and the payload length (little-endian
//! 64-bit), then [`MAGIC`].

/// Marks the end of a packaged file (and the version of this layout).
pub const MAGIC: [u8; 8] = *b"BVPACK01";

/// Size of the trailer, in bytes.
pub const TRAILER_LEN: usize = 32;

/// Which file of a release this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The installer (it also performs updates of the installed version).
    Setup,
    /// The portable single file.
    Portable,
}

impl Kind {
    fn code(self) -> u8 {
        match self {
            Kind::Setup => 1,
            Kind::Portable => 2,
        }
    }

    fn from_code(code: u8) -> Option<Kind> {
        match code {
            1 => Some(Kind::Setup),
            2 => Some(Kind::Portable),
            _ => None,
        }
    }
}

/// What the trailer of a packaged file says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trailer {
    /// Which file it is.
    pub kind: Kind,
    /// Length of the program, at the start of the file.
    pub program_len: u64,
    /// Length of the payload, right after the program.
    pub payload_len: u64,
}

impl Trailer {
    /// Reads the trailer from the last [`TRAILER_LEN`] bytes of a file of `file_len`
    /// bytes. `None` when the file is not a packaged file (or is damaged: the lengths must
    /// account for the whole file).
    pub fn parse(tail: &[u8], file_len: u64) -> Option<Trailer> {
        let tail: &[u8; TRAILER_LEN] = tail.try_into().ok()?;
        if tail[24..32] != MAGIC || tail[1..8] != [0; 7] {
            return None;
        }
        let kind = Kind::from_code(tail[0])?;
        let word = |range: std::ops::Range<usize>| {
            tail.get(range).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
        };
        let program_len = word(8..16)?;
        let payload_len = word(16..24)?;
        let total = program_len.checked_add(payload_len)?.checked_add(TRAILER_LEN as u64)?;
        (total == file_len && program_len > 0).then_some(Trailer { kind, program_len, payload_len })
    }

    /// The trailer's bytes.
    pub fn bytes(&self) -> [u8; TRAILER_LEN] {
        let mut out = [0u8; TRAILER_LEN];
        out[0] = self.kind.code();
        out[8..16].copy_from_slice(&self.program_len.to_le_bytes());
        out[16..24].copy_from_slice(&self.payload_len.to_le_bytes());
        out[24..32].copy_from_slice(&MAGIC);
        out
    }
}

/// Builds a packaged file of `kind` from the program and the payload.
pub fn build(kind: Kind, program: &[u8], payload: &[u8]) -> Vec<u8> {
    let trailer =
        Trailer { kind, program_len: program.len() as u64, payload_len: payload.len() as u64 };
    let mut out = Vec::with_capacity(program.len() + payload.len() + TRAILER_LEN);
    out.extend_from_slice(program);
    out.extend_from_slice(payload);
    out.extend_from_slice(&trailer.bytes());
    out
}

/// A version number `MAJOR.MINOR.PATCH`, with an optional pre-release suffix
/// (`2.0.0-rc.1`), compared as semantic versioning says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// Major, minor and patch numbers.
    pub numbers: [u64; 3],
    /// Dot-separated identifiers after `-` (empty for a release).
    pub pre: Vec<String>,
}

impl Version {
    /// Reads `1.2.3`, `v1.2.3` or `1.2.3-beta.2` (build metadata after `+` is ignored).
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next().unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = [0u64; 3];
        let mut parts = core.split('.');
        for number in &mut numbers {
            let part = parts.next()?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            *number = part.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some(pre) => {
                let ids: Vec<String> = pre.split('.').map(str::to_owned).collect();
                let valid = |id: &String| {
                    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                };
                if !ids.iter().all(valid) {
                    return None;
                }
                ids
            }
            None => Vec::new(),
        };
        Some(Version { numbers, pre })
    }

    /// Whether this is a pre-release (`2.0.0-rc.1`).
    pub fn is_pre_release(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [major, minor, patch] = self.numbers;
        write!(f, "{major}.{minor}.{patch}")?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.numbers.cmp(&other.numbers).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // A release comes after its pre-releases.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            // Numeric identifiers come before alphanumeric ones.
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packaged_file_reads_back() {
        let file = build(Kind::Portable, b"program", b"payload!");
        let tail = &file[file.len() - TRAILER_LEN..];
        let trailer = Trailer::parse(tail, file.len() as u64).unwrap();
        assert_eq!(trailer.kind, Kind::Portable);
        assert_eq!(trailer.program_len, 7);
        assert_eq!(trailer.payload_len, 8);
        assert_eq!(&file[..7], b"program");
        assert_eq!(&file[7..15], b"payload!");
        let setup = build(Kind::Setup, b"p", b"");
        let tail = &setup[setup.len() - TRAILER_LEN..];
        assert_eq!(Trailer::parse(tail, setup.len() as u64).unwrap().kind, Kind::Setup);
    }

    #[test]
    fn other_files_are_not_packaged() {
        // A plain executable ends with anything but the trailer.
        assert_eq!(Trailer::parse(&[0u8; TRAILER_LEN], 4096), None);
        // Too short to hold one.
        assert_eq!(Trailer::parse(&[0u8; 10], 10), None);
        let file = build(Kind::Setup, b"program", b"payload");
        let tail = &file[file.len() - TRAILER_LEN..];
        // Lengths that do not account for the whole file (truncated or appended to).
        assert_eq!(Trailer::parse(tail, file.len() as u64 + 1), None);
        assert_eq!(Trailer::parse(tail, file.len() as u64 - 1), None);
        // Unknown kind, damaged padding.
        let mut bad = tail.to_vec();
        bad[0] = 9;
        assert_eq!(Trailer::parse(&bad, file.len() as u64), None);
        let mut bad = tail.to_vec();
        bad[3] = 1;
        assert_eq!(Trailer::parse(&bad, file.len() as u64), None);
        // Lengths that overflow.
        let mut huge = tail.to_vec();
        huge[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(Trailer::parse(&huge, file.len() as u64), None);
        // An empty program is not a program.
        let empty = build(Kind::Setup, b"", b"payload");
        let tail = &empty[empty.len() - TRAILER_LEN..];
        assert_eq!(Trailer::parse(tail, empty.len() as u64), None);
    }

    #[test]
    fn versions_parse_and_print() {
        let v = Version::parse("v2.0.1").unwrap();
        assert_eq!(v.numbers, [2, 0, 1]);
        assert!(!v.is_pre_release());
        assert_eq!(v.to_string(), "2.0.1");
        let v = Version::parse("2.0.0-rc.1+build.5").unwrap();
        assert_eq!(v.pre, ["rc", "1"]);
        assert!(v.is_pre_release());
        assert_eq!(v.to_string(), "2.0.0-rc.1");
        for bad in ["", "2", "2.0", "2.0.0.0", "2.x.0", "2.0.-1", "2.0.0-", "2.0.0-a..b", " "] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn versions_order_like_semantic_versioning() {
        let order = [
            "0.9.1",
            "1.0.0",
            "2.0.0-alpha",
            "2.0.0-alpha.1",
            "2.0.0-alpha.beta",
            "2.0.0-beta",
            "2.0.0-beta.2",
            "2.0.0-beta.11",
            "2.0.0-rc.1",
            "2.0.0",
            "2.0.1",
            "2.1.0",
            "10.0.0",
        ];
        for pair in order.windows(2) {
            let (a, b) = (Version::parse(pair[0]).unwrap(), Version::parse(pair[1]).unwrap());
            assert!(a < b, "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(Version::parse("v2.0.0"), Version::parse("2.0.0"));
    }
}
