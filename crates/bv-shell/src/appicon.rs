//! Application icons for the volume rows (FR-062): loaded from an executable or an icon
//! resource at the exact pixel size needed, converted to pixels by the design system's
//! backend, and kept in a small cache so later openings cost nothing.

use std::collections::HashMap;
use std::rc::Rc;

use bv_ui_kit::backend::{Graphics, Pixels};
use windows::core::PCWSTR;
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::UI::Shell::SHDefExtractIconW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

/// Most icons kept by default (a few dozen applications at most play at once).
const CACHE_LIMIT: usize = 64;

/// Icons already loaded, by source and size (`None`: the source has no usable icon).
pub(crate) struct IconCache {
    icons: HashMap<(String, u32), Option<Rc<Pixels>>>,
    limit: usize,
}

impl Default for IconCache {
    fn default() -> Self {
        Self::with_limit(CACHE_LIMIT)
    }
}

impl IconCache {
    /// A cache keeping at most `limit` icons (it starts over beyond).
    pub fn with_limit(limit: usize) -> Self {
        Self { icons: HashMap::new(), limit: limit.max(1) }
    }

    /// The icon of `source` at `pixels` × `pixels`, loaded on first request.
    pub fn get(&mut self, source: &str, pixels: u32, graphics: &Graphics) -> Option<Rc<Pixels>> {
        let key = (source.to_owned(), pixels);
        if let Some(icon) = self.icons.get(&key) {
            return icon.clone();
        }
        if self.icons.len() >= self.limit {
            self.icons.clear();
        }
        let icon = load(source, pixels, graphics).map(Rc::new);
        self.icons.insert(key, icon.clone());
        icon
    }
}

/// Splits `path,index` (the index may be negative: a resource identifier).
fn split_source(source: &str) -> (&str, i32) {
    let source = source.trim().trim_start_matches('@');
    match source.rsplit_once(',') {
        Some((path, index)) => match index.trim().parse::<i32>() {
            Ok(index) => (path.trim(), index),
            Err(_) => (source, 0),
        },
        None => (source, 0),
    }
}

/// Expands `%VARIABLE%` parts of a path.
fn expand(path: &str) -> Vec<u16> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated; the buffer length is passed.
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) };
    if written == 0 || written as usize > buffer.len() {
        return wide;
    }
    buffer.truncate(written as usize);
    buffer
}

fn load(source: &str, pixels: u32, graphics: &Graphics) -> Option<Pixels> {
    let (path, index) = split_source(source);
    if path.is_empty() {
        return None;
    }
    let path = expand(path);
    let mut icon = HICON::default();
    // SAFETY: `path` is NUL-terminated; one large icon requested at the exact size
    // (low word of the size argument), no small icon.
    let result = unsafe {
        SHDefExtractIconW(PCWSTR(path.as_ptr()), index, 0, Some(&mut icon), None, pixels & 0xFFFF)
    };
    if result.is_err() || icon.is_invalid() {
        return None;
    }
    let pixels = graphics.icon_pixels(icon.0 as isize).ok();
    // SAFETY: the icon was created above for us and is destroyed once.
    let _ = unsafe { DestroyIcon(icon) };
    pixels.filter(|p| p.data.as_chunks::<4>().0.iter().any(|px| px[3] != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_split_into_path_and_index() {
        assert_eq!(split_source(r"C:\Apps\player.exe"), (r"C:\Apps\player.exe", 0));
        assert_eq!(
            split_source(r"@%SystemRoot%\System32\x.dll,-203"),
            (r"%SystemRoot%\System32\x.dll", -203)
        );
        assert_eq!(split_source(r"C:\a,b\c.exe"), (r"C:\a,b\c.exe", 0));
        assert_eq!(split_source(" "), ("", 0));
    }

    #[test]
    fn an_executable_icon_is_loaded_at_the_asked_size() {
        let graphics = Graphics::new(bv_ui_kit::backend::DeviceKind::Software).expect("graphics");
        let mut cache = IconCache::default();
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let notepad = format!(r"{windir}\System32\notepad.exe");
        if !std::path::Path::new(&notepad).exists() {
            return; // a machine without Notepad: nothing to load
        }
        let icon = cache.get(&notepad, 30, &graphics).expect("notepad icon");
        assert_eq!((icon.width, icon.height), (30, 30));
        // Cached: the same pixels again.
        assert!(Rc::ptr_eq(&icon, &cache.get(&notepad, 30, &graphics).unwrap()));
        assert!(cache.get(r"C:\nothing\here.exe", 30, &graphics).is_none());
    }
}
