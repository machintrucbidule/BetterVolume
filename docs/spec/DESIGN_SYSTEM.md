# Design system

How BetterVolume looks and moves: the tray icon, context menu and flyout frame, the flyout
controls (sections 8 to 14), the device icon catalogue (15), accessibility (16), the
settings window (17) and the bubble (18).

Everything here lives in `crates/bv-ui-kit` and nowhere else (NFR-04): feature code asks
the design system for a themed control and never hard-codes a colour, size, duration or
easing. Restyling = changing this crate.

## 1. Sources of truth

| Concern | Where | Source of the values |
|---|---|---|
| Colours | `theme.rs` (`Theme::shell`, `Theme::app`) | WinUI 3 Fluent theme resources, written in their `#AARRGGBB` notation |
| Metrics, type ramp | `tokens.rs` | WinUI 3 control templates (menu, flyout) |
| Motion | `motion.rs` | Windows 11 easing curves (decelerate 0,0,0,1; accelerate 1,0,1,1) |
| Icons | `icons.rs` (glyphs), `controls/tray.rs` (tray badges), `controls/device_icon.rs` (catalogue) | Segoe Fluent Icons (the system's own font); own drawings in its style |
| Flyout controls | `controls/flyout/` (content, layout, painting, input, accessibility), `controls/tooltip.rs` | WinUI 3 control templates (slider, toggle button, selector bar), Windows 11 quick settings |
| Accessibility | `access.rs` (tree), exposed by `bv-shell/src/uia.rs` | UI Automation control types and patterns |
| Text fonts | `tokens.rs` (`TEXT_FAMILY`), `Graphics::set_text_locale` | Segoe UI Variable; characters it lacks come from Windows' font fallback chosen by the interface language (the Japanese fonts for Japanese, the Chinese ones for Chinese: the same characters are drawn differently) |

## 2. Theme resolution (FR-004)

Inputs are read by `bv-shell` (`appearance.rs`) at start, on every Windows setting-change
broadcast, and before each popup opens: system mode, apps mode, accent palette (the eight
shades Windows computes), transparency effects, high contrast and its system colours,
animation effects, text scaling.

- **Shell surfaces** (tray icon, flyout, menu, bubble) follow the *system* mode, like the
  system flyouts next to them. **Application windows** (settings) follow the *apps* mode.
- Accent fills use the Windows shades: `light2` in dark mode, `dark1` in light mode; the
  Windows default blue when none is set. Text on accent: black in dark, white in light.
- **Material**: translucent system backdrop (DWM transient window) plus a tint layer
  (dark: `#202020` at 45 %, light: `#F3F3F3` at 35 %) approaching the darker material of
  the system flyouts. Opaque (`#202020` / `#EEEEEE`) when transparency effects are off.
- **High contrast**: only the user's system colours, opaque surfaces, solid outlines; a
  black high-contrast theme counts as dark.
- **Text scaling** 100–225 % enlarges text and the rows holding it; icons keep their size.
- **Animation effects off**: popups appear and disappear without motion.

## 3. Tray icon (FR-010, FR-011, FR-083, FR-084)

- Size: the Windows small-icon size at the taskbar's scaling (16 px at 100 %), drawn on a
  16-DIP canvas. Colour: white on a dark taskbar, near-black on a light one; dimmed when
  no output device exists.
- Steps: muted (speaker with a cross), 0 %, 1–33 % (one wave), 34–66 % (two), 67–100 %
  (three), as Windows 11. Below full volume the waves not reached are drawn too, in
  the "disabled text" colour (`#5DFFFFFF` dark, `#5C000000` light, GrayText in high
  contrast), under the reached ones.
- Device drawings (FR-011): speakers, headphones, headset, display, filled (the speaker
  cabinet with its drivers cut out, a thick headband and filled cups, a full screen on
  its stand), always in their natural proportions (`tray::proportions`: cabinet 1.4 times
  as tall as wide, headphones square, screen 0.62 plus a 0.28 stand), never taller than
  the Windows speaker (11.24 DIP), centred on y = 8. Other device types keep the speaker.
  A translucent colour (the light taskbar's `#E4000000`, the dimmed one) is applied to
  a drawing as a whole (`Canvas::with_opacity`), so its overlapping parts stay even.
- Look "Windows": the Windows glyph in one colour. With the picture "device", the glyph
  is unchanged (its waves or cross at their own size, cut at x = 7.55 DIP, the middle of
  the gap) and only its speaker (x 0–7 DIP) is replaced by the device drawing, 7 DIP
  wide, in the glyph colour.
- Look "BetterVolume" (FR-084): our own waves (`tray::waves`): three arcs of ±56° about a
  centre on y = 8, stroke 1 DIP (the Windows one), 1.45 DIP apart (closer, they merged
  into one stroke at the real size), the outer one 0.2 DIP from the right edge; the
  cross of the muted level is two 1-DIP strokes there. Left of them, 1 DIP away: with the
  speaker, the speaker of the application icon (its 16–32 px drawing, without the two
  roads), 7.6 DIP wide; with a device, the arcs start smaller (first radius 1.4 instead
  of 1.9) and the device drawing gets 8.28 DIP, more than half the icon. Everything is
  painted with one vertical gradient of accent shades, from y = 2 to y = 14 (`TrayColors::accent`):
  accent light 3 to light 1 on a dark taskbar, accent dark 1 to dark 3 on a light one;
  the waves not reached are a pale accent (accent light 2 at 32 % on a dark taskbar,
  dark 1 at 32 % on a light one). Without a device the icon is dimmed as in the Windows
  look. High contrast draws the Windows look (system colours only).
- Pixel tests, at every Windows scaling: nothing of the Windows speaker remains next to a
  device; a column of nothing between the picture and the waves (from 150 %); the
  Windows look with a device keeps the Windows waves exactly; our three waves are three
  separate strokes across the middle row (from 150 %); the device proportions; the
  BetterVolume device wider than half the icon; the colours follow the accent, the
  gradient, the pale waves, the application speaker at least as large as the Windows
  one, and the high-contrast fallback.
- Size "Large" (FR-083): the drawing above (any picture and look) stretched vertically by 20/17
  about y = 8 DIP (`tray::LARGE_STRETCH`): the loudest step's outer wave (1.25 to
  14.75 DIP) then spans 0.06 to 15.94 DIP. Widths, strokes and the level cut are
  unchanged. Inside the stretch, icon glyphs are drawn without grid fitting and without
  pixel snapping of their position (`Canvas::stretched_vertically`): both would move
  the glyph's vertical extremes by up to half a pixel, past the square's edge at some
  scalings. Pixel tests at every scaling (100 to 300 %) check the same columns as the
  Windows size, the height grown by the stretch, nothing drawn past the square, and no
  loss of sharpness; the badge tests run at both sizes.

## 4. Context menu (FR-100)

| Token | Value (DIP) |
|---|---|
| Vertical padding | 4 |
| Highlight inset (left/right) | 4 |
| Entry height | 32 at 100 % text (grows with text scaling) |
| Gap between entries | 2 |
| Highlight corner radius | 4 |
| Icon: left offset / size / gap to label | 12 / 16 / 12 |
| Space after the label | 24 |
| Separator block (line centred) | 9, line spans the full width |
| Minimum width | 200 |
| Gap to the taskbar | 8 |
| Text | Segoe UI Variable Text 14/20 regular |

States: hover = subtle fill (dark `#0FFFFFFF`, light `#09000000`), pressed = tertiary
subtle fill, disabled = disabled text colour. Corner radius 8 and outline come from the
Windows frame. Icons: sound settings `E767`, mixer `F4C3`, classic panel `E7F5`, settings
`E713`, quit `E711`.

Placement: above a bottom taskbar (below a top one, beside a vertical one), centred on the
tray icon along the taskbar, like the menus of the Windows 11 tray icons (an icon in the
hidden-icons panel: centred next to it, on the side away from the taskbar); opened
anywhere else, ending at the pointer when it fits on that side. Always kept inside the
usable area of the monitor under the pointer (auto-hidden taskbar cut out). The plain
Windows menu used when drawing fails keeps the pointer placement.

## 5. Flyout frame (FR-020, FR-021)

- Width 360, gap 12 to the work-area edges, in the tray corner of the taskbar's monitor
  (left corner for a right-to-left taskbar). Height (FR-023): the work-area height
  minus both gaps whatever the content (default), or following the content, at
  most that; only the application list scrolls when it does not fit (thin indicator, 3
  DIP).
- Corner radius 8 and outline from the Windows frame; material as in §2.

## 6. Motion

| Transition | Duration | Curve | Movement |
|---|---|---|---|
| Flyout open | 250 ms | decelerate (0, 0, 0, 1) | rises from behind the taskbar edge |
| Flyout close | 167 ms | accelerate (1, 0, 1, 1) | sinks back behind the taskbar edge |
| Menu open | 200 ms | decelerate | rises from behind the taskbar edge |
| Menu close | none | — | disappears (Windows menus) |
| Tooltip | 500 ms delay | — | appears in place, no motion |
| Flyout height change ("fits the content" only) | 250 ms | decelerate | the edge away from the taskbar moves; the content above the application list stays, the master volume and footer stay against the taskbar, the list is revealed or cut |
| Bubble open | 250 ms | decelerate | rises from the bottom edge of the screen whatever the taskbar position (from behind a bottom taskbar; in place when a full-screen program is in front), stepped by a 10 ms timer: the UI thread is never held |
| Bubble close | 167 ms | accelerate | sinks back to the bottom edge, about 2 s after the last update |

The whole window moves, frame and material included, one position per compositor frame,
placed just below the taskbar in the z-order so the part not yet risen is hidden behind
it (`bv-shell/src/popup.rs`). Values compared with the system flyouts.

## 7. Gallery

`cargo run -p bv-ui-kit --example gallery -- <folder>` renders the tray icons (every
scaling, both taskbar colours, both pictures, both looks (`-bv`), both sizes), an icon-font sheet and the menu (both modes,
100/150/225 %) to PNG files through the real backend, for visual checks. The language
sheets (`bv-app`, test `language_sheets`, run with `--ignored` and `BV_SHEETS=<folder>`)
draw every window of the application in every shipped language with the real views.

## 8. Flyout layout

Three blocks, top to bottom (`controls/flyout/layout.rs`):

| Block | Content | Anchored to |
|---|---|---|
| Top | padding 16, tab bar (if microphone management is on), gap 12, device tiles | top edge |
| (free space) | nothing | full height only: between the top block and the list |
| Middle | gap 12, line, gap 12, "Applications" header (32), gap 4, rows spaced 4 | full height: right above the bottom block; fitting the content: below the top block; scrolls when it does not fit |
| Bottom | gap 12, line, gap 12, main volume row, padding 8, footer (48) | bottom edge |

Content padding 16 left and right. Lines span the whole width (divider colour).

## 9. Tab bar (FR-030)

Height 36 at 100 % text (grows with text scaling), padding 3, radius 6, container fill
and control stroke. Two equal tabs, radius 4. Selected: control fill and stroke, label
body strong, accent pill 16 × 3 centred at the bottom. Others: subtle hover. Icon 16
(speaker with waves, microphone) and label centred together, gap 8.

## 10. Device tiles (FR-040, FR-041, FR-046)

- Rows balanced, three at most, fuller rows first; every tile of a row has the same width
  and the rows fill the content width; gap 8.
- Height: 12 + icon 24 + 6 + two caption lines + 8 (80 at 100 % text).
- Radius 4. At rest: control fill with control stroke; hover: control fill hover;
  pressed: control fill pressed, secondary text. Current device: accent fill (hover
  90 %, pressed 80 %), text on accent, label caption strong.
- Label: caption, centred, wrapped on two lines then ellipsised; the full name shows in
  the tooltip when cut.
- Calls marker: handset glyph 12, 6 from the top and right edges, label colour; tooltip
  "Used for calls".
- No device: "no device" caption text centred in a 48-DIP area.

## 11. Volume rows (FR-050, FR-062)

- Label line (caption, secondary text, one line, ellipsised) above a 40-DIP row: mute
  button 36 × 36 (subtle, radius 4), gap 12, slider, gap 4, value 32 wide (body,
  secondary, right-aligned). Application labels start above the slider; the master
  label is indented 4.
- Mute button: master output shows the Windows volume glyph of its step (muted, 0, low,
  medium, high); the microphone shows the microphone glyph (crossed when muted). An
  application shows its icon (20 × 20, loaded at the exact pixel size) or a generic app
  glyph; System sounds a speaker glyph. A muted application: icon at 50 % and a 14-DIP
  badge (solid disc, outline, crossed speaker) at its bottom-right corner.
- Disabled row (no device): disabled colours, no value.

## 12. Slider (FR-051)

Hit area 32 high. Track 4, rounded, full width: accent up to the thumb, slider track
colour after. Thumb 20: solid thumb colour, 1-DIP outline; inner accent disc 12 at rest,
14 under the pointer, 10 pressed (accent 80 %). Disabled: accent-disabled colour
everywhere. Travel: the thumb centre moves from 10 to width - 10.

## 13. Section header and footer (FR-060, FR-033)

- Header: content width + 4 on each side, height 32 (grows with text), subtle hover,
  caption strong label in secondary text, chevron 12 at the right (down collapsed, up
  expanded).
- Footer: band 48 high with the footer fill and a line on top; text button "Windows sound
  settings" (caption, secondary, padding 8, height 36) 8 from the left; settings button
  36 × 36 with the settings glyph 16, 8 from the right.
- Update line: when shown, the footer band grows by 40 above
  its buttons (one fill, one line on top). Download glyph 16 in the hyperlink colour at
  x 16, gap 8, caption text (one line, ellipsised) up to 8 before the buttons; at the
  right, 8 from the edge: close button 36 wide (lined up with the settings button) and 32
  high (cross glyph 12, secondary text, subtle
  hover, tooltip), then 4 apart a hyperlink-style button (caption strong, hyperlink
  colour, padding 8, height 32, subtle hover). Line and buttons grow with the Windows
  text size (line height + 16, button + 8). Keyboard: its buttons are stops between
  the master volume and the footer link; screen readers read the text, then the buttons.

## 14. Focus, tooltip, colours

- Keyboard focus indicator (only after keyboard use): 2-DIP outer ring and 1-DIP inner
  ring, 3 DIP outside the control, radius of the control plus the offset.
- Tooltip: caption text, padding 8 × 5, radius 4, solid fill and outline, up to 320 wide
  (wraps); 8 above the control, centred on it (below when there is no room), 8 from the
  monitor edges; shown on disabled controls too; never takes the focus, lets clicks through.
- Control colours (`Theme::controls`, WinUI 3 resources). Dark: fill `#0FFFFFFF`, hover
  `#15FFFFFF`, pressed `#08FFFFFF`, stroke `#12FFFFFF`, slider track `#8BFFFFFF`, thumb
  `#454545` (outline `#23FFFFFF`), footer `#26000000`, solid `#2C2C2C`. Light: fill
  `#B3FFFFFF`, hover `#80F9F9F9`, pressed `#4DF9F9F9`, stroke `#0F000000`, slider track
  `#72000000`, thumb `#FFFFFF` (outline `#29000000`), footer `#09000000`, solid `#F9F9F9`.
  High contrast: button face, button text and highlight system colours only.

## 15. Device icon catalogue (FR-068)

27 drawings (`controls/device_icon.rs`), each on a 16-DIP grid with a 1-DIP round stroke,
rounded corners and small filled details, natural proportions, about 1 DIP of margin,
scaled uniformly (tile: 24 DIP, so a 1.5-DIP stroke). Stable identifiers (kebab case) for
the settings. Automatic choice from the Windows device type (`bv-app`, FR-067).

## 16. Accessibility (FR-024)

Every control describes itself as an `AccessNode` (role, name, state, bounds); the shell
exposes the tree through UI Automation. Flyout: pane, tab list (tabs, selectable), list of
devices (list items: invoke, selectable; help text = calls marker), expander
(Applications), one group per volume row (toggle button "Mute: name", slider "Volume:
name", 0 to 100, small change 1, large change 10), footer buttons. Menu: menu, menu items and separators.

## 17. Settings window (FR-070)

`controls/settings/` (content, layout, painting, input, accessibility), hosted by
`bv-shell/src/settings.rs`. Tokens in `tokens::settings`, colours in `Theme::window`
(WinUI 3 resources), apps mode.

- Window: standard Windows frame and title bar (system buttons), Mica material over the
  client area (`Theme::window.mica`: transparency effects on, no high contrast), opaque
  `#202020` / `#F3F3F3` otherwise. Default 980 × 700, smallest 680 × 460 (client area).
- Navigation pane 290 wide: entries 36 high (grow with text), inset 4, gap 4, radius 4,
  icon 16 at 12, label at 44; current entry: subtle fill and accent pill 3 × 16 at its
  left edge; hover subtle fill.
- Page: padding 36 left and right, 12 on top, 36 at the bottom, content at most 1000
  wide; title "Title" style (28/36 semibold); gap 20 to the first card; group header body
  strong, 26 above (between groups) and 8 below.
- Card: at least 68 high, padding 16 × 12, radius 4, card fill and card stroke; icon 20,
  gap 20; title body and description caption secondary, both wrapping (title up to 3
  lines); control right-aligned, at least 16 from the text; gap 4 between cards. A card
  narrower than 476 (286 without an icon, WinUI `SettingsCard` wrap thresholds) puts its
  control under its text instead: 8 below it, aligned with the text, narrowed to the room
  left if wider.
- Toggle switch: track 40 × 20; off: subtle track, 1-DIP strong outline and knob
  (secondary text colour); on: accent track, knob in text-on-accent colour; knob 12, 14
  under the pointer, stretched to 17 while pressed; "On" / "Off" label at its left (gap
  12), sized for the longer of the two so the switch never moves.
- Combo box: height 32 (grows with text), at least 160 wide, control fill and stroke,
  radius 4, text padding 11, chevron 12. Open list: drawn inside the window, the current
  entry over the box, kept 8 from the window edges (scrolls when taller), solid fill,
  outline, radius 8, padding 4, entries 36 high, current entry subtle fill and accent
  pill.
- Hyperlink button: accent text (`light3` in dark mode, `dark2` in light mode), padding
  11, height 32, subtle fill under the pointer.
- Scrolling: wheel 64 DIP per notch; thin indicator 3 wide at the right edge (6 under the
  pointer or dragged), grab area 12.
- Keyboard: Tab / Shift+Tab through the navigation list (one stop) and every usable
  control; arrows inside the navigation; Space toggles; Enter / Space / Alt+Down / F4 open
  a list; arrows on a closed list change its value; in an open list arrows, Page Up/Down,
  Home/End, Enter choose, Escape closes. Focus indicator after keyboard use only.
- Accessibility: pane, navigation list of list items, a group per card (name = title,
  help = description) holding a toggle button, a combo box (expand/collapse, its choices
  as list items) or a hyperlink.

### 17.0 Picture choices (FR-078, FR-083, FR-084)

A card of picture choices, after the Start layout choice of Windows 11: the card's icon,
title and description, then groups side by side from the card's left padding, each a
label (Body) over a row of tiles; a group that does not fit goes to the next row.

| Token | Value (DIP) |
|---|---|
| Tile: least width / picture height | 76 / 52 (wider for a long label, all tiles of a group alike) |
| Tile: padding / gap picture to label / picture corner radius | 4 / 6 / 4 |
| Label padding (left and right) | 6; the label (Body, centred) wraps on 2 lines at most |
| Icon on a tile | 32 (the tray icon at twice its taskbar size, `tray::paint_in`) |
| Gaps: tiles / groups side by side / rows of groups | 4 / 20 / 16 |
| Gaps: under the card text / under a group label | 16 / 8 |

The picture is a strip of the taskbar colour (`TrayColors::taskbar`: `#1C1C1C` dark,
`#EEEEEE` light, the window colour in high contrast) with a 1-DIP card outline. The chosen
tile has the subtle hover fill and a 2-DIP accent outline (as the cells of the icon
picker); the others a 1-DIP card outline, the hover or pressed fill under the pointer.
Each group is one Tab stop (a radio group): the arrows move to the tile before or after
and choose it, Home and End the first and last; screen readers see each group as a list
whose items are the tiles, the chosen one selected. The three groups of the tray icon
fit side by side at the window's default size in every shipped language (tested with the
real text measures).

### 17.1 Device lists (FR-085 to FR-089)

- Help paragraph: body, secondary text, wraps, 12 below.
- Device row: card fill and stroke, radius 4, at least 64 high; drag handle glyph 16 at 8
  from the left (secondary), gap 8; icon button 40 x 40 (control fill and stroke, radius
  4) holding the device drawing at 24, gap 12; name (body) over details (caption,
  secondary), one line each, ellipsised; buttons 36 x 36 (subtle, glyph 16, gap 4) at the
  right: bin (disconnected rows only), pencil, eye (crossed when hidden). Hidden rows:
  name and icon in secondary text, details in disabled text.
- Dragging: past 4 DIP the row lifts (solid fill, accent outline) and follows the pointer;
  a 2-DIP accent line in the gap shows where it will go.
- Text box: height 32 (grows with text), padding 10, radius 4, control stroke; at rest
  control fill and a 1-DIP strong bottom line; focused `Theme::window.input_focused` and
  a 2-DIP accent bottom line; selection accent at 45 %; caret 1 DIP, steady. Refusal
  message: caption in `Theme::window.critical` under the box.
- Icon picker: a card-styled panel under the row, padding 12, cells 88 x 76 (grow with
  text), gap 4, drawing 24 near the top and the name in caption (two lines, centred); the
  current choice: subtle fill and a 2-DIP accent outline.
- Forget confirmation: flyout 320 wide under the bin (above if no room), solid fill,
  outline, radius 8, padding 16; question in body strong, explanation in body
  secondary, two equal buttons (accent "Forget", standard "Cancel", height 32).
- Keyboard: every row button is a stop; Alt+Up / Alt+Down move the row; in the picker
  arrows move by cell and row, Enter picks, Escape closes; the confirmation keeps Tab
  between its two buttons, Escape cancels.
- Accessibility: one group per row (name, details and the move hint as help), with the
  icon expander, the text box (edit, with its value), rename, show/hide and forget
  buttons; the picker as a list; the confirmation as a group with two buttons.

- Application rows (FR-096, `RowPicture::App` / `SystemSounds`): the same row, without
  the icon button: the application's own icon 24 (dimmed to 60 % on a hidden row) or, for
  System sounds and an application without an icon, a glyph 20, centred in the 40 x 40
  place of the button; no pencil (no rename), the bin next to the eye on every row but
  System sounds. The picture is not a stop. A button row above a list keeps 12 below it.

### 17.2 Forms, notices and the close question (FR-090 to FR-092)

- Text field (card control, WinUI TextBox): 260 wide, the text box of 17.1; at rest it
  shows its text or, when empty, its placeholder in secondary text. Taking the focus
  selects the whole text; the caret and selection show only while it has the focus (no
  focus ring: the accent bottom line is the focus sign, like WinUI). Password fields
  (WinUI PasswordBox) show one bullet U+25CF per character and refuse copy and cut;
  digits-only fields drop other characters. Every change is reported at once.
- Push button (card control or button row): height 32 (grows with text), padding 12,
  at least 120 wide, radius 4, control stroke; accent fill (text on accent) or control
  fill; disabled: disabled fill and text. A button row sits at the left, 8 below the last
  card, buttons 8 apart.
- Notice (WinUI InfoBar): full page width, radius 4, card stroke, at least 48 high,
  padding 16 x 14; background by severity (`Theme::window.notice_background`: information,
  success, caution, critical, the WinUI "system fill background" colours); severity icon
  16 = filled circle (U+F136) in `notice_icon` with the severity glyph (U+F13F, U+F13E,
  U+F13C, U+F13D) in `notice_glyph` on it, then 12; title in body strong and message in
  body on one line when they fit, the message under the title otherwise (a long title
  wraps, up to 3 lines; the message up to 12); an optional
  standard button at the right, or under the text (aligned with it, 12 below) when less
  than 320 would be left to the text beside it.
- Close question (WinUI ContentDialog): veil `Theme::window.smoke` over the window; dialog
  480 wide (less on a narrow window), centred, radius 8, solid fill and popup outline,
  padding 24; title in the subtitle style (20/28 semibold), text in body 12 below; a
  footer band (`controls.footer`) with three equal buttons 8 apart: accent "Save",
  "Don't save", "Cancel". Modal: only its buttons react; Tab and arrows cycle them, Enter
  activates the focused one (Save at first), Escape cancels.
- Question of the content (`Question`): the same dialog, shown at once
  while the content holds it, with two equal buttons (accent primary, secondary); Escape
  or closing the window answers the secondary one. Any dialog grows wider (within the
  window) when its button labels need it, all buttons keeping one width.
- A notice the window was opened for can be scrolled into view once
  (`SettingsContent::reveal`).
- Accessibility: fields are edits (a password field exposes no value), buttons are
  buttons, a notice is a group (title, message as help) holding its button, the close
  question a group with its three buttons.

## 18. Bubble (FR-017, FR-105)

`controls/bubble.rs`, hosted by `bv-shell/src/bubble.rs` (class `BetterVolume.Bubble`).
Metrics measured on the Windows 11 volume indicator at 225 %: same place, size,
glyph, bar and number to the pixel.

- Window: passive (no focus, layered and transparent to clicks), Windows frame for the
  rounded corners (8) and the shadow, frame outline removed (the indicator has none).
  Opaque fill `controls.solid` (`#2C2C2C` dark, `#F9F9F9` light; the indicator's
  material looks opaque); shell theme (system mode); high contrast: system colours and
  an outline.
- Place: bottom centre of the usable area of the taskbar's monitor, 14 DIP above its
  bottom (truncated to pixels: 31 px at 225 %, as measured), whatever the taskbar
  position: above a bottom taskbar, above the screen's bottom edge with a taskbar at the
  top or on a side, as the Windows indicator does. It always rises from the bottom edge.
- Volume content: 192 × 48 at 100 % text (the height grows with the text, the width with
  the number); volume glyph 16 at x 13 (the Windows step glyph, muted included), text
  colour; level bar from x 42, 110 long, 4 thick, rounded: accent up to the level,
  slider track colour after (no thumb); number in body text, right-aligned 12 from the
  right edge, room at least 24 ("100").
- Device content: device drawing 24 at x 12 in the accent colour (validated mockup),
  gap 12, name (body, one line, ellipsised) over the kind (caption, secondary); padding 8
  above and below, 16 after the text; width between 192 and 360.
- Notice content ("BetterVolume updated"): the same layout with a glyph
  (18, the update arrows) in the accent colour in place of the device drawing, the title
  over the detail ("Version X").
- One window for both contents: a new content replaces the old one (and stops a sink),
  the bubble stays centred.
