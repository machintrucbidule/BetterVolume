//! Application audio sessions of the current output (FR-061 to FR-065).

use std::collections::HashMap;

use windows::core::Interface;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionEvents,
    IAudioSessionManager2, IAudioSessionNotification, IMMDevice, ISimpleAudioVolume,
};
use windows::Win32::System::Com::CLSCTX_ALL;

use crate::com::{audio_error, take_string};
use crate::exit::{ExitWatch, Watch};
use crate::process;
use crate::signals::{SessionCreatedWatcher, SessionWatcher, Signal, Sink};
use crate::{AppKey, AudioError, Stream};

/// One session being followed.
struct Tracked {
    /// Engine-wide number carried by this session's signals.
    id: u64,
    /// Windows identifier of this session instance.
    instance: String,
    control: IAudioSessionControl2,
    volume: ISimpleAudioVolume,
    watcher: IAudioSessionEvents,
    /// Wait on the session's process (FR-063); `None` for System sounds and for
    /// processes that cannot be opened.
    exit: Option<ExitWatch>,
    stream: Stream,
    /// The session plays now (read with its description).
    playing: bool,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        // SAFETY: `watcher` was registered on `control` by `SessionWatch::track`.
        let _ = unsafe { self.control.UnregisterAudioSessionNotification(&self.watcher) };
    }
}

/// Friendly names of executables, by lower-case path, kept for the engine's lifetime.
pub(crate) type Names = std::rc::Rc<std::cell::RefCell<HashMap<String, String>>>;

/// Most friendly names kept (far more than the applications that use audio).
const NAMES_LIMIT: usize = 256;

/// The sessions of one output device, with their notifications.
pub(crate) struct SessionWatch {
    /// The device whose sessions these are.
    pub(crate) device_id: String,
    /// Generation of this watch: signals of an older watch are ignored.
    pub(crate) generation: u64,
    sink: Sink,
    manager: IAudioSessionManager2,
    watcher: IAudioSessionNotification,
    tracked: Vec<Tracked>,
    next_id: u64,
    /// Friendly names already looked up, by executable path (shared by every watch of
    /// the engine: reading them from the executables is the slow part of an opening).
    names: Names,
    /// Sessions Windows still lists although their process has ended: skipped without
    /// looking at them again (forgotten once Windows stops listing them).
    ended: Vec<String>,
}

