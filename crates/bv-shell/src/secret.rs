//! Secrets protected for the current Windows user (FR-092): the Home Assistant
//! password is stored encrypted with the Data Protection API, so the settings file is
//! useless on another account or another machine.

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Copies the output blob of a DPAPI call and frees it.
fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI returned `cbData` readable bytes at `pbData`, allocated with
    // LocalAlloc; they are copied before being freed exactly once.
    unsafe {
        let bytes = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(blob.pbData.cast())));
        bytes
    }
}

/// Encrypts `secret` for the current Windows user. `None` if Windows refused (logged).
pub fn protect(secret: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(secret.len()).ok()?,
        pbData: secret.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points to `secret`, alive and unmodified during the call (DPAPI only
    // reads it); `output` is filled by the call and freed by `take`.
    let result = unsafe {
        CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
    };
    match result {
        Ok(()) => Some(take(output)),
        Err(error) => {
            crate::log::error(&format!("cannot protect a secret: 0x{:08X}", error.code().0 as u32));
            None
        }
    }
}

/// Decrypts what [`protect`] produced, for the same Windows user. `None` if it cannot be
/// read (another user or machine, damaged data; logged).
pub fn unprotect(protected: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(protected.len()).ok()?,
        pbData: protected.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`.
    let result = unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
    };
    match result {
        Ok(()) => Some(take(output)),
        Err(error) => {
            crate::log::error(&format!(
                "cannot read a protected secret: 0x{:08X}",
                error.code().0 as u32
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_reads_back_and_is_not_stored_in_clear() {
        let secret = "mot de passe é ✓".as_bytes();
        let protected = protect(secret).expect("protected");
        assert!(protected.len() > secret.len());
        assert!(!protected.windows(secret.len()).any(|w| w == secret));
        assert_eq!(unprotect(&protected).as_deref(), Some(secret));
        assert_eq!(protect(b"").and_then(|p| unprotect(&p)).as_deref(), Some(&b""[..]));
        let mut damaged = protected;
        let last = damaged.len() - 1;
        damaged[last] ^= 0xFF;
        assert_eq!(unprotect(&damaged), None);
    }
}
