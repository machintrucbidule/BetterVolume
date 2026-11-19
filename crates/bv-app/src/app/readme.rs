//! Documentation images: the panel, the settings window, the bubble and an animated demo,
//! drawn by the application's own views and drawing code (never a screen capture), in
//! English. Not part of the normal test run:
//!
//!   set BV_README=<output folder>
//!   cargo test -p bv-app readme_images -- --ignored
//!   python tools/readme-images.py <output folder> docs/images
//!
//! This writes layers (premultiplied BGRA, `*.raw`) and a description of every still and
//! every animation frame (`scenes.json`); the script composes them over a background
//! (translucent materials, rounded corners, shadows, pointer) and writes the images.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

use bv_audio::fake::FakeEngine;
use bv_audio::{AppKey, Bus, DeviceInfo, DeviceState, Flow, FormFactor, Stream};
use bv_core::devices::KnownDevice;
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{FlyoutHeight, Settings, TrayIconLook, TrayIconSize, TrayIconStyle};
use bv_ui_kit::access::{AccessNode, AccessRole};
use bv_ui_kit::backend::{DeviceKind, Graphics, Pixels, TextAlign};
use bv_ui_kit::controls::bubble::{self, BubbleContent};
use bv_ui_kit::controls::device_icon::DeviceIcon;
use bv_ui_kit::controls::flyout::{
    FlyoutContent, FlyoutView, GraphicsMeasure, Response, VolumeIcon,
};
use bv_ui_kit::controls::settings::SettingsView;
use bv_ui_kit::controls::tray;
use bv_ui_kit::motion::{Transition, FLYOUT_CLOSE, FLYOUT_OPEN, FLYOUT_RESIZE};
use bv_ui_kit::theme::{Theme, ThemeInputs, ThemeMode};
use bv_ui_kit::tokens::typography::CAPTION;
use bv_ui_kit::{Color, Point, Rect, Size};

use super::settings_window::Page;
use super::App;

/// Pixels per DIP of every image (crisp on high-density screens, light files).
const SCALE: f32 = 1.5;
/// Scene of the panel images: room for the panel, its gaps and the taskbar corner.
const SCENE: (f32, f32) = (440.0, 600.0);
/// Height of the taskbar (Windows 11 at 100 %).
const TASKBAR: f32 = 48.0;
/// Gap between the panel and the screen edge and taskbar (Windows 11 flyouts).
const GAP: f32 = 12.0;
/// How the pointer moves between two places (gentle start and stop).
const POINTER: bv_ui_kit::motion::CubicBezier =
    bv_ui_kit::motion::CubicBezier { x1: 0.45, y1: 0.0, x2: 0.2, y2: 1.0 };
/// One animation frame, in milliseconds.
const FRAME_MS: u32 = 40;
/// Settings window size for its image (the whole General page fits).
const SETTINGS: (f32, f32) = (980.0, 1170.0);
/// Application icons, by executable: the icons installed on the PC that draws the images.
const ICONS: [(&str, &str); 2] = [
    ("firefox.exe", r"%ProgramFiles%\Mozilla Firefox\firefox.exe"),
    ("discord.exe", r"%LOCALAPPDATA%\Discord\app.ico"),
];

fn device(id: &str, name: &str, form_factor: FormFactor, bus: Bus) -> DeviceInfo {
    DeviceInfo {
        id: id.into(),
        windows_name: name.into(),
        form_factor,
        bus,
        state: DeviceState::Active,
    }
}

fn stream(file: &str, name: &str, volume: f32) -> Stream {
    let path = format!(r"C:\Apps\{file}");
    let icon = ICONS.iter().find(|(f, _)| *f == file).map(|(_, source)| (*source).to_owned());
    Stream {
        app: AppKey::executable(&path),
        process: 7,
        name: name.into(),
        icon,
        path: Some(path),
        volume,
        muted: false,
        heard_here: true,
    }
}

fn known(id: &str, name: &str, icon: DeviceIcon) -> KnownDevice {
    KnownDevice {
        custom_name: Some(name.into()),
        icon: Some(icon.id().into()),
        ..KnownDevice::new(id)
    }
}

