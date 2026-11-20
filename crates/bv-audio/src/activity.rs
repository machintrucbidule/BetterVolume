//! Which applications belong to the current output.
//!
//! Windows keeps a session open, without sound, on every device an application once used
//! (a launcher that opened a speaker when it was the default keeps that session for
//! hours), and does not say where the application's next sound will go. Some applications
//! (browsers built on Chromium) even close their stream when paused, so they have no
//! session at all on an output chosen while they were paused. The application list
//! therefore shows an application from where it was last *heard* playing (FR-061):
//!
//! - heard on the output that was the current one: it plays wherever the current output
//!   is, so it is listed on the current output, with or without a session there;
//! - heard on another output (set there in the Windows settings, or by the application
//!   itself): it is listed on that output only.
//!
//! To know this even while the flyout is closed, a watch follows the *state* of the
//! sessions of every output (playing or not), and nothing else: no process is opened, no
//! name or icon is read. It only wakes the UI thread when a session starts or stops
//! playing, or appears.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, IAudioSessionControl2, IAudioSessionEvents, IAudioSessionManager2,
    IAudioSessionNotification, IMMDevice, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::com::{audio_error, take_string};
use crate::sessions::Names;
use crate::signals::{ActivityWatcher, SessionCreatedWatcher, Signal, Sink};
use crate::{AudioError, Stream};

/// One session whose state is followed.
struct Followed {
    instance: String,
    process: u32,
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
                    self.followed.push(Followed {
                        instance: instance.clone(),
                        process: pid,
                        control,
                        watcher,
                    });
                }
            }
            seen.push(instance);
        }
        self.followed.retain(|f| seen.contains(&f.instance));
        (playing, present)
    }

    /// The volume of the sessions of `process` here (the loudest), and whether they are
    /// all muted; `None` if it has no session here.
    pub(crate) fn level(&self, process: u32) -> Option<(f32, bool)> {
        let mut level: Option<(f32, bool)> = None;
        for followed in self.followed.iter().filter(|f| f.process == process) {
            let Ok(volume) = followed.control.cast::<ISimpleAudioVolume>() else { continue };
            // SAFETY: plain calls on a valid session.
            let read = unsafe { (volume.GetMasterVolume(), volume.GetMute()) };
            let (Ok(value), Ok(muted)) = read else { continue };
            let muted = muted.as_bool();
            level = Some(match level {
                Some((loudest, all_muted)) => (loudest.max(value), all_muted && muted),
                None => (value, muted),
            });
        }
        level
    }

    /// The processes that have a session here, as last listed.
    pub(crate) fn processes(&self) -> Vec<u32> {
        let mut processes: Vec<u32> = self.followed.iter().map(|f| f.process).collect();
        processes.sort_unstable();
        processes.dedup();
        processes
    }

    /// What the application list needs about the sessions of `process` here (described
    /// from the first one, at the level of [`ActivityWatch::level`]), if it has any.
    pub(crate) fn describe(&self, process: u32, names: &Names) -> Option<Stream> {
        let followed = self.followed.iter().find(|f| f.process == process)?;
        let volume = followed.control.cast::<ISimpleAudioVolume>().ok()?;
        let mut stream = crate::sessions::describe(&followed.control, &volume, names);
        if let Some((level, muted)) = self.level(process) {
            stream.volume = level;
            stream.muted = muted;
        }
        Some(stream)
    }

    /// Gives the sessions of the processes in `processes` the volume and mute state
    /// asked for (what is `None` is left as it is). Returns whether a session was found.
    pub(crate) fn apply(&self, processes: &[u32], wanted: Wanted) -> bool {
        let mut found = false;
        for followed in self.followed.iter().filter(|f| processes.contains(&f.process)) {
            let Ok(volume) = followed.control.cast::<ISimpleAudioVolume>() else { continue };
            found = true;
            // SAFETY: plain calls on a valid session; a null event context is allowed.
            unsafe {
                if let Some(value) = wanted.volume {
                    let _ = volume.SetMasterVolume(value, std::ptr::null());
                }
                if let Some(muted) = wanted.muted {
                    let _ = volume.SetMute(muted, std::ptr::null());
                }
            }
        }
        found
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

/// A volume or mute change asked for an application that has no session on the current
/// output yet (it was paused when the output changed): given to its sessions there as
/// soon as they appear.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Wanted {
    pub(crate) volume: Option<f32>,
    pub(crate) muted: Option<bool>,
}

