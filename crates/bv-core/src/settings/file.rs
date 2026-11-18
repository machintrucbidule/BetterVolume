//! The settings file: a small, human-readable TOML document (ARCHITECTURE §1).
//!
//! [`write`] produces the whole file; [`read`] parses one. Only the subset of TOML this
//! file needs is understood: comments, `[table]` and `[[array]]` headers, and
//! `key = value` lines whose value is a basic string, an integer or a boolean. Anything
//! else is a syntax error: the caller keeps the file aside and starts from defaults
//! (FR-126). Inside a well-formed file, an unknown key is ignored and a known key with a
//! value of the wrong kind or out of range keeps its default; both are reported as
//! [`Loaded::warnings`] (English, no value quoted: the log carries no personal data).

use super::{
    classify_version, FlyoutHeight, General, HomeAssistant, Settings, SwitchScope, TrayIconStyle,
    VersionStatus, WindowPlacement,
};
use crate::devices::KnownDevice;

/// A settings file that was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The settings, normalised.
    pub settings: Settings,
    /// Keys that were ignored or kept their default (for the log).
    pub warnings: Vec<String>,
}

/// Why a settings file cannot be used at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Not a valid settings document (line number, 1-based).
    Syntax {
        /// Line where reading stopped.
        line: usize,
    },
    /// No version, or a version this application cannot interpret (written by a newer
    /// one).
    Version(Option<u32>),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Syntax { line } => write!(f, "syntax error at line {line}"),
            ReadError::Version(Some(v)) => write!(f, "unsupported settings version {v}"),
            ReadError::Version(None) => write!(f, "settings version missing"),
        }
    }
}

impl std::error::Error for ReadError {}

/// A value of the subset.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Text(String),
    Integer(i64),
    Bool(bool),
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Value::Text(_) => "text",
            Value::Integer(_) => "integer",
            Value::Bool(_) => "boolean",
        }
    }
}

/// Which part of the document a line belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Section {
    Root,
    Table(String),
    /// An element of an array of tables (`[[name]]`), by index.
    Element(String, usize),
}

/// One parsed `key = value` line.
struct Entry {
    section: Section,
    key: String,
    value: Value,
}

const SCOPES: [(SwitchScope, &str); 3] = [
    (SwitchScope::MediaAndCalls, "media-and-calls"),
    (SwitchScope::MediaOnly, "media-only"),
    (SwitchScope::CallsOnly, "calls-only"),
];

const TRAY_STYLES: [(TrayIconStyle, &str); 2] = [
    (TrayIconStyle::SameAsWindows, "same-as-windows"),
    (TrayIconStyle::WithDeviceBadge, "with-device-badge"),
];

const FLYOUT_HEIGHTS: [(FlyoutHeight, &str); 2] =
    [(FlyoutHeight::Full, "full"), (FlyoutHeight::Content, "content")];

/// The automatic language, as written in the file.
const AUTOMATIC: &str = "auto";

