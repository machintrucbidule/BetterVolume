# Functional specification — BetterVolume v2

Requirements are numbered so that code, tests and documents can reference them. "Output" = playback device, "input" = microphone.
Wording: **must** = required for v1.

## 1. General

- **FR-001** The app runs as a single background instance per Windows session; launching it
  again brings up the flyout of the running instance instead of starting a second one.
- **FR-002** No taskbar button and no main window; the only permanent presence is the tray icon.
- **FR-003** Every state shown anywhere (tray, flyout, settings, Home Assistant) is updated
  live when it changes from any source (the app, Windows, another program, Home Assistant).
  No manual refresh exists.
- **FR-004** Appearance follows Windows live: light/dark mode, accent colour, text scaling,
  display scaling (per-monitor), reduced-animation and high-contrast preferences.
- **FR-005** Errors never surface as raw technical dialogs. Recoverable problems are shown
  in context in plain language; others are logged to a local log file.

## 2. Tray icon

- **FR-010** The icon shows the master volume of the current output with the same visual
  steps as Windows 11 (muted, 0, low, medium, high).
- **FR-011** Picture, selectable in settings: "speaker" (default) or "device": the
  speaker part of the icon is replaced by a larger drawing of the type of the current
  output, the waves made smaller to give it room and still showing the level.
- **FR-012** Tooltip: current output display name and volume, or "muted", with each
  language's typography: `Speakers: 40%` / `Haut-parleurs : 40 %`.
  A name too long for the tooltip is shortened with an ellipsis; the volume always stays
  visible.
- **FR-013** Left click toggles the flyout (open if closed, close if open).
- **FR-014** Right click opens the context menu (section 6).
- **FR-015** Mouse wheel over the icon changes master volume by the configured step
  (FR-073), without opening the flyout. Changing volume this way unmutes, like Windows.
- **FR-016** Middle click toggles output mute.
- **FR-017** Wheel and middle-click actions show the bubble (section 7) with the new level,
  so the user gets feedback without the flyout: a copy of the Windows 11 volume indicator
  (volume glyph of the level step or muted, level bar, number), without the device name.
  Shown even when the volume is already at an end; can be turned off (FR-081).
- **FR-018** The app can hide the built-in Windows volume icon itself, through a reversible
  setting (off by default, offered on first run). It must use the most reliable method
  available, request administrator rights at most once if needed, never ask the user to
  perform manual steps, restore the Windows icon when the setting is turned off or the app
  is uninstalled, and fail safely (Windows icon simply stays) if a Windows update breaks the
  method. Method: in-taskbar helper with guard rails, see ARCHITECTURE §7 (no administrator rights needed; auto-disables on incident).
  - First-run offer: a dialog over the settings window when it opens on the first
    launch ("Hide the Windows volume icon?", "Hide the Windows icon" / "Keep both";
    dismissing it = keep both). Asked once; installations older than this question are
    asked once at their next start.
  - Turning it on (switch or first-run answer) also shows BetterVolume's own icon directly
    on the taskbar rather than behind the hidden-icons arrow, at that moment only.
  - The Windows icon comes back whenever BetterVolume is not running (quit, crash, ended by
    the user) and is hidden again at each start and after each taskbar restart.
  - The risks are explained under the switch, always.
  - Cannot hide (unknown taskbar layout, load refused): the switch stays on and says so;
    BetterVolume retries at each start and taskbar restart.
  - The taskbar crashes right after hiding: the option turns off and the settings window
    opens on the General page with a notice under the switch.

## 3. Flyout panel

### 3.1 Window behaviour
- **FR-020** Small floating panel in Windows 11 style, anchored to the tray icon's corner of
  the screen, on the monitor holding the taskbar; respects taskbar position and auto-hide.
- **FR-021** Opens and closes with the standard Windows 11 flyout animation; closes on focus
  loss, on Escape, and on a second click on the tray icon.
- **FR-022** Opening is perceived as instant (target defined in the architecture budget).
- **FR-023** Panel height, chosen in the settings (FR-082):
  - *Whole screen height*: the panel takes the whole height of
    the work area, with the Windows 11 gaps; it never changes size and the tiles never
    move, whatever the device, the tab or the applications. The free space lies between
    the tiles and the application list, which stands right above the master volume.
  - *Fits the content* (default): the height follows the content (changes animated), the
    list right below the tiles.
  In both, only the application list scrolls, and only when it does not fit (in the space
  left, or on the screen); tiles and master volume always stay visible.
