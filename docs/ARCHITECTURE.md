# Architecture

The stack was chosen from measured prototypes (throw-away programs, not kept in this
repository).

## 1. Stack

| Concern | Choice |
|---|---|
| Language | **Rust** (stable, MSVC target), one native executable, no runtime |
| Windows APIs | `windows` crate (Microsoft's official bindings): Win32, COM, WinRT where needed |
| Windowing | Raw Win32 windows (popup flyout, standard settings window, hidden message window) |
| Rendering | Direct2D + DirectWrite into a DXGI composition swap chain, presented through DirectComposition |
| Rendering device | **Software rasteriser (WARP) by default**, hardware device as a fallback switch |
| Animation | Popup entrance and exit: the whole window (frame and system material included) moved once per compositor frame for the duration of the transition only (`bv-shell/src/popup.rs`); composition animations for content motion inside a window |
| Materials | DWM system backdrop + rounded corners + immersive dark mode attributes |
| Audio | Core Audio COM APIs (device enumeration/notifications, endpoint volume, audio sessions, policy config for default-device switching) |
| MQTT | Own MQTT 3.1.1 client in `bv-mqtt`: two threads per connection, no async runtime; encryption through Windows (Schannel via `native-tls`) |
| Settings | One human-readable TOML file (`settings.toml`, strict subset read and written by `bv_core::settings::file`), atomic writes, corrupt file kept aside |
| Secrets | Windows DPAPI (current user) |

### Why (measurements on a development machine, 225 % display scaling)

Two prototypes with identical behaviour (tray icon, flyout with a live master-volume
slider bound to the real default device, compositor-driven open animation):

| Variant | Exe size | Idle private RAM (never opened) | Private RAM, flyout resources kept | Threads kept | Idle CPU | Open: first / next |
|---|---|---|---|---|---|---|
| Rust, hardware device, kept | 158 KB | 1.7 MB | 15.2 MB | 85 | 0 | ~235 ms / ~4 ms |
| Rust, hardware device, released on close | 158 KB | 1.7 MB | 6–11 MB after close | 7 | 0 | ~300 ms / ~200 ms |
| Rust, hardware + single-thread flags, kept | 158 KB | 1.7 MB | 12.0 MB | 35 | 0 | ~276 ms / ~4 ms |
| **Rust, software rasteriser, kept** | 158 KB | 1.7 MB | **3.7 MB** | **6** | 0 | **~44 ms / ~4 ms** |
| .NET 9 NativeAOT, hardware, kept | 865 KB | 1.9 MB | 16.5 MB | 86 | 0 | ~237 ms / ~1 ms* |
| .NET 9 NativeAOT, hardware, released on close | 865 KB | 1.9 MB | 7–11 MB after close | 8 | 0 | ~260 ms / ~190 ms |

\* the two prototypes do not stop the latency clock at exactly the same point; treat
single-digit values as equivalent.

Findings:

1. **Language is not the footprint lever**: Rust and .NET NativeAOT are within ~10 %.
2. **The graphics device is**: a hardware Direct3D device costs ~12 MB and ~80 driver
   threads and takes ~200 ms to create. The software rasteriser renders the same pixels
   (verified by screenshot), costs ~2 MB, no extra threads, and is created in ~40 ms. Our
   surfaces are small and mostly static; motion is done by the compositor on the GPU anyway.
3. Rust chosen over .NET NativeAOT for: complete, typed, officially maintained API
   bindings (the .NET prototype needed hand-counted COM vtable slots and hit toolchain
   discovery friction), no garbage collector, 5× smaller binary, strong compile-time
   guarantees, and the ability to build a
   tiny native helper library if the Windows-icon hiding ends up needing one.

Rejected: WinUI 3 / WPF (tens of MB at idle, against the footprint goal); web-view based shells.

## 2. Footprint budget (NFR-01)

Measured on the release build, private working set, after 60 s idle:

| State | Budget | Target |
|---|---|---|
| Running, flyout never opened, Home Assistant off | ≤ 5 MB | ≤ 3 MB |
| Running, flyout used then closed, Home Assistant on | ≤ 12 MB | ≤ 8 MB |
| Idle CPU | 0 ms/min (no timers, no polling) | — |
| Flyout open latency | first ≤ 80 ms, next ≤ 16 ms | — |
| Executable | ≤ 3 MB | ≤ 1.5 MB |

A change that exceeds a budget is not accepted. Numbers are measured by
`tools/measure.ps1` (run by `tools/check.ps1`), which reads the budget from `tools/budget.json`
(a mirror of the table above: change both together) and fails when a value is exceeded.
Idle CPU is judged as "less than one process-clock tick (15.6 ms) over the idle period":
a single tick already fails.

## 3. Process and threading model

- One process, one UI thread running a classic message loop; it owns all windows, all
  rendering and all application state.
- Core Audio and MQTT callbacks arrive on other threads: they never touch state. They push
  a typed signal into their source's channel and call `bv_shell::Waker::wake`, which posts
  one message to the hidden window (bursts are coalesced); the UI thread then drains the
  channel (see §4, "Audio engine"). No shared mutable state, no locks in feature code.
- Strictly event-driven. Work outside events exists only transiently: popup entrance and
  exit (a frame-paced loop on the UI thread for a quarter of a second), hover
  tracking of the tray icon while the pointer is over it (100 ms timer), the tray icon
  retry while the taskbar is not ready (at most one minute), the MQTT keep-alive owned
  by the MQTT thread, and the loader child of the taskbar helper with the thread that
  waits for it (about a second, only when the Windows volume icon is hidden or restored).
- Lazy everything: graphics stack, settings window, MQTT client are created on first need.
  The tray icon is drawn with temporary graphics objects until a popup is first shown;
  popup graphics resources are then kept (cheap with the software rasteriser) and
  rebuilt after a drawing failure (lost device, display change).
- Single instance through a named mutex; a second launch signals the first to open the flyout.

## 4. Modules (Cargo workspace)

```
crates/
  bv-core       Pure logic, no Windows types: device list model (order/hidden/rename/
                icons, hidden-but-active rule), volume maths, settings schema + migration,
                Home Assistant payload/discovery builders, i18n lookup. 100 % unit-testable.
  bv-audio      Core Audio wrapper: enumerate, watch, switch default (media/calls), endpoint
                volume/mute, application sessions grouped per app. Emits events.
  bv-ui-kit     DESIGN SYSTEM. Tokens (colours, metrics, typography, motion curves),
                theme resolution (light/dark/accent/high contrast), and the control
                renderers (tile, slider, toggle, tab, button, list row, text box, menu,
                bubble). Knows nothing about audio or features.
  bv-shell      Windows plumbing: tray icon, raw-input wheel capture, flyout host window,
                settings host window, DWM attributes, DPI, autostart, single instance,
                Windows volume icon hiding.
  bv-mqtt       MQTT connection, discovery, state/command topics, availability, reconnect.
  bv-taskbar-   cdylib loaded into the taskbar process (XAML Diagnostics TAP) that
   helper       collapses the Windows volume tray element; guard rails in section 7. Not
                linked by the executable; shipped beside it, loaded on demand by a child
                process of the executable (bv-shell `taskbar.rs`).
  bv-app        The executable: wires events, owns state, builds views from bv-ui-kit
                controls. Contains no drawing code and no colour/metric literals.
locales/        en.toml, fr.toml translation files (compiled into static tables at build
                time by the bv-core build script; en.toml defines the message keys)
                (icons: Windows icon-font glyphs in bv-ui-kit `icons.rs`, device drawings
                as vector paths in code, `controls/tray.rs`; the application icon is the
                only asset file)
assets/icon/    the application icon: two SVG drawings and the generated .ico
tools/          check.ps1 (all quality gates), measure.ps1 + budget.json (smoke test and
                footprint), audio-live.ps1 (live audio tests that change the audio
                state), package.ps1 (release files, section 9), icon.ps1 (icon rendering),
                update-test.ps1 (updates against a local imitation, section 10)
```

What each crate holds: `bv-core` (translations, settings schema with device lists and its
TOML file format, audio vocabulary, device list model with natural-order learning and
numbered duplicate names and the list edits of the Devices page, volume maths, application rows and the remembered application list, text helpers, Home Assistant entities, topics and payloads, Base64 / hex / SHA-256), `bv-audio` (the Core Audio engine), `bv-ui-kit` (theme resolution,
tokens, motion, icons, vector paths, device icon catalogue, accessibility tree, tray icon,
menu, flyout view, tooltip, bubble and settings window controls with text fields, buttons,
notices and the close question, rendering backend in
`src/backend`), `bv-shell`
(message window and loop, tray icon drawn by the design system, wheel capture, custom
context menu, flyout host, tooltip and bubble popups with their placement and motion, application
icon loading, the list of running programs, UI Automation provider, Windows appearance preferences, single instance, OS
languages, waker, resume and suspend events, log file, Windows sound screens, settings window
host, secrets protected for the Windows user (DPAPI), media keys,
autostart entry, hiding the Windows volume icon through the taskbar helper and its loader
child, our icon's taskbar preference), `bv-taskbar-helper` (section 7), `bv-mqtt` (MQTT 3.1.1 codec, client with reconnection and optional
encryption, test broker behind the `test-broker` feature) and `bv-app` (application state,
settings file store, Home Assistant link and page) hold real code.

Settings: `bv-app/src/store.rs` reads the file at start (defaults if it
is missing or unusable, never blocking) and writes it atomically after every event that
changed the settings (`App::take_dirty`). The settings window follows the flyout's
pattern: `App::settings_content` builds a `SettingsContent`, `Shell::show_settings` /
`update_settings` host the design system's `SettingsView` in a standard window
(`bv-shell/src/settings.rs`), the user's requests come back as `Event::Settings`, and
`Event::SettingsClosed` reports where the window was. The variable
`BETTERVOLUME_DATA_DIR` replaces the data folder for the tools.

The bubble: after every event the handler asks
`App::device_bubble` (the defaults last seen are compared with the engine's; a change gives
a `BubbleContent`) and, after a wheel turn or a middle click, `App::volume_bubble`; it
hands the content to `Shell::show_bubble`. The shell paints it in a passive popup and
moves it with a timer of its own window (`bv-shell/src/bubble.rs`): unlike the flyout
entrance, the UI thread is never held, so wheel turns keep applying while it rises.
The Home Assistant link (`bv-app/src/app/ha.rs`): when the
integration is on and configured, `App::ha_start` builds `bv_mqtt::Options` (address,
encryption, credentials decrypted with DPAPI, the `offline` last will, the command
subscriptions from `bv_core::ha::Identity`) and starts a `bv_mqtt::Client` through the
`Platform` given by `main.rs` (tests give a fake `Link`). The client's threads report
through a channel and the shell's `Waker`; on `Event::Wake` the handler calls
`App::on_ha_events` (connection state, commands applied to the engine, media keys and
the volume bubble returned as `HaEffects`), and after every event `App::ha_sync`
publishes what changed (`bv_core::ha::Publisher` diffs the state built from the same
device list as the flyout, FR-112). `Event::Suspending` and Quit stop the link with an
`offline` farewell, `Event::Resumed` starts it again; turning the integration off removes
every entity. The settings page (`ha_page.rs`) keeps a draft form until "Save" and runs
"Test connection" on a thread of its own (`Platform::test`).

`bv-shell` depends on `bv-ui-kit` (it hosts the design-system controls in its windows)
and on `bv-core` (the release-file format).

How the executable is wired: `bv-app` calls `bv_shell::run(config, init)`; the shell
creates the hidden window, calls `init(&Shell)` once on the UI thread (this is where the
application takes a `Waker` and creates the audio engine), and runs the message loop,
reporting typed `Event`s to the handler `init` returned; the handler receives a `Shell`
handle to act back (menu, tooltip, quit). New features extend the `Event` enum and the
`Shell` handle rather than adding window procedures to `bv-app`. The application logic
itself lives in `bv-app/src/app.rs`, generic over the `bv_audio::AudioEngine` trait, so it
is tested with the in-memory engine (`bv-audio` feature `fake`).

The flyout: `bv-app` builds a `FlyoutContent` (`App::flyout_content`, in
`bv-app/src/app/flyout.rs`) and hands it to `Shell::toggle_flyout` / `show_flyout` /
`update_flyout`; the shell hosts the design system's `FlyoutView`, forwards input to it
and reports what the user asked for as `Event::Flyout(FlyoutAction)`, which the
application applies (`App::flyout_action`) before pushing the new content. Closing, by
any means, is reported as `Event::FlyoutClosed`. The application watches the audio
sessions only while the flyout shows the application list (open, output view, list
expanded).

### Audio engine

- `bv_audio::Engine` lives on the UI thread (COM single-threaded apartment). It keeps a
  snapshot (device lists, defaults per flow and role, endpoint volume of the current
  output and input, sessions) that the accessors of `AudioEngine` return without calling
  Windows.
- Callbacks (`signals.rs`) push a typed `Signal` into an `mpsc` channel and call the
  `Notify` function; the application passes one that calls `bv_shell::Waker::wake`. The
  waker posts one message to the hidden window and coalesces bursts until the UI thread
  acknowledges it; the shell then delivers `Event::Wake`, and the handler calls
  `process_signals`, which drains the channel, re-reads what changed and returns
  `Changes`. Signals carry the *generation* of the registration that produced them, so a
  late notification from a previous device is ignored.
- Roles: "media" = console + multimedia, "calls" = communications; the current
  device of a flow is its console default. Default switching goes through the
  undocumented policy config interface, isolated in `policy.rs`.
- Application sessions are watched only while `watch_sessions(true)` (the flyout, or the
  Applications page of the settings window), so they cost nothing at idle. Each followed
  session also waits on its process through the thread pool (`exit.rs`), so the row of an
  application that ends (even killed) disappears although Windows may keep its session.
  An application is identified by the file name of its executable (`AppKey`, FR-061);
  for processes that cannot be opened, the path is read from the session identifier, and
  their name and icon from that executable, through the `\\?\GLOBALROOT` prefix.
- Activity watch (`activity.rs`): at all times, only the playing
  state of the sessions of the current output is followed (session created, state
  changed). The engine remembers, per process, the output where it was
  last heard playing; published streams carry `heard_here`, and the application list
  leaves out sessions not heard on the current output (`bv_core::apps::rows`). A process
  heard playing for the first time is identified once (executable path and name; one
  that cannot be opened is left to the session list) and handed out by
  `take_heard_apps`, so the Applications page remembers it even while the flyout is
  closed (FR-096).
- Remembered applications (`bv_core::apps::AppList`, settings `[[applications]]` and
  `apps_custom_order`): learnt by `App::learn_apps` after every audio event and when the
  sessions start being watched; `AppList::arrange` gives the flyout its order and
  exclusions. The Applications page (`bv-app/src/app/apps_page.rs`) reads which programs
  run (`bv_shell::running_executables`, a process snapshot taken when the page is built,
  never watched); the settings host loads the row icons into a cache that lives with the
  window.
- Robustness (NFR-05): every Windows failure is a value (`AudioError`), never a panic; a
  "device gone" failure schedules a re-read; the audio service is watched through
  `SubscribeServiceChangeNotifications` (thread pool, no polling) and the engine rebuilds
  itself when it restarts; `Event::Resumed` (power broadcast) triggers a full re-read.

Dependency rule: `bv-core` depends on nothing internal; `bv-ui-kit` depends on nothing
internal; feature code never draws directly and never hard-codes a colour, size, duration
or easing — it asks `bv-ui-kit`. This is what makes a restyle possible without touching
the rest (NFR-04) and is checked in review.

## 5. UI approach

- Retained, minimal view tree of design-system controls with layout in DIPs; re-render
  only dirty windows on state change; hit-testing and keyboard focus handled by the tree.
- Accessibility (FR-024): controls describe themselves as a tree of
  `bv_ui_kit::access::AccessNode`s (role, name, bounds, state, value; no Windows types).
  `bv-shell/src/uia.rs` exposes that tree through Windows UI Automation: a popup answers
  `WM_GETOBJECT` with its `AccessSource`; the providers handed to Windows hold only the
  window and an element identifier and re-read the current tree on every query (an
  element that disappeared answers "not available"); requests of assistive tools come back
  to the control as `AccessAction`s. Providers use COM threading, so every call arrives on
  the UI thread from a message loop (including the menu's nested loop): sources must only
  try-borrow their state and must queue, not run, application work. Focus, property and
  structure events are raised only while an assistive tool listens. The provider is tested
  with the real UI Automation client of the process on a window of its own thread.
- Visual fidelity (materials, shadows, exact motion) is compared side by side against
  the real system flyouts.

## 6. Tray wheel capture (FR-015)

Windows does not deliver wheel events to tray icons. Technique: when the icon reports a mouse
move, register Raw Input for the mouse on the hidden window and poll the pointer position
every 100 ms; while the pointer is over the icon rectangle, wheel deltas adjust the
volume; when it leaves, unregister. No global hook and no cost when the pointer is elsewhere.

## 7. Hiding the Windows volume icon (FR-018)

Tested on Windows build 26300: the
`HideSCAVolume` Explorer policy has **no effect** — per-user or machine-wide, with or
without an Explorer restart. No setting-based method exists on current builds.

Decision: a small in-taskbar helper shipped with the app.

- Separate tiny native library (`bv-taskbar-helper`, own crate, `cdylib`), loaded into the
  taskbar process through the XAML Diagnostics mechanism (`InitializeXamlDiagnosticsEx`),
  the same supported entry point used by published taskbar customisation tools. No
  function hooking, no administrator rights.
- It only locates the system tray volume element in the taskbar's visual tree and
  collapses it; it restores it when asked, when the app exits, or when the setting is off.
- Guard rails (mandatory): off by default; does nothing if the expected element is not
  found (unknown Windows build → Windows icon simply stays); re-applied after Explorer
  restarts; **auto-disables itself** and informs the user if the taskbar process dies
  shortly after loading; never keeps the taskbar from starting; removed on uninstall.
- The main executable stays free of this code: the feature can be dropped or disabled
  without touching anything else.
- Known risks, stated under the setting's switch: may stop working after a Windows update
  until the app is updated; some antivirus products distrust this technique.

### As built

- **Loading.** The application never loads the XAML library itself (footprint): it
  starts itself as a short-lived child (`BetterVolume.exe --taskbar-helper <hide|restore>
  <pid> <window> <helper> <folder>`, `bv_shell::run_loader`). The child copies
  `bv_taskbar_helper.dll` (beside the executable) to `<data folder>\taskbar-helper\`
  under a name made of its content hash (the taskbar keeps a loaded library open until it
  restarts; the copy keeps the installed file replaceable and lets a new version load
  beside an old one), removes older copies not in use, finds the taskbar process
  (`Shell_TrayWnd`) and calls `InitializeXamlDiagnosticsEx("VisualDiagConnection<N>", pid,
  NULL, copy, CLSID, "<command> <owner pid> <owner window>")`, trying N = 1, 2, … while the
  answer is `ERROR_NOT_FOUND` (one endpoint per XAML core). No COM registration is needed:
  the XAML host loads the library by path and calls its `DllGetClassObject`.
- **Copies left behind.** The taskbar holds every copy it loaded until it restarts, so
  one copy per version loaded since the last restart of Windows (or Explorer) stays; it
  is deleted by the next load after that restart, or, while the volume icon is not to be
  hidden (no loader runs), at the application's start
  (`bv_shell::remove_unused_helper_copies`, which also removes the empty folder). A copy
  changed less than a minute ago is in a loader's hands (perhaps of another copy of
  BetterVolume sharing the folder) and is left alone; a reused copy is marked as just
  used before it is checked. A temporary file left by a loader that ended halfway goes
  on the same terms. After an uninstall, a copy still held stays until removed by hand.
- **In the taskbar** (`bv-taskbar-helper`): `SetSite` reads the instruction, a thread of
  its own subscribes to the visual tree (the whole existing tree streams during the call),
  waits up to 10 s more only if the icon was not there yet, then unsubscribes: a
  connection lasts a fraction of a second normally, which keeps the diagnostics' element
  references and the XAML composition diagnostics (found unsafe with several UI threads by
  published tools) short-lived. Each load costs the taskbar about 30 KB (measured: 20 loads,
  +0.6 MB private memory in Explorer).
- **Identification** (language-independent): a `SystemTray.IconView` named
  "SystemTrayIcon" whose font content (`SystemTray.TextIconContent` > `TextBlock`
  "InnerTextBlock") draws one of the Windows volume glyphs (E74F, E992–E995, EA85, EBC5),
  the rule of the published tray tweak tool. Nothing else is touched if it is not found.
- **Hiding:** the icon's `Visibility` is set to Collapsed through
  `IVisualTreeService::SetProperty` (on the element's own UI thread, inside the callback).
  The stack of quick-settings icons keeps its spacing next to the empty slot (a gap of
  4 DIP); the stack's padding on that side is reduced by the spacing, only while it is
  symmetric (Windows' own value, set in its markup), and given back on restore only when it
  is exactly the spacing short: repeated commands never shift anything. Negative margins on
  a neighbour were tried and have no effect.
- **Restoring:** clears the helper's Visibility value (Windows' own applies again) and
  gives the padding back. Triggered by the switch, by quitting, and by the helper itself:
  while the icon is hidden it watches the BetterVolume process (thread-pool wait on its
  handle) and, when it ends in any way, loads itself again inside the taskbar with
  "restore" (crash, Task Manager, uninstall).
- **Outcomes** come back to the message window as the registered message
  "BetterVolume.TaskbarHelper" (hidden, restored, not found, failed; `LPARAM` = taskbar
  process id), or from the thread that waited for a loader child that failed; a failure is
  retried twice, 3 s apart. "Not found" or "failed" while wanted = the switch stays on and
  says the icon cannot be hidden on this Windows.
- **Taskbar re-created** (`TaskbarCreated`): loaded again 3 s later. The taskbar is
  watched from the moment a hiding load starts (a crash while the helper works in it
  counts). If it ended within 30 s of the load with a crash code (NTSTATUS error, not the
  0, 1 or -1 of a process ended on purpose), or ended that soon for the second time in a
  row (whatever the code), the shell stops and reports an incident instead: the option turns off and the settings window opens on the
  General page, scrolled to the notice under the switch.
- **Our own icon**: turning the option on also sets `IsPromoted` = 1 in Windows'
  record of this executable's tray icon (`HKCU\Control Panel\NotifyIconSettings\*`, matched
  by executable path, known-folder prefixes resolved, and icon id); the taskbar follows it
  at once.
- **Release profile:** `panic = "abort"`, so a panic in the helper would end the taskbar;
  the helper never unwraps or indexes data it receives.
- **Trust:** the library the taskbar loads is a copy in the user's own data folder, which
  any program running as the user can write; such a program could equally load its own
  code into the taskbar, so this opens nothing new (accepted).

## 8. Testing strategy

- `bv-core`: exhaustive unit tests (device model rules, settings migration, MQTT payloads).
- `bv-audio`, `bv-mqtt`: integration tests behind traits with fakes; a live smoke test on
  the dev machine (real devices). `bv-mqtt` and the application link are tested against
  the test broker of `bv-mqtt` on the loopback address (plain and encrypted). For
  audio:
  `crates/bv-audio/tests/live.rs` holds read-only live tests, run by every `check.ps1`
  (they only open silent streams, in the test process or in a child test process it
  starts, and cope with a machine without audio devices), and state-changing ones
  (`#[ignore]`: default switch, volume, mute, all restored) run by `tools/audio-live.ps1`
  after a change of the audio engine.
- `bv-ui-kit`: unit tests of layout and behaviour (menu), theme resolution and motion; pixel
  tests through the real software rasteriser where a defect class was found (tray icon
  badge, every Windows scaling); the gallery example renders every surface for visual
  validation (`docs/spec/DESIGN_SYSTEM.md` §7). Whole-image ("golden") comparisons were
  dropped.
- App level: scripted smoke test (start, open flyout via message, measure, quit) on every run.
- `cargo clippy -D warnings`, `cargo fmt --check`, zero `unsafe` outside `bv-audio`,
  `bv-shell`, `bv-taskbar-helper`, `bv-ui-kit` rendering backends (compiler-enforced).
- All of the above run with one command, `tools/check.ps1`, which is also what CI runs
  (`.github/workflows/ci.yml`).

## 9. Packaging and installation

One executable plays every role; what it does is decided at start (`bv-app/src/main.rs`):

| Started as | Role |
|---|---|
| `--taskbar-helper …` | loader child of the taskbar helper (§7) |
| `--uninstall` | the uninstaller window (`bv-app/src/setup.rs`) |
| the setup file (trailer of kind "setup") | the installer window (`setup.rs`) |
| the portable file (trailer of kind "portable") | the application, settings and log beside the file (or in the profile when that folder cannot be written) |
| anything else (installed copy, development build) | the application, settings and log in `%LOCALAPPDATA%\BetterVolume` |

Release files: `[program][taskbar helper library][32-byte trailer]`, format and version
ordering in `bv_core::package` (one definition, unit-tested); `bv_shell::Package` reads
this executable as such a file; the bv-core example `package` writes and reads back the
two files, run by `tools/package.ps1` (gate of `tools/check.ps1`, output in
`target/package/`). The portable file's loader child reads the helper from its own
payload (`embedded` helper argument) and writes its copy to the profile folder.

Installation (`bv-shell/src/install.rs`): files written beside then renamed into
place; Start-menu entry through `IShellLinkW`; per-user installed-apps entry; autostart
entry for the installed program; a running copy is asked to quit through the registered
message `BetterVolume.Exit` (`Event::ExitRequested`, handled like Quit) and ended after
10 s. Uninstall: entries removed, then the two installed files and the empty folder by a
hidden `cmd` started as the uninstaller exits; with the settings deleted, this PC is
first removed from Home Assistant (`app::ha::remove_at_uninstall`) and the profile folder
deleted.

A first installation shows the planned folder with "Change…" (`Shell::choose_folder`,
Windows' folder picker opened from a wake-up, never while the window handles input); the
choice gets a `BetterVolume` sub-folder and is refused when this user cannot create a
file there (`bv_shell::folder_usable`); an existing installation keeps its folder.
At the first start of each copy the tray icon is promoted on the taskbar when Windows'
record of it holds no preference yet (`tray.rs` `promote(true)`, looked for again every
3 s, five times).

The installer and uninstaller window is the settings window at `COMPACT_SIZE` with no
navigation entries (`Shell::show_compact_window`); the shell runs without tray icon
(`ShellConfig::tray = false`) and with a message window class of its own. The window
logic (`setup::Setup`: phases, content, effects) is pure and unit-tested; the work runs
on a thread and reports through the shell's waker.

Resources (`bv-app/build.rs`): the application icon (`assets/icon/BetterVolume.ico`,
rendered from two SVG drawings by `tools/icon.ps1`) and the version information are
written as a compiled `.res` file the MSVC linker takes directly.

## 10. Updates (FR-132)

| Part | Where |
|---|---|
| Release answer, offer rule, line rule, file check, daily rhythm | `bv_core::update` (pure, unit-tested), JSON reader `bv_core::json` |
| HTTP (`GET` through WinHTTP, delay-loaded) | `bv_shell::http` |
| Timer of the next check (`Shell::set_update_timer`, `Event::UpdateDue`), opening a release page, starting a file with arguments, waiting for a process, local date and time | `bv-shell/src/update.rs` |
| State: checking, offered, downloading, installing, failed; flyout line; About cards; the updated bubble | `bv-app/src/app/update.rs` (`UpdatePlatform` given by `main.rs`, fakes in tests) |
| Project page link of the About page (FR-095): the repository the release build names, `None` otherwise | `updater::project_page`, `App::set_project_page`, `SettingsEffect::OpenProjectPage` opened by `main.rs` through `bv_shell::open_web_page` |
| Threads: fetch, download and check of the file; where releases are looked for; portable file swap; leftovers | `bv-app/src/updater.rs` |
| Local imitation of the release service | `bv-app` example `release-server` (built by `tools/check.ps1`), driven by `tools/update-test.ps1` |
| Release automation | `.github/workflows/release.yml` (version tag → gates → draft release with the two files) |

Flow: `main.rs` gives the application an `UpdatePlatform` when `updater::source()` names a
release service (the repository compiled into release builds, or the loopback imitation).
After every event the handler takes the timer (`App::take_update_timer`) and an effect
(`App::take_update_effect`). Answers of the check and download threads arrive through
channels and the shell's `Waker`, drained by `App::on_update_events` on `Event::Wake`.

Installing: the installed copy (`bv_shell::running_installed`) downloads the setup file to
`%TEMP%\BetterVolume-update` and starts it with `--update`: the setup window starts the
installation at once, ends the running copy with the exit message, installs and
starts the new program. A portable copy whose folder can be written downloads the portable
file as `<name>.new`, renames itself `<name>.old`, puts the new file in its place, starts it
with `--replaced <pid>` and quits; the new copy waits for that process before taking the
single-instance mutex, then removes `.old` / `.new` and the temporary download folder
(`updater::clean_up`, at every start). Any other copy (development build, read-only
folder, or a release without the needed file) opens the release page instead. The first
start of a newer version shows the "updated" bubble (`[updates] last_version`).