/// Three outputs (speakers current), a microphone, three applications playing.
fn app() -> App<FakeEngine> {
    let mut engine = FakeEngine::new()
        .with_device(
            Flow::Output,
            device("spk", "Speakers (Realtek(R) Audio)", FormFactor::Speakers, Bus::Other),
        )
        .with_device(
            Flow::Output,
            device("hs", "Headset (USB Audio)", FormFactor::Headset, Bus::Usb),
        )
        .with_device(
            Flow::Output,
            device("tv", "TV (NVIDIA High Definition Audio)", FormFactor::Display, Bus::Other),
        )
        .with_device(
            Flow::Input,
            device("mic", "Microphone (USB Audio)", FormFactor::Microphone, Bus::Usb),
        );
    engine.levels.insert("spk".into(), (0.62, false));
    engine.levels.insert("hs".into(), (0.48, false));
    engine.levels.insert("tv".into(), (0.30, false));
    engine.streams = vec![
        stream("discord.exe", "Discord", 0.7),
        stream("firefox.exe", "Firefox", 0.85),
        Stream { app: AppKey::System, name: String::new(), icon: None, ..stream("x", "", 0.5) },
    ];
    engine.sync();
    let mut settings = Settings::default();
    settings.general.flyout_height = FlyoutHeight::Content;
    settings.general.first_run_done = true;
    settings.general.volume_icon_question_done = true;
    settings.general.hide_windows_volume_icon = true;
    settings.devices.outputs = vec![
        known("spk", "Speakers", DeviceIcon::BookshelfSpeakers),
        known("hs", "Headset", DeviceIcon::GamingHeadset),
        known("tv", "TV", DeviceIcon::Tv),
    ];
    settings.devices.inputs = vec![known("mic", "Microphone", DeviceIcon::DeskMicrophone)];
    let mut app = App::new(engine, settings, Language::ENGLISH);
    app.set_process_list(|| vec!["firefox.exe".into(), "discord.exe".into()]);
    app.on_audio_signals();
    app
}

fn shell_theme(mode: ThemeMode) -> Theme {
    Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() })
}

fn mode_name(mode: ThemeMode) -> &'static str {
    if mode == ThemeMode::Dark {
        "dark"
    } else {
        "light"
    }
}

/// Layers already written, by content, and the description being built.
struct Output {
    gfx: Graphics,
    out: PathBuf,
    layers: HashMap<Vec<u8>, String>,
    sizes: String,
    icons: HashMap<String, Option<Rc<Pixels>>>,
}

impl Output {
    /// Writes `image` once (identical images share one file) and returns its name.
    fn layer(&mut self, prefix: &str, image: &Pixels) -> String {
        let mut key = image.data.clone();
        key.extend_from_slice(&image.width.to_le_bytes());
        if let Some(name) = self.layers.get(&key) {
            return name.clone();
        }
        let name = format!("{prefix}-{}", self.layers.len());
        std::fs::write(self.out.join(format!("{name}.raw")), &image.data).expect("layer");
        let _ = write!(
            self.sizes,
            "{}\"{name}\": [{}, {}]",
            if self.sizes.is_empty() { "" } else { ", " },
            image.width,
            image.height
        );
        self.layers.insert(key, name.clone());
        name
    }

    /// The content with the application icons loaded, as the shell does.
    fn with_icons(&mut self, mut content: FlyoutContent) -> FlyoutContent {
        let pixels = (bv_ui_kit::tokens::volume_row::APP_ICON * SCALE).round() as u32;
        if let Some(apps) = content.apps.as_mut() {
            for row in &mut apps.rows {
                if let (VolumeIcon::App(None), Some(source)) = (&row.icon, &row.icon_source) {
                    let gfx = &self.gfx;
                    let icon = self
                        .icons
                        .entry(source.clone())
                        .or_insert_with(|| bv_shell::app_icon(source, pixels, gfx).map(Rc::new));
                    row.icon = VolumeIcon::App(icon.clone());
                }
            }
        }
        content
    }

    fn flyout_layer(&mut self, view: &FlyoutView, mode: ThemeMode) -> String {
        let theme = shell_theme(mode);
        let (w, h) = view.size().to_pixels(SCALE);
        let image =
            self.gfx.rasterize(w, h, SCALE, 1.0, |c| view.paint(c, &theme)).expect("flyout");
        self.layer(&format!("flyout-{}", mode_name(mode)), &image)
    }

