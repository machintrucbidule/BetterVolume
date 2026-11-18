//! Notification callbacks registered with Windows, and the signals they send.
//!
//! Windows calls these objects on its own threads. They never touch the engine's state:
//! each one pushes a [`Signal`] into the engine's channel and calls the `notify` function
//! (`docs/ARCHITECTURE.md` §3). Every signal carries the *generation* of the registration
//! that produced it, so a late notification from a registration the engine has already
//! replaced (previous device) is recognised and ignored.

use std::sync::mpsc::Sender;

use windows::core::{implement, Ref, BOOL, GUID, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::{
    PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName,
};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AudioSessionDisconnectReason, AudioSessionState, EDataFlow, ERole, IAudioSessionControl,
    IAudioSessionEvents, IAudioSessionEvents_Impl, IAudioSessionNotification,
    IAudioSessionNotification_Impl, IMMNotificationClient, IMMNotificationClient_Impl,
    PKEY_AudioEndpoint_FormFactor, AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE,
};

use crate::{Flow, Notify};

/// What a callback tells the UI thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Signal {
    /// A device was added, removed, changed availability or was renamed: re-read the list.
    Devices,
    /// A default device changed: re-read the defaults.
    Defaults,
    /// The volume or mute state of the current device of `flow` changed.
    Endpoint { flow: Flow, generation: u64, volume: f32, muted: bool },
    /// A session appeared, ended or was renamed on the current output: re-read sessions.
    Sessions { generation: u64 },
    /// A session of the current output started or stopped playing, or appeared.
    Activity { generation: u64 },
    /// The volume or mute state of one session changed.
    SessionVolume { generation: u64, session: u64, volume: f32, muted: bool },
    /// The Windows audio service changed state (stopping, restarting).
    Service,
}

/// Where callbacks send their signals.
#[derive(Clone)]
pub(crate) struct Sink {
    sender: Sender<Signal>,
    notify: Notify,
}

impl Sink {
    pub(crate) fn new(sender: Sender<Signal>, notify: Notify) -> Self {
        Self { sender, notify }
    }

    /// Queues `signal` and wakes the UI thread. Does nothing once the engine is gone.
    pub(crate) fn send(&self, signal: Signal) {
        if self.sender.send(signal).is_ok() {
            (self.notify)();
        }
    }
}

/// Device list and default device notifications.
#[implement(IMMNotificationClient)]
pub(crate) struct DeviceWatcher {
    sink: Sink,
}

impl DeviceWatcher {
    pub(crate) fn create(sink: Sink) -> IMMNotificationClient {
        DeviceWatcher { sink }.into()
    }
}

/// Property changes that affect what the application shows. Windows sends many others
/// (format, effects, levels...) which must not cause a re-read.
fn is_relevant_property(key: &PROPERTYKEY) -> bool {
    [PKEY_Device_FriendlyName, PKEY_Device_DeviceDesc, PKEY_AudioEndpoint_FormFactor]
        .iter()
        .any(|k| k.fmtid == key.fmtid && k.pid == key.pid)
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.sink.send(Signal::Devices);
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.sink.send(Signal::Devices);
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.sink.send(Signal::Devices);
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.sink.send(Signal::Defaults);
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, key: &PROPERTYKEY) -> windows::core::Result<()> {
        if is_relevant_property(key) {
            self.sink.send(Signal::Devices);
        }
        Ok(())
    }
}

/// Volume and mute notifications of one endpoint.
#[implement(IAudioEndpointVolumeCallback)]
pub(crate) struct VolumeWatcher {
    sink: Sink,
    flow: Flow,
    generation: u64,
}

impl VolumeWatcher {
    pub(crate) fn create(sink: Sink, flow: Flow, generation: u64) -> IAudioEndpointVolumeCallback {
        VolumeWatcher { sink, flow, generation }.into()
    }
}

impl IAudioEndpointVolumeCallback_Impl for VolumeWatcher_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        // SAFETY: Windows passes either null or a structure valid for the duration of the
        // call; it is only read here.
        if let Some(data) = unsafe { data.as_ref() } {
            self.sink.send(Signal::Endpoint {
                flow: self.flow,
                generation: self.generation,
                volume: data.fMasterVolume,
                muted: data.bMuted.as_bool(),
            });
        }
        Ok(())
    }
}

/// "A session was created" notifications of the current output: sends `signal`.
#[implement(IAudioSessionNotification)]
pub(crate) struct SessionCreatedWatcher {
    sink: Sink,
    signal: Signal,
}

impl SessionCreatedWatcher {
    pub(crate) fn create(sink: Sink, signal: Signal) -> IAudioSessionNotification {
        SessionCreatedWatcher { sink, signal }.into()
    }
}

impl IAudioSessionNotification_Impl for SessionCreatedWatcher_Impl {
    fn OnSessionCreated(&self, _session: Ref<IAudioSessionControl>) -> windows::core::Result<()> {
        self.sink.send(self.signal);
        Ok(())
    }
}

