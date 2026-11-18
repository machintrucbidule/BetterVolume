//! Small readers of per-user registry values.

use windows::core::HSTRING;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

/// A text value under `HKEY_CURRENT_USER\<key>`, or `None`.
pub(crate) fn read_text(key: &str, name: &str) -> Option<String> {
    let mut buffer = vec![0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `size` bytes; the names are NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let units = (size as usize / 2).min(buffer.len());
    let text = &buffer[..units];
    let end = text.iter().position(|&c| c == 0).unwrap_or(text.len());
    Some(String::from_utf16_lossy(&text[..end]))
}

/// A 32-bit number under `HKEY_CURRENT_USER\<key>`, or `None`.
pub(crate) fn read_dword(key: &str, name: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: a 4-byte buffer; the names are NUL-terminated.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}
