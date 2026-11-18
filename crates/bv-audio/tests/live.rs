//! Live tests against the real Windows audio system of the machine running them.
//!
//! Two kinds:
//!
//! - Ordinary tests (run by `cargo test` and `tools/check.ps1`) only *read* the machine's
//!   audio state. The only things they create are silent streams, inside the test process
//!   itself or in a child test process they start and kill, which change nothing the user
//!   can hear; they change no volume or mute state. On a machine without audio devices
//!   (a CI runner) they check that the engine copes and stop there.
//! - Tests marked `#[ignore]` *change* the real state (default devices, volume, mute) and
//!   restore it at the end, even when they fail. They are run on purpose with
//!   `tools/audio-live.ps1`, after a change of the audio engine, telling whoever uses the
//!   machine first (sound may change).
// These tests report what they found on the machine (devices, sessions) for whoever runs
// them with `--nocapture`; printing is what they are for.
#![allow(clippy::print_stderr)]

use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bv_audio::{AppKey, AudioEngine, AudioError, Engine, Flow, Role, SwitchScope};
use bv_core::apps;

/// How long Windows gets to deliver a notification.
const TIMEOUT: Duration = Duration::from_secs(5);

/// An engine whose wake-ups arrive on a channel the test can wait on.
fn new_engine() -> (Engine, Receiver<()>) {
    let (sender, receiver) = mpsc::channel();
    let engine = Engine::new(Arc::new(move || {
        let _ = sender.send(());
    }));
    (engine, receiver)
}

/// Processes notifications until `done` holds or the timeout expires.
fn wait_for(
    engine: &mut Engine,
    wakes: &Receiver<()>,
    mut done: impl FnMut(&Engine) -> bool,
) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        engine.process_signals();
        if done(engine) {
            return true;
        }
        let Some(left) = deadline.checked_duration_since(Instant::now()) else { return false };
        let _ = wakes.recv_timeout(left);
    }
}

fn has_output(engine: &Engine) -> bool {
    engine.default_device(Flow::Output, Role::Media).is_some()
}

#[test]
fn reads_devices_defaults_and_endpoints() {
    let (engine, _wakes) = new_engine();
    for flow in Flow::ALL {
        let devices = engine.devices(flow);
        eprintln!("{flow:?}: {} device(s)", devices.len());
        for device in devices {
            eprintln!(
                "  {:?} {:?} {:?} {:?}",
                device.windows_name, device.form_factor, device.bus, device.state
            );
            assert!(!device.id.is_empty());
            assert!(!device.windows_name.is_empty());
        }
        let ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            "duplicate identifiers"
        );
        for role in Role::ALL {
            if let Some(id) = engine.default_device(flow, role) {
                let device = devices.iter().find(|d| d.id == id).expect("default is listed");
                assert!(device.state.is_available(), "default device must be available");
            }
        }
        match engine.default_device(flow, Role::Media) {
            Some(id) => {
                let endpoint = engine.endpoint(flow).expect("current device has a volume");
                assert_eq!(endpoint.device_id, id, "endpoint follows the current device");
                assert!((0.0..=1.0).contains(&endpoint.volume));
                eprintln!("  current: {}% muted={}", endpoint.percent(), endpoint.muted);
            }
            None => assert!(engine.endpoint(flow).is_none()),
        }
    }
}

#[test]
fn sessions_are_not_watched_until_asked() {
    let (mut engine, wakes) = new_engine();
    assert!(engine.sessions().is_empty());
    engine.watch_sessions(true);
    let _ = wait_for(&mut engine, &wakes, |_| true);
    let rows = apps::rows(engine.sessions());
    eprintln!("{} application row(s)", rows.len());
    for row in &rows {
        eprintln!(
            "  {:?} {:?} {}% muted={} ({} session(s))",
            row.app, row.name, row.volume, row.muted, row.sessions
        );
        if !row.is_system() && row.name.is_empty() {
            // Depends on what runs on this machine: reported, not a failure.
            eprintln!("  note: no friendly name found for {:?}", row.app);
        }
    }
    engine.watch_sessions(false);
    assert!(engine.sessions().is_empty());
}

/// A silent output stream owned by this test process: it creates an audio session that
/// the engine must see, without changing anything the user can hear.
struct SilentStream {
    client: windows::Win32::Media::Audio::IAudioClient,
}

