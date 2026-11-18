//! Translations (FR-120 to FR-122).
//!
//! The files in `locales/` are compiled into static tables by the build script, so a lookup
//! is an array index: no parsing, no allocation, no file access at run time. Every
//! user-visible text of the application must be a [`Msg`]; nothing may be hard-coded.
//!
//! Language selection follows FR-121: an explicit choice if it is still shipped, otherwise
//! the first OS display language we have a translation for, otherwise English.

include!(concat!(env!("OUT_DIR"), "/i18n_generated.rs"));

impl Msg {
    /// Every message, in the order of `locales/en.toml`.
    pub fn all() -> &'static [Msg] {
        &ALL_MSGS
    }

    /// The key of this message in the translation files (for example `menu.quit`).
    pub fn key(self) -> &'static str {
        KEYS[self as usize]
    }
}

/// A language shipped with the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Language(usize);

impl Language {
    /// English: the reference language and the fallback.
    pub const ENGLISH: Language = Language(0);

    /// Every shipped language, English first.
    pub fn all() -> impl Iterator<Item = Language> {
        (0..LANGUAGES.len()).map(Language)
    }

    /// The language tag, taken from the translation file name (for example `fr`).
    pub fn tag(self) -> &'static str {
        LANGUAGES[self.0].tag
    }

    /// The name of the language written in that language (for the settings list).
    pub fn native_name(self) -> &'static str {
        self.text(Msg::LanguageName)
    }

    /// The translated text of `msg`. A text missing from a translation is the English one.
    pub fn text(self, msg: Msg) -> &'static str {
        LANGUAGES[self.0].texts[msg as usize]
    }

    /// The translated text of `msg` with its `{name}` placeholders replaced (see
    /// [`substitute`]).
    pub fn format(self, msg: Msg, args: &[(&str, &str)]) -> String {
        substitute(self.text(msg), args)
    }

    /// The shipped language whose tag is exactly `tag` (ignoring case and `_`/`-`).
    pub fn from_tag(tag: &str) -> Option<Language> {
        let wanted = normalise(tag);
        Language::all().find(|l| l.tag() == wanted)
    }

    /// The shipped language that best serves a user asking for `tag`:
    /// the exact tag, else the same base language (`fr-CA` is served by `fr`, and `pt` by
    /// `pt-br` if that is the only Portuguese shipped).
    pub fn best_match(tag: &str) -> Option<Language> {
        let wanted = normalise(tag);
        if wanted.is_empty() {
            return None;
        }
        let base = primary(&wanted);
        Language::all()
            .find(|l| l.tag() == wanted)
            .or_else(|| Language::all().find(|l| l.tag() == base))
            .or_else(|| Language::all().find(|l| primary(l.tag()) == base))
    }

    /// Chooses the language to display.
    ///
    /// `preference` is the explicit choice stored in the settings (`None` = automatic).
    /// `os_languages` are the OS display languages, most preferred first.
    pub fn resolve<S: AsRef<str>>(preference: Option<&str>, os_languages: &[S]) -> Language {
        preference
            .and_then(Language::from_tag)
            .or_else(|| os_languages.iter().find_map(|tag| Language::best_match(tag.as_ref())))
            .unwrap_or(Language::ENGLISH)
    }
}

fn normalise(tag: &str) -> String {
    tag.trim().replace('_', "-").to_ascii_lowercase()
}

fn primary(tag: &str) -> &str {
    tag.split('-').next().unwrap_or(tag)
}

