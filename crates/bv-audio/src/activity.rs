//! Which applications play on the current output.
//!
//! Windows keeps a session open, without sound, on every device an application once used
//! (a launcher that opened a speaker when it was the default keeps that session for
//! hours), and does not say where the application's next sound will go. The application
//! list therefore shows an application on the output it plays on now, or was last heard
//! on. To know where an application was last heard even while the flyout is closed, this
//! watch follows the *state* of the sessions of the current output (playing or not), and
//! nothing else: no process is opened, no name or icon is read. It only wakes the UI
//! thread when a session starts or stops playing, or appears.

use std::collections::HashMap;

use windows::core::Interface;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl2, IAudioSessionEvents, IAudioSessionManager2,
    IAudioSessionNotification, IMMDevice,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::com::{audio_error, take_string};
use crate::signals::{ActivityWatcher, SessionCreatedWatcher, Signal, Sink};
use crate::AudioError;

/// One session whose state is followed.
struct Followed {
    instance: String,
    control: IAudioSessionControl2,
    watcher: IAudioSessionEvents,
}

impl Drop for Followed {
    fn drop(&mut self) {
        // SAFETY: `watcher` was registered on `control` by `ActivityWatch::scan`.
        let _ = unsafe { self.control.UnregisterAudioSessionNotification(&self.watcher) };
    }
}

/// The playing state of the sessions of one output device.
pub(crate) struct ActivityWatch {
    /// The device whose sessions these are.
    pub(crate) device_id: String,
    /// Generation of this watch: signals of an older watch are ignored.
    pub(crate) generation: u64,
    sink: Sink,
    manager: IAudioSessionManager2,
    watcher: IAudioSessionNotification,
    followed: Vec<Followed>,
}

impl ActivityWatch {
    /// Starts following the sessions of `device`.
    pub(crate) fn start(
        device: &IMMDevice,
        device_id: &str,
        generation: u64,
        sink: Sink,
    ) -> Result<Self, AudioError> {
        // SAFETY: activating a documented interface on a valid device.
        let manager: IAudioSessionManager2 =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| audio_error(&e))?;
        let watcher = SessionCreatedWatcher::create(sink.clone(), Signal::Activity { generation });
        // SAFETY: `watcher` stays alive in the returned value until `drop` unregisters it.
        unsafe { manager.RegisterSessionNotification(&watcher) }.map_err(|e| audio_error(&e))?;
        Ok(Self {
            device_id: device_id.to_owned(),
            generation,
            sink,
            manager,
            watcher,
            followed: Vec::new(),
        })
    }

    /// Lists the sessions again (Windows only reports new sessions once a list has been
    /// requested), follows the new ones, forgets the gone ones, and returns the process
    /// identifiers of the sessions playing now, then of every session listed.
    pub(crate) fn scan(&mut self) -> (Vec<u32>, Vec<u32>) {
        let mut playing = Vec::new();
        let mut present = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        // SAFETY: plain call on a valid manager.
        let Ok(list) = (unsafe { self.manager.GetSessionEnumerator() }) else {
            self.followed.clear();
            return (playing, present);
        };
        // SAFETY: plain call on a valid enumerator.
        let count = unsafe { list.GetCount() }.unwrap_or(0);
        for index in 0..count {
            // SAFETY: `index` is within the count Windows just gave.
            let Ok(session) = (unsafe { list.GetSession(index) }) else { continue };
            let Ok(control) = session.cast::<IAudioSessionControl2>() else { continue };
            // SAFETY: plain calls on a valid session; the returned string is ours.
            let (state, pid, instance) = unsafe {
                (
                    control.GetState(),
                    control.GetProcessId().unwrap_or(0),
                    control.GetSessionInstanceIdentifier().map(take_string),
                )
            };
            let Ok(instance) = instance else { continue };
            if pid != 0 {
                present.push(pid);
                if state.is_ok_and(|s| s == AudioSessionStateActive) {
                    playing.push(pid);
                }
            }
            if !self.followed.iter().any(|f| f.instance == instance) {
                let watcher = ActivityWatcher::create(self.sink.clone(), self.generation);
                // SAFETY: `watcher` stays alive in `Followed` until its `drop` unregisters
                // it.
                if unsafe { control.RegisterAudioSessionNotification(&watcher) }.is_ok() {
                    self.followed.push(Followed { instance: instance.clone(), control, watcher });
                }
            }
            seen.push(instance);
        }
        self.followed.retain(|f| seen.contains(&f.instance));
        (playing, present)
    }
}