impl SessionWatch {
    /// Starts watching the sessions of `device`.
    pub(crate) fn start(
        device: &IMMDevice,
        device_id: &str,
        generation: u64,
        sink: Sink,
        names: Names,
    ) -> Result<Self, AudioError> {
        // SAFETY: activating a documented interface on a valid device.
        let manager: IAudioSessionManager2 =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| audio_error(&e))?;
        let watcher = SessionCreatedWatcher::create(sink.clone(), Signal::Sessions { generation });
        // SAFETY: `watcher` stays alive in the returned value until it is unregistered
        // in `drop`.
        unsafe { manager.RegisterSessionNotification(&watcher) }.map_err(|e| audio_error(&e))?;
        let mut watch = Self {
            device_id: device_id.to_owned(),
            generation,
            sink,
            manager,
            watcher,
            tracked: Vec::new(),
            next_id: 0,
            names,
            ended: Vec::new(),
        };
        // Windows only starts sending "session created" notifications once a session
        // enumerator has been requested, which `refresh` does.
        watch.refresh();
        Ok(watch)
    }

    /// The sessions, in Windows order.
    pub(crate) fn streams(&self) -> Vec<Stream> {
        self.tracked.iter().map(|t| t.stream.clone()).collect()
    }

    /// Process identifiers of the sessions playing now.
    pub(crate) fn playing_processes(&self) -> Vec<u32> {
        self.tracked
            .iter()
            .filter(|t| t.playing && t.stream.process != 0)
            .map(|t| t.stream.process)
            .collect()
    }

    /// Re-reads the list of sessions: follows the new ones, forgets the ended ones and
    /// refreshes the names and volumes of the others.
    pub(crate) fn refresh(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        let mut listed: Vec<String> = Vec::new();
        // SAFETY: plain call on a valid manager.
        let Ok(list) = (unsafe { self.manager.GetSessionEnumerator() }) else {
            self.tracked.clear();
            return;
        };
        // SAFETY: plain call on a valid enumerator.
        let count = unsafe { list.GetCount() }.unwrap_or(0);
        for index in 0..count {
            // SAFETY: `index` is within the count Windows just gave.
            let Ok(session) = (unsafe { list.GetSession(index) }) else { continue };
            let Ok(control) = session.cast::<IAudioSessionControl2>() else { continue };
            // SAFETY: plain calls on a valid session.
            if unsafe { control.GetState() }.is_ok_and(|s| s == AudioSessionStateExpired) {
                continue;
            }
            // SAFETY: as above; the returned string is ours (`take_string` frees it).
            let Ok(instance) = (unsafe { control.GetSessionInstanceIdentifier() }).map(take_string)
            else {
                continue;
            };
            if seen.contains(&instance) || listed.contains(&instance) {
                continue;
            }
            listed.push(instance.clone());
            if self.ended.contains(&instance) {
                continue;
            }
            if let Some(tracked) = self.tracked.iter_mut().find(|t| t.instance == instance) {
                if tracked.exit.as_ref().is_some_and(ExitWatch::has_exited) {
                    // Its process ended: the row goes, even if Windows keeps the session.
                    self.ended.push(instance);
                    continue;
                }
                tracked.stream = describe(&tracked.control, &tracked.volume, &self.names);
                tracked.playing = playing(&tracked.control);
            } else if let Some(tracked) = self.track(control, &instance) {
                self.tracked.push(tracked);
            } else {
                continue;
            }
            seen.push(instance);
        }
        self.tracked.retain(|t| seen.contains(&t.instance));
        self.ended.retain(|e| listed.contains(e));
    }

    /// Applies a volume notification of one session. Returns `true` if something changed.
    pub(crate) fn apply_volume(&mut self, session: u64, volume: f32, muted: bool) -> bool {
        let Some(tracked) = self.tracked.iter_mut().find(|t| t.id == session) else { return false };
        let changed = tracked.stream.volume != volume || tracked.stream.muted != muted;
        tracked.stream.volume = volume;
        tracked.stream.muted = muted;
        changed
    }

    /// Sets the volume of every session of `app` and unmutes the muted ones (FR-062).
    pub(crate) fn set_volume(&mut self, app: &AppKey, volume: f32) -> Result<(), AudioError> {
        let mut found = false;
        for tracked in self.tracked.iter_mut().filter(|t| t.stream.app == *app) {
            found = true;
            // SAFETY: plain call on a valid session; a null event context is allowed.
            unsafe { tracked.volume.SetMasterVolume(volume, std::ptr::null()) }
                .map_err(|e| audio_error(&e))?;
            tracked.stream.volume = volume;
            if tracked.stream.muted {
                // SAFETY: as above.
                unsafe { tracked.volume.SetMute(false, std::ptr::null()) }
                    .map_err(|e| audio_error(&e))?;
                tracked.stream.muted = false;
            }
        }
        if found {
            Ok(())
        } else {
            Err(AudioError::UnknownApp)
        }
    }

    /// Mutes or unmutes every session of `app` (FR-062).
    pub(crate) fn set_mute(&mut self, app: &AppKey, muted: bool) -> Result<(), AudioError> {
        let mut found = false;
        for tracked in self.tracked.iter_mut().filter(|t| t.stream.app == *app) {
            found = true;
            // SAFETY: plain call on a valid session; a null event context is allowed.
            unsafe { tracked.volume.SetMute(muted, std::ptr::null()) }
                .map_err(|e| audio_error(&e))?;
            tracked.stream.muted = muted;
        }
        if found {
            Ok(())
        } else {
            Err(AudioError::UnknownApp)
        }
    }

    fn track(&mut self, control: IAudioSessionControl2, instance: &str) -> Option<Tracked> {
        let volume = control.cast::<ISimpleAudioVolume>().ok()?;
        let stream = describe(&control, &volume, &self.names);
        let exit = match stream.app {
            AppKey::System => None,
            _ => match ExitWatch::start(stream.process, self.sink.clone(), self.generation) {
                Watch::Running(watch) => Some(watch),
                // A session left behind by a process that already ended: never listed,
                // however often Windows still enumerates it.
                Watch::Gone => {
                    self.ended.push(instance.to_owned());
                    return None;
                }
                Watch::Unwatchable => None,
            },
        };
        let id = self.next_id;
        self.next_id += 1;
        let watcher = SessionWatcher::create(self.sink.clone(), self.generation, id);
        // SAFETY: `watcher` stays alive in the returned value until `Tracked::drop`
        // unregisters it.
        unsafe { control.RegisterAudioSessionNotification(&watcher) }.ok()?;
        let playing = playing(&control);
        Some(Tracked {
            id,
            instance: instance.to_owned(),
            control,
            volume,
            watcher,
            exit,
            stream,
            playing,
        })
    }
}