- **FR-024** Fully usable with keyboard (Tab/arrows/Space/Enter) and exposed to screen readers.

### 3.2 Layout (top to bottom)
- **FR-030** View switch with two tabs, *Output* and *Microphone*. Shown only when microphone
  management is enabled. The flyout always opens on *Output*.
- **FR-031** *Output* view: device tiles at the top → free space → application volumes
  (collapsible) → master volume at the very bottom.
- **FR-032** *Microphone* view replaces the whole content: input device tiles → microphone
  level at the bottom. No application list.
- **FR-033** Footer: shortcut to Windows sound settings and a button opening BetterVolume settings.

### 3.3 Device tiles (outputs and inputs)
- **FR-040** One large tile per visible, available device: icon above display name, at
  most three tiles per row, rows balanced and filling the width (4 = 2 + 2,
  5 = 3 + 2, 7 = 3 + 2 + 2), Windows 11 styling. Long names wrap on two lines then are
  ellipsised (full name in tooltip). Before the user orders them (settings, FR-085),
  devices first seen together are in natural alphabetical order.
- **FR-041** The current device's tile is highlighted (accent). Clicking another tile makes
  it the current device for the configured scope (FR-072) and the highlight moves immediately.
- **FR-042** Tile order, names and icons come from settings (section 5.2).
- **FR-043** Unavailable devices (unplugged, disabled, powered off) have no tile; they
  reappear at their configured position when available again.
- **FR-044** A never-seen device gets a tile immediately, at the end, with its Windows name
  and an automatically chosen icon.
- **FR-045** A hidden device that becomes the current device (chosen from elsewhere) is shown
  temporarily, last, for as long as it is current.
- **FR-046** If the "media" and "calls" defaults differ, the highlighted tile is the media
  one and the calls one carries a discreet marker (handset in the corner, tooltip "Used
  for calls"). With the scope "media and calls", a click on the highlighted tile
  brings calls back to it. The flyout stays open after a click.
- **FR-047** No device available: the tile area shows a short "no device" message; volume
  controls are disabled.

### 3.4 Master volume / microphone level
- **FR-050** Row = mute button (icon reflects level/mute) + slider 0–100 + numeric value,
  preceded by the current device's display name.
- **FR-051** Slider: click anywhere jumps there; drag is continuous and applied live; mouse
  wheel changes by the configured step; keyboard arrows by 1, PageUp/Down by 10.
- **FR-052** Moving the slider while muted unmutes (Windows behaviour).
- **FR-053** When the current device changes, the row immediately shows the new device's
  volume and mute state.

### 3.5 Application volumes
- **FR-060** Collapsible section; expanded/collapsed state is remembered across openings and
  restarts.
- **FR-061** Lists every application that plays on the current output, or whose sound was
  last heard on it, plus "System sounds". One row per application, named after
  its executable (same name on every device): multiple streams of the same application
  are grouped and controlled together. An application is recognised by the file name of
  its executable (case ignored), not its folder: a program that moves to a new versioned
  folder at each update stays the same application, and two programs with the same file
  name are one. When the output changes, an application last heard on the previous output
  that comes to the new one (as most applications do, sometimes a moment after the switch,
  for example on a device just plugged in) keeps its row there at once, without waiting to
  be heard again.
- **FR-062** Row = application icon (click toggles mute for that app) + friendly name +
  slider + numeric value. Same slider behaviour as FR-051.
- **FR-063** Rows appear and disappear live as applications start/stop using audio, without
  moving the control currently under the pointer while it is being dragged.
- **FR-064** Stable ordering: the order of the Applications page (FR-098). Until the user
  moves a row there, the list stays alphabetical by friendly name, System sounds last, and
  an application never seen before takes its alphabetical place; from the first move the
  order is the user's and a new application goes at the end (just before System sounds
  while System sounds is last). Applications excluded on that page are not listed
  (FR-097).
- **FR-065** No application: the section shows only "System sounds" (when Windows has no
  System sounds session either, there is nothing to control: only the section header
  shows).

## 4. Device identity rules

- **FR-066** A device is recognised by its stable Windows identifier, so custom name, icon,
  order and visibility survive unplug/replug and restarts.
- **FR-067** Automatic icon choice uses the device type declared by Windows (speakers,
  headphones, headset, display/TV, microphone…), with a generic fallback.