impl SilentStream {
    fn start() -> Option<Self> {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
            AUDCLNT_SHAREMODE_SHARED,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};
        // SAFETY: COM is initialised on this thread by the engine created before; every
        // call gets valid arguments, and the mix format is freed once after use.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None).ok()?;
            let format = client.GetMixFormat().ok()?;
            // One second of buffer, in 100 ns units. Nothing is ever written: silence.
            let result =
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10_000_000, 0, format, None);
            CoTaskMemFree(Some(format as *const _));
            result.ok()?;
            client.Start().ok()?;
            Some(Self { client })
        }
    }
}

impl Drop for SilentStream {
    fn drop(&mut self) {
        // SAFETY: the client was started by `start`.
        let _ = unsafe { self.client.Stop() };
    }
}

#[test]
fn follows_the_session_of_this_process_live() {
    let (mut engine, wakes) = new_engine();
    if !has_output(&engine) {
        eprintln!("no output device on this machine: nothing more to check");
        return;
    }
    engine.watch_sessions(true);
    let me = AppKey::executable(&std::env::current_exe().unwrap().to_string_lossy());
    // Matched by process, not by program: the test of a killed child process runs in
    // parallel with the same program.
    let own = std::process::id();
    assert!(
        !engine.sessions().iter().any(|s| s.process == own),
        "no session before the stream exists"
    );

    let Some(stream) = SilentStream::start() else {
        eprintln!("the default output refuses a shared stream: nothing more to check");
        return;
    };
    assert!(
        wait_for(&mut engine, &wakes, |e| e
            .sessions()
            .iter()
            .any(|s| s.process == own && s.app == me && s.heard_here)),
        "a new session appears live (FR-063), marked as heard on this output"
    );
    let rows = apps::rows(engine.sessions());
    let row = rows.iter().find(|r| *r.app == me).unwrap();
    assert!(!row.name.is_empty());

    // Not watched: nothing is reported (and nothing is followed), even while a stream
    // plays. Watched again: the session is listed at once.
    engine.watch_sessions(false);
    assert!(engine.sessions().is_empty());
    engine.watch_sessions(true);
    assert!(engine.sessions().iter().any(|s| s.process == own));
    drop(stream);
    assert_eq!(engine.set_app_volume(&AppKey::Process(u32::MAX), 10), Err(AudioError::UnknownApp));
}

#[test]
#[ignore = "changes the volume Windows keeps for this test program; run with tools/audio-live.ps1"]
fn application_volume_and_mute_round_trip() {
    // Only the session of this test process is changed: nothing the user hears changes,
    // but Windows may remember the level for this executable, so it is not an ordinary
    // gate.
    let (mut engine, wakes) = new_engine();
    if !has_output(&engine) {
        eprintln!("no output device on this machine: nothing more to check");
        return;
    }
    engine.watch_sessions(true);
    let me = AppKey::executable(&std::env::current_exe().unwrap().to_string_lossy());
    let Some(_stream) = SilentStream::start() else {
        eprintln!("the default output refuses a shared stream: nothing more to check");
        return;
    };
    assert!(wait_for(&mut engine, &wakes, |e| e.sessions().iter().any(|s| s.app == me)));
    let start = apps::rows(engine.sessions()).into_iter().find(|r| *r.app == me).unwrap();
    // Puts back what Windows had for this program, also when an assertion fails.
    struct RestoreApp {
        app: AppKey,
        volume: u8,
        muted: bool,
    }
    impl Drop for RestoreApp {
        fn drop(&mut self) {
            let (mut engine, wakes) = new_engine();
            engine.watch_sessions(true);
            let _ =
                wait_for(&mut engine, &wakes, |e| e.sessions().iter().any(|s| s.app == self.app));
            let _ = engine.set_app_volume(&self.app, self.volume);
            let _ = engine.set_app_mute(&self.app, self.muted);
            eprintln!("application volume restored");
        }
    }
    let _restore = RestoreApp { app: me.clone(), volume: start.volume, muted: start.muted };

    engine.set_app_mute(&me, true).unwrap();
    assert!(apps::rows(engine.sessions()).iter().find(|r| *r.app == me).unwrap().muted);
    engine.set_app_volume(&me, 37).unwrap();
    let row = apps::rows(engine.sessions()).into_iter().find(|r| *r.app == me).unwrap();
    assert_eq!(row.volume, 37);
    assert!(!row.muted, "changing an application volume unmutes it");

    // A second engine (another program, as far as Windows is concerned) sees the change
    // through its notifications.
    let (mut observer, observer_wakes) = new_engine();
    observer.watch_sessions(true);
    assert!(wait_for(&mut observer, &observer_wakes, |e| {
        apps::rows(e.sessions()).iter().any(|r| *r.app == me && r.volume == 37)
    }));
    engine.set_app_volume(&me, 64).unwrap();
    assert!(
        wait_for(&mut observer, &observer_wakes, |e| {
            apps::rows(e.sessions()).iter().any(|r| *r.app == me && r.volume == 64)
        }),
        "volume changes from elsewhere arrive live (FR-003)"
    );
    eprintln!("application volume and mute round trip ok");
    // `_restore` was declared after `_stream`, so it runs first, while the session exists.
}

