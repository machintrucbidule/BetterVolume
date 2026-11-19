//! The Windows implementation of [`AudioEngine`].

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_EnumeratorName, PKEY_Device_FriendlyName,
};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioEndpointVolumeCallback};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator,
    IMMNotificationClient, MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor, DEVICE_STATE,
    DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

use bv_core::audio::bus_from_enumerator;
use bv_core::volume;

use crate::activity::{ActivityWatch, Heard};
use crate::com::{
    audio_error, pcwstr, string_property, take_string, u32_property, wide, Apartment,
};
use crate::process;
use crate::service::ServiceWatch;
use crate::sessions::SessionWatch;
use crate::signals::{DeviceWatcher, Signal, Sink, VolumeWatcher};
use crate::{
    AppKey, AudioEngine, AudioError, Changes, DeviceInfo, DeviceState, Endpoint, Flow, FormFactor,
    HeardApp, Notify, Role, Stream, SwitchScope,
};

/// What the engine currently knows; what the [`AudioEngine`] accessors return.
#[derive(Default)]
struct State {
    devices: [Vec<DeviceInfo>; 2],
    /// Indexed by flow, then role.
    defaults: [[Option<String>; 2]; 2],
    endpoints: [Option<Endpoint>; 2],
    streams: Vec<Stream>,
}

/// The volume interface of the current device of one flow, with its notification.
struct BoundEndpoint {
    device_id: String,
    generation: u64,
    volume: IAudioEndpointVolume,
    watcher: IAudioEndpointVolumeCallback,
}

impl Drop for BoundEndpoint {
    fn drop(&mut self) {
        // SAFETY: `watcher` was registered on `volume` by `Engine::bind_endpoint`.
        let _ = unsafe { self.volume.UnregisterControlChangeNotify(&self.watcher) };
    }
}

/// Everything that depends on the audio service being up. Rebuilt after a restart.
struct Core {
    enumerator: IMMDeviceEnumerator,
    watcher: IMMNotificationClient,
    endpoints: [Option<BoundEndpoint>; 2],
    sessions: Option<SessionWatch>,
    /// Playing state of the sessions of the current output, followed at all times.
    activity: Option<ActivityWatch>,
}

impl Core {
    fn new(sink: Sink) -> Option<Self> {
        // SAFETY: creating the documented device enumerator; COM is initialised on this
        // thread (the engine holds an `Apartment`).
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
        let watcher = DeviceWatcher::create(sink);
        // SAFETY: `watcher` stays alive in `Core` until `drop` unregisters it.
        unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) }.ok()?;
        Some(Self { enumerator, watcher, endpoints: [None, None], sessions: None, activity: None })
    }

    fn device(&self, id: &str) -> Result<IMMDevice, AudioError> {
        let id = wide(id);
        // SAFETY: `id` is a NUL-terminated buffer that outlives the call.
        unsafe { self.enumerator.GetDevice(pcwstr(&id)) }.map_err(|e| audio_error(&e))
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // Release in dependency order: sessions and endpoints, then the device watcher.
        self.sessions = None;
        self.activity = None;
        self.endpoints = [None, None];
        // SAFETY: `watcher` was registered by `Core::new`.
        let _ = unsafe { self.enumerator.UnregisterEndpointNotificationCallback(&self.watcher) };
    }
}

/// Most processes remembered as already identified (far more than play at once).
const IDENTIFIED_LIMIT: usize = 256;

/// The audio engine. Lives on the UI thread (it is neither `Send` nor `Sync`).
pub struct Engine {
    sink: Sink,
    receiver: Receiver<Signal>,
    /// Incremented for every new registration; signals carry the value of theirs.
    generation: u64,
    sessions_wanted: bool,
    /// Friendly application names, kept across session watches.
    names: crate::sessions::Names,
    /// Where each process was last heard, and the last move of the current output.
    heard: Heard,
    /// The current output the activity watch was last asked to follow.
    output: Option<String>,
    /// Processes already looked at for the Applications page (FR-096): each playing
    /// process is identified once.
    identified: HashSet<u32>,
    /// Applications identified and not handed out yet.
    heard_apps: Vec<HeardApp>,
    state: State,
    core: Option<Core>,
    service: Option<ServiceWatch>,
    // Last field: COM stays initialised until every COM object above is released.
    _apartment: Apartment,
}

