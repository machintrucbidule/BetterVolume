//! OS language preferences (FR-121).

use windows::core::PWSTR;
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

use crate::wide::split_multi_string;

/// The user's Windows display languages as language tags (for example `fr-FR`), most
/// preferred first. Empty if Windows cannot tell.
pub fn os_display_languages() -> Vec<String> {
    let mut count = 0u32;
    let mut size = 0u32;
    // SAFETY: with no buffer, the call only writes the language count and the required
    // buffer size (in UTF-16 units) through the two valid pointers.
    let sized =
        unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut size) };
    if sized.is_err() || size == 0 {
        return Vec::new();
    }

    let mut buffer = vec![0u16; size as usize];
    // SAFETY: `buffer` is valid for `size` UTF-16 units, which is the capacity passed.
    let filled = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut size,
        )
    };
    if filled.is_err() {
        return Vec::new();
    }
    buffer.truncate(size as usize);
    split_multi_string(&buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reports_well_formed_language_tags() {
        let languages = os_display_languages();
        assert!(!languages.is_empty(), "Windows always has at least one display language");
        for tag in &languages {
            assert!(
                !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "unexpected language tag: {tag:?}"
            );
        }
    }
}