/// How long after the current output changed sound still heard on the output left counts
/// as sound of the current output: Windows moves the streams of the applications that
/// follow the default within moments (a device just plugged in may take a little longer),
/// and an application must not be taken as set to the output left meanwhile.
pub(crate) const FOLLOW_WINDOW: Duration = Duration::from_secs(30);

/// Where one process was last heard.
#[derive(Debug)]
struct Record {
    /// The output it was last heard playing on.
    device: String,
    /// That output was the current one then (or had just stopped being it): the process
    /// plays wherever the current output is.
    follows: bool,
    /// The end of the process is watched (the engine tells [`Heard::ended`]); otherwise
    /// it is forgotten once it no longer has a session on `device`.
    end_watched: bool,
    /// Its volume and mute state on `device`, as last read (shown while it has no
    /// session on the current output).
    level: Option<(f32, bool)>,
}

/// Where each application was last heard, and which output is the current one.
#[derive(Default)]
pub(crate) struct Heard {
    at: HashMap<u32, Record>,
    current: Option<String>,
    /// Outputs that stopped being the current one less than [`FOLLOW_WINDOW`] ago.
    left: Vec<(String, Instant)>,
}

/// Most processes remembered with the output they were last heard on.
const HEARD_LIMIT: usize = 512;

impl Heard {
    /// The current output is now `to` (from `now`). Returns whether it changed.
    pub(crate) fn output(&mut self, to: &str, now: Instant) -> bool {
        if self.current.as_deref() == Some(to) {
            return false;
        }
        if let Some(previous) = self.current.take() {
            self.left.push((previous, now));
        }
        self.left.retain(|(device, at)| device != to && within(*at, now));
        self.current = Some(to.to_owned());
        true
    }

    /// Forgets the outputs left more than [`FOLLOW_WINDOW`] ago. Returns whether there
    /// were any: sound still heard on them is no longer taken as following the current
    /// output, so they deserve a new look.
    pub(crate) fn settle(&mut self, now: Instant) -> bool {
        let before = self.left.len();
        self.left.retain(|(_, at)| within(*at, now));
        self.left.len() != before
    }

    /// Records a look at the sessions of `device`: the processes in `playing` were heard
    /// there last. A process whose end is not watched and that no longer has any session
    /// there (in `present`) has ended or left that output, and is forgotten, so a later
    /// process given the same identifier by Windows inherits nothing. Returns the
    /// processes remembered for the first time (the engine then watches their end).
    pub(crate) fn look(
        &mut self,
        device: &str,
        playing: &[u32],
        present: &[u32],
        now: Instant,
    ) -> Vec<u32> {
        self.at
            .retain(|process, r| r.end_watched || r.device != device || present.contains(process));
        if self.at.len() >= HEARD_LIMIT {
            // Processes accumulate slowly; starting over only forgets where idle
            // applications were last heard.
            self.at.clear();
        }
        // The outputs left are only pruned by `settle`, which tells the engine to look
        // again once one has expired.
        let follows = self.current.as_deref() == Some(device)
            || self.left.iter().any(|(left, at)| left == device && within(*at, now));
        let mut new = Vec::new();
        for &process in playing.iter().filter(|p| **p != 0) {
            match self.at.get_mut(&process) {
                Some(record) => {
                    if record.device != device || record.follows != follows {
                        device.clone_into(&mut record.device);
                        record.follows = follows;
                        record.level = None;
                    }
                }
                None => {
                    let record = Record {
                        device: device.to_owned(),
                        follows,
                        end_watched: false,
                        level: None,
                    };
                    self.at.insert(process, record);
                    new.push(process);
                }
            }
        }
        new
    }

    /// The end of `process` is watched from now on.
    pub(crate) fn end_watched(&mut self, process: u32) {
        if let Some(record) = self.at.get_mut(&process) {
            record.end_watched = true;
        }
    }

    /// `process` has ended.
    pub(crate) fn ended(&mut self, process: u32) {
        self.at.remove(&process);
    }

    /// Whether `process` is remembered.
    pub(crate) fn knows(&self, process: u32) -> bool {
        self.at.contains_key(&process)
    }