/// Set in the environment of the child process started by
/// `the_row_of_a_killed_process_disappears`.
const CHILD_VARIABLE: &str = "BV_AUDIO_LIVE_CHILD";

/// Not a test of its own: the body of the child process used by
/// `the_row_of_a_killed_process_disappears` (a silent stream held until it is killed). It
/// does nothing when run any other way.
#[test]
#[ignore = "helper run as a child process by another test"]
fn child_holding_a_silent_stream() {
    if std::env::var_os(CHILD_VARIABLE).is_none() {
        return;
    }
    let (_engine, _wakes) = new_engine(); // initialises COM on this thread
    let _stream = SilentStream::start();
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn the_row_of_a_killed_process_disappears() {
    let (mut engine, wakes) = new_engine();
    if !has_output(&engine) {
        eprintln!("no output device on this machine: nothing more to check");
        return;
    }
    engine.watch_sessions(true);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["child_holding_a_silent_stream", "--exact", "--ignored", "--test-threads=1"])
        .env(CHILD_VARIABLE, "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start the child process");
    let pid = child.id();
    let appeared = wait_for(&mut engine, &wakes, |e| e.sessions().iter().any(|s| s.process == pid));
    if !appeared {
        let _ = child.kill();
        let _ = child.wait();
        eprintln!(
            "the child's stream never appeared (shared streams refused?): nothing more to check"
        );
        return;
    }
    // Killed, not closed cleanly: the case where Windows may keep the session.
    child.kill().unwrap();
    let _ = child.wait();
    assert!(
        wait_for(&mut engine, &wakes, |e| !e.sessions().iter().any(|s| s.process == pid)),
        "the row of an ended application disappears live (FR-063)"
    );
    // ...and does not come back when the sessions are read again later, even if Windows
    // still lists the session of the dead process.
    engine.refresh();
    assert!(!engine.sessions().iter().any(|s| s.process == pid), "no comeback after a re-read");
    engine.watch_sessions(false);
    engine.watch_sessions(true);
    assert!(!engine.sessions().iter().any(|s| s.process == pid), "not listed when watching starts");
}

#[test]
fn refuses_unknown_devices_cleanly() {
    let (mut engine, _wakes) = new_engine();
    for flow in Flow::ALL {
        assert_eq!(
            engine.set_default_device(flow, "not a device", SwitchScope::MediaAndCalls),
            Err(AudioError::NoDevice)
        );
    }
}

// ---------------------------------------------------------------------------------------
// Tests that change the machine's audio state (run only through tools/audio-live.ps1).
// ---------------------------------------------------------------------------------------

/// Restores the volume, mute state and default devices found at creation, when dropped
/// (also when the test fails).
struct Restore {
    endpoints: Vec<(Flow, String, u8, bool)>,
    defaults: Vec<(Flow, Option<String>, Option<String>)>,
}

impl Restore {
    fn capture(engine: &Engine) -> Self {
        let endpoints = Flow::ALL
            .into_iter()
            .filter_map(|f| {
                engine.endpoint(f).map(|e| (f, e.device_id.clone(), e.percent(), e.muted))
            })
            .collect();
        let defaults = Flow::ALL
            .into_iter()
            .map(|f| {
                (
                    f,
                    engine.default_device(f, Role::Media).map(str::to_owned),
                    engine.default_device(f, Role::Calls).map(str::to_owned),
                )
            })
            .collect();
        Self { endpoints, defaults }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        let (mut engine, _wakes) = new_engine();
        for (flow, media, calls) in &self.defaults {
            if let Some(media) = media {
                let _ = engine.set_default_device(*flow, media, SwitchScope::MediaOnly);
            }
            if let Some(calls) = calls {
                let _ = engine.set_default_device(*flow, calls, SwitchScope::CallsOnly);
            }
        }
        for (flow, device, percent, muted) in &self.endpoints {
            if engine.endpoint(*flow).is_some_and(|e| e.device_id == *device) {
                let _ = engine.set_volume(*flow, *percent);
                let _ = engine.set_mute(*flow, *muted);
            }
        }
        eprintln!("audio state restored");
    }
}