    /// The bottom-right corner of the taskbar: hidden-icons arrow, BetterVolume's icon,
    /// network, time and date (no Windows volume icon: BetterVolume replaces it). Returns
    /// the layer and the centre of BetterVolume's icon in the scene.
    fn taskbar(&mut self, app: &App<FakeEngine>, mode: ThemeMode, hover: bool) -> (String, Point) {
        let theme = shell_theme(mode);
        let ink = theme.text_primary;
        let width = SCENE.0;
        let (w, h) = Size::new(width, TASKBAR).to_pixels(SCALE);
        let icon = app.tray_icon();
        let clock = Rect::new(width - 8.0 - 78.0, 0.0, 78.0, TASKBAR);
        let network = Rect::new(clock.x - 40.0, 4.0, 40.0, TASKBAR - 8.0);
        let ours = Rect::new(network.x - 4.0 - 32.0, 4.0, 32.0, TASKBAR - 8.0);
        let chevron = Rect::new(ours.x - 28.0, 4.0, 28.0, TASKBAR - 8.0);
        let (iw, ih) = Size::new(16.0, 16.0).to_pixels(SCALE);
        let glyph = self
            .gfx
            .rasterize(iw, ih, SCALE, 1.0, |c| tray::paint(c, &icon, &theme))
            .expect("icon");
        let image = self
            .gfx
            .rasterize(w, h, SCALE, 1.0, |c| {
                c.clear(Color::TRANSPARENT);
                if hover {
                    c.fill_rounded(ours, 4.0, theme.subtle_hover);
                }
                c.glyph('\u{E70E}', 12.0, chevron, ink);
                c.image(&glyph, Rect::new(ours.x + 8.0, ours.y + 12.0, 16.0, 16.0), 1.0);
                c.glyph('\u{E839}', 16.0, network, ink);
                let time = Rect::new(clock.x, 7.0, clock.width - 8.0, 16.0);
                let date = Rect::new(clock.x, 25.0, clock.width - 8.0, 16.0);
                c.text("2:32 PM", CAPTION, time, TextAlign::Trailing, ink);
                c.text("11/19/2026", CAPTION, date, TextAlign::Trailing, ink);
            })
            .expect("taskbar");
        let name = self.layer(&format!("taskbar-{}", mode_name(mode)), &image);
        let top = SCENE.1 - TASKBAR;
        (name, Point::new(ours.x + ours.width / 2.0, top + ours.y + ours.height / 2.0))
    }
}

/// The element of an accessibility tree whose role is `role` and whose name contains
/// `name`.
fn find(node: &AccessNode, role: AccessRole, name: &str) -> Option<Rect> {
    if node.role == role && node.name.contains(name) {
        return Some(node.bounds);
    }
    node.children.iter().find_map(|child| find(child, role, name))
}

fn centre(r: Rect) -> Point {
    Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
}

/// One step of an animation: what the script composes.
struct Frame {
    ms: u32,
    mix: f32,
    flyout: Option<[String; 2]>,
    at: (f32, f32),
    taskbar: [String; 2],
    cursor: Point,
    down: bool,
}

/// The demo being recorded: the application, the open panel, the pointer, the frames.
struct Demo<'a> {
    out: &'a mut Output,
    app: App<FakeEngine>,
    view: Option<FlyoutView>,
    /// Shown height of the panel (it changes while its content changes).
    shown: f32,
    /// Vertical offset of the panel from its resting place (entrance and exit).
    offset: f32,
    /// 0 = dark, 1 = light (in between: the theme change fades).
    mix: f32,
    cursor: Point,
    down: bool,
    hover: bool,
    frames: Vec<Frame>,
}

