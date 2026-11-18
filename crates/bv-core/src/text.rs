//! Small text helpers for user-visible strings.

use std::borrow::Cow;

/// The ellipsis appended to shortened texts.
pub const ELLIPSIS: char = '…';

/// `text` shortened so that it takes at most `max_units` UTF-16 code units (the unit
/// Windows counts in), ending with an ellipsis when something was cut. Characters are
/// never split, and trailing spaces before the ellipsis are dropped.
pub fn ellipsize(text: &str, max_units: usize) -> Cow<'_, str> {
    if text.encode_utf16().count() <= max_units {
        return Cow::Borrowed(text);
    }
    if max_units == 0 {
        return Cow::Borrowed("");
    }
    // Room for the text before the ellipsis (which is one UTF-16 unit).
    let budget = max_units - 1;
    let mut used = 0;
    let mut end = 0;
    for (index, c) in text.char_indices() {
        if used + c.len_utf16() > budget {
            break;
        }
        used += c.len_utf16();
        end = index + c.len_utf8();
    }
    let mut out = text[..end].trim_end().to_owned();
    out.push(ELLIPSIS);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_texts_are_untouched() {
        assert_eq!(ellipsize("Speakers", 8), "Speakers");
        assert_eq!(ellipsize("", 0), "");
        assert!(matches!(ellipsize("abc", 10), Cow::Borrowed(_)));
    }

    #[test]
    fn long_texts_end_with_an_ellipsis_within_the_limit() {
        assert_eq!(ellipsize("Speakers", 5), "Spea…");
        assert_eq!(ellipsize("Speakers (USB audio)", 10), "Speakers…");
        assert_eq!(ellipsize("abc", 1), "…");
        assert_eq!(ellipsize("abc", 0), "");
    }

    #[test]
    fn characters_outside_the_basic_plane_are_never_split() {
        // Each emoji takes two UTF-16 units.
        let text = "🎧🎧🎧";
        assert_eq!(ellipsize(text, 6), text);
        assert_eq!(ellipsize(text, 5), "🎧🎧…");
        assert_eq!(ellipsize(text, 4), "🎧…");
        for limit in 0..8 {
            assert!(ellipsize(text, limit).encode_utf16().count() <= limit);
        }
    }

    #[test]
    fn accented_names_are_counted_per_character() {
        assert_eq!(ellipsize("Écouteurs", 9), "Écouteurs");
        assert_eq!(ellipsize("Écouteurs", 4), "Éco…");
    }
}