impl Drop for ActivityWatch {
    fn drop(&mut self) {
        // Sessions first (their own notifications), then the "created" notification.
        self.followed.clear();
        // SAFETY: `watcher` was registered by `start`.
        let _ = unsafe { self.manager.UnregisterSessionNotification(&self.watcher) };
    }
}

/// Most processes remembered with the output they were last heard on.
const HEARD_LIMIT: usize = 512;

/// Records what a look at the sessions of `device` showed: the processes in
/// `playing` were heard there last; a process remembered on `device` that no longer has
/// any session there (in `present`) has ended or left that output, and is forgotten, so
/// a later process given the same identifier by Windows inherits nothing.
pub(crate) fn hear(
    heard: &mut HashMap<u32, String>,
    device: &str,
    playing: &[u32],
    present: &[u32],
) {
    heard.retain(|process, at| at != device || present.contains(process));
    if heard.len() >= HEARD_LIMIT {
        // Processes heard on other outputs accumulate slowly; starting over only forgets
        // where idle applications were last heard.
        heard.clear();
    }
    for process in playing {
        if heard.get(process).map(String::as_str) != Some(device) {
            heard.insert(*process, device.to_owned());
        }
    }
}

/// The current output moved from `from` to `to`: an application last heard on
/// `from` that already has a session on `to` (in `present`) is taken as following the
/// default, as most applications do a moment later, and keeps its row on the new output
/// instead of disappearing until it is heard there.
pub(crate) fn follow_default(
    heard: &mut HashMap<u32, String>,
    from: &str,
    to: &str,
    present: &[u32],
) {
    for (process, at) in heard.iter_mut() {
        if at == from && present.contains(process) {
            to.clone_into(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applications_heard_on_the_previous_output_follow_the_switch() {
        let mut heard = HashMap::new();
        // A browser (10) and a chat program pinned to the headset (40) were heard on it;
        // a game (50) on the TV.
        hear(&mut heard, "headset", &[10, 40], &[10, 40]);
        hear(&mut heard, "tv", &[50], &[50]);
        // Switch to the TV: the browser has a session there, the chat program has none.
        follow_default(&mut heard, "headset", "tv", &[10, 50]);
        assert_eq!(at(&heard, 10), Some("tv"), "keeps its row on the new output");
        assert_eq!(at(&heard, 40), Some("headset"), "stays where it plays");
        assert_eq!(at(&heard, 50), Some("tv"));
    }

    fn at(heard: &HashMap<u32, String>, process: u32) -> Option<&str> {
        heard.get(&process).map(String::as_str)
    }

    #[test]
    fn an_application_belongs_to_the_output_it_was_last_heard_on() {
        let mut heard = HashMap::new();
        // A browser plays on the headset; a launcher sits idle there.
        hear(&mut heard, "headset", &[10], &[10, 20]);
        assert_eq!(at(&heard, 10), Some("headset"));
        assert_eq!(at(&heard, 20), None, "never heard: no row anywhere");
        // Paused: still the headset's.
        hear(&mut heard, "headset", &[], &[10, 20]);
        assert_eq!(at(&heard, 10), Some("headset"));
        // It plays on the TV: it leaves the headset's list.
        hear(&mut heard, "tv", &[10], &[10]);
        assert_eq!(at(&heard, 10), Some("tv"));
    }

    #[test]
    fn a_process_gone_from_its_output_is_forgotten() {
        let mut heard = HashMap::new();
        hear(&mut heard, "headset", &[10], &[10]);
        hear(&mut heard, "tv", &[30], &[30]);
        // Process 10 ended: no session left on the headset. Process 30 is untouched
        // (remembered on another output).
        hear(&mut heard, "headset", &[], &[]);
        assert_eq!(at(&heard, 10), None);
        assert_eq!(at(&heard, 30), Some("tv"));
        // Windows reuses identifier 10 for an idle launcher: it inherits nothing.
        hear(&mut heard, "headset", &[], &[10]);
        assert_eq!(at(&heard, 10), None);
    }

    #[test]
    fn the_memory_stays_bounded() {
        let mut heard = HashMap::new();
        for process in 0..HEARD_LIMIT as u32 {
            hear(&mut heard, &format!("device {process}"), &[process], &[process]);
        }
        assert_eq!(heard.len(), HEARD_LIMIT);
        hear(&mut heard, "headset", &[9999], &[9999]);
        assert_eq!(heard.len(), 1);
        assert_eq!(at(&heard, 9999), Some("headset"));
    }
}