#[test]
#[ignore = "changes the real volume and mute state (restored); run with tools/audio-live.ps1"]
fn volume_and_mute_round_trip() {
    let (mut engine, _wakes) = new_engine();
    let _restore = Restore::capture(&engine);
    let (mut observer, observer_wakes) = new_engine();
    for flow in Flow::ALL {
        let Some(start) = engine.endpoint(flow).cloned() else {
            eprintln!("{flow:?}: no current device, skipped");
            continue;
        };
        let target = if start.percent() < 100 { start.percent() + 1 } else { 99 };

        engine.set_mute(flow, true).unwrap();
        assert!(engine.endpoint(flow).unwrap().muted);
        assert!(
            wait_for(&mut observer, &observer_wakes, |e| e.endpoint(flow).is_some_and(|x| x.muted)),
            "{flow:?}: mute seen live from elsewhere"
        );

        engine.set_volume(flow, target).unwrap();
        let now = engine.endpoint(flow).unwrap();
        assert_eq!(now.percent(), target);
        assert!(!now.muted, "{flow:?}: changing the volume unmutes (FR-015, FR-052)");
        assert!(
            wait_for(&mut observer, &observer_wakes, |e| {
                e.endpoint(flow).is_some_and(|x| x.percent() == target && !x.muted)
            }),
            "{flow:?}: volume seen live from elsewhere"
        );
        // Our own notifications come back without reporting a second change.
        assert!(wait_for(&mut engine, &_wakes, |_| true));
        assert_eq!(engine.endpoint(flow).unwrap().percent(), target);
        eprintln!("{flow:?}: volume {} -> {target} and mute round trip ok", start.percent());
    }
}

#[test]
#[ignore = "switches the real default devices (restored); run with tools/audio-live.ps1"]
fn default_device_switch_round_trip() {
    let (mut engine, _wakes) = new_engine();
    let _restore = Restore::capture(&engine);
    let (mut observer, observer_wakes) = new_engine();
    let mut switched = 0;
    for flow in Flow::ALL {
        let Some(media) = engine.default_device(flow, Role::Media).map(str::to_owned) else {
            eprintln!("{flow:?}: no current device, skipped");
            continue;
        };
        let calls = engine.default_device(flow, Role::Calls).map(str::to_owned);
        let Some(other) = engine
            .devices(flow)
            .iter()
            .find(|d| d.state.is_available() && d.id != media)
            .map(|d| d.id.clone())
        else {
            eprintln!("{flow:?}: only one available device, switching cannot be tested");
            continue;
        };

        // Media and calls (the default scope).
        engine.set_default_device(flow, &other, SwitchScope::MediaAndCalls).unwrap();
        assert_eq!(
            engine.default_device(flow, Role::Media),
            Some(other.as_str()),
            "{flow:?}: immediate"
        );
        assert_eq!(engine.default_device(flow, Role::Calls), Some(other.as_str()));
        assert_eq!(
            engine.endpoint(flow).map(|e| e.device_id.as_str()),
            Some(other.as_str()),
            "{flow:?}: the volume follows the new device (FR-053)"
        );
        assert!(
            wait_for(&mut observer, &observer_wakes, |e| {
                e.default_device(flow, Role::Media) == Some(other.as_str())
                    && e.endpoint(flow).is_some_and(|x| x.device_id == other)
            }),
            "{flow:?}: the switch is seen live from elsewhere"
        );

        // Calls only: media stays.
        let calls_back = calls.clone().unwrap_or_else(|| media.clone());
        engine.set_default_device(flow, &calls_back, SwitchScope::CallsOnly).unwrap();
        assert_eq!(engine.default_device(flow, Role::Calls), Some(calls_back.as_str()));
        assert_eq!(
            engine.default_device(flow, Role::Media),
            Some(other.as_str()),
            "{flow:?}: media kept"
        );

        // Media only: calls stay.
        engine.set_default_device(flow, &media, SwitchScope::MediaOnly).unwrap();
        assert_eq!(engine.default_device(flow, Role::Media), Some(media.as_str()));
        assert_eq!(
            engine.default_device(flow, Role::Calls),
            Some(calls_back.as_str()),
            "{flow:?}: calls kept"
        );
        assert_eq!(engine.endpoint(flow).map(|e| e.device_id.as_str()), Some(media.as_str()));
        switched += 1;
        eprintln!("{flow:?}: switch round trip ok");
    }
    eprintln!("{switched} flow(s) switched");
}