impl Engine {
    /// Starts the engine on the current thread and reads the current state. `notify` is
    /// called from Windows threads whenever [`AudioEngine::process_signals`] has work.
    ///
    /// Never fails: when the audio service is not running (or Windows refuses), the engine
    /// reports no device and recovers by itself once the service is back.
    pub fn new(notify: Notify) -> Self {
        let apartment = Apartment::enter();
        let (sender, receiver) = mpsc::channel();
        let sink = Sink::new(sender, notify);
        let service = ServiceWatch::start(sink.clone());
        let mut engine = Self {
            core: Core::new(sink.clone()),
            sink,
            receiver,
            generation: 0,
            sessions_wanted: false,
            names: crate::sessions::Names::default(),
            heard: Heard::default(),
            output: None,
            identified: HashSet::new(),
            heard_apps: Vec::new(),
            state: State::default(),
            service,
            _apartment: apartment,
        };
        engine.refresh();
        engine
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Re-reads both device lists. Returns `true` if they changed.
    fn read_devices(&mut self) -> bool {
        let lists = Flow::ALL.map(|flow| match &self.core {
            Some(core) => enumerate(&core.enumerator, flow),
            None => Vec::new(),
        });
        let changed = lists != self.state.devices;
        self.state.devices = lists;
        changed
    }

    /// Re-reads the defaults; re-binds endpoints (and sessions) to the current devices.
    /// `force` re-binds the endpoints even when the current device did not change (after
    /// sleep, or when the device list changed: an unplug and replug under the same
    /// identifier leaves dead interfaces); `force_sessions` does the same for the session
    /// watch. A device list change alone does not rebuild the session watch: that would
    /// move the application rows under a dragged slider (FR-063) and re-describe every
    /// session; a dead session watch is noticed by a failing session operation instead.
    fn read_defaults(&mut self, force: bool, force_sessions: bool, changes: &mut Changes) {
        for flow in Flow::ALL {
            for role in Role::ALL {
                let id =
                    self.core.as_ref().and_then(|core| default_id(&core.enumerator, flow, role));
                let slot = &mut self.state.defaults[flow.index()][role.index()];
                if *slot != id {
                    *slot = id;
                    changes.defaults = true;
                }
            }
            self.bind_endpoint(flow, force, changes);
        }
        // Also rebuilt when the device list changed: an unplug and replug of the current
        // output under the same identifier leaves a dead session manager.
        self.bind_activity(force || force_sessions);
        self.bind_sessions(force_sessions, changes);
    }

    /// Whether a default device is missing from the device list of its flow.
    fn has_unknown_default(&self) -> bool {
        Flow::ALL.into_iter().any(|flow| {
            let devices = &self.state.devices[flow.index()];
            self.state.defaults[flow.index()]
                .iter()
                .flatten()
                .any(|id| !devices.iter().any(|d| d.id == *id))
        })
    }

    fn current(&self, flow: Flow) -> Option<&str> {
        self.state.defaults[flow.index()][Role::Media.index()].as_deref()
    }

    /// Makes the endpoint of `flow` follow its current device (FR-053).
    fn bind_endpoint(&mut self, flow: Flow, force: bool, changes: &mut Changes) {
        let current = self.current(flow).map(str::to_owned);
        let generation = self.next_generation();
        let sink = self.sink.clone();
        let Some(core) = self.core.as_mut() else {
            if self.state.endpoints[flow.index()].take().is_some() {
                changes.set_endpoint(flow);
            }
            return;
        };
        let bound_to = core.endpoints[flow.index()].as_ref().map(|b| b.device_id.as_str());
        let same = bound_to.is_some() && bound_to == current.as_deref() && !force;
        // Same device: only refresh the values. If that fails, the interface we hold is
        // dead (the device was unplugged and plugged back under the same identifier):
        // bind it again.
        let kept = if same {
            let bound = core.endpoints[flow.index()].as_ref();
            bound.and_then(|bound| read_endpoint(&bound.volume, &bound.device_id))
        } else {
            None
        };
        let endpoint = if kept.is_some() {
            kept
        } else {
            core.endpoints[flow.index()] = None;
            let fresh = current.as_deref().and_then(|id| {
                let device = core.device(id).ok()?;
                bind(&device, id, flow, generation, sink)
            });
            let endpoint = fresh.as_ref().and_then(|b| read_endpoint(&b.volume, &b.device_id));
            core.endpoints[flow.index()] = fresh;
            endpoint
        };
        if self.state.endpoints[flow.index()] != endpoint {
            self.state.endpoints[flow.index()] = endpoint;
            changes.set_endpoint(flow);
        }
    }

    /// Starts, stops or moves the session watch so it follows the current output.
    fn bind_sessions(&mut self, force: bool, changes: &mut Changes) {
        let wanted =
            if self.sessions_wanted { self.current(Flow::Output).map(str::to_owned) } else { None };
        let generation = self.next_generation();
        let sink = self.sink.clone();
        let names = self.names.clone();
        if let Some(core) = self.core.as_mut() {
            let same = core.sessions.as_ref().map(|w| w.device_id.as_str()) == wanted.as_deref();
            if !same || force {
                core.sessions = None;
                core.sessions = wanted.as_deref().and_then(|id| {
                    let device = core.device(id).ok()?;
                    SessionWatch::start(&device, id, generation, sink, names).ok()
                });
            }
        }
        self.publish_sessions(changes);
    }

    /// Starts or moves the activity watch so it follows the current output, whatever the
    /// flyout does, and records what plays there.
    fn bind_activity(&mut self, force: bool) {
        let wanted = self.current(Flow::Output).map(str::to_owned);
        let generation = self.next_generation();
        let sink = self.sink.clone();
        // Kept apart from the watch, which may fail to start or be dropped with the audio
        // objects: a restart of the audio service is not a move. A moment without any
        // output is not one either: the output before it is the one left.
        if let Some(to) = wanted.as_deref().filter(|to| self.output.as_deref() != Some(to)) {
            self.heard.moved(self.output.as_deref(), to, Instant::now());
            self.output = Some(to.to_owned());
        }
        if let Some(core) = self.core.as_mut() {
            let same = core.activity.as_ref().map(|w| w.device_id.as_str()) == wanted.as_deref();
            if !same || force {
                core.activity = None;
                core.activity = wanted.as_deref().and_then(|id| {
                    let device = core.device(id).ok()?;
                    ActivityWatch::start(&device, id, generation, sink).ok()
                });
            }
        }
        self.scan_activity();
    }

    /// Records which processes play on the current output now.
    fn scan_activity(&mut self) {
        let Some(watch) = self.core.as_mut().and_then(|c| c.activity.as_mut()) else { return };
        let (playing, present) = watch.scan();
        let device = watch.device_id.clone();
        self.heard.look(&device, &playing, &present, Instant::now());
        self.identify(&playing);
    }

    /// Reads the executable and name of every process in `playing` not looked at yet, for
    /// [`AudioEngine::take_heard_apps`]. Only processes that start playing are opened, once.
    fn identify(&mut self, playing: &[u32]) {
        for &pid in playing {
            if pid == 0 || self.identified.contains(&pid) {
                continue;
            }
            if self.identified.len() >= IDENTIFIED_LIMIT {
                // Starting over only means looking at a playing process once more.
                self.identified.clear();
            }
            self.identified.insert(pid);
            let Some(path) = process::image_path(pid) else { continue };
            let name = crate::sessions::friendly_name_cached(&self.names, &path);
            self.heard_apps.push(HeardApp {
                app: AppKey::executable(&path),
                name,
                path: Some(path),
            });
        }
    }

    fn publish_sessions(&mut self, changes: &mut Changes) {
        let watch = self.core.as_ref().and_then(|c| c.sessions.as_ref());
        let read = watch.map(|w| (w.streams(), w.device_id.clone(), w.playing_processes()));
        let streams = match read {
            Some((streams, device, playing)) => {
                let present: Vec<u32> = streams.iter().map(|s| s.process).collect();
                self.heard.look(&device, &playing, &present, Instant::now());
                streams
                    .into_iter()
                    .map(|mut s| {
                        s.heard_here = self.heard.get(s.process) == Some(device.as_str());
                        s
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        if streams != self.state.streams {
            self.state.streams = streams;
            changes.sessions = true;
        }
    }

    /// The audio service changed state: drop every audio object and, if the service runs,
    /// create them anew. The caller then re-reads everything.
    fn rebuild_core(&mut self) {
        let running = self.service.as_ref().is_none_or(ServiceWatch::is_running);
        self.core = None;
        if running {
            self.core = Core::new(self.sink.clone());
        }
    }

    /// After an operation failed because the device vanished: re-read on the next
    /// `process_signals`, when Windows has settled.
    fn device_failed(&self, error: AudioError) -> AudioError {
        match error {
            AudioError::DeviceGone => {
                self.sink.send(Signal::Devices);
                self.sink.send(Signal::Defaults);
            }
            // The audio objects are dead (service restarted unnoticed): rebuild them.
            AudioError::ServiceUnavailable => self.sink.send(Signal::Service),
            _ => {}
        }
        error
    }

    /// An operation found no volume interface for `flow`. If Windows does have a current
    /// device, binding to it failed earlier (device still settling): try again on the
    /// next `process_signals`.
    fn unbound(&self, flow: Flow, error: AudioError) -> AudioError {
        if error == AudioError::NoDevice && self.current(flow).is_some() {
            self.sink.send(Signal::Defaults);
        }
        self.device_failed(error)
    }

    fn bound(&self, flow: Flow) -> Result<&BoundEndpoint, AudioError> {
        let core = self.core.as_ref().ok_or(AudioError::ServiceUnavailable)?;
        core.endpoints[flow.index()].as_ref().ok_or(AudioError::NoDevice)
    }

    fn sessions_mut(&mut self) -> Result<&mut SessionWatch, AudioError> {
        self.core.as_mut().and_then(|c| c.sessions.as_mut()).ok_or(AudioError::UnknownApp)
    }
}

impl AudioEngine for Engine {
    fn devices(&self, flow: Flow) -> &[DeviceInfo] {
        &self.state.devices[flow.index()]
    }

    fn default_device(&self, flow: Flow, role: Role) -> Option<&str> {
        self.state.defaults[flow.index()][role.index()].as_deref()
    }

    fn endpoint(&self, flow: Flow) -> Option<&Endpoint> {
        self.state.endpoints[flow.index()].as_ref()
    }

    fn set_default_device(
        &mut self,
        flow: Flow,
        device_id: &str,
        scope: SwitchScope,
    ) -> Result<(), AudioError> {
        let available = self.state.devices[flow.index()]
            .iter()
            .any(|d| d.id == device_id && d.state.is_available());
        if !available {
            return Err(AudioError::NoDevice);
        }
        if self.core.is_none() {
            return Err(AudioError::ServiceUnavailable);
        }
        crate::policy::set_default(device_id, scope)?;
        // Reflect the change at once (FR-041); the notifications confirm it.
        let mut changes = Changes::default();
        self.read_defaults(false, false, &mut changes);
        Ok(())
    }

    fn set_volume(&mut self, flow: Flow, percent: u8) -> Result<(), AudioError> {
        let level = volume::to_scalar(percent);
        let bound = match self.bound(flow) {
            Ok(bound) => bound,
            Err(error) => return Err(self.unbound(flow, error)),
        };
        // SAFETY: plain calls on a valid interface; a null event context is allowed.
        let result = unsafe {
            bound.volume.SetMasterVolumeLevelScalar(level, std::ptr::null()).and_then(|()| {
                // FR-015, FR-052: changing the volume unmutes, like Windows.
                bound.volume.SetMute(false, std::ptr::null())
            })
        };
        result.map_err(|e| self.device_failed(audio_error(&e)))?;
        if let Some(endpoint) = self.state.endpoints[flow.index()].as_mut() {
            endpoint.volume = level;
            endpoint.muted = false;
        }
        Ok(())
    }

    fn set_mute(&mut self, flow: Flow, muted: bool) -> Result<(), AudioError> {
        let bound = match self.bound(flow) {
            Ok(bound) => bound,
            Err(error) => return Err(self.unbound(flow, error)),
        };
        // SAFETY: plain call on a valid interface; a null event context is allowed.
        unsafe { bound.volume.SetMute(muted, std::ptr::null()) }
            .map_err(|e| self.device_failed(audio_error(&e)))?;
        if let Some(endpoint) = self.state.endpoints[flow.index()].as_mut() {
            endpoint.muted = muted;
        }
        Ok(())
    }

    fn watch_sessions(&mut self, watch: bool) {
        if self.sessions_wanted != watch {
            self.sessions_wanted = watch;
            let mut changes = Changes::default();
            self.bind_sessions(false, &mut changes);
        }
    }

    fn sessions(&self) -> &[Stream] {
        &self.state.streams
    }

    fn take_heard_apps(&mut self) -> Vec<HeardApp> {
        std::mem::take(&mut self.heard_apps)
    }

    fn set_app_volume(&mut self, app: &AppKey, percent: u8) -> Result<(), AudioError> {
        let level = volume::to_scalar(percent);
        let result = self.sessions_mut().and_then(|watch| watch.set_volume(app, level));
        let mut changes = Changes::default();
        self.publish_sessions(&mut changes);
        result.map_err(|e| self.device_failed(e))
    }

    fn set_app_mute(&mut self, app: &AppKey, muted: bool) -> Result<(), AudioError> {
        let result = self.sessions_mut().and_then(|watch| watch.set_mute(app, muted));
        let mut changes = Changes::default();
        self.publish_sessions(&mut changes);
        result.map_err(|e| self.device_failed(e))
    }

    fn process_signals(&mut self) -> Changes {
        let mut changes = Changes::default();
        let (mut devices, mut defaults, mut sessions, mut service) = (false, false, false, false);
        let mut activity = false;
        let mut volumes: Vec<Signal> = Vec::new();
        while let Ok(signal) = self.receiver.try_recv() {
            match signal {
                Signal::Devices => devices = true,
                Signal::Defaults => defaults = true,
                Signal::Service => service = true,
                Signal::Sessions { generation } => {
                    let current = self.core.as_ref().and_then(|c| c.sessions.as_ref());
                    sessions |= current.is_some_and(|w| w.generation == generation);
                }
                Signal::Activity { generation } => {
                    let current = self.core.as_ref().and_then(|c| c.activity.as_ref());
                    activity |= current.is_some_and(|w| w.generation == generation);
                }
                Signal::Endpoint { .. } | Signal::SessionVolume { .. } => volumes.push(signal),
            }
        }

        if service {
            self.rebuild_core();
            return self.read_everything();
        }
        if devices {
            changes.devices |= self.read_devices();
        }
        if devices || defaults {
            // A changed device list may hide an unplug and replug of the current device
            // (same identifier, dead interfaces): bind again in that case.
            let force = changes.devices;
            self.read_defaults(force, false, &mut changes);
            // A new device may become the default before its own "added" notification
            // arrives: read the list now rather than show a default nobody knows.
            if !devices && self.has_unknown_default() {
                changes.devices |= self.read_devices();
            }
        }
        if activity {
            // Where applications were last heard changed: the list may show other rows.
            self.scan_activity();
        }
        if sessions || activity {
            if sessions {
                if let Some(watch) = self.core.as_mut().and_then(|c| c.sessions.as_mut()) {
                    watch.refresh();
                }
            }
            self.publish_sessions(&mut changes);
        }

        // Volume notifications, oldest first: the last one of each source wins.
        let mut session_volume_changed = false;
        for signal in volumes {
            match signal {
                Signal::Endpoint { flow, generation, volume, muted } => {
                    let bound = self.core.as_ref().and_then(|c| c.endpoints[flow.index()].as_ref());
                    if bound.is_none_or(|b| b.generation != generation) {
                        continue; // from a previous device
                    }
                    let device_id = bound.map(|b| b.device_id.clone()).unwrap_or_default();
                    let fresh = Endpoint { device_id, volume, muted };
                    // Also fills in a first read that had failed.
                    if self.state.endpoints[flow.index()].as_ref() != Some(&fresh) {
                        self.state.endpoints[flow.index()] = Some(fresh);
                        changes.set_endpoint(flow);
                    }
                }
                Signal::SessionVolume { generation, session, volume, muted } => {
                    if let Some(watch) = self.core.as_mut().and_then(|c| c.sessions.as_mut()) {
                        if watch.generation == generation {
                            session_volume_changed |= watch.apply_volume(session, volume, muted);
                        }
                    }
                }
                _ => {}
            }
        }
        if session_volume_changed {
            self.publish_sessions(&mut changes);
        }
        changes
    }

    fn refresh(&mut self) -> Changes {
        // Everything queued is covered by the full re-read below, except an audio
        // service restart, which needs new audio objects first.
        let mut service = false;
        while let Ok(signal) = self.receiver.try_recv() {
            service |= signal == Signal::Service;
        }
        // Also when the audio objects could not be created before (service starting at
        // logon): try again.
        if service || self.core.is_none() {
            self.rebuild_core();
        }
        self.read_everything()
    }
}

impl Engine {
    /// Re-reads devices, defaults (binding again to the current devices) and sessions.
    fn read_everything(&mut self) -> Changes {
        let mut changes = Changes { devices: self.read_devices(), ..Changes::default() };
        self.read_defaults(true, true, &mut changes);
        if let Some(watch) = self.core.as_mut().and_then(|c| c.sessions.as_mut()) {
            watch.refresh();
        }
        self.publish_sessions(&mut changes);
        changes
    }
}

fn data_flow(flow: Flow) -> EDataFlow {
    match flow {
        Flow::Output => eRender,
        Flow::Input => eCapture,
    }
}

fn windows_role(role: Role) -> ERole {
    match role {
        Role::Media => eConsole,
        Role::Calls => eCommunications,
    }
}

fn default_id(enumerator: &IMMDeviceEnumerator, flow: Flow, role: Role) -> Option<String> {
    // SAFETY: plain calls on valid interfaces; fails with "not found" when there is no
    // device for this role.
    let device =
        unsafe { enumerator.GetDefaultAudioEndpoint(data_flow(flow), windows_role(role)) }.ok()?;
    // SAFETY: as above; the returned string is ours (`take_string` frees it).
    unsafe { device.GetId() }.ok().map(take_string).filter(|id| !id.is_empty())
}

/// Every device of `flow` Windows knows (active, disabled or unplugged), in Windows order.
fn enumerate(enumerator: &IMMDeviceEnumerator, flow: Flow) -> Vec<DeviceInfo> {
    let mask =
        DEVICE_STATE(DEVICE_STATE_ACTIVE.0 | DEVICE_STATE_DISABLED.0 | DEVICE_STATE_UNPLUGGED.0);
    // SAFETY: plain call on a valid interface.
    let Ok(collection) = (unsafe { enumerator.EnumAudioEndpoints(data_flow(flow), mask) }) else {
        return Vec::new();
    };
    // SAFETY: as above.
    let count = unsafe { collection.GetCount() }.unwrap_or(0);
    (0..count)
        .filter_map(|index| {
            // SAFETY: `index` is within the count Windows just gave.
            let device = unsafe { collection.Item(index) }.ok()?;
            describe(&device)
        })
        .collect()
}

fn describe(device: &IMMDevice) -> Option<DeviceInfo> {
    // SAFETY: plain calls on a valid device; the returned string is ours.
    let id = unsafe { device.GetId() }.ok().map(take_string).filter(|id| !id.is_empty())?;
    // SAFETY: as above.
    let state = match unsafe { device.GetState() }.ok()? {
        s if s == DEVICE_STATE_ACTIVE => DeviceState::Active,
        s if s == DEVICE_STATE_DISABLED => DeviceState::Disabled,
        s if s == DEVICE_STATE_UNPLUGGED => DeviceState::Unplugged,
        _ => return None,
    };
    // SAFETY: as above.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok();
    let text = |key| store.as_ref().and_then(|s| string_property(s, key));
    let windows_name = text(&PKEY_Device_FriendlyName).unwrap_or_else(|| id.clone());
    let bus =
        text(&PKEY_Device_EnumeratorName).map(|n| bus_from_enumerator(&n)).unwrap_or_default();
    let form_factor = store
        .as_ref()
        .and_then(|s| u32_property(s, &PKEY_AudioEndpoint_FormFactor))
        .map(form_factor)
        .unwrap_or_default();
    Some(DeviceInfo { id, windows_name, form_factor, bus, state })
}

/// Maps the Windows `EndpointFormFactor` values.
fn form_factor(value: u32) -> FormFactor {
    match value {
        0 => FormFactor::Remote,
        1 => FormFactor::Speakers,
        2 => FormFactor::LineLevel,
        3 => FormFactor::Headphones,
        4 => FormFactor::Microphone,
        5 => FormFactor::Headset,
        6 => FormFactor::Handset,
        8 => FormFactor::Spdif,
        9 => FormFactor::Display,
        _ => FormFactor::Unknown,
    }
}

fn bind(
    device: &IMMDevice,
    id: &str,
    flow: Flow,
    generation: u64,
    sink: Sink,
) -> Option<BoundEndpoint> {
    // SAFETY: activating a documented interface on a valid device.
    let volume: IAudioEndpointVolume = unsafe { device.Activate(CLSCTX_ALL, None) }.ok()?;
    let watcher = VolumeWatcher::create(sink, flow, generation);
    // SAFETY: `watcher` stays alive in `BoundEndpoint` until its `drop` unregisters it.
    unsafe { volume.RegisterControlChangeNotify(&watcher) }.ok()?;
    Some(BoundEndpoint { device_id: id.to_owned(), generation, volume, watcher })
}

fn read_endpoint(volume: &IAudioEndpointVolume, id: &str) -> Option<Endpoint> {
    // SAFETY: plain calls on a valid interface.
    let (level, muted) =
        unsafe { (volume.GetMasterVolumeLevelScalar().ok()?, volume.GetMute().ok()?) };
    Some(Endpoint { device_id: id.to_owned(), volume: level, muted: muted.as_bool() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_form_factors_are_mapped() {
        assert_eq!(form_factor(1), FormFactor::Speakers);
        assert_eq!(form_factor(3), FormFactor::Headphones);
        assert_eq!(form_factor(5), FormFactor::Headset);
        assert_eq!(form_factor(9), FormFactor::Display);
        assert_eq!(form_factor(7), FormFactor::Unknown);
        assert_eq!(form_factor(10), FormFactor::Unknown);
        assert_eq!(form_factor(u32::MAX), FormFactor::Unknown);
    }

    fn quiet_engine() -> Engine {
        Engine::new(std::sync::Arc::new(|| {}))
    }

    /// Device lists, media defaults and endpoint devices, per flow.
    type Snapshot = (Vec<Vec<DeviceInfo>>, Vec<Option<String>>, Vec<Option<String>>);

    /// What the engine shows, for comparing two moments.
    fn snapshot(engine: &Engine) -> Snapshot {
        let devices = Flow::ALL.iter().map(|f| engine.devices(*f).to_vec()).collect();
        let defaults = Flow::ALL
            .iter()
            .map(|f| engine.default_device(*f, Role::Media).map(str::to_owned))
            .collect();
        let endpoints =
            Flow::ALL.iter().map(|f| engine.endpoint(*f).map(|e| e.device_id.clone())).collect();
        (devices, defaults, endpoints)
    }

    #[test]
    fn a_service_restart_rebuilds_the_same_view() {
        // Read-only: the engine starts again from scratch, as after an audio service
        // restart, and must show the same state without anything being lost.
        let mut engine = quiet_engine();
        let before = snapshot(&engine);
        engine.sink.send(Signal::Service);
        engine.process_signals();
        assert_eq!(snapshot(&engine), before);
        if engine.service.as_ref().is_none_or(ServiceWatch::is_running) {
            assert!(engine.core.is_some(), "rebuilt while the service runs");
        }
    }

    #[test]
    fn late_signals_from_a_previous_registration_are_ignored() {
        let mut engine = quiet_engine();
        let Some(before) = engine.endpoint(Flow::Output).cloned() else { return };
        let stale = Signal::Endpoint {
            flow: Flow::Output,
            generation: 0, // no registration ever has generation 0
            volume: if before.volume > 0.5 { 0.0 } else { 1.0 },
            muted: !before.muted,
        };
        engine.sink.send(stale);
        let changes = engine.process_signals();
        assert!(!changes.output);
        assert_eq!(engine.endpoint(Flow::Output), Some(&before));
    }

    #[test]
    fn device_gone_schedules_a_re_read() {
        let mut engine = quiet_engine();
        assert_eq!(engine.device_failed(AudioError::DeviceGone), AudioError::DeviceGone);
        let mut queued = Vec::new();
        while let Ok(signal) = engine.receiver.try_recv() {
            queued.push(signal);
        }
        assert_eq!(queued, [Signal::Devices, Signal::Defaults]);
        assert_eq!(engine.device_failed(AudioError::UnknownApp), AudioError::UnknownApp);
        assert!(engine.receiver.try_recv().is_err());
        // Dead audio objects: rebuild them.
        engine.device_failed(AudioError::ServiceUnavailable);
        assert_eq!(engine.receiver.try_recv(), Ok(Signal::Service));
        assert!(engine.receiver.try_recv().is_err());
        engine.process_signals();
    }

    #[test]
    fn a_missing_volume_interface_is_bound_again_later() {
        let mut engine = quiet_engine();
        engine.state.defaults[Flow::Output.index()][Role::Media.index()] = Some("device".into());
        assert_eq!(engine.unbound(Flow::Output, AudioError::NoDevice), AudioError::NoDevice);
        assert_eq!(engine.receiver.try_recv(), Ok(Signal::Defaults));
        // No current device: nothing to bind to, nothing scheduled.
        engine.state.defaults[Flow::Output.index()][Role::Media.index()] = None;
        engine.unbound(Flow::Output, AudioError::NoDevice);
        assert!(engine.receiver.try_recv().is_err());
    }

    #[test]
    fn a_default_missing_from_the_device_list_is_noticed() {
        let mut engine = quiet_engine();
        engine.state.devices = [Vec::new(), Vec::new()];
        engine.state.defaults = [[None, None], [None, None]];
        assert!(!engine.has_unknown_default());
        engine.state.defaults[Flow::Input.index()][Role::Calls.index()] = Some("new".into());
        assert!(engine.has_unknown_default());
        engine.state.devices[Flow::Input.index()]
            .push(DeviceInfo { id: "new".into(), ..DeviceInfo::default() });
        assert!(!engine.has_unknown_default());
    }

    #[test]
    fn roles_and_flows_are_mapped() {
        assert_eq!(data_flow(Flow::Output), eRender);
        assert_eq!(data_flow(Flow::Input), eCapture);
        assert_eq!(windows_role(Role::Media), eConsole);
        assert_eq!(windows_role(Role::Calls), eCommunications);
    }
}