/// The text of the settings file for `settings`.
pub fn write(settings: &Settings) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str(
        "# BetterVolume settings. Written by the application: changes made by hand while\n",
    );
    out.push_str(
        "# it runs are overwritten. A file it cannot read is kept aside and replaced.\n\n",
    );
    line(&mut out, "version", &Value::Integer(i64::from(settings.version)));

    let g = &settings.general;
    out.push_str("\n[general]\n");
    let scope = SCOPES.iter().find(|(s, _)| *s == g.switch_scope).map_or("", |(_, n)| n);
    line(&mut out, "switch_scope", &Value::Text(scope.into()));
    line(&mut out, "wheel_step", &Value::Integer(i64::from(g.wheel_step)));
    let language = g.language.clone().unwrap_or_else(|| AUTOMATIC.into());
    line(&mut out, "language", &Value::Text(language));
    line(&mut out, "microphone_enabled", &Value::Bool(g.microphone_enabled));
    line(&mut out, "switch_notification", &Value::Bool(g.switch_notification));
    line(&mut out, "volume_bubble", &Value::Bool(g.volume_bubble));
    let style = TRAY_STYLES.iter().find(|(s, _)| *s == g.tray_icon_style).map_or("", |(_, n)| n);
    line(&mut out, "tray_icon_style", &Value::Text(style.into()));
    let height = FLYOUT_HEIGHTS.iter().find(|(h, _)| *h == g.flyout_height).map_or("", |(_, n)| n);
    line(&mut out, "flyout_height", &Value::Text(height.into()));
    line(&mut out, "hide_windows_volume_icon", &Value::Bool(g.hide_windows_volume_icon));
    line(&mut out, "check_updates", &Value::Bool(g.check_updates));

    out.push_str("\n[interface]\n");
    line(&mut out, "apps_section_expanded", &Value::Bool(g.apps_section_expanded));
    line(&mut out, "first_run_done", &Value::Bool(g.first_run_done));
    line(&mut out, "volume_icon_question_done", &Value::Bool(g.volume_icon_question_done));

    let ha = &settings.home_assistant;
    out.push_str("\n[home_assistant]\n");
    line(&mut out, "enabled", &Value::Bool(ha.enabled));
    line(&mut out, "host", &Value::Text(ha.host.clone()));
    line(&mut out, "port", &Value::Integer(i64::from(ha.port)));
    line(&mut out, "tls", &Value::Bool(ha.tls));
    line(&mut out, "username", &Value::Text(ha.username.clone()));
    // Encrypted for the Windows user (FR-092): unreadable elsewhere.
    if let Some(password) = &ha.password {
        line(&mut out, "password", &Value::Text(password.clone()));
    }
    if let Some(name) = &ha.pc_name {
        line(&mut out, "pc_name", &Value::Text(name.clone()));
    }
    line(&mut out, "discovery_prefix", &Value::Text(ha.discovery_prefix.clone()));
    if let Some(id) = &ha.node_id {
        line(&mut out, "node_id", &Value::Text(id.clone()));
    }
    if let Some(fingerprint) = &ha.trusted_certificate {
        line(&mut out, "trusted_certificate", &Value::Text(fingerprint.clone()));
    }

    let u = &settings.updates;
    out.push_str("\n[updates]\n");
    line(&mut out, "previews", &Value::Bool(u.previews));
    if let Some(when) = u.last_check.and_then(|t| i64::try_from(t).ok()) {
        line(&mut out, "last_check", &Value::Integer(when));
    }
    if let Some(version) = &u.dismissed {
        line(&mut out, "dismissed", &Value::Text(version.clone()));
    }
    if let Some(version) = &u.last_version {
        line(&mut out, "last_version", &Value::Text(version.clone()));
    }

    if let Some(w) = settings.settings_window {
        out.push_str("\n[settings_window]\n");
        line(&mut out, "x", &Value::Integer(i64::from(w.x)));
        line(&mut out, "y", &Value::Integer(i64::from(w.y)));
        line(&mut out, "width", &Value::Integer(i64::from(w.width)));
        line(&mut out, "height", &Value::Integer(i64::from(w.height)));
        line(&mut out, "maximized", &Value::Bool(w.maximized));
    }

    for (name, list) in
        [("outputs", &settings.devices.outputs), ("inputs", &settings.devices.inputs)]
    {
        for device in list {
            out.push_str(&format!("\n[[{name}]]\n"));
            line(&mut out, "id", &Value::Text(device.id.clone()));
            line(&mut out, "windows_name", &Value::Text(device.windows_name.clone()));
            if let Some(custom) = &device.custom_name {
                line(&mut out, "name", &Value::Text(custom.clone()));
            }
            line(&mut out, "hidden", &Value::Bool(device.hidden));
            if let Some(icon) = &device.icon {
                line(&mut out, "icon", &Value::Text(icon.clone()));
            }
        }
    }
    out
}