/// State notifications of one session of the current output: only the start
/// and end of playing, and the end of the session, wake the UI thread.
#[implement(IAudioSessionEvents)]
pub(crate) struct ActivityWatcher {
    sink: Sink,
    generation: u64,
}

impl ActivityWatcher {
    pub(crate) fn create(sink: Sink, generation: u64) -> IAudioSessionEvents {
        ActivityWatcher { sink, generation }.into()
    }
}

impl IAudioSessionEvents_Impl for ActivityWatcher_Impl {
    fn OnDisplayNameChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnIconPathChanged(&self, _: &PCWSTR, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnSimpleVolumeChanged(&self, _: f32, _: BOOL, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnChannelVolumeChanged(
        &self,
        _: u32,
        _: *const f32,
        _: u32,
        _: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnGroupingParamChanged(&self, _: *const GUID, _: *const GUID) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnStateChanged(&self, _state: AudioSessionState) -> windows::core::Result<()> {
        self.sink.send(Signal::Activity { generation: self.generation });
        Ok(())
    }

    fn OnSessionDisconnected(
        &self,
        _reason: AudioSessionDisconnectReason,
    ) -> windows::core::Result<()> {
        self.sink.send(Signal::Activity { generation: self.generation });
        Ok(())
    }
}

/// Notifications of one session.
#[implement(IAudioSessionEvents)]
pub(crate) struct SessionWatcher {
    sink: Sink,
    generation: u64,
    session: u64,
}

impl SessionWatcher {
    pub(crate) fn create(sink: Sink, generation: u64, session: u64) -> IAudioSessionEvents {
        SessionWatcher { sink, generation, session }.into()
    }

    fn sessions_changed(&self) {
        self.sink.send(Signal::Sessions { generation: self.generation });
    }
}

impl IAudioSessionEvents_Impl for SessionWatcher_Impl {
    fn OnDisplayNameChanged(
        &self,
        _name: &PCWSTR,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        self.sessions_changed();
        Ok(())
    }

    fn OnIconPathChanged(
        &self,
        _path: &PCWSTR,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        self.sessions_changed();
        Ok(())
    }

    fn OnSimpleVolumeChanged(
        &self,
        volume: f32,
        muted: BOOL,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        self.sink.send(Signal::SessionVolume {
            generation: self.generation,
            session: self.session,
            volume,
            muted: muted.as_bool(),
        });
        Ok(())
    }

    fn OnChannelVolumeChanged(
        &self,
        _count: u32,
        _volumes: *const f32,
        _changed: u32,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnGroupingParamChanged(
        &self,
        _param: *const GUID,
        _context: *const GUID,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnStateChanged(&self, _state: AudioSessionState) -> windows::core::Result<()> {
        self.sessions_changed();
        Ok(())
    }

    fn OnSessionDisconnected(
        &self,
        _reason: AudioSessionDisconnectReason,
    ) -> windows::core::Result<()> {
        self.sessions_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};

    use super::*;

    #[test]
    fn sink_queues_then_notifies_and_stops_once_the_engine_is_gone() {
        let count = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&count);
        let (sender, receiver) = mpsc::channel();
        let sink = Sink::new(
            sender,
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        sink.send(Signal::Devices);
        assert_eq!(receiver.try_recv(), Ok(Signal::Devices));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        drop(receiver);
        sink.send(Signal::Defaults);
        assert_eq!(count.load(Ordering::SeqCst), 1, "no wake-up for a closed channel");
    }

    #[test]
    fn only_displayed_properties_are_relevant() {
        assert!(is_relevant_property(&PKEY_Device_FriendlyName));
        assert!(is_relevant_property(&PKEY_AudioEndpoint_FormFactor));
        let other = PROPERTYKEY { fmtid: PKEY_Device_FriendlyName.fmtid, pid: 999 };
        assert!(!is_relevant_property(&other));
    }

    #[test]
    fn callbacks_translate_into_signals() {
        let (sender, receiver) = mpsc::channel();
        let sink = Sink::new(sender, Arc::new(|| {}));
        let watcher = VolumeWatcher::create(sink.clone(), Flow::Input, 7);
        let mut data = AUDIO_VOLUME_NOTIFICATION_DATA {
            fMasterVolume: 0.25,
            bMuted: true.into(),
            ..Default::default()
        };
        // SAFETY: `data` is a valid structure for the duration of the call.
        unsafe { watcher.OnNotify(&mut data) }.unwrap();
        // SAFETY: a null pointer is accepted and ignored.
        unsafe { watcher.OnNotify(std::ptr::null_mut()) }.unwrap();
        assert_eq!(
            receiver.try_recv(),
            Ok(Signal::Endpoint { flow: Flow::Input, generation: 7, volume: 0.25, muted: true })
        );
        assert!(receiver.try_recv().is_err());

        let session = SessionWatcher::create(sink, 3, 11);
        // SAFETY: plain values; the context pointer may be null.
        unsafe { session.OnSimpleVolumeChanged(0.5, false, std::ptr::null()) }.unwrap();
        assert_eq!(
            receiver.try_recv(),
            Ok(Signal::SessionVolume { generation: 3, session: 11, volume: 0.5, muted: false })
        );
    }
}
