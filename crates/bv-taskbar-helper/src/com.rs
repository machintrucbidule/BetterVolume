//! Small COM utilities shared by the TAP: an outstanding-object counter (so
//! `DllCanUnloadNow` can answer truthfully) and a reader for borrowed wide strings.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Append a diagnostic line to `%TEMP%\bv-taskbar-helper.log`. Used only while proving the
/// feasibility of the helper on the dev machine; it is compiled in behind the `trace`
/// feature and left out of the shipped library.
#[cfg(feature = "trace")]
pub fn trace(line: &str) {
    use std::io::Write;
    // A fixed, always-writable path, independent of the taskbar process environment.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("C:\\Users\\Public\\bv-taskbar-helper.log")
    {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(not(feature = "trace"))]
#[inline]
pub fn trace(_line: &str) {}

static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Number of live COM objects this library has handed out. The host unloads us only when
/// this is zero.
pub fn outstanding() -> usize {
    OUTSTANDING.load(Ordering::SeqCst)
}

/// Record a new live object.
pub fn add_object() {
    OUTSTANDING.fetch_add(1, Ordering::SeqCst);
}

/// Record a freed object.
pub fn release_object() {
    OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
}

/// Read a NUL-terminated wide string that the caller still owns (callback struct fields
/// and `BSTR`s we must not free). Returns an empty string for a null pointer.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string that stays valid for the
/// duration of the call. The content is copied; the pointer is never freed here.
pub unsafe fn borrowed_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: the caller guarantees a NUL-terminated buffer; we only read it.
    unsafe {
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(p, len))
    }
}