fn line(out: &mut String, key: &str, value: &Value) {
    out.push_str(key);
    out.push_str(" = ");
    match value {
        Value::Text(text) => quote(out, text),
        Value::Integer(n) => out.push_str(&n.to_string()),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
    out.push('\n');
}

/// Writes `text` as a TOML basic string.
fn quote(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7F => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Reads a settings file.
pub fn read(text: &str) -> Result<Loaded, ReadError> {
    let entries = parse(text)?;
    let version = entries
        .iter()
        .find(|e| e.section == Section::Root && e.key == "version")
        .and_then(|e| match e.value {
            Value::Integer(n) => u32::try_from(n).ok(),
            _ => None,
        });
    let Some(version) = version else { return Err(ReadError::Version(None)) };
    match classify_version(version) {
        VersionStatus::Current => {}
        // No older version exists yet: the first schema change adds its migration here.
        VersionStatus::NeedsMigration { .. } | VersionStatus::Unsupported => {
            return Err(ReadError::Version(Some(version)))
        }
    }

    let mut settings = Settings::default();
    let mut warnings = Vec::new();
    let mut window = WindowPlacement::default();
    let mut window_keys = 0;
    for entry in &entries {
        let place = match &entry.section {
            Section::Root => entry.key.clone(),
            Section::Table(t) => format!("{t}.{}", entry.key),
            Section::Element(a, i) => format!("{a}[{i}].{}", entry.key),
        };
        let outcome = match &entry.section {
            Section::Root if entry.key == "version" => Ok(()),
            Section::Table(t) if t == "general" => general(&mut settings.general, entry),
            Section::Table(t) if t == "interface" => interface(&mut settings.general, entry),
            Section::Table(t) if t == "home_assistant" => {
                home_assistant(&mut settings.home_assistant, entry)
            }
            Section::Table(t) if t == "updates" => updates(&mut settings.updates, entry),
            Section::Table(t) if t == "settings_window" => {
                let applied = placement(&mut window, entry);
                if applied.is_ok() {
                    window_keys += 1;
                }
                applied
            }
            Section::Element(a, _) if a == "outputs" || a == "inputs" => Ok(()),
            _ => Err("unknown key".to_owned()),
        };
        if let Err(problem) = outcome {
            warnings.push(format!("{place}: {problem}"));
        }
    }
    // The window place is all or nothing: a partial one would put the window anywhere.
    if window_keys == 5 {
        settings.settings_window = Some(window);
    } else if window_keys > 0 {
        warnings.push("settings_window: incomplete, ignored".into());
    }
    for (name, list) in
        [("outputs", &mut settings.devices.outputs), ("inputs", &mut settings.devices.inputs)]
    {
        *list = devices(&entries, name, &mut warnings);
    }
    settings.normalise();
    Ok(Loaded { settings, warnings })
}

fn wrong(value: &Value, wanted: &str) -> String {
    format!("{} found, {wanted} expected; default kept", value.kind())
}

fn boolean(value: &Value) -> Result<bool, String> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(wrong(other, "boolean")),
    }
}

fn general(g: &mut General, entry: &Entry) -> Result<(), String> {
    let v = &entry.value;
    match entry.key.as_str() {
        "switch_scope" => g.switch_scope = named(v, &SCOPES)?,
        "wheel_step" => match v {
            Value::Integer(n) => {
                g.wheel_step = u8::try_from(*n)
                    .ok()
                    .filter(|s| (super::WHEEL_STEP_MIN..=super::WHEEL_STEP_MAX).contains(s))
                    .ok_or("out of range; default kept")?;
            }
            other => return Err(wrong(other, "integer")),
        },
        "language" => match v {
            Value::Text(t) if t.trim().eq_ignore_ascii_case(AUTOMATIC) => g.language = None,
            Value::Text(t) => g.language = Some(t.clone()),
            other => return Err(wrong(other, "text")),
        },
        "microphone_enabled" => g.microphone_enabled = boolean(v)?,
        "switch_notification" => g.switch_notification = boolean(v)?,
        "volume_bubble" => g.volume_bubble = boolean(v)?,
        "tray_icon_style" => g.tray_icon_style = named(v, &TRAY_STYLES)?,
        "flyout_height" => g.flyout_height = named(v, &FLYOUT_HEIGHTS)?,
        "hide_windows_volume_icon" => g.hide_windows_volume_icon = boolean(v)?,
        "check_updates" => g.check_updates = boolean(v)?,
        _ => return Err("unknown key".into()),
    }
    Ok(())
}