    /// Whether `process` belongs to the list of `device`: it follows the current output
    /// and `device` is the current output, or it is set to another output and `device` is
    /// that output. An application is never listed on two outputs.
    pub(crate) fn listed_on(&self, process: u32, device: &str) -> bool {
        self.at.get(&process).is_some_and(|r| {
            if r.follows {
                self.current.as_deref() == Some(device)
            } else {
                r.device == device
            }
        })
    }

    /// The processes listed on `device` (see [`Heard::listed_on`]), in no particular order.
    pub(crate) fn listed(&self, device: &str) -> Vec<u32> {
        self.at.keys().copied().filter(|p| self.listed_on(*p, device)).collect()
    }

    /// The output `process` was last heard on.
    pub(crate) fn device(&self, process: u32) -> Option<&str> {
        self.at.get(&process).map(|r| r.device.as_str())
    }

    /// The volume and mute state of `process` on the output it was last heard on, as last
    /// read.
    pub(crate) fn level(&self, process: u32) -> Option<(f32, bool)> {
        self.at.get(&process).and_then(|r| r.level)
    }

    /// Remembers what was just read of the volume of `process` on its output.
    pub(crate) fn set_level(&mut self, process: u32, level: (f32, bool)) {
        if let Some(record) = self.at.get_mut(&process) {
            record.level = Some(level);
        }
    }
}