/// Replaces every `{name}` placeholder of `template` by the matching value of `args`.
///
/// A placeholder with no matching argument is left untouched, so a translation mistake is
/// visible instead of silently losing text. Values are inserted as they are (never
/// re-scanned for placeholders): device and application names are safe to pass.
pub fn substitute(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let replacement = after.find('}').and_then(|close| {
            let name = &after[..close];
            args.iter().find(|(n, _)| *n == name).map(|(_, value)| (close, *value))
        });
        match replacement {
            Some((close, value)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The placeholder names used by `template`, sorted and without duplicates.
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find(['{', '}']) {
            Some(end) if after.as_bytes()[end] == b'}' => {
                names.push(&after[..end]);
                rest = &after[end + 1..];
            }
            _ => rest = after,
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_OS: &[&str] = &[];

    #[test]
    fn english_is_first_and_french_is_shipped() {
        assert_eq!(Language::ENGLISH.tag(), "en");
        assert_eq!(Language::all().next(), Some(Language::ENGLISH));
        assert!(Language::from_tag("fr").is_some());
    }

    #[test]
    fn every_language_has_exactly_the_reference_keys() {
        assert!(
            MISSING.is_empty(),
            "texts missing from a translation (language, key): {MISSING:?}"
        );
        assert!(EXTRA.is_empty(), "unknown keys in a translation (language, key): {EXTRA:?}");
    }

    #[test]
    fn no_text_is_empty() {
        for language in Language::all() {
            for &msg in Msg::all() {
                assert!(
                    !language.text(msg).trim().is_empty(),
                    "empty text for '{}' in '{}'",
                    msg.key(),
                    language.tag()
                );
            }
        }
    }

    #[test]
    fn translations_keep_the_placeholders_of_the_reference() {
        for language in Language::all() {
            for &msg in Msg::all() {
                assert_eq!(
                    placeholders(language.text(msg)),
                    placeholders(Language::ENGLISH.text(msg)),
                    "placeholders differ for '{}' in '{}'",
                    msg.key(),
                    language.tag()
                );
            }
        }
    }

    #[test]
    fn keys_and_messages_line_up() {
        assert_eq!(Msg::all().len(), MSG_COUNT);
        assert_eq!(Msg::LanguageName.key(), "language.name");
        assert_eq!(Msg::MenuQuit.key(), "menu.quit");
        for (index, &msg) in Msg::all().iter().enumerate() {
            assert_eq!(msg as usize, index);
        }
    }

    #[test]
    fn lookup_returns_the_translation() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::ENGLISH.text(Msg::MenuQuit), "Quit");
        assert_eq!(french.text(Msg::MenuQuit), "Quitter");
        assert_eq!(french.native_name(), "Français");
        assert_eq!(Language::ENGLISH.native_name(), "English");
    }

    #[test]
    fn tags_are_matched_loosely() {
        let french = Language::from_tag("fr");
        assert!(french.is_some());
        assert_eq!(Language::from_tag("FR"), french);
        assert_eq!(Language::from_tag(" fr "), french);
        assert_eq!(Language::from_tag("fr-FR"), None);
        assert_eq!(Language::best_match("fr-FR"), french);
        assert_eq!(Language::best_match("fr_CA"), french);
        assert_eq!(Language::best_match("en-GB"), Some(Language::ENGLISH));
        assert_eq!(Language::best_match("de-DE"), None);
        assert_eq!(Language::best_match(""), None);
    }

    #[test]
    fn automatic_mode_takes_the_first_supported_os_language() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::resolve(None, &["fr-FR", "en-US"]), french);
        assert_eq!(Language::resolve(None, &["de-DE", "fr-BE", "en-US"]), french);
        assert_eq!(Language::resolve(None, &["en-US", "fr-FR"]), Language::ENGLISH);
    }

    #[test]
    fn automatic_mode_falls_back_to_english() {
        assert_eq!(Language::resolve(None, &["de-DE", "ja-JP"]), Language::ENGLISH);
        assert_eq!(Language::resolve(None, NO_OS), Language::ENGLISH);
    }

    #[test]
    fn explicit_choice_wins_over_the_os_language() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::resolve(Some("en"), &["fr-FR"]), Language::ENGLISH);
        assert_eq!(Language::resolve(Some("fr"), &["en-US"]), french);
    }

    #[test]
    fn unknown_explicit_choice_behaves_like_automatic() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::resolve(Some("xx"), &["fr-FR"]), french);
        assert_eq!(Language::resolve(Some(""), NO_OS), Language::ENGLISH);
    }

    #[test]
    fn substitute_replaces_named_placeholders() {
        assert_eq!(
            substitute("{device}: {volume} %", &[("device", "Speakers"), ("volume", "42")]),
            "Speakers: 42 %"
        );
        assert_eq!(substitute("{a}{a}", &[("a", "x")]), "xx");
        assert_eq!(substitute("no placeholder", &[("a", "x")]), "no placeholder");
        assert_eq!(substitute("", &[]), "");
    }

    #[test]
    fn substitute_leaves_unknown_or_broken_placeholders() {
        assert_eq!(substitute("{unknown} {a}", &[("a", "x")]), "{unknown} x");
        assert_eq!(substitute("open { only", &[("a", "x")]), "open { only");
        assert_eq!(substitute("{ {a}", &[("a", "x")]), "{ x");
        assert_eq!(substitute("}{", &[("a", "x")]), "}{");
    }

    #[test]
    fn substitute_does_not_rescan_inserted_values() {
        assert_eq!(substitute("{a} {b}", &[("a", "{b}"), ("b", "y")]), "{b} y");
        assert_eq!(substitute("{device}", &[("device", "Écouteurs {x}")]), "Écouteurs {x}");
    }

    #[test]
    fn placeholders_are_listed_sorted_and_unique() {
        assert_eq!(placeholders("{b} {a} {b}"), vec!["a", "b"]);
        assert_eq!(placeholders("{ {a}"), vec!["a"]);
        assert!(placeholders("none { here").is_empty());
    }

    #[test]
    fn format_uses_the_translated_template() {
        let french = Language::from_tag("fr").unwrap();
        // A text without placeholder is returned as is, whatever the arguments.
        assert_eq!(french.format(Msg::MenuQuit, &[("unused", "x")]), "Quitter");
        let args = [("device", "Casque"), ("volume", "40")];
        // French typography: non-breaking spaces (U+00A0) before ":" and "%".
        let nbsp = char::from(0xA0_u8);
        assert_eq!(french.format(Msg::TrayTooltip, &args), format!("Casque{nbsp}: 40{nbsp}%"));
        assert_eq!(Language::ENGLISH.format(Msg::TrayTooltip, &args), "Casque: 40%");
    }
}