- **FR-068** Icon catalogue for manual choice: a large, visually consistent set drawn in a
  single style matching Windows 11 icons, covering at least: desktop speakers, bookshelf
  speakers, soundbar, AV receiver/amplifier, subwoofer, portable speaker, wired headphones, Bluetooth
  headphones, gaming headset, headset with microphone, wired earbuds, true-wireless earbuds,
  bone-conduction headphones, monitor, TV, projector, laptop, phone, VR headset, desk
  microphone, studio microphone, lavalier microphone, webcam, audio interface, virtual
  device, generic output, generic input.

## 5. Settings window

- **FR-070** Standard resizable window in Windows 11 Settings style: navigation on the left
  (General, Devices, Applications, Home Assistant, About), cards on the right. Changes apply immediately
  (no global Save button), except the MQTT connection form which has Test and Save.
  At any width and in any language nothing is cut: in a narrow window each setting moves
  under its text, as in Windows 11 Settings, and long titles wrap.
- **FR-071** Opens automatically on the very first launch only; later via the context menu
  or the flyout footer. Exceptions: once at the next start of an installation that has not
  answered the FR-018 question yet, and after a guard-rail incident.

### 5.1 General

The page is in four titled groups: "Startup and language" (FR-074, FR-075), "Devices"
(FR-072, FR-076), "Icon next to the clock" (FR-078, FR-079, FR-073, FR-081) and "Panel
and notifications" (FR-082, FR-077).

- **FR-072** Device-switch scope: "media and calls" (default) / "media only" / "calls only".
- **FR-073** Wheel step: 1–10, default 2. Applies to tray icon and all sliders.
- **FR-074** Start with Windows: on/off (default on for the installed version; the installer
  also offers it).
- **FR-075** Language: "Automatic" (OS display language, English fallback, FR-121) or an
  explicit language from those shipped, each in its own name, in alphabetical order of
  those names as the Windows language list (Latin scripts first, then Japanese and
  Chinese). Changing it applies without restart.
- **FR-076** Microphone management: on/off (default on). Off removes the tabs, the inputs
  list in settings, and all microphone entities in Home Assistant.
- **FR-077** Device-switch notification: on/off (default on).
- **FR-078** Tray icon, one card of picture choices: three groups side by side (size
  FR-083, picture FR-011, look FR-084), two tiles each; every tile shows the icon as it
  would be with that choice and the two others, at the current level and device, on a
  strip of the taskbar colour. Applied at once.
- **FR-079** Hide the Windows volume icon (FR-018): off by default; the description under
  the switch always states the risks.
- **FR-080** Check for updates automatically: on/off (default on). Shown on the About
  page with the update status, not on the General page; "Check for updates" there
  works also when it is off. Beside it, "Receive preview versions" (off by
  default).
- **FR-081** Volume bubble of the wheel and middle click (FR-017): on/off (default on).
- **FR-082** Flyout height: "Fits the content" (default) or "Whole screen height" (FR-023).
  A choice already saved is kept.
- **FR-083** Tray icon size: "Normal" (default, the size of the Windows icon) or "Large",
  for every picture (FR-011) and look (FR-084), applied at once. The Windows volume
  glyph already spans the whole width of the square Windows gives a notification-area
  icon (the loudest step touches both sides) and Windows shows a larger picture scaled
  down, so "Large" grows in height only: the same drawing stretched vertically about
  its middle until the loudest step
  fills the square (DESIGN_SYSTEM §3), strokes and widths unchanged, nothing cut.
- **FR-084** Tray icon look: "Windows" (default, the Windows volume icon in the
  taskbar's text colour) or "BetterVolume": the speaker of the application icon (without
  its two roads) and the waves in a gradient of the Windows accent colour, following
  accent changes at once, the devices of FR-011 as filled drawings in that gradient. In
  high contrast the icon keeps the Windows look. A choice already saved is kept.

### 5.2 Devices
- **FR-085** Two lists (outputs; inputs when microphone management is on), each row: drag
  handle, icon, custom name, Windows name + availability as secondary text, rename, show/hide.
- **FR-086** Reorder by drag-and-drop and by keyboard. Order applies to flyout and Home Assistant.
- **FR-087** Rename inline; empty name restores the Windows name. Duplicate display names
  are refused with an inline message (they must be unique for Home Assistant). Devices
  that end up with the same name without being renamed (two identical devices) are
  numbered automatically everywhere, in list order: "Headset", "Headset (2)".
- **FR-088** Icon picker opens from the row's icon (catalogue FR-068) with a "back to
  automatic" choice.
