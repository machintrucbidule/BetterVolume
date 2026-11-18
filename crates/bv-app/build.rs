//! Build script of the executable.
//!
//! The libraries of the network, of encryption and of protected secrets are only needed
//! while the Home Assistant link is on (FR-118: no footprint otherwise). They are loaded on
//! first use instead of at start (MSVC delay loading), which keeps them out of the idle
//! process when the link is off.
//!
//! It also gives the executable its resources: the application icon (resource 1,
//! shown by Explorer, the Start menu, Windows' installed-apps list and the settings
//! window) and its version information (file properties). They are written as a compiled
//! resource file (`.res`) that the linker takes as an input, so no resource compiler is
//! needed.

use std::path::PathBuf;

/// Resource types (`winuser.h`).
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// Resource number of the application icon (read by `bv-shell/src/settings.rs`).
const APP_ICON: u16 = 1;
/// English (United States), Unicode: the language of the version strings.
const LANGUAGE: u16 = 0x0409;
const CODE_PAGE: u16 = 1200;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    for library in ["crypt32.dll", "secur32.dll", "ws2_32.dll", "winhttp.dll"] {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{library}");
    }
    println!("cargo:rustc-link-arg-bins=delayimp.lib");

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let icon = manifest.join("..").join("..").join("assets").join("icon").join("BetterVolume.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    let icon = std::fs::read(&icon).expect("assets/icon/BetterVolume.ico (tools/icon.ps1)");
    // The update test tool builds a copy with another number (tools/update-test.ps1);
    // releases never set it (see `VERSION` in main.rs).
    println!("cargo:rerun-if-env-changed=BETTERVOLUME_VERSION");
    let version = std::env::var("BETTERVOLUME_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("package version"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("resources.res");
    std::fs::write(&out, resources(&icon, &version)).expect("write resources");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// The compiled resource file: an empty first entry (the format's marker), the icon
/// images, the icon group and the version information.
fn resources(icon: &[u8], version: &str) -> Vec<u8> {
    let mut res = Vec::new();
    entry(&mut res, 0, 0, 0, &[]);
    let images = icon_images(icon);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (i, image) in images.iter().enumerate() {
        let id = i as u16 + 1;
        entry(&mut res, RT_ICON, id, LANGUAGE, image.data);
        // GRPICONDIRENTRY: the ICONDIRENTRY fields, with the resource number for offset.
        group.extend_from_slice(&image.header[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut res, RT_GROUP_ICON, APP_ICON, LANGUAGE, &group);
    entry(&mut res, RT_VERSION, 1, LANGUAGE, &version_info(version));
    res
}

struct IconImage<'a> {
    /// The 16-byte ICONDIRENTRY of the image.
    header: &'a [u8],
    data: &'a [u8],
}

/// The images of an `.ico` file.
fn icon_images(icon: &[u8]) -> Vec<IconImage<'_>> {
    let word = |at: usize| u16::from_le_bytes([icon[at], icon[at + 1]]);
    let dword =
        |at: usize| u32::from_le_bytes([icon[at], icon[at + 1], icon[at + 2], icon[at + 3]]);
    assert!(icon.len() >= 6 && word(2) == 1, "not an icon file");
    (0..usize::from(word(4)))
        .map(|i| {
            let at = 6 + 16 * i;
            let (size, offset) = (dword(at + 8) as usize, dword(at + 12) as usize);
            IconImage { header: &icon[at..at + 16], data: &icon[offset..offset + size] }
        })
        .collect()
}

/// Appends one resource (RESOURCEHEADER with numeric type and name, then the data),
/// both padded to four bytes.
fn entry(res: &mut Vec<u8>, kind: u16, name: u16, language: u16, data: &[u8]) {
    let mut header = Vec::new();
    header.extend_from_slice(&(data.len() as u32).to_le_bytes());
    header.extend_from_slice(&32u32.to_le_bytes());
    header.extend_from_slice(&[0xFF, 0xFF]);
    header.extend_from_slice(&kind.to_le_bytes());
    header.extend_from_slice(&[0xFF, 0xFF]);
    header.extend_from_slice(&name.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    // Moveable, pure, discardable (what resource compilers write); none for the marker.
    let flags: u16 = if kind == 0 { 0 } else { 0x1030 };
    header.extend_from_slice(&flags.to_le_bytes());
    header.extend_from_slice(&language.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    res.extend_from_slice(&header);
    res.extend_from_slice(data);
    pad(res);
}

fn pad(bytes: &mut Vec<u8>) {
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
}

fn utf16z(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

/// One version block: `wLength`, `wValueLength`, `wType`, key, padding, value, padding,
/// children. `value_len` is in bytes for binary values and in characters for text.
fn block(key: &str, value: &[u8], value_len: u16, text: bool, children: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0];
    out.extend_from_slice(&value_len.to_le_bytes());
    out.extend_from_slice(&u16::from(text).to_le_bytes());
    out.extend_from_slice(&utf16z(key));
    pad(&mut out);
    out.extend_from_slice(value);
    if !children.is_empty() {
        pad(&mut out);
        out.extend_from_slice(children);
    }
    let len = out.len() as u16;
    out[0..2].copy_from_slice(&len.to_le_bytes());
    out
}

fn padded(mut bytes: Vec<u8>) -> Vec<u8> {
    pad(&mut bytes);
    bytes
}

/// The VS_VERSIONINFO resource for `version` (`MAJOR.MINOR.PATCH[-pre]`).
fn version_info(version: &str) -> Vec<u8> {
    let core = version.split(['-', '+']).next().unwrap_or("0.0.0");
    let mut numbers = core.split('.').map(|n| n.parse::<u16>().unwrap_or(0));
    let (major, minor, patch) =
        (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
    let ms = (u32::from(major) << 16) | u32::from(minor);
    let ls = u32::from(patch) << 16;
    let mut fixed = Vec::new();
    for value in [
        0xFEEF_04BDu32, // signature
        0x0001_0000,    // structure version
        ms,
        ls, // file version
        ms,
        ls,          // product version
        0x3F,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0, // subtype, date
    ] {
        fixed.extend_from_slice(&value.to_le_bytes());
    }

    let mut strings = Vec::new();
    for (key, value) in [
        ("CompanyName", "BetterVolume"),
        ("FileDescription", "BetterVolume"),
        ("FileVersion", version),
        ("InternalName", "BetterVolume"),
        ("LegalCopyright", "MIT License"),
        ("OriginalFilename", "BetterVolume.exe"),
        ("ProductName", "BetterVolume"),
        ("ProductVersion", version),
    ] {
        let chars = value.encode_utf16().count() as u16 + 1;
        strings.extend(padded(block(key, &utf16z(value), chars, true, &[])));
    }
    let table = block(&format!("{LANGUAGE:04X}{CODE_PAGE:04X}"), &[], 0, true, &strings);
    let string_info = block("StringFileInfo", &[], 0, true, &padded(table));
    let mut translation = Vec::new();
    translation.extend_from_slice(&LANGUAGE.to_le_bytes());
    translation.extend_from_slice(&CODE_PAGE.to_le_bytes());
    let var = block("Translation", &translation, 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &padded(var));
    let mut children = padded(string_info);
    children.extend(padded(var_info));
    block("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &children)
}
