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

    /// The language tag with the usual letter case (`pt-BR`, `zh-Hans`), for Windows APIs
    /// that take a locale name (the text locale chooses the fonts of Asian scripts).
    pub fn locale_name(self) -> String {
        canonical_case(self.tag())
    }

    /// Every shipped language in the order of the language list (FR-075): alphabetical
    /// order of their own names, accents and case ignored, as the Windows language list;
    /// names in a Latin script first, then the other scripts in the order of their tags.
    pub fn in_display_order() -> Vec<Language> {
        let mut languages: Vec<Language> = Language::all().collect();
        languages.sort_by_cached_key(|l| {
            let name = l.native_name();
            if name.chars().next().is_some_and(is_latin_letter) {
                (0, fold(name))
            } else {
                (1, l.tag().to_owned())
            }
        });
        languages
    }

    /// The shipped language that best serves a user asking for `tag`:
    /// the exact tag, else the same base language (`fr-CA` is served by `fr`, and `pt-PT`
    /// by `pt-br` if that is the only Portuguese shipped), never one written in another
    /// script (Traditional Chinese, `zh-TW`, is not served by Simplified Chinese).
    pub fn best_match(tag: &str) -> Option<Language> {
        let wanted = normalise(tag);
        if wanted.is_empty() {
            return None;
        }
        let base = primary(&wanted);
        let wanted_script = script(&wanted);
        let same_script = |l: &Language| match (wanted_script, script(l.tag())) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        Language::all()
            .find(|l| l.tag() == wanted)
            .or_else(|| Language::all().find(|l| l.tag() == base && same_script(l)))
            .or_else(|| Language::all().find(|l| primary(l.tag()) == base && same_script(l)))
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

/// The script of a normalised tag when it matters for matching: its script subtag
/// (`zh-hant-tw` -> `hant`), else for Chinese the script implied by the region (Taiwan,
/// Hong Kong and Macao write Traditional Chinese, everywhere else Simplified).
fn script(tag: &str) -> Option<&str> {
    let mut parts = tag.split('-');
    let language = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    if let Some(script) =
        rest.iter().find(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()))
    {
        return Some(script);
    }
    if language != "zh" {
        return None;
    }
    if rest.iter().any(|p| matches!(*p, "tw" | "hk" | "mo")) {
        Some("hant")
    } else {
        Some("hans")
    }
}

/// `pt-br` -> `pt-BR`, `zh-hans` -> `zh-Hans`: language lower case, script title case,
/// region upper case (BCP 47 conventions).
fn canonical_case(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len());
    for (index, part) in tag.split('-').enumerate() {
        if index > 0 {
            out.push('-');
        }
        let script = index > 0 && part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic());
        let region = index > 0
            && ((part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
                || (part.len() == 3 && part.chars().all(|c| c.is_ascii_digit())));
        if script {
            out.push_str(&part[..1].to_ascii_uppercase());
            out.push_str(&part[1..].to_ascii_lowercase());
        } else if region {
            out.push_str(&part.to_ascii_uppercase());
        } else {
            out.push_str(&part.to_ascii_lowercase());
        }
    }
    out
}

/// A letter of the Latin script (ASCII, Latin-1 Supplement, Latin Extended-A and B).
fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || (('\u{C0}'..='\u{24F}').contains(&c) && c.is_alphabetic())
}