- **FR-089** Known but currently unavailable devices stay listed (marked as disconnected)
  so they can be configured, under the last name Windows gave them; each can be forgotten
  manually, after a confirmation.

### 5.6 Applications (shown in the navigation between Devices and Home Assistant)
- **FR-096** One list of every application that has played sound on this PC, remembered
  (its display name and executable only), plus "System sounds". An application is
  remembered as soon as it is heard playing on the current output, even while the flyout
  is closed, and when the flyout or this page shows its session. Each row: drag handle,
  icon, name over "executable file name · In use / Closed" (whether a program with that
  file name runs now), show/hide. Any remembered application can be forgotten, after a
  confirmation (System sounds cannot), also while it runs: it is then new again as soon
  as it plays (hiding is the way to stop seeing one).
- **FR-097** Hiding ("excluding") an application removes it from the flyout's application
  list only: its sound and volume are not touched. System sounds can be hidden too.
- **FR-098** Reorder by drag-and-drop and by keyboard; the order applies to the flyout
  (FR-064). Default order and new applications: FR-064. A "Sort alphabetically" button
  above the list asks for a confirmation, then puts the list back in alphabetical order
  and in the automatic mode of FR-064.
- **FR-099** Remembered applications are kept in the settings file only; they are never
  published (Home Assistant has no per-application control) nor written to the log.

### 5.3 Home Assistant
- **FR-090** Enable switch; broker address, port, optional encrypted connection, user name,
  password, display name of this PC (default: computer name), discovery prefix (default
  `homeassistant`). The server form applies on "Save"; closing the window with
  unsaved changes asks first.
- **FR-091** "Test connection" gives a plain-language result (success, unreachable, refused
  credentials, encryption problem). Live connection status is always displayed. A
  certificate Windows does not trust can be trusted explicitly for this server.
- **FR-092** The password is stored encrypted for the current Windows user and never shown
  back, logged, or exported.

### 5.4 About
- **FR-095** Version, licence, link to the project page, update status with a "check now"
  button, link to the log folder. Update status: up to date (with the time of the last
  check), checking, could not check, "Version X available" with "Install" and a "See
  what's new" link to its release page, downloading (percent), installing, or
  failed with "Try again"; then the two update switches (FR-080). A copy built without a
  release service (development build) says updates are not available for it and shows no
  project page link: the link is the repository the release build was made from.

## 6. Context menu

- **FR-100** Items: Sound settings (Windows) · Volume mixer (Windows) · Classic Sound panel
  (Windows) · — · BetterVolume settings · — · Quit. Windows 11 menu styling.

## 7. Device-switch bubble

- **FR-105** Small custom bubble, Windows 11 volume-indicator style, at the bottom centre of
  the screen (just above the taskbar when it is at the bottom), where Windows shows its own
  volume indicator whatever the taskbar position:
  device icon + display name + kind ("Audio output" / "Microphone", with "· calls" when
  only the calls device changed). Appears on every change of a default device
  (output, and microphone while microphone management is on; media or calls) whatever the
  source, also over full-screen applications and in "Do not disturb"; lasts about
  2 seconds after its last update, never takes focus, lets clicks through, never plays a
  sound, is not stored in the notification centre. The devices current at start are not
  announced. When the output and the microphone change at the same moment (one
  headset plugged in), the output is announced; a bubble always names one device.
- **FR-106** Not shown while the flyout is open (the flyout shows the change; this covers a
  switch made from a tile). Rapid successive changes replace the bubble content instead of
  stacking; the device and volume bubbles share one place.

## 8. Home Assistant integration (MQTT)

- **FR-110** Uses MQTT discovery: entities appear in Home Assistant automatically, grouped
  under one device per PC, named after the configured PC name. Several PCs can coexist.
- **FR-111** Entities:
  - Output device — select (drop-down) listing display names in configured order.
  - Output volume — number 0–100, in increments of 1 (slider).
  - Output mute — switch.
  - Microphone device / level / mute — same three, only when microphone management is on.
  - Media: play/pause, next track, previous track — buttons (act like keyboard media keys).
- **FR-112** Select options mirror the flyout exactly: hidden devices absent, unavailable
  devices absent, temporary entry for a hidden-but-current device (FR-045). Options update
  live when devices come/go or settings change.
- **FR-113** Two-way and live: a change on the PC is reflected in Home Assistant within a
  fraction of a second, and commands from Home Assistant are applied immediately.