fn interface(g: &mut General, entry: &Entry) -> Result<(), String> {
    match entry.key.as_str() {
        "apps_section_expanded" => g.apps_section_expanded = boolean(&entry.value)?,
        "first_run_done" => g.first_run_done = boolean(&entry.value)?,
        "volume_icon_question_done" => g.volume_icon_question_done = boolean(&entry.value)?,
        _ => return Err("unknown key".into()),
    }
    Ok(())
}

fn updates(u: &mut super::Updates, entry: &Entry) -> Result<(), String> {
    let v = &entry.value;
    let text = |v: &Value| match v {
        Value::Text(t) => Ok(t.clone()),
        other => Err(wrong(other, "text")),
    };
    match entry.key.as_str() {
        "previews" => u.previews = boolean(v)?,
        "last_check" => match v {
            Value::Integer(n) => {
                u.last_check = Some(u64::try_from(*n).map_err(|_| "out of range; default kept")?);
            }
            other => return Err(wrong(other, "integer")),
        },
        "dismissed" => u.dismissed = Some(text(v)?),
        "last_version" => u.last_version = Some(text(v)?),
        _ => return Err("unknown key".into()),
    }
    Ok(())
}

fn home_assistant(ha: &mut HomeAssistant, entry: &Entry) -> Result<(), String> {
    let v = &entry.value;
    let text = |v: &Value| match v {
        Value::Text(t) => Ok(t.clone()),
        other => Err(wrong(other, "text")),
    };
    match entry.key.as_str() {
        "enabled" => ha.enabled = boolean(v)?,
        "host" => ha.host = text(v)?,
        "port" => match v {
            Value::Integer(n) => {
                ha.port = u16::try_from(*n)
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or("out of range; default kept")?;
            }
            other => return Err(wrong(other, "integer")),
        },
        "tls" => ha.tls = boolean(v)?,
        "username" => ha.username = text(v)?,
        "password" => ha.password = Some(text(v)?),
        "pc_name" => ha.pc_name = Some(text(v)?),
        "discovery_prefix" => ha.discovery_prefix = text(v)?,
        "node_id" => ha.node_id = Some(text(v)?),
        "trusted_certificate" => ha.trusted_certificate = Some(text(v)?),
        _ => return Err("unknown key".into()),
    }
    Ok(())
}

fn placement(w: &mut WindowPlacement, entry: &Entry) -> Result<(), String> {
    let integer = |v: &Value| match v {
        Value::Integer(n) => i32::try_from(*n).map_err(|_| "out of range; ignored".to_owned()),
        other => Err(wrong(other, "integer")),
    };
    match entry.key.as_str() {
        "x" => w.x = integer(&entry.value)?,
        "y" => w.y = integer(&entry.value)?,
        "width" => w.width = integer(&entry.value)?,
        "height" => w.height = integer(&entry.value)?,
        "maximized" => w.maximized = boolean(&entry.value)?,
        _ => return Err("unknown key".into()),
    }
    Ok(())
}

fn named<T: Copy>(value: &Value, names: &[(T, &str)]) -> Result<T, String> {
    match value {
        Value::Text(t) => names
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(t.trim()))
            .map(|(v, _)| *v)
            .ok_or_else(|| "unknown choice; default kept".to_owned()),
        other => Err(wrong(other, "text")),
    }
}

/// The devices of the array `name`, in file order. An element without a text `id` is
/// dropped; other wrong values keep their default.
fn devices(entries: &[Entry], name: &str, warnings: &mut Vec<String>) -> Vec<KnownDevice> {
    let mut list: Vec<(usize, KnownDevice)> = Vec::new();
    let mut ids: Vec<(usize, bool)> = Vec::new();
    for entry in entries {
        let Section::Element(array, index) = &entry.section else { continue };
        if array != name {
            continue;
        }
        let position = match list.iter().position(|(i, _)| i == index) {
            Some(p) => p,
            None => {
                list.push((*index, KnownDevice::default()));
                ids.push((*index, false));
                list.len() - 1
            }
        };
        let device = &mut list[position].1;
        let text = |v: &Value| match v {
            Value::Text(t) => Ok(t.clone()),
            other => Err(wrong(other, "text")),
        };
        let outcome = match entry.key.as_str() {
            "id" => text(&entry.value).map(|t| {
                device.id = t;
                ids[position].1 = true;
            }),
            "windows_name" => text(&entry.value).map(|t| device.windows_name = t),
            "name" => text(&entry.value).map(|t| device.custom_name = Some(t)),
            "hidden" => boolean(&entry.value).map(|b| device.hidden = b),
            "icon" => text(&entry.value).map(|t| device.icon = Some(t)),
            _ => Err("unknown key".into()),
        };
        if let Err(problem) = outcome {
            warnings.push(format!("{name}[{index}].{}: {problem}", entry.key));
        }
    }
    list.into_iter()
        .zip(ids)
        .filter_map(|((index, device), (_, has_id))| {
            if !has_id {
                warnings.push(format!("{name}[{index}]: no identifier; dropped"));
            }
            has_id.then_some(device)
        })
        .collect()
}