impl Drop for SessionWatch {
    fn drop(&mut self) {
        // Sessions first (their own notifications), then the "created" notification.
        self.tracked.clear();
        // SAFETY: `watcher` was registered by `start`.
        let _ = unsafe { self.manager.UnregisterSessionNotification(&self.watcher) };
    }
}

/// Whether a session plays now.
fn playing(control: &IAudioSessionControl2) -> bool {
    // SAFETY: plain call on a valid session.
    unsafe { control.GetState() }.is_ok_and(|s| s == AudioSessionStateActive)
}

/// What the application list needs about one session.
fn describe(control: &IAudioSessionControl2, volume: &ISimpleAudioVolume, names: &Names) -> Stream {
    // SAFETY: plain calls on valid session interfaces; strings returned by COM are ours
    // (`take_string` frees them).
    let (system, pid, display, icon, level, muted) = unsafe {
        (
            control.IsSystemSoundsSession() == S_OK,
            control.GetProcessId().unwrap_or(0),
            control.GetDisplayName().map(take_string).unwrap_or_default(),
            control.GetIconPath().map(take_string).unwrap_or_default(),
            volume.GetMasterVolume().unwrap_or(0.0),
            volume.GetMute().is_ok_and(|m| m.as_bool()),
        )
    };
    if system {
        let icon = Some(icon).filter(|i| !i.trim().is_empty());
        return Stream {
            app: AppKey::System,
            process: pid,
            name: String::new(),
            icon,
            volume: level,
            muted,
            heard_here: false,
        };
    }
    let path = process::image_path(pid);
    // A process we may not open (administrator rights, service): its session identifier
    // still names the executable, in the kernel's path form.
    let kernel_path = match &path {
        Some(_) => None,
        // SAFETY: plain call on a valid session; the returned string is ours.
        None => unsafe { control.GetSessionIdentifier() }
            .map(take_string)
            .ok()
            .and_then(|id| process::executable_from_session_identifier(&id).map(str::to_owned)),
    };
    let app = match (&path, &kernel_path) {
        (Some(path), _) | (None, Some(path)) => AppKey::executable(path),
        (None, None) => AppKey::Process(pid),
    };
    // The executable's own description first: an application names some of its sessions
    // and not others (a browser: "Mozilla Firefox" on one device, nothing on another),
    // and its row must keep one name everywhere.
    // A process we may not open (a browser's sandboxed audio process) is named from its
    // executable too, reached through its kernel path.
    let readable = path.clone().or_else(|| kernel_path.as_deref().map(process::readable_path));
    let name = if let Some(readable) = &readable {
        let mut names = names.borrow_mut();
        if names.len() >= NAMES_LIMIT {
            names.clear();
        }
        names
            .entry(readable.to_lowercase())
            .or_insert_with(|| process::friendly_name(readable))
            .clone()
    } else {
        process::resolve_indirect(&display).unwrap_or_default()
    };
    // The kernel path form cannot be used to load an icon.
    let icon = Some(icon).filter(|i| !i.trim().is_empty()).or(path);
    Stream { app, process: pid, name, icon, volume: level, muted, heard_here: false }
}