impl Demo<'_> {
    fn max_height(&self) -> f32 {
        SCENE.1 - TASKBAR - 2.0 * GAP
    }

    fn origin(&self) -> Point {
        let width = self.view.as_ref().map_or(0.0, FlyoutView::width);
        Point::new(SCENE.0 - GAP - width, SCENE.1 - TASKBAR - GAP - self.shown + self.offset)
    }

    fn refresh(&mut self) -> bool {
        let content = self.app.flyout_content();
        let content = self.out.with_icons(content);
        let max = self.max_height();
        let measure = GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 };
        self.view.as_mut().is_some_and(|view| view.set_content(content, &measure, 1.0, max).resized)
    }

    fn record(&mut self, ms: u32) {
        // Only the themes shown: dark before the change, light after, both while it fades.
        let wanted = [self.mix < 1.0, self.mix > 0.0];
        let modes = [ThemeMode::Dark, ThemeMode::Light];
        let shown = self.shown;
        let flyout = self.view.as_ref().map(|view| {
            // The window is as tall as the shown height (during a height change).
            let mut view = view.clone();
            view.set_shown_height(shown);
            [0, 1].map(|i| {
                if wanted[i] {
                    self.out.flyout_layer_at(&view, modes[i], shown)
                } else {
                    String::new()
                }
            })
        });
        let app = &self.app;
        let hover = self.hover;
        let taskbar = [0, 1].map(|i| {
            if wanted[i] {
                self.out.taskbar(app, modes[i], hover).0
            } else {
                String::new()
            }
        });
        let origin = self.origin();
        self.frames.push(Frame {
            ms,
            mix: self.mix,
            flyout,
            at: (origin.x, origin.y),
            taskbar,
            cursor: self.cursor,
            down: self.down,
        });
    }

    fn hold(&mut self, ms: u32) {
        self.record(ms);
    }

    /// Moves the pointer to `to` (scene coordinates) in `ms`, telling the panel.
    fn point(&mut self, to: Point, ms: u32) {
        let from = self.cursor;
        let steps = (ms / FRAME_MS).max(1);
        for i in 1..=steps {
            let t = Transition { duration_ms: ms, easing: POINTER }.progress((i * FRAME_MS) as f32);
            self.cursor = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            self.pointer_moved();
            self.record(FRAME_MS);
        }
    }

    fn pointer_moved(&mut self) {
        let origin = self.origin();
        let p = Point::new(self.cursor.x - origin.x, self.cursor.y - origin.y);
        if let Some(view) = self.view.as_mut() {
            let response = view.pointer_move(p);
            self.act(response);
        }
    }

    fn act(&mut self, response: Response) {
        if let Some(action) = response.action {
            let _ = self.app.flyout_action(action);
            self.app.on_audio_signals();
            if self.refresh() {
                self.resize();
            }
        }
    }

    fn press(&mut self) {
        self.down = true;
        let origin = self.origin();
        let p = Point::new(self.cursor.x - origin.x, self.cursor.y - origin.y);
        if let Some(view) = self.view.as_mut() {
            let response = view.press(p);
            self.act(response);
        }
        self.record(FRAME_MS * 2);
    }

    fn release(&mut self) {
        self.down = false;
        let origin = self.origin();
        let p = Point::new(self.cursor.x - origin.x, self.cursor.y - origin.y);
        if let Some(view) = self.view.as_mut() {
            let response = view.release(p);
            self.act(response);
        }
        self.record(FRAME_MS);
    }

    fn click(&mut self) {
        self.press();
        self.release();
    }

    /// The panel height follows its content (the "fits the content" mode).
    fn resize(&mut self) {
        let Some(to) = self.view.as_ref().map(FlyoutView::height) else { return };
        let from = self.shown;
        let steps = FLYOUT_RESIZE.duration_ms / FRAME_MS;
        for i in 1..=steps {
            let t = FLYOUT_RESIZE.progress((i * FRAME_MS) as f32);
            self.shown = from + (to - from) * t;
            self.record(FRAME_MS);
        }
        self.shown = to;
        if let Some(view) = self.view.as_mut() {
            view.end_transition();
        }
    }

    /// Where an element of the panel is, in the scene.
    fn element(&self, role: AccessRole, name: &str) -> Point {
        let view = self.view.as_ref().expect("panel open");
        let bounds = find(&view.access_tree(), role, name)
            .unwrap_or_else(|| panic!("no {role:?} named {name}"));
        let origin = self.origin();
        let c = centre(bounds);
        Point::new(origin.x + c.x, origin.y + c.y)
    }

    fn slider(&self, name: &str, value: f32) -> Point {
        let view = self.view.as_ref().expect("panel open");
        let bounds = find(&view.access_tree(), AccessRole::Slider, name)
            .unwrap_or_else(|| panic!("no slider named {name}"));
        let origin = self.origin();
        // The thumb travels the track between half a thumb from each end.
        let half = 10.0;
        let x = bounds.x + half + (bounds.width - 2.0 * half) * value / 100.0;
        Point::new(origin.x + x, origin.y + bounds.y + bounds.height / 2.0)
    }

    fn open(&mut self) {
        self.app.flyout_opening();
        let content = self.out.with_icons(self.app.flyout_content());
        let measure = GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 };
        let view = FlyoutView::new(content, &measure, 1.0, self.max_height());
        self.shown = view.height();
        self.view = Some(view);
        self.slide(FLYOUT_OPEN, true);
    }

    fn close(&mut self) {
        self.slide(FLYOUT_CLOSE, false);
        self.view = None;
        self.app.flyout_closed();
        self.offset = 0.0;
    }

    /// The panel rises from behind the taskbar, or sinks back behind it.
    fn slide(&mut self, motion: Transition, opening: bool) {
        let hidden = GAP + self.shown + TASKBAR;
        let steps = motion.duration_ms / FRAME_MS;
        for i in 0..=steps {
            let t = motion.progress((i * FRAME_MS) as f32);
            self.offset = if opening { hidden * (1.0 - t) } else { hidden * t };
            self.record(FRAME_MS);
        }
    }

    fn fade_to_light(&mut self, ms: u32) {
        let steps = ms / FRAME_MS;
        for i in 1..=steps {
            self.mix = i as f32 / steps as f32;
            self.record(FRAME_MS);
        }
        self.mix = 1.0;
    }
}