/// Lower case without accents, for sorting names of the Latin script.
fn fold(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        let plain = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'æ' => "ae",
            'ç' | 'ć' | 'č' => "c",
            'ď' | 'đ' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
            'ğ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => "i",
            'ĺ' | 'ľ' | 'ł' => "l",
            'ñ' | 'ń' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
            'œ' => "oe",
            'ŕ' | 'ř' => "r",
            'ś' | 'š' | 'ş' | 'ș' => "s",
            'ß' => "ss",
            'ť' | 'ţ' | 'ț' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => "u",
            'ý' | 'ÿ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            other => {
                out.push(other);
                continue;
            }
        };
        out.push_str(plain);
    }
    out
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
        assert_eq!(Language::best_match("sv-SE"), None);
        assert_eq!(Language::best_match(""), None);
    }

    #[test]
    fn automatic_mode_takes_the_first_supported_os_language() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::resolve(None, &["fr-FR", "en-US"]), french);
        assert_eq!(Language::resolve(None, &["sv-SE", "fr-BE", "en-US"]), french);
        assert_eq!(Language::resolve(None, &["en-US", "fr-FR"]), Language::ENGLISH);
    }

    #[test]
    fn automatic_mode_falls_back_to_english() {
        assert_eq!(Language::resolve(None, &["sv-SE", "ko-KR"]), Language::ENGLISH);
        assert_eq!(Language::resolve(None, NO_OS), Language::ENGLISH);
    }

    #[test]
    fn explicit_choice_wins_over_the_os_language() {
        let french = Language::from_tag("fr").unwrap();
        assert_eq!(Language::resolve(Some("en"), &["fr-FR"]), Language::ENGLISH);
        assert_eq!(Language::resolve(Some("fr"), &["en-US"]), french);
    }

    #[test]
    fn the_eight_added_languages_ship() {
        for tag in ["de", "es", "it", "pt-BR", "nl", "pl", "ja", "zh-Hans"] {
            assert!(Language::from_tag(tag).is_some(), "{tag} ships");
        }
        assert_eq!(Language::all().count(), 10);
    }

    #[test]
    fn chinese_is_matched_by_script() {
        let simplified = Language::from_tag("zh-Hans");
        assert!(simplified.is_some());
        for tag in ["zh", "zh-CN", "zh-SG", "zh-Hans", "zh-Hans-HK", "ZH_cn"] {
            assert_eq!(Language::best_match(tag), simplified, "{tag}");
        }
        // Traditional Chinese is another script: English rather than Simplified.
        for tag in ["zh-TW", "zh-HK", "zh-MO", "zh-Hant", "zh-Hant-TW", "zh_tw"] {
            assert_eq!(Language::best_match(tag), None, "{tag}");
        }
        assert_eq!(Language::resolve(None, &["zh-TW", "en-US"]), Language::ENGLISH);
        assert_eq!(Language::resolve(None, &["zh-TW", "zh-CN"]), simplified.unwrap());
    }

    #[test]
    fn any_portuguese_is_served_by_brazilian_portuguese() {
        let brazil = Language::from_tag("pt-BR");
        assert!(brazil.is_some());
        for tag in ["pt", "pt-BR", "pt-PT", "pt-AO"] {
            assert_eq!(Language::best_match(tag), brazil, "{tag}");
        }
        let german = Language::from_tag("de");
        assert_eq!(Language::best_match("de-AT"), german);
        assert_eq!(Language::best_match("de-CH"), german);
        assert_eq!(Language::best_match("ja-JP"), Language::from_tag("ja"));
    }

    #[test]
    fn locale_names_have_the_usual_case() {
        let name = |tag: &str| Language::from_tag(tag).map(Language::locale_name);
        assert_eq!(name("pt-br").as_deref(), Some("pt-BR"));
        assert_eq!(name("zh-hans").as_deref(), Some("zh-Hans"));
        assert_eq!(name("ja").as_deref(), Some("ja"));
        assert_eq!(canonical_case("ES-419"), "es-419");
        assert_eq!(canonical_case("sr-latn-rs"), "sr-Latn-RS");
    }

    #[test]
    fn the_language_list_is_in_the_order_of_their_names() {
        let names: Vec<&str> =
            Language::in_display_order().into_iter().map(Language::native_name).collect();
        let tags: Vec<&str> = Language::in_display_order().into_iter().map(Language::tag).collect();
        assert_eq!(names.len(), Language::all().count());
        assert_eq!(
            tags,
            ["de", "en", "es", "fr", "it", "nl", "pl", "pt-br", "ja", "zh-hans"],
            "{names:?}"
        );
        assert_eq!(fold("Čeština Ébène"), "cestina ebene");
        assert!(fold("Español") > fold("English"));
        assert!(is_latin_letter('É') && is_latin_letter('a'));
        assert!(!is_latin_letter('日') && !is_latin_letter('Ж'));
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