/// Splits the document into entries.
fn parse(text: &str) -> Result<Vec<Entry>, ReadError> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut entries: Vec<Entry> = Vec::new();
    let mut section = Section::Root;
    let mut tables: Vec<String> = Vec::new();
    let mut arrays: Vec<(String, usize)> = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let error = ReadError::Syntax { line: number + 1 };
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("[[") {
            let name = header(rest, "]]").ok_or(error.clone())?;
            if tables.contains(&name) {
                return Err(error);
            }
            let count = match arrays.iter_mut().find(|(n, _)| *n == name) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    arrays.push((name.clone(), 0));
                    0
                }
            };
            section = Section::Element(name, count);
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = header(rest, "]").ok_or(error.clone())?;
            if tables.contains(&name) || arrays.iter().any(|(n, _)| *n == name) {
                return Err(error);
            }
            tables.push(name.clone());
            section = Section::Table(name);
            continue;
        }
        let (key, rest) = line.split_once('=').ok_or(error.clone())?;
        let key = key.trim();
        if !is_bare_key(key) {
            return Err(error);
        }
        let (value, rest) = value(rest.trim_start()).ok_or(error.clone())?;
        let rest = rest.trim_start();
        if !(rest.is_empty() || rest.starts_with('#')) {
            return Err(error);
        }
        if entries.iter().any(|e| e.section == section && e.key == key) {
            return Err(error);
        }
        entries.push(Entry { section: section.clone(), key: key.to_owned(), value });
    }
    Ok(entries)
}

/// The name of a table header, given the text after its opening bracket(s).
fn header(rest: &str, close: &str) -> Option<String> {
    let (name, after) = rest.split_once(close)?;
    let after = after.trim_start();
    let name = name.trim();
    (is_bare_key(name) && (after.is_empty() || after.starts_with('#'))).then(|| name.to_owned())
}

fn is_bare_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// One value at the start of `text`, and what follows it.
fn value(text: &str) -> Option<(Value, &str)> {
    if let Some(rest) = text.strip_prefix('"') {
        return string(rest).map(|(s, rest)| (Value::Text(s), rest));
    }
    for (word, b) in [("true", true), ("false", false)] {
        if let Some(rest) = text.strip_prefix(word) {
            if !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                return Some((Value::Bool(b), rest));
            }
        }
    }
    let end = text
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+'))))
        .map_or(text.len(), |(i, _)| i);
    let digits = &text[..end];
    let unsigned = digits.trim_start_matches(['-', '+']);
    // TOML forbids leading zeros ("007").
    if unsigned.is_empty() || (unsigned.len() > 1 && unsigned.starts_with('0')) {
        return None;
    }
    digits.parse::<i64>().ok().map(|n| (Value::Integer(n), &text[end..]))
}

