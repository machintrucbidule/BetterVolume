//! BetterVolume audio engine.
//!
//! Wraps the Windows Core Audio APIs: device enumeration and notifications, default device
//! switching (media / calls), endpoint volume and mute of the current output and input,
//! and the application audio sessions of the current output.
//!
//! Threading (`docs/ARCHITECTURE.md` §3): the engine lives on the UI thread. Windows calls
//! its notification callbacks on its own threads; they never touch state, they only push a
//! typed signal into a channel and call the `notify` function given to [`Engine::new`]
//! (which wakes the UI thread). The UI thread then calls
//! [`AudioEngine::process_signals`], which drains the channel, re-reads what changed and
//! reports it as [`Changes`]. Nothing polls.
//!
//! The rest of the application uses the [`AudioEngine`] trait, so its logic can be tested
//! with the in-memory engine of the `fake` feature.
//!
//! `unsafe` is allowed in this crate (COM interop); every block carries a `SAFETY:`
//! comment.

use std::fmt;
use std::sync::Arc;

pub use bv_core::apps::{AppKey, Stream};
pub use bv_core::audio::{Bus, DeviceInfo, DeviceState, Flow, FormFactor, Role};
pub use bv_core::settings::SwitchScope;

mod activity;
mod com;
mod engine;
mod exit;
#[cfg(feature = "fake")]
pub mod fake;
mod policy;
mod process;
mod service;
mod sessions;
mod signals;

pub use engine::Engine;

/// Called from Windows audio threads when the engine has news; must wake the UI thread,
/// which then calls [`AudioEngine::process_signals`]. It must return quickly and must not
/// call into the engine.
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Volume state of the current device of a flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    /// The device this state belongs to.
    pub device_id: String,
    /// Windows volume scalar, 0.0 to 1.0.
    pub volume: f32,
    /// Muted.
    pub muted: bool,
}

impl Endpoint {
    /// The volume in whole percent.
    pub fn percent(&self) -> u8 {
        bv_core::volume::to_percent(self.volume)
    }
}

/// What changed since the previous call (or since the engine was created).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The device list of a flow changed (device added, removed, renamed in Windows,
    /// availability changed).
    pub devices: bool,
    /// A default device changed (any flow, any role).
    pub defaults: bool,
    /// The current output, its volume or its mute state changed.
    pub output: bool,
    /// The current input, its volume or its mute state changed.
    pub input: bool,
    /// The application sessions changed (only while they are watched).
    pub sessions: bool,
}

impl Changes {
    /// Everything changed (after a full re-read).
    pub const ALL: Changes =
        Changes { devices: true, defaults: true, output: true, input: true, sessions: true };

    /// Whether the endpoint of `flow` changed.
    pub fn endpoint(&self, flow: Flow) -> bool {
        match flow {
            Flow::Output => self.output,
            Flow::Input => self.input,
        }
    }

    /// Whether anything changed.
    pub fn any(&self) -> bool {
        *self != Changes::default()
    }

    /// Adds the changes of `other`.
    pub fn merge(&mut self, other: Changes) {
        self.devices |= other.devices;
        self.defaults |= other.defaults;
        self.output |= other.output;
        self.input |= other.input;
        self.sessions |= other.sessions;
    }

    pub(crate) fn set_endpoint(&mut self, flow: Flow) {
        match flow {
            Flow::Output => self.output = true,
            Flow::Input => self.input = true,
        }
    }
}

/// Why an audio operation failed. Never shown as is: the application turns it into a
/// plain-language message (FR-005).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// There is no such device, or no current device for this flow.
    NoDevice,
    /// The device disappeared during the operation (unplugged, disabled). The engine
    /// re-reads the devices on its next [`AudioEngine::process_signals`].
    DeviceGone,
    /// The Windows audio service is not running.
    ServiceUnavailable,
    /// Windows does not offer the default-device switching interface on this system.
    SwitchUnavailable,
    /// Windows refused to change the default device.
    SwitchRefused,
    /// No session of this application exists on the current output (or sessions are not
    /// being watched).
    UnknownApp,
    /// Any other Windows failure, with its error code.
    Windows(i32),
}

