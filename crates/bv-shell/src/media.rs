//! Media keys (FR-111): the Home Assistant buttons press the keyboard's media keys, so the
//! program Windows routes them to (the current media session) reacts as to a real key.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK,
};

/// A media key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKey {
    /// Play/pause.
    PlayPause,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
}

impl MediaKey {
    fn virtual_key(self) -> VIRTUAL_KEY {
        match self {
            MediaKey::PlayPause => VK_MEDIA_PLAY_PAUSE,
            MediaKey::Next => VK_MEDIA_NEXT_TRACK,
            MediaKey::Previous => VK_MEDIA_PREV_TRACK,
        }
    }

    /// Presses and releases the key. Returns `false` if Windows refused (logged), for
    /// example while a program running as administrator has the foreground.
    pub fn press(self) -> bool {
        let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: self.virtual_key(), dwFlags: flags, ..Default::default() },
            },
        };
        let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
        // SAFETY: a valid array of two keyboard inputs and its element size.
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        let ok = sent as usize == inputs.len();
        if !ok {
            crate::log::error(&format!("media key {self:?} not delivered"));
        }
        ok
    }
}