impl Output {
    fn flyout_layer_at(&mut self, view: &FlyoutView, mode: ThemeMode, shown: f32) -> String {
        let theme = shell_theme(mode);
        let (w, h) = Size::new(view.width(), shown).to_pixels(SCALE);
        let image =
            self.gfx.rasterize(w, h, SCALE, 1.0, |c| view.paint(c, &theme)).expect("flyout");
        self.layer(&format!("flyout-{}", mode_name(mode)), &image)
    }
}

fn frames_json(frames: &[Frame]) -> String {
    let mut json = String::new();
    for (i, f) in frames.iter().enumerate() {
        let flyout = match &f.flyout {
            Some([dark, light]) => format!("[\"{dark}\", \"{light}\"]"),
            None => "null".into(),
        };
        let _ = write!(
            json,
            "{}{{\"ms\": {}, \"mix\": {:.3}, \"flyout\": {flyout}, \"at\": [{:.2}, {:.2}], \
             \"taskbar\": [\"{}\", \"{}\"], \"cursor\": [{:.2}, {:.2}], \"down\": {}}}",
            if i == 0 { "" } else { ",\n    " },
            f.ms,
            f.mix,
            f.at.0,
            f.at.1,
            f.taskbar[0],
            f.taskbar[1],
            f.cursor.x,
            f.cursor.y,
            f.down
        );
    }
    json
}

/// The animated demo: open, switch the output, change an application's volume, fold the
/// application list, change from dark to light, close.
fn demo(out: &mut Output) -> String {
    let mut demo = Demo {
        out,
        app: app(),
        view: None,
        shown: 0.0,
        offset: 0.0,
        mix: 0.0,
        cursor: Point::new(SCENE.0 * 0.42, SCENE.1 * 0.45),
        down: false,
        hover: false,
        frames: Vec::new(),
    };
    let icon = demo.out.taskbar(&demo.app, ThemeMode::Dark, false).1;
    demo.hold(900);
    demo.point(icon, 720);
    demo.hover = true;
    demo.hold(240);
    demo.press();
    demo.down = false;
    demo.open();
    demo.hover = false;
    demo.hold(700);
    // Switch the output.
    let headset = demo.element(AccessRole::ListItem, "Headset");
    demo.point(headset, 640);
    demo.hold(200);
    demo.click();
    demo.hold(800);
    // Lower Firefox.
    let from = demo.slider("Firefox", 85.0);
    demo.point(from, 680);
    demo.hold(160);
    demo.press();
    let to = demo.slider("Firefox", 35.0);
    demo.point(to, 640);
    demo.release();
    demo.hold(700);
    // Fold the applications.
    let fold = demo.element(AccessRole::Expander, "Applications");
    demo.point(fold, 600);
    demo.hold(160);
    demo.click();
    demo.hold(700);
    // Dark to light.
    demo.fade_to_light(320);
    demo.hold(1100);
    // Close from the icon.
    demo.point(icon, 700);
    demo.hover = true;
    demo.hold(200);
    demo.press();
    demo.down = false;
    demo.close();
    demo.hover = false;
    demo.point(Point::new(SCENE.0 * 0.42, SCENE.1 * 0.45), 700);
    demo.hold(1400);
    frames_json(&demo.frames)
}