- **FR-114** Availability: entities become "unavailable" when the PC is off, asleep, the app
  is closed, or the connection drops (including unclean disconnection, detected in about
  45 s). Exception: the output select, output volume and output mute stay
  usable; the last order of each is applied as soon as the PC is connected again.
- **FR-119** A volume or mute change ordered from Home Assistant shows the volume bubble
  (FR-017) unless it is switched off (FR-081); a device switch shows the device bubble.
  Entity names follow the application's language.
- **FR-115** Automatic reconnection with increasing delay; no user action needed after a
  broker restart, network loss, or resume from sleep. State is re-published on reconnect.
- **FR-116** Entity identifiers are stable: renaming the PC or a device changes labels, not
  identifiers. Disabling the integration or microphone management removes the corresponding
  entities from Home Assistant cleanly.
- **FR-117** Invalid commands (unknown device name, out-of-range value) are ignored and logged.
- **FR-118** With the integration disabled (default), the app makes no network connection
  for it and its footprint is unaffected.

## 9. Languages

- **FR-120** Ships with English and French; version 2.1.0 adds German, Spanish, Italian,
  Portuguese (Brazil), Dutch, Polish, Japanese and Simplified Chinese. All user-visible
  text comes from translation files; adding a language requires only a new file. Texts
  use the terms Windows 11 uses in each language. Japanese and Chinese are drawn with the
  fonts Windows uses for that language.
- **FR-121** Automatic mode picks the first OS display language that a shipped language
  serves, falling back to English. A regional variant is served by its language (`fr-CA`
  by French, any Portuguese by Portuguese (Brazil)), never by one written in another
  script: Traditional Chinese (Taiwan, Hong Kong, Macao, `zh-Hant`) gets English, not
  Simplified Chinese.
- **FR-122** Custom device names and application names are never translated.

## 10. Settings storage

- **FR-125** Installed version: settings in the user's profile. Portable version: a settings
  file next to the executable, falling back to the user's profile if that folder is not writable.
- **FR-126** A corrupt or unreadable settings file never prevents startup: defaults are used
  and the bad file is kept aside.

## 11. Distribution and updates

- **FR-130** Windows 11 only. Deliverables, two distinct files per release: an
  installer (Start-menu entry, no desktop shortcut, clean uninstall, per-user, no
  administrator rights needed; the folder can be chosen at a first installation)
  in Windows 11 style, home-made with the design system, and a portable single file that never asks to be installed. First official
  version: 2.0.0.
- **FR-131** Uninstall removes the autostart entry and restores the Windows volume icon if
  the app had hidden it; it offers to keep or delete settings. Deleting the settings also
  removes this PC from Home Assistant when the link is configured.
- **FR-132** Update check (if enabled): at most once a day against the project's public
  releases, sending no personal data. When a newer version exists, a discreet line at the
  bottom of the flyout offers to install it, and the About page shows it; the line
  can be closed and then comes back only for an even newer version. Nothing is installed
  without the user's consent. The portable version replaces its own file and restarts,
  or opens the download page when its folder cannot be written. The installed
  version downloads the setup file and starts it, which updates at once without asking
  again. After an update, the bubble says "BetterVolume updated · version X"
  once. Automatic checks: about 2 minutes after start when a day has passed since
  the last answer, then daily; after a failed automatic check, again an hour later.
  A downloaded file is used only if its size, its SHA-256 digest (as GitHub
  gives it) and its kind (setup or portable) are the ones announced.
- **FR-133** Releases are not code-signed initially; the documentation explains the
  Windows first-run warning.

## 12. Quality requirements

- **NFR-01** Idle footprint (flyout closed, Home Assistant on): RAM and CPU budgets fixed
  in `docs/ARCHITECTURE.md` from measurements; idle CPU must be effectively zero
  (event-driven, no polling).
- **NFR-02** Flyout opening and all interactions feel instant; slider dragging has no
  perceptible lag.
- **NFR-03** Visual and animation fidelity to Windows 11 is a first-class requirement:
  metrics, colours, typography, materials and motion are matched against the real system.
- **NFR-04** The design system is isolated: restyling does not touch feature code.
- **NFR-05** The app survives without crashing: device removal during any operation, audio
  service restart, sleep/resume, display changes, Explorer restart (tray icon is restored).

## 13. Out of scope for v1

Global keyboard shortcuts · per-application volume in Home Assistant · routing individual
applications to different devices · Windows 10 · right-to-left languages ·
code signing · silent auto-update.