/// Whether `now` is within [`FOLLOW_WINDOW`] of `at`.
fn within(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) <= FOLLOW_WINDOW
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` after `start`.
    fn after(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    /// A memory where `current` is the current output since `t`.
    fn on(current: &str, t: Instant) -> Heard {
        let mut heard = Heard::default();
        heard.output(current, t);
        heard
    }

    #[test]
    fn a_paused_application_follows_the_output_without_a_session_there() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        // A browser (10) played on the headset, then was paused: its stream is closed.
        heard.look("headset", &[10], &[10], t);
        heard.look("headset", &[], &[10], after(t, 1));
        // The bedroom speaker becomes the current output: the browser has no session
        // there, yet its sound will go there.
        heard.output("bedroom", after(t, 60));
        heard.look("bedroom", &[], &[], after(t, 60));
        assert!(heard.listed_on(10, "bedroom"));
        assert!(!heard.listed_on(10, "headset"), "listed once, on the current output");
        assert_eq!(heard.listed("bedroom"), vec![10]);
        // And on every later output.
        heard.output("tv", after(t, 120));
        assert!(heard.listed_on(10, "tv"));
        assert!(!heard.listed_on(10, "bedroom"));
    }

    #[test]
    fn an_application_set_to_another_output_stays_there() {
        let t = Instant::now();
        let mut heard = on("speakers", t);
        // A chat program plays on the headset, which is not the current output (set there
        // in the Windows settings).
        heard.look("headset", &[40], &[40], t);
        assert!(heard.listed_on(40, "headset"));
        assert!(!heard.listed_on(40, "speakers"));
        heard.output("tv", after(t, 60));
        assert!(!heard.listed_on(40, "tv"));
        // Shown when the headset is chosen.
        heard.output("headset", after(t, 120));
        assert!(heard.listed_on(40, "headset"));
    }

    #[test]
    fn sound_still_heard_on_the_output_just_left_follows() {
        let t = Instant::now();
        let mut heard = on("speakers", t);
        heard.look("speakers", &[10], &[10], t);
        heard.output("headset", after(t, 10));
        // Windows has not moved the browser yet: still heard on the speakers.
        heard.look("speakers", &[10], &[10], after(t, 11));
        assert!(heard.listed_on(10, "headset"), "follows the current output");
        // Much later, an application heard on the speakers is set there.
        heard.look("speakers", &[20], &[10, 20], after(t, 10 + FOLLOW_WINDOW.as_secs() + 1));
        assert!(!heard.listed_on(20, "headset"));
        assert!(heard.listed_on(20, "speakers"));
    }

    #[test]
    fn a_following_application_heard_on_another_output_stops_following() {
        let t = Instant::now();
        let mut heard = on("speakers", t);
        heard.look("speakers", &[20], &[20], t);
        heard.output("headset", after(t, 1));
        assert!(heard.listed_on(20, "headset"));
        // It plays on the speakers long after the switch: it is set to them.
        heard.look("speakers", &[20], &[20], after(t, FOLLOW_WINDOW.as_secs() + 2));
        assert!(!heard.listed_on(20, "headset"));
        assert!(heard.listed_on(20, "speakers"));
        // Heard on the current output again: it follows again.
        heard.look("headset", &[20], &[20], after(t, FOLLOW_WINDOW.as_secs() + 3));
        heard.output("tv", after(t, FOLLOW_WINDOW.as_secs() + 4));
        assert!(heard.listed_on(20, "tv"));
    }

    #[test]
    fn the_end_of_the_moment_after_a_switch_asks_for_a_new_look() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        heard.look("headset", &[40], &[40], t);
        assert!(heard.output("speakers", after(t, 1)), "a change");
        assert!(!heard.output("speakers", after(t, 2)), "no change");
        assert!(!heard.settle(after(t, 10)), "the headset was left a moment ago");
        // A chat program set to the headset keeps playing there: taken as following
        // while the moment lasts.
        heard.look("headset", &[40], &[40], after(t, 10));
        assert!(heard.listed_on(40, "speakers"));
        // Once it is over, the engine looks again: it is set to the headset.
        let later = after(t, FOLLOW_WINDOW.as_secs() + 2);
        assert!(heard.settle(later));
        assert!(!heard.settle(later), "said once");
        heard.look("headset", &[40], &[40], later);
        assert!(!heard.listed_on(40, "speakers"));
        assert!(heard.listed_on(40, "headset"));
    }

    #[test]
    fn quick_switches_carry_the_applications_along() {
        let t = Instant::now();
        let mut heard = on("speakers", t);
        heard.look("speakers", &[10], &[10], t);
        // Its end is watched (as the engine does): kept although Windows dropped its
        // session on the speakers.
        heard.end_watched(10);
        for (second, output) in [(1, "headset"), (2, "tv"), (3, "speakers")] {
            heard.output(output, after(t, second));
            heard.look(output, &[], &[], after(t, second));
            assert!(heard.listed_on(10, output));
        }
    }

    #[test]
    fn a_process_never_heard_is_listed_nowhere() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        // A launcher keeps an idle session.
        assert!(heard.look("headset", &[], &[30], t).is_empty());
        assert!(!heard.listed_on(30, "headset"));
        assert!(!heard.knows(30));
    }

    #[test]
    fn new_processes_are_reported_once() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        assert_eq!(heard.look("headset", &[10, 0], &[10], t), vec![10], "never System sounds");
        assert!(heard.look("headset", &[10], &[10], t).is_empty());
        assert_eq!(heard.look("tv", &[10, 20], &[10, 20], t), vec![20]);
    }

    #[test]
    fn a_process_gone_from_its_output_is_forgotten_unless_its_end_is_watched() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        heard.look("headset", &[10, 20], &[10, 20], t);
        heard.end_watched(20);
        heard.look("tv", &[30], &[30], t);
        // No session left on the headset (unplugged, or ended).
        heard.look("headset", &[], &[], after(t, 1));
        assert!(!heard.knows(10), "its end is not watched: forgotten");
        assert!(heard.knows(20), "its end is watched: kept until it ends");
        assert!(heard.knows(30), "remembered on another output");
        heard.ended(20);
        assert!(!heard.knows(20));
        // Windows reuses identifier 10 for an idle launcher: it inherits nothing.
        heard.look("headset", &[], &[10], after(t, 2));
        assert!(!heard.listed_on(10, "headset"));
    }

    #[test]
    fn the_level_last_read_is_kept_until_the_process_is_heard_elsewhere() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        heard.look("headset", &[10], &[10], t);
        heard.set_level(10, (0.4, false));
        heard.look("headset", &[10], &[10], t);
        assert_eq!(heard.level(10), Some((0.4, false)));
        assert_eq!(heard.device(10), Some("headset"));
        heard.output("tv", after(t, 60));
        heard.look("tv", &[10], &[10], after(t, 60));
        assert_eq!(heard.level(10), None, "another output, another level");
        assert_eq!(heard.device(10), Some("tv"));
    }

    #[test]
    fn the_memory_stays_bounded() {
        let t = Instant::now();
        let mut heard = on("headset", t);
        for process in 1..=HEARD_LIMIT as u32 {
            heard.look(&format!("device {process}"), &[process], &[process], t);
        }
        assert_eq!(heard.at.len(), HEARD_LIMIT);
        heard.look("headset", &[9999], &[9999], t);
        assert_eq!(heard.at.len(), 1);
        assert!(heard.listed_on(9999, "headset"));
    }
}