/// The panel at rest in a theme, in its scene.
fn panel(out: &mut Output, mode: ThemeMode) -> String {
    let mut app = app();
    app.flyout_opening();
    let content = out.with_icons(app.flyout_content());
    let measure = GraphicsMeasure { graphics: &out.gfx, text_scale: 1.0 };
    let view = FlyoutView::new(content, &measure, 1.0, SCENE.1 - TASKBAR - 2.0 * GAP);
    let layer = out.flyout_layer(&view, mode);
    let taskbar = out.taskbar(&app, mode, false).0;
    let x = SCENE.0 - GAP - view.width();
    let y = SCENE.1 - TASKBAR - GAP - view.height();
    format!(
        "{{\"mode\": \"{}\", \"flyout\": \"{layer}\", \"at\": [{x:.2}, {y:.2}], \"taskbar\": \"{taskbar}\"}}",
        mode_name(mode)
    )
}

/// The device bubble above the taskbar, with the tray icon, in a theme.
fn bubble_scene(out: &mut Output, mode: ThemeMode) -> String {
    let app = app();
    let theme = bubble::theme(shell_theme(mode));
    let content = BubbleContent::Device {
        icon: DeviceIcon::GamingHeadset,
        name: "Headset".into(),
        kind: Language::ENGLISH.text(Msg::BubbleOutput).to_owned(),
    };
    let size = bubble::size(&content, &out.gfx, 1.0);
    let (w, h) = size.to_pixels(SCALE);
    let image = out
        .gfx
        .rasterize(w, h, SCALE, 1.0, |c| {
            c.clear(Color::TRANSPARENT);
            c.clipped(Rect::from_size(size), |c| bubble::paint(c, &theme, &content, size));
        })
        .expect("bubble");
    let layer = out.layer(&format!("bubble-{}", mode_name(mode)), &image);
    let taskbar = out.taskbar(&app, mode, false).0;
    let x = (SCENE.0 - size.width) / 2.0;
    let y = SCENE.1 - TASKBAR - 14.0 - size.height;
    format!(
        "{{\"mode\": \"{}\", \"bubble\": \"{layer}\", \"at\": [{x:.2}, {y:.2}], \"taskbar\": \"{taskbar}\"}}",
        mode_name(mode)
    )
}

/// The settings window on its General page.
fn settings_scene(out: &mut Output, mode: ThemeMode) -> String {
    let mut app = app();
    // The tray icon choices as the tray image shows them last.
    app.settings.general.tray_icon_style = TrayIconStyle::WithDeviceBadge;
    app.settings.general.tray_icon_size = TrayIconSize::Large;
    app.settings.general.tray_icon_look = TrayIconLook::BetterVolume;
    app.settings_opening(false);
    app.settings_action(bv_ui_kit::controls::settings::SettingsAction::SelectPage(
        Page::General.key(),
    ));
    let content = app.settings_content();
    let mut theme = Theme::app(&ThemeInputs { apps_mode: mode, ..ThemeInputs::default() });
    theme.window.mica = false;
    let measure = GraphicsMeasure { graphics: &out.gfx, text_scale: 1.0 };
    let title = content.name.clone();
    let size = Size::new(SETTINGS.0, SETTINGS.1);
    let view = SettingsView::new(content, size, &measure, 1.0);
    let (w, h) = size.to_pixels(SCALE);
    let image =
        out.gfx.rasterize(w, h, SCALE, 1.0, |c| view.paint(c, &theme, &measure)).expect("settings");
    let layer = out.layer(&format!("settings-{}", mode_name(mode)), &image);
    // The title bar Windows draws (a standard window): icon, title, the three buttons.
    let icon_file = concat!(env!("CARGO_MANIFEST_DIR"), r"\..\..\assets\icon\BetterVolume.ico");
    let icon = bv_shell::app_icon(icon_file, (16.0 * SCALE).round() as u32, &out.gfx);
    let ink = theme.text_primary;
    let (cw, ch) = Size::new(SETTINGS.0, CAPTION_HEIGHT).to_pixels(SCALE);
    let caption = out
        .gfx
        .rasterize(cw, ch, SCALE, 1.0, |c| {
            c.clear(Color::TRANSPARENT);
            if let Some(icon) = &icon {
                c.image(icon, Rect::new(12.0, 8.0, 16.0, 16.0), 1.0);
            }
            c.text(&title, CAPTION, Rect::new(40.0, 8.0, 400.0, 16.0), TextAlign::Leading, ink);
            for (i, glyph) in ['\u{E8BB}', '\u{E922}', '\u{E921}'].into_iter().enumerate() {
                let x = SETTINGS.0 - 46.0 * (i as f32 + 1.0);
                c.glyph(glyph, 10.0, Rect::new(x, 0.0, 46.0, CAPTION_HEIGHT), ink);
            }
        })
        .expect("caption");
    let caption = out.layer(&format!("caption-{}", mode_name(mode)), &caption);
    format!(
        "{{\"mode\": \"{}\", \"window\": \"{layer}\", \"caption\": \"{caption}\"}}",
        mode_name(mode)
    )
}