impl fmt::Display for AudioError {
    // Technical text for the log file only; never shown to the user.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AudioError::NoDevice => f.write_str("no such audio device"),
            AudioError::DeviceGone => f.write_str("audio device disappeared"),
            AudioError::ServiceUnavailable => f.write_str("Windows audio service not running"),
            AudioError::SwitchUnavailable => f.write_str("default-device switching unavailable"),
            AudioError::SwitchRefused => f.write_str("default-device switch refused by Windows"),
            AudioError::UnknownApp => f.write_str("no session for this application"),
            AudioError::Windows(code) => write!(f, "Windows error 0x{code:08X}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// The audio engine as the rest of the application sees it.
///
/// Every setter updates the engine's own view at once when Windows accepts the change, so
/// the interface can reflect it immediately; the notification that Windows sends
/// afterwards confirms it without reporting a second change.
pub trait AudioEngine {
    /// Every device of `flow` Windows knows, available or not (see [`DeviceInfo::state`]),
    /// in Windows order. Devices Windows considers absent for good are not listed.
    fn devices(&self, flow: Flow) -> &[DeviceInfo];

    /// The default device of `flow` for `role`, if any.
    fn default_device(&self, flow: Flow, role: Role) -> Option<&str>;

    /// The volume state of the current (media default) device of `flow`.
    fn endpoint(&self, flow: Flow) -> Option<&Endpoint>;

    /// Makes `device_id` the default device of `flow` for the roles of `scope` (FR-041,
    /// FR-072). The endpoint and sessions follow the new current device (FR-053).
    fn set_default_device(
        &mut self,
        flow: Flow,
        device_id: &str,
        scope: SwitchScope,
    ) -> Result<(), AudioError>;

    /// Sets the volume of the current device of `flow`, in percent (above 100 counts as
    /// 100). Unmutes it, like Windows (FR-015, FR-052).
    fn set_volume(&mut self, flow: Flow, percent: u8) -> Result<(), AudioError>;

    /// Mutes or unmutes the current device of `flow` (FR-016).
    fn set_mute(&mut self, flow: Flow, muted: bool) -> Result<(), AudioError>;

    /// Starts or stops watching the application sessions of the current output. Watching
    /// costs memory and callbacks, so it is only on while someone looks at them.
    fn watch_sessions(&mut self, watch: bool);

    /// The audio sessions of the current output (empty when not watched). Group them with
    /// [`bv_core::apps::rows`].
    fn sessions(&self) -> &[Stream];

    /// Sets the volume of every session of `app`, in percent, unmuting them (FR-062).
    fn set_app_volume(&mut self, app: &AppKey, percent: u8) -> Result<(), AudioError>;

    /// Mutes or unmutes every session of `app` (FR-062).
    fn set_app_mute(&mut self, app: &AppKey, muted: bool) -> Result<(), AudioError>;

    /// Applies every notification received since the last call and reports what changed.
    /// Call it on the UI thread each time the `notify` function has fired.
    fn process_signals(&mut self) -> Changes;

    /// Re-reads everything from Windows (after resuming from sleep, NFR-05).
    fn refresh(&mut self) -> Changes;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_merge_and_report() {
        let mut changes = Changes::default();
        assert!(!changes.any());
        changes.merge(Changes { input: true, ..Changes::default() });
        assert!(changes.any());
        assert!(changes.endpoint(Flow::Input) && !changes.endpoint(Flow::Output));
        changes.set_endpoint(Flow::Output);
        assert!(changes.endpoint(Flow::Output));
        changes.merge(Changes::ALL);
        assert_eq!(changes, Changes::ALL);
    }

    #[test]
    fn endpoint_percent_is_rounded() {
        let endpoint = Endpoint { device_id: "x".into(), volume: 0.416, muted: false };
        assert_eq!(endpoint.percent(), 42);
    }

    #[test]
    fn errors_have_a_technical_description() {
        assert_eq!(AudioError::Windows(-2_147_024_809).to_string(), "Windows error 0x80070057");
        assert!(!AudioError::SwitchRefused.to_string().is_empty());
    }
}
