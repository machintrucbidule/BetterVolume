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
use std::time::{Duration, Instant};

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

/// How long after a move of the current output applications arriving on the new output
/// are taken as following the default. Windows moves the open streams of the applications
/// within moments, a device just plugged in included; an application that only comes
/// later does so by playing, and is heard there anyway.
pub(crate) const FOLLOW_WINDOW: Duration = Duration::from_secs(30);

/// A recent move of the current output.
struct Move {
    /// The outputs left, the last one included (several when the output moved again
    /// before the applications arrived).
    from: Vec<String>,
    to: String,
    at: Instant,
}

/// Where each application was last heard, and the last move of the current output.
#[derive(Default)]
pub(crate) struct Heard {
    /// Process identifier to the output device it was last heard on.
    at: HashMap<u32, String>,
    moved: Option<Move>,
}

impl Heard {
    /// The current output became `to` at `now`; `from` is the one it replaced, if known
    /// (`None` after a restart of the audio service: nothing follows).
    pub(crate) fn moved(&mut self, from: Option<&str>, to: &str, now: Instant) {
        let Some(from) = from else {
            self.moved = None;
            return;
        };
        // Moved again before the applications arrived: the outputs left before still give
        // theirs.
        let mut left: Vec<String> = match self.moved.take() {
            Some(previous) if previous.to == from && within(previous.at, now) => previous.from,
            _ => Vec::new(),
        };
        left.push(from.to_owned());
        left.retain(|device| device != to);
        left.dedup();
        self.moved = Some(Move { from: left, to: to.to_owned(), at: now });
    }

    /// Records a look at the sessions of `device` (see [`hear`]). Shortly after the
    /// default moved to `device` ([`FOLLOW_WINDOW`]), an application last heard on the
    /// output left that has a session here follows the default (see [`follow_default`]):
    /// also when its session appears after the move, as on a device just plugged in, where
    /// Windows moves the applications a moment after making it the default.
    pub(crate) fn look(&mut self, device: &str, playing: &[u32], present: &[u32], now: Instant) {
        if self.moved.as_ref().is_some_and(|m| !within(m.at, now)) {
            self.moved = None;
        }
        if let Some(moved) = self.moved.as_ref().filter(|m| m.to == device) {
            for from in &moved.from {
                follow_default(&mut self.at, from, device, present);
            }
        }
        hear(&mut self.at, device, playing, present);
    }

    /// The output `process` was last heard on.
    pub(crate) fn get(&self, process: u32) -> Option<&str> {
        self.at.get(&process).map(String::as_str)
    }
}

/// Whether `now` is within [`FOLLOW_WINDOW`] of the move made at `at`.
fn within(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= FOLLOW_WINDOW
}

/// Most processes remembered with the output they were last heard on.
const HEARD_LIMIT: usize = 512;

/// Records what a look at the sessions of `device` showed: the processes in
/// `playing` were heard there last; a process remembered on `device` that no longer has
/// any session there (in `present`) has ended or left that output, and is forgotten, so
/// a later process given the same identifier by Windows inherits nothing.
fn hear(heard: &mut HashMap<u32, String>, device: &str, playing: &[u32], present: &[u32]) {
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
fn follow_default(heard: &mut HashMap<u32, String>, from: &str, to: &str, present: &[u32]) {
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

    /// `seconds` after `start`.
    fn after(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn applications_that_reach_a_just_plugged_output_late_follow_the_switch() {
        let t = Instant::now();
        let mut heard = Heard::default();
        // A browser (10) and a paused player (20) were heard on the speakers; a chat
        // program (40) on the TV.
        heard.look("speakers", &[10, 20], &[10, 20], t);
        heard.look("speakers", &[10], &[10, 20], t);
        heard.look("tv", &[40], &[40], t);
        // A new headset is plugged in and becomes the default: nothing has a session on
        // it yet.
        heard.moved(Some("speakers"), "headset", t);
        heard.look("headset", &[], &[], t);
        assert_eq!(heard.get(10), Some("speakers"));
        // A moment later Windows moves the applications there, silent: they keep their
        // rows on the new output. The chat program was not on the previous output.
        heard.look("headset", &[], &[10, 20, 40], after(t, 2));
        assert_eq!(heard.get(10), Some("headset"));
        assert_eq!(heard.get(20), Some("headset"));
        assert_eq!(heard.get(40), Some("tv"), "was not on the output just left");
    }

    #[test]
    fn only_the_output_moved_to_takes_the_applications() {
        let t = Instant::now();
        let mut heard = Heard::default();
        heard.look("speakers", &[10], &[10], t);
        heard.moved(Some("speakers"), "headset", t);
        // A look at another output (a stale look) takes nothing from the speakers.
        heard.look("tv", &[], &[10], t);
        assert_eq!(heard.get(10), Some("speakers"));
        heard.look("headset", &[], &[10], t);
        assert_eq!(heard.get(10), Some("headset"));
    }

    #[test]
    fn long_after_the_switch_an_application_stays_where_it_was_heard() {
        let t = Instant::now();
        let mut heard = Heard::default();
        // A player was heard on the speakers, then the headset became the default; the
        // player stays on the speakers (set there in the Windows settings).
        heard.look("speakers", &[20], &[20], t);
        heard.moved(Some("speakers"), "headset", t);
        heard.look("headset", &[], &[], t);
        // Much later it opens a silent session on the headset: not heard there.
        heard.look("headset", &[], &[20], after(t, FOLLOW_WINDOW.as_secs() + 1));
        assert_eq!(heard.get(20), Some("speakers"));
        // Heard playing there: now it is the headset's.
        heard.look("headset", &[20], &[20], after(t, FOLLOW_WINDOW.as_secs() + 2));
        assert_eq!(heard.get(20), Some("headset"));
    }

    #[test]
    fn quick_switches_carry_the_applications_along() {
        let t = Instant::now();
        let mut heard = Heard::default();
        heard.look("speakers", &[10], &[10], t);
        // Speakers to a just-plugged headset (nothing there yet), then at once to the TV.
        heard.moved(Some("speakers"), "headset", t);
        heard.look("headset", &[], &[], t);
        heard.moved(Some("headset"), "tv", after(t, 1));
        heard.look("tv", &[], &[10], after(t, 2));
        assert_eq!(heard.get(10), Some("tv"));
        // And back to the speakers: the browser was never moved away from them on Windows'
        // side either, it is found there again.
        heard.moved(Some("tv"), "speakers", after(t, 3));
        heard.look("speakers", &[], &[10], after(t, 3));
        assert_eq!(heard.get(10), Some("speakers"));
    }

    #[test]
    fn nothing_follows_when_the_output_left_is_unknown() {
        let t = Instant::now();
        let mut heard = Heard::default();
        heard.look("speakers", &[10], &[10], t);
        heard.moved(Some("speakers"), "headset", t);
        // The first output known (start): nothing to follow, the earlier move is dropped.
        heard.moved(None, "tv", t);
        heard.look("headset", &[], &[10], t);
        heard.look("tv", &[], &[10], t);
        assert_eq!(heard.get(10), Some("speakers"));
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
