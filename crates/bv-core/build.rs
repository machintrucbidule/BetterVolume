//! Compiles the translation files in `locales/` into static tables.
//!
//! Each `locales/<tag>.toml` file is a flat list of `key = "text"` lines (a strict subset
//! of TOML). `en.toml` is the reference: its keys define the generated `Msg` enum. Any
//! other file becomes an extra language with no code change (FR-120). A key missing
//! from a language is filled with the English text at build time and reported in the
//! generated `MISSING` table, which a unit test requires to be empty for shipped languages.
#![allow(clippy::print_stdout)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

const REFERENCE_TAG: &str = "en";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let locales_dir = manifest_dir.join("../../locales");
    println!("cargo:rerun-if-changed={}", locales_dir.display());

    let mut languages: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let entries = fs::read_dir(&locales_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", locales_dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let tag = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned();
        assert!(is_valid_tag(&tag), "{}: file name is not a valid language tag", path.display());
        let previous = languages.insert(tag.to_ascii_lowercase(), parse_file(&path));
        assert!(previous.is_none(), "{}: language defined twice", path.display());
    }

    let reference = languages
        .remove(REFERENCE_TAG)
        .unwrap_or_else(|| panic!("locales/{REFERENCE_TAG}.toml is required"));
    let keys: Vec<&str> = reference.iter().map(|(k, _)| k.as_str()).collect();

    let mut variants: Vec<String> = Vec::with_capacity(keys.len());
    for key in &keys {
        let variant = variant_name(key);
        assert!(!variants.contains(&variant), "key '{key}' collides with another key");
        variants.push(variant);
    }

    let mut out = String::new();
    let n = keys.len();

    out.push_str("/// Identifier of a translatable text (generated from `locales/en.toml`).\n");
    out.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n#[repr(u16)]\n");
    out.push_str("pub enum Msg {\n");
    for (key, variant) in keys.iter().zip(&variants) {
        writeln!(out, "    /// `{key}`\n    {variant},").unwrap();
    }
    out.push_str("}\n\n");

    writeln!(out, "pub(crate) const MSG_COUNT: usize = {n};").unwrap();
    out.push_str("pub(crate) static ALL_MSGS: [Msg; MSG_COUNT] = [\n");
    for variant in &variants {
        writeln!(out, "    Msg::{variant},").unwrap();
    }
    out.push_str("];\n");
    out.push_str("pub(crate) static KEYS: [&str; MSG_COUNT] = [\n");
    for key in &keys {
        writeln!(out, "    {key:?},").unwrap();
    }
    out.push_str("];\n\n");

    let mut missing: Vec<(String, String)> = Vec::new();
    let mut extra: Vec<(String, String)> = Vec::new();

    out.push_str("pub(crate) struct LanguageData {\n    pub(crate) tag: &'static str,\n");
    out.push_str("    pub(crate) texts: [&'static str; MSG_COUNT],\n}\n");
    writeln!(out, "pub(crate) static LANGUAGES: [LanguageData; {}] = [", languages.len() + 1)
        .unwrap();
    let mut emit = |tag: &str, texts: &[&str]| {
        writeln!(out, "    LanguageData {{ tag: {tag:?}, texts: [").unwrap();
        for text in texts {
            writeln!(out, "        {text:?},").unwrap();
        }
        out.push_str("    ] },\n");
    };

    // English is always index 0 (the fallback).
    let reference_texts: Vec<&str> = reference.iter().map(|(_, v)| v.as_str()).collect();
    emit(REFERENCE_TAG, &reference_texts);
    for (tag, entries) in &languages {
        let mut texts = Vec::with_capacity(n);
        for (key, english) in &reference {
            match entries.iter().find(|(k, _)| k == key) {
                Some((_, text)) => texts.push(text.as_str()),
                None => {
                    missing.push((tag.clone(), key.clone()));
                    texts.push(english.as_str());
                }
            }
        }
        for (key, _) in entries {
            if !keys.contains(&key.as_str()) {
                extra.push((tag.clone(), key.clone()));
            }
        }
        emit(tag, &texts);
    }
    out.push_str("];\n\n");

    for (name, list) in [("MISSING", &missing), ("EXTRA", &extra)] {
        writeln!(out, "#[allow(dead_code)]\npub(crate) static {name}: &[(&str, &str)] = &[")
            .unwrap();
        for (tag, key) in list {
            writeln!(out, "    ({tag:?}, {key:?}),").unwrap();
        }
        out.push_str("];\n");
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    fs::write(out_dir.join("i18n_generated.rs"), out).expect("write generated translations");
}

fn is_valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.split('-').all(|part| {
            !part.is_empty() && part.len() <= 8 && part.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|part| {
            !part.is_empty()
                && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// `tray.tooltip_muted` -> `TrayTooltipMuted`
fn variant_name(key: &str) -> String {
    let mut name = String::new();
    for word in key.split(['.', '_']).filter(|w| !w.is_empty()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.push_str(chars.as_str());
        }
    }
    assert!(
        name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()),
        "key '{key}' must start with a letter"
    );
    name
}

fn parse_file(path: &Path) -> Vec<(String, String)> {
    let content =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut entries: Vec<(String, String)> = Vec::new();
    for (index, raw) in content.lines().enumerate() {
        let fail = |what: &str| -> ! { panic!("{}:{}: {what}", path.display(), index + 1) };
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once('=') else { fail("expected `key = \"text\"`") };
        let key = key.trim();
        if !is_valid_key(key) {
            fail("invalid key (use lowercase letters, digits, `_`, separated by `.`)");
        }
        if entries.iter().any(|(k, _)| k == key) {
            fail("duplicate key");
        }
        let value = match parse_string(rest.trim()) {
            Ok(value) => value,
            Err(what) => fail(what),
        };
        entries.push((key.to_owned(), value));
    }
    entries
}

/// Parses a TOML basic string (`"..."`), optionally followed by a `#` comment.
fn parse_string(input: &str) -> Result<String, &'static str> {
    let mut chars = input.chars();
    if chars.next() != Some('"') {
        return Err("text must be enclosed in double quotes");
    }
    let mut value = String::new();
    loop {
        match chars.next() {
            None => return Err("missing closing double quote"),
            Some('"') => break,
            Some('\\') => match chars.next() {
                Some('"') => value.push('"'),
                Some('\\') => value.push('\\'),
                Some('n') => value.push('\n'),
                Some('t') => value.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok().filter(|_| hex.len() == 4);
                    match code.and_then(char::from_u32) {
                        Some(c) => value.push(c),
                        None => return Err("invalid \\uXXXX escape"),
                    }
                }
                _ => return Err("unsupported escape (allowed: \\\" \\\\ \\n \\t \\uXXXX)"),
            },
            Some(c) if c.is_control() => return Err("control character in text"),
            Some(c) => value.push(c),
        }
    }
    let rest = chars.as_str().trim();
    if !rest.is_empty() && !rest.starts_with('#') {
        return Err("unexpected content after the closing double quote");
    }
    Ok(value)
}
