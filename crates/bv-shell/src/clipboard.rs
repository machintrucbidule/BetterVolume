//! Text on the Windows clipboard (copy, cut and paste in the rename text box).

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Closes the clipboard when dropped.
struct Opened;

impl Opened {
    fn open(owner: HWND) -> Option<Opened> {
        // SAFETY: our own window; the clipboard is closed by `Drop`.
        unsafe { OpenClipboard(Some(owner)) }.ok().map(|()| Opened)
    }
}

impl Drop for Opened {
    fn drop(&mut self) {
        // SAFETY: the clipboard was opened by this thread.
        let _ = unsafe { CloseClipboard() };
    }
}

/// The text on the clipboard, if any.
pub(crate) fn text(owner: HWND) -> Option<String> {
    let _open = Opened::open(owner)?;
    // SAFETY: the clipboard is open; the handle belongs to it and is only read while
    // locked, up to its terminating NUL (bounded by the block size).
    unsafe {
        let handle = GetClipboardData(u32::from(CF_UNICODETEXT.0)).ok()?;
        let memory = HGLOBAL(handle.0);
        let size = windows::Win32::System::Memory::GlobalSize(memory) / 2;
        let data = GlobalLock(memory) as *const u16;
        if data.is_null() {
            return None;
        }
        let units = std::slice::from_raw_parts(data, size);
        let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..end]);
        let _ = GlobalUnlock(memory);
        Some(text)
    }
}

/// Puts `text` on the clipboard. A failure is logged (nothing else to do).
pub(crate) fn set_text(owner: HWND, text: &str) {
    if put(owner, text).is_none() {
        crate::log::error("clipboard not written");
    }
}

fn put(owner: HWND, text: &str) -> Option<()> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let _open = Opened::open(owner)?;
    // SAFETY: the clipboard is open; the block is allocated with the size written, and
    // handed to the clipboard (which then owns it) or freed on failure.
    unsafe {
        EmptyClipboard().ok()?;
        let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2).ok()?;
        let data = GlobalLock(memory) as *mut u16;
        if data.is_null() {
            let _ = GlobalFree(Some(memory));
            return None;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr(), data, units.len());
        let _ = GlobalUnlock(memory);
        if SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))).is_err() {
            let _ = GlobalFree(Some(memory));
            return None;
        }
    }
    Some(())
}
