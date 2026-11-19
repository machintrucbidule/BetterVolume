//! An in-memory audio engine for tests of the crates that use [`AudioEngine`].
//!
//! It behaves like the real engine (the endpoint follows the media default, changing the
//! volume unmutes, sessions are empty while not watched) and lets a test play the part of
//! Windows: change things "from outside", then report them through
//! [`AudioEngine::process_signals`], or make the next operation fail.

use std::collections::HashMap;

use crate::{
    AppKey, AudioEngine, AudioError, Changes, DeviceInfo, Endpoint, Flow, HeardApp, Role, Stream,
    SwitchScope,
};

/// The fake engine. Every field is public so tests can arrange any situation.
#[derive(Debug, Default)]
pub struct FakeEngine {
    /// Device lists, indexed by [`Flow::index`].
    pub devices: [Vec<DeviceInfo>; 2],
    /// Defaults, indexed by [`Flow::index`] then [`Role::index`].
    pub defaults: [[Option<String>; 2]; 2],
    /// Volume and mute of each device, by identifier (a missing entry reads as 50 %,
    /// not muted).
    pub levels: HashMap<String, (f32, bool)>,
    /// Sessions of the current output (reported only while watched).
    pub streams: Vec<Stream>,
    /// Whether sessions are watched.
    pub watching_sessions: bool,
    /// Applications heard playing while the sessions were not watched, handed out by the
    /// next `take_heard_apps`.
    pub heard_apps: Vec<HeardApp>,
    /// When set, the next operation fails with this error (then it is cleared).
    pub fail_next: Option<AudioError>,
    /// Changes reported by the next `process_signals` (set by the test, as if Windows had
    /// sent notifications).
    pub pending: Changes,
    /// Number of `refresh` calls.
    pub refreshes: usize,
    /// Simulates a volume binding that failed for these flows (indexed by
    /// [`Flow::index`]): no endpoint despite a current device, until `refresh`.
    pub binding_failed: [bool; 2],
    endpoints: [Option<Endpoint>; 2],
}

impl FakeEngine {
    /// An engine with no device.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a device to `flow` and makes it the default for both roles if it is the first.
    pub fn with_device(mut self, flow: Flow, info: DeviceInfo) -> Self {
        if self.devices[flow.index()].is_empty() {
            self.defaults[flow.index()] = [Some(info.id.clone()), Some(info.id.clone())];
        }
        self.devices[flow.index()].push(info);
        self.sync();
        self
    }

    /// Recomputes the endpoints from the defaults and levels. Call after changing fields.
    pub fn sync(&mut self) {
        for flow in Flow::ALL {
            let current = self.defaults[flow.index()][Role::Media.index()].clone();
            if self.binding_failed[flow.index()] {
                self.endpoints[flow.index()] = None;
                continue;
            }
            self.endpoints[flow.index()] = current.map(|id| {
                let (volume, muted) = self.levels.get(&id).copied().unwrap_or((0.5, false));
                Endpoint { device_id: id, volume, muted }
            });
        }
    }

    fn check(&mut self) -> Result<(), AudioError> {
        match self.fail_next.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn current(&self, flow: Flow) -> Result<String, AudioError> {
        self.defaults[flow.index()][Role::Media.index()].clone().ok_or(AudioError::NoDevice)
    }
}

impl AudioEngine for FakeEngine {
    fn devices(&self, flow: Flow) -> &[DeviceInfo] {
        &self.devices[flow.index()]
    }

    fn default_device(&self, flow: Flow, role: Role) -> Option<&str> {
        self.defaults[flow.index()][role.index()].as_deref()
    }

    fn endpoint(&self, flow: Flow) -> Option<&Endpoint> {
        self.endpoints[flow.index()].as_ref()
    }

    fn set_default_device(
        &mut self,
        flow: Flow,
        device_id: &str,
        scope: SwitchScope,
    ) -> Result<(), AudioError> {
        self.check()?;
        let available =
            self.devices[flow.index()].iter().any(|d| d.id == device_id && d.state.is_available());
        if !available {
            return Err(AudioError::NoDevice);
        }
        let roles: &[Role] = match scope {
            SwitchScope::MediaAndCalls => &[Role::Media, Role::Calls],
            SwitchScope::MediaOnly => &[Role::Media],
            SwitchScope::CallsOnly => &[Role::Calls],
        };
        for role in roles {
            self.defaults[flow.index()][role.index()] = Some(device_id.to_owned());
        }
        self.sync();
        Ok(())
    }

