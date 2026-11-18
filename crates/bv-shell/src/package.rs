//! Reading this executable as a release file (setup or portable): its trailer, its
//! program part and its payload (`bv_core::package`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use bv_core::package::{Kind, Trailer, TRAILER_LEN};

/// This executable, when it is a release file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// Which release file it is.
    pub kind: Kind,
    path: PathBuf,
    trailer: Trailer,
}

impl Package {
    /// This executable as a release file, or `None` for an installed copy or a development
    /// build (no trailer). Reads 32 bytes of the file.
    pub fn own() -> Option<Package> {
        Package::of(&std::env::current_exe().ok()?)
    }

    /// The release file at `path`, if it is one.
    pub fn of(path: &Path) -> Option<Package> {
        let mut file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let start = len.checked_sub(TRAILER_LEN as u64)?;
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut tail = [0u8; TRAILER_LEN];
        file.read_exact(&mut tail).ok()?;
        let trailer = Trailer::parse(&tail, len)?;
        Some(Package { kind: trailer.kind, path: path.to_owned(), trailer })
    }

    /// The program part (what is installed as `BetterVolume.exe`).
    pub fn program(&self) -> std::io::Result<Vec<u8>> {
        self.read(0, self.trailer.program_len)
    }

    /// The payload (the taskbar helper library).
    pub fn payload(&self) -> std::io::Result<Vec<u8>> {
        self.read(self.trailer.program_len, self.trailer.payload_len)
    }

    fn read(&self, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
        file.take(len).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != len {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_file_is_read_back_from_disk() {
        let folder = std::env::temp_dir().join(format!("bv-package-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("portable.exe");
        std::fs::write(&path, bv_core::package::build(Kind::Portable, b"MZprogram", b"helper"))
            .unwrap();
        let package = Package::of(&path).unwrap();
        assert_eq!(package.kind, Kind::Portable);
        assert_eq!(package.program().unwrap(), b"MZprogram");
        assert_eq!(package.payload().unwrap(), b"helper");
        // A plain file is not a release file.
        let plain = folder.join("plain.exe");
        std::fs::write(&plain, vec![7u8; 100]).unwrap();
        assert_eq!(Package::of(&plain), None);
        let tiny = folder.join("tiny.exe");
        std::fs::write(&tiny, b"MZ").unwrap();
        assert_eq!(Package::of(&tiny), None);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