/// A basic string, given the text after its opening quote.
fn string(text: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &text[i + 1..])),
            '\\' => {
                let (_, escape) = chars.next()?;
                match escape {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{C}'),
                    'u' | 'U' => {
                        let len = if escape == 'u' { 4 } else { 8 };
                        let hex: String =
                            (0..len).filter_map(|_| chars.next().map(|(_, h)| h)).collect();
                        if hex.len() != len {
                            return None;
                        }
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    _ => return None,
                }
            }
            c if (c as u32) < 0x20 && c != '\t' => return None,
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SCHEMA_VERSION;

    fn sample() -> Settings {
        let mut settings = Settings::default();
        settings.general.switch_scope = SwitchScope::CallsOnly;
        settings.general.wheel_step = 7;
        settings.general.language = Some("fr".into());
        settings.general.microphone_enabled = false;
        settings.general.switch_notification = false;
        settings.general.volume_bubble = false;
        settings.general.tray_icon_style = TrayIconStyle::WithDeviceBadge;
        settings.general.flyout_height = FlyoutHeight::Content;
        settings.general.apps_section_expanded = false;
        settings.general.first_run_done = true;
        settings.general.volume_icon_question_done = true;
        settings.general.hide_windows_volume_icon = true;
        settings.settings_window =
            Some(WindowPlacement { x: -1200, y: 40, width: 1100, height: 800, maximized: true });
        settings.devices.outputs = vec![
            KnownDevice {
                id: "{0.0.0.00000000}.{a}".into(),
                windows_name: "Speakers \"Realtek\" \\ C:\\x\ttab\u{1}".into(),
                custom_name: Some("Bureau é 🎧".into()),
                hidden: true,
                icon: Some("soundbar".into()),
            },
            KnownDevice { windows_name: "Headset".into(), ..KnownDevice::new("{b}") },
        ];
        settings.devices.inputs = vec![KnownDevice::new("{mic}")];
        settings.home_assistant = HomeAssistant {
            enabled: true,
            host: "broker.example".into(),
            port: 8883,
            tls: true,
            username: "user \"x\"".into(),
            password: Some("AQIDBA==".into()),
            pc_name: Some("Bureau é".into()),
            discovery_prefix: "ha/discovery".into(),
            node_id: Some("0123abcd".into()),
            trusted_certificate: Some("0f".repeat(32)),
        };
        settings.updates = crate::settings::Updates {
            previews: true,
            last_check: Some(1_790_000_000),
            dismissed: Some("2.1.0".into()),
            last_version: Some("2.0.0-rc.1".into()),
        };
        settings
    }

    #[test]
    fn a_file_from_before_home_assistant_keeps_it_off() {
        let loaded = read("version = 1\n[general]\nwheel_step = 3\n").expect("readable");
        assert_eq!(loaded.settings.home_assistant, HomeAssistant::default());
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let loaded = read("version = 1\n[home_assistant]\nport = 70000\ntls = 1\n").unwrap();
        assert_eq!(loaded.settings.home_assistant, HomeAssistant::default());
        let warnings = loaded.warnings.join("\n");
        assert!(warnings.contains("home_assistant.port: out of range"), "{warnings}");
        assert!(warnings.contains("home_assistant.tls: integer found"), "{warnings}");
    }

    #[test]
    fn written_settings_read_back_identical() {
        for settings in [Settings::default(), sample()] {
            let text = write(&settings);
            let loaded = read(&text).expect("readable");
            assert_eq!(loaded.settings, settings, "{text}");
            assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        }
    }

    #[test]
    fn the_file_is_readable_by_a_person() {
        let text = write(&sample());
        assert!(text.contains("switch_scope = \"calls-only\"\n"));
        assert!(text.contains("language = \"fr\"\n"));
        assert!(text.contains("[[outputs]]\nid = \"{0.0.0.00000000}.{a}\"\n"));
        assert!(write(&Settings::default()).contains("language = \"auto\"\n"));
        assert!(!write(&Settings::default()).contains("[settings_window]"));
    }

    #[test]
    fn syntax_errors_are_refused_with_their_line() {
        for (text, line) in [
            ("version = 1\n[general\n", 2),
            ("version = 1\nwheel_step 3\n", 2),
            ("version = 1\nx = \"open\n", 2),
            ("version = 1\nx = 007\n", 2),
            ("version = 1\nx = 1 2\n", 2),
            ("version = 1\nx = [1]\n", 2),
            ("version = 1\nversion = 1\n", 2),
            ("version = 1\n[a]\n[a]\n", 3),
            ("version = 1\nx = \"\\q\"\n", 2),
            ("version = 1\nbad key = 1\n", 2),
            ("\u{0}", 1),
        ] {
            assert_eq!(read(text), Err(ReadError::Syntax { line }), "{text:?}");
        }
    }

    #[test]
    fn missing_or_unknown_versions_are_refused() {
        assert_eq!(read(""), Err(ReadError::Version(None)));
        assert_eq!(read("version = \"1\""), Err(ReadError::Version(None)));
        assert_eq!(read("version = -1"), Err(ReadError::Version(None)));
        let newer = SCHEMA_VERSION + 1;
        assert_eq!(read(&format!("version = {newer}")), Err(ReadError::Version(Some(newer))));
        assert_eq!(read("version = 0"), Err(ReadError::Version(Some(0))));
    }

    #[test]
    fn wrong_values_keep_their_default_and_are_reported() {
        let text = "\u{FEFF}version = 1 # comment\n\
            [general]\n\
            wheel_step = 42\n\
            switch_scope = \"sometimes\"\n\
            microphone_enabled = \"yes\"\n\
            language = \"AUTO\"\n\
            future_option = true\n\
            [settings_window]\n\
            x = 10\n\
            [[outputs]]\n\
            windows_name = \"nameless device\"\n\
            [[outputs]]\n\
            id = \"kept\"\n\
            hidden = 1\n\
            [unknown]\n\
            a = 1\n";
        let loaded = read(text).expect("readable");
        assert_eq!(loaded.settings.general, General::default());
        assert_eq!(loaded.settings.settings_window, None);
        assert_eq!(loaded.settings.devices.outputs, [KnownDevice::new("kept")]);
        let warnings = loaded.warnings.join("\n");
        for expected in [
            "general.wheel_step: out of range",
            "general.switch_scope: unknown choice",
            "general.microphone_enabled: text found",
            "general.future_option: unknown key",
            "settings_window: incomplete",
            "outputs[0]: no identifier",
            "outputs[1].hidden: integer found",
            "unknown.a: unknown key",
        ] {
            assert!(warnings.contains(expected), "missing {expected:?} in\n{warnings}");
        }
        // Warnings never quote values (no personal data in the log).
        assert!(!warnings.contains("nameless"));
    }

    #[test]
    fn a_file_from_before_the_volume_icon_question_asks_it_once() {
        // Older files have no `volume_icon_question_done` key.
        let text = "version = 1
[interface]
first_run_done = true
";
        let loaded = read(text).expect("readable");
        assert!(loaded.settings.general.first_run_done);
        assert!(!loaded.settings.general.volume_icon_question_done);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn a_file_from_before_the_volume_bubble_keeps_it_on() {
        // Older files have no `volume_bubble` key (FR-081).
        let text = "version = 1
[general]
switch_notification = false
";
        let loaded = read(text).expect("readable");
        assert!(!loaded.settings.general.switch_notification);
        assert!(loaded.settings.general.volume_bubble);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn update_values_out_of_range_or_damaged_are_dropped() {
        let text = "version = 1
[updates]
last_check = -5
dismissed = \"nonsense\"
last_version = \"2.0.0\"
previous = true
";
        let loaded = read(text).expect("readable");
        let updates = &loaded.settings.updates;
        assert_eq!(updates.last_check, None);
        assert_eq!(updates.dismissed, None, "not a version");
        assert_eq!(updates.last_version.as_deref(), Some("2.0.0"));
        let warnings = loaded.warnings.join(
            "
",
        );
        assert!(warnings.contains("updates.last_check: out of range"), "{warnings}");
        assert!(warnings.contains("updates.previous: unknown key"), "{warnings}");
    }

    #[test]
    fn escapes_and_unicode_are_decoded() {
        let text = "version = 1\n[[inputs]]\nid = \"a\\u00E9\\U0001F3A7\\\"\"\n";
        let loaded = read(text).expect("readable");
        assert_eq!(loaded.settings.devices.inputs[0].id, "a\u{e9}\u{1f3a7}\"");
        assert!(read("version = 1\nx = \"\\u12\"\n").is_err());
        assert!(read("version = 1\nx = \"\\uD800\"\n").is_err(), "no surrogate halves");
    }
}