    fn set_volume(&mut self, flow: Flow, percent: u8) -> Result<(), AudioError> {
        self.check()?;
        let id = self.current(flow)?;
        self.levels.insert(id, (bv_core::volume::to_scalar(percent), false));
        self.sync();
        Ok(())
    }

    fn set_mute(&mut self, flow: Flow, muted: bool) -> Result<(), AudioError> {
        self.check()?;
        let id = self.current(flow)?;
        let volume = self.levels.get(&id).map_or(0.5, |l| l.0);
        self.levels.insert(id, (volume, muted));
        self.sync();
        Ok(())
    }

    fn watch_sessions(&mut self, watch: bool) {
        self.watching_sessions = watch;
    }

    fn sessions(&self) -> &[Stream] {
        if self.watching_sessions {
            &self.streams
        } else {
            &[]
        }
    }

    fn take_heard_apps(&mut self) -> Vec<HeardApp> {
        std::mem::take(&mut self.heard_apps)
    }

    fn set_app_volume(&mut self, app: &AppKey, percent: u8) -> Result<(), AudioError> {
        self.check()?;
        let mut found = false;
        for stream in self.streams.iter_mut().filter(|s| s.app == *app && self.watching_sessions) {
            stream.volume = bv_core::volume::to_scalar(percent);
            stream.muted = false;
            found = true;
        }
        if found {
            Ok(())
        } else {
            Err(AudioError::UnknownApp)
        }
    }

    fn set_app_mute(&mut self, app: &AppKey, muted: bool) -> Result<(), AudioError> {
        self.check()?;
        let mut found = false;
        for stream in self.streams.iter_mut().filter(|s| s.app == *app && self.watching_sessions) {
            stream.muted = muted;
            found = true;
        }
        if found {
            Ok(())
        } else {
            Err(AudioError::UnknownApp)
        }
    }

    fn process_signals(&mut self) -> Changes {
        self.sync();
        std::mem::take(&mut self.pending)
    }

    fn refresh(&mut self) -> Changes {
        self.refreshes += 1;
        self.binding_failed = [false, false];
        self.sync();
        self.pending = Changes::default();
        Changes::ALL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> DeviceInfo {
        DeviceInfo { id: id.into(), windows_name: id.into(), ..Default::default() }
    }

    #[test]
    fn behaves_like_the_real_engine() {
        let mut engine = FakeEngine::new()
            .with_device(Flow::Output, device("a"))
            .with_device(Flow::Output, device("b"));
        assert_eq!(engine.endpoint(Flow::Output).unwrap().device_id, "a");
        assert!(engine.endpoint(Flow::Input).is_none());

        engine.set_mute(Flow::Output, true).unwrap();
        assert!(engine.endpoint(Flow::Output).unwrap().muted);
        engine.set_volume(Flow::Output, 30).unwrap();
        let endpoint = engine.endpoint(Flow::Output).unwrap();
        assert!(!endpoint.muted, "changing the volume unmutes");
        assert_eq!(endpoint.percent(), 30);

        engine.set_default_device(Flow::Output, "b", SwitchScope::MediaOnly).unwrap();
        assert_eq!(engine.endpoint(Flow::Output).unwrap().device_id, "b");
        assert_eq!(engine.default_device(Flow::Output, Role::Calls), Some("a"));
        assert_eq!(
            engine.set_default_device(Flow::Output, "zz", SwitchScope::MediaOnly),
            Err(AudioError::NoDevice)
        );

        engine.fail_next = Some(AudioError::DeviceGone);
        assert_eq!(engine.set_mute(Flow::Output, true), Err(AudioError::DeviceGone));
        assert!(engine.set_mute(Flow::Output, true).is_ok(), "only the next call fails");
    }

    #[test]
    fn sessions_are_reported_only_while_watched() {
        let mut engine = FakeEngine::new();
        engine.streams.push(Stream {
            app: AppKey::System,
            process: 0,
            name: String::new(),
            icon: None,
            path: None,
            volume: 1.0,
            muted: true,
            heard_here: true,
        });
        assert!(engine.sessions().is_empty());
        assert_eq!(engine.set_app_volume(&AppKey::System, 20), Err(AudioError::UnknownApp));
        engine.watch_sessions(true);
        engine.set_app_volume(&AppKey::System, 20).unwrap();
        assert_eq!(engine.sessions()[0].volume, 0.2);
        assert!(!engine.sessions()[0].muted);
    }
}