/// Height of a standard Windows 11 title bar.
const CAPTION_HEIGHT: f32 = 32.0;

/// The tray icon at the "Large" size in both looks (FR-084): the speaker, then the
/// picture "device" for each of the three outputs, in a theme (the taskbar with each
/// output current).
fn tray_scenes(out: &mut Output, mode: ThemeMode) -> Vec<String> {
    let pictures = [
        ("speaker", "spk", TrayIconStyle::SameAsWindows),
        ("spk", "spk", TrayIconStyle::WithDeviceBadge),
        ("hs", "hs", TrayIconStyle::WithDeviceBadge),
        ("tv", "tv", TrayIconStyle::WithDeviceBadge),
    ];
    [("windows", TrayIconLook::Windows), ("bettervolume", TrayIconLook::BetterVolume)]
        .into_iter()
        .flat_map(|look| pictures.map(|picture| (look, picture)))
        .map(|((look_name, look), (name, id, picture))| {
            let mut app = app();
            app.settings.general.tray_icon_style = picture;
            app.settings.general.tray_icon_size = TrayIconSize::Large;
            app.settings.general.tray_icon_look = look;
            app.engine.defaults[Flow::Output.index()] = [Some(id.into()), Some(id.into())];
            app.engine.sync();
            app.on_audio_signals();
            let (taskbar, icon) = out.taskbar(&app, mode, false);
            format!(
                "{{\"mode\": \"{}\", \"look\": \"{look_name}\", \"picture\": \"{name}\", \
                 \"taskbar\": \"{taskbar}\", \"icon\": [{:.2}, {:.2}]}}",
                mode_name(mode),
                icon.x,
                icon.y
            )
        })
        .collect()
}

#[test]
#[ignore = "writes the documentation images' layers; run on demand (see the module documentation)"]
fn readme_images() {
    let out = PathBuf::from(std::env::var("BV_README").expect("BV_README = output folder"));
    std::fs::create_dir_all(&out).expect("output folder");
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    gfx.set_text_locale("en-US");
    let mut output =
        Output { gfx, out, layers: HashMap::new(), sizes: String::new(), icons: HashMap::new() };
    let output = &mut output;
    let panels = [ThemeMode::Dark, ThemeMode::Light].map(|m| panel(output, m)).join(", ");
    let bubbles = [ThemeMode::Dark, ThemeMode::Light].map(|m| bubble_scene(output, m)).join(", ");
    let settings =
        [ThemeMode::Dark, ThemeMode::Light].map(|m| settings_scene(output, m)).join(", ");
    let trays = [ThemeMode::Dark, ThemeMode::Light]
        .into_iter()
        .flat_map(|m| tray_scenes(output, m))
        .collect::<Vec<_>>()
        .join(", ");
    let frames = demo(output);
    let json = format!(
        "{{\n  \"scale\": {SCALE},\n  \"scene\": [{}, {}],\n  \"taskbar\": {TASKBAR},\n  \
         \"layers\": {{{}}},\n  \"panels\": [{panels}],\n  \"bubbles\": [{bubbles}],\n  \
         \"settings\": [{settings}],\n  \"trays\": [{trays}],\n  \"frames\": [\n    {frames}\n  ]\n}}\n",
        SCENE.0, SCENE.1, output.sizes
    );
    std::fs::write(output.out.join("scenes.json"), json).expect("scenes");
}
