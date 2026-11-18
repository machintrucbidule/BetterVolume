//! UTF-16 helpers for Win32 calls.

/// Copies `text` into a fixed-size, NUL-terminated UTF-16 buffer, truncating if needed
/// without ever cutting a surrogate pair in half.
pub(crate) fn copy_truncated(text: &str, buffer: &mut [u16]) {
    buffer.fill(0);
    let Some(capacity) = buffer.len().checked_sub(1) else { return };
    let mut len = 0;
    for ch in text.chars().take_while(|&ch| ch != '\0') {
        let mut units = [0u16; 2];
        let encoded = ch.encode_utf16(&mut units);
        if len + encoded.len() > capacity {
            break;
        }
        buffer[len..len + encoded.len()].copy_from_slice(encoded);
        len += encoded.len();
    }
}

/// Splits a double-NUL-terminated list of UTF-16 strings (a Win32 "multi-string").
pub(crate) fn split_multi_string(buffer: &[u16]) -> Vec<String> {
    buffer
        .split(|&unit| unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_fits() {
        let mut buffer = [0xFFFFu16; 8];
        copy_truncated("abc", &mut buffer);
        assert_eq!(buffer, [97, 98, 99, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn copy_truncates_and_keeps_the_terminator() {
        let mut buffer = [0xFFFFu16; 4];
        copy_truncated("abcdef", &mut buffer);
        assert_eq!(buffer, [97, 98, 99, 0]);
    }

    #[test]
    fn copy_never_splits_a_surrogate_pair() {
        // U+1F50A (speaker) needs two UTF-16 units; only one slot is left after "ab".
        let mut buffer = [0xFFFFu16; 4];
        copy_truncated("ab\u{1F50A}", &mut buffer);
        assert_eq!(buffer, [97, 98, 0, 0]);

        let mut buffer = [0xFFFFu16; 5];
        copy_truncated("ab\u{1F50A}", &mut buffer);
        assert_eq!(buffer, [97, 98, 0xD83D, 0xDD0A, 0]);
    }

    #[test]
    fn copy_handles_degenerate_buffers() {
        let mut empty: [u16; 0] = [];
        copy_truncated("abc", &mut empty);
        let mut one = [0xFFFFu16; 1];
        copy_truncated("abc", &mut one);
        assert_eq!(one, [0]);
    }

    #[test]
    fn multi_string_is_split() {
        let buffer: Vec<u16> = "fr-FR\0en-US\0\0".encode_utf16().collect();
        assert_eq!(split_multi_string(&buffer), vec!["fr-FR", "en-US"]);
        assert!(split_multi_string(&[0, 0]).is_empty());
        assert!(split_multi_string(&[]).is_empty());
        // Missing final terminators are tolerated.
        let buffer: Vec<u16> = "fr-FR".encode_utf16().collect();
        assert_eq!(split_multi_string(&buffer), vec!["fr-FR"]);
    }
}
