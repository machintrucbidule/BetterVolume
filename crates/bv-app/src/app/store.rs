//! Microsoft Store images: six screenshots in every shipped language and the frames of
//! the English demonstration video, drawn by the application's own views and drawing
//! code (never a screen capture). Not part of the normal test run:
//!
//!   set BV_STORE=<layers folder>
//!   cargo test -p bv-app store_images -- --ignored
//!   python tools/store-images.py <layers folder> <output folder>
//!
//! This writes layers (premultiplied BGRA, `*.raw`) and a description of every
//! screenshot and every video frame (`scenes.json`); the script composes them on a
//! 1920 x 1080 desktop (a 1280 x 720 screen at 150 %) and writes the screenshots, the
//! video and its thumbnail. Shared pieces come from the README images (`readme.rs`).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use bv_audio::fake::FakeEngine;
use bv_audio::{AppKey, Bus, Flow, FormFactor, Stream};
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{FlyoutHeight, Settings, TrayIconLook, TrayIconSize, TrayIconStyle};
use bv_ui_kit::access::{AccessNode, AccessRole};
use bv_ui_kit::backend::{DeviceKind, Graphics, TextAlign};
use bv_ui_kit::controls::bubble::{self, BubbleContent};
use bv_ui_kit::controls::device_icon::DeviceIcon;
use bv_ui_kit::controls::flyout::{FlyoutAction, FlyoutTab, FlyoutView, GraphicsMeasure, Response};
use bv_ui_kit::controls::settings::{SettingsAction, SettingsView};
use bv_ui_kit::controls::tray;
use bv_ui_kit::motion::{Transition, FLYOUT_CLOSE, FLYOUT_OPEN, FLYOUT_RESIZE};
use bv_ui_kit::theme::{Theme, ThemeInputs, ThemeMode};
use bv_ui_kit::tokens::typography::CAPTION;
use bv_ui_kit::{Color, Point, Rect, Size};

use super::ha::Status;
use super::readme::{
    centre, device, find, known, mode_name, shell_theme, stream, Output, CAPTION_HEIGHT, FRAME_MS,
    GAP, POINTER, SCALE, TASKBAR,
};
use super::settings_window::Page;
use super::App;
use super::FlyoutEffect;

/// The screen, in DIPs (1920 x 1080 pixels at 150 %).
const DESKTOP: (f32, f32) = (1280.0, 720.0);
/// The settings window's client area (the title bar is drawn above it).
const WINDOW: (f32, f32) = (1000.0, 580.0);
/// The settings window's top-left corner (title bar included), centred above the taskbar.
const WINDOW_AT: (f32, f32) =
    ((DESKTOP.0 - WINDOW.0) / 2.0, (DESKTOP.1 - TASKBAR - WINDOW.1 - CAPTION_HEIGHT) / 2.0);
/// How long the settings window takes to appear (Windows 11 window opening).
const WINDOW_FADE_MS: u32 = 200;

/// The sample names and the clock of a language: what a user of that language would see.
struct Sample {
    tag: &'static str,
    speakers: &'static str,
    headset: &'static str,
    tv: &'static str,
    microphone: &'static str,
    webcam: &'static str,
    pc: &'static str,
    time: &'static str,
    date: &'static str,
}

#[rustfmt::skip]
const SAMPLES: [Sample; 10] = [
    Sample { tag: "en", speakers: "Speakers", headset: "Headset", tv: "TV", microphone: "Microphone", webcam: "Webcam", pc: "Office PC", time: "2:32 PM", date: "11/19/2026" },
    Sample { tag: "fr", speakers: "Haut-parleurs", headset: "Casque", tv: "TV", microphone: "Microphone", webcam: "Webcam", pc: "PC du bureau", time: "14:32", date: "19/11/2026" },
    Sample { tag: "de", speakers: "Lautsprecher", headset: "Headset", tv: "TV", microphone: "Mikrofon", webcam: "Webcam", pc: "Büro-PC", time: "14:32", date: "19.11.2026" },
    Sample { tag: "es", speakers: "Altavoces", headset: "Auriculares", tv: "TV", microphone: "Micrófono", webcam: "Cámara web", pc: "PC de la oficina", time: "14:32", date: "19/11/2026" },
    Sample { tag: "it", speakers: "Altoparlanti", headset: "Cuffie", tv: "TV", microphone: "Microfono", webcam: "Webcam", pc: "PC dell'ufficio", time: "14:32", date: "19/11/2026" },
    Sample { tag: "pt-br", speakers: "Alto-falantes", headset: "Headset", tv: "TV", microphone: "Microfone", webcam: "Webcam", pc: "PC do escritório", time: "14:32", date: "19/11/2026" },
    Sample { tag: "nl", speakers: "Luidsprekers", headset: "Headset", tv: "Tv", microphone: "Microfoon", webcam: "Webcam", pc: "Kantoor-pc", time: "14:32", date: "19-11-2026" },
    Sample { tag: "pl", speakers: "Głośniki", headset: "Słuchawki", tv: "TV", microphone: "Mikrofon", webcam: "Kamera internetowa", pc: "Komputer biurowy", time: "14:32", date: "19.11.2026" },
    Sample { tag: "ja", speakers: "スピーカー", headset: "ヘッドセット", tv: "テレビ", microphone: "マイク", webcam: "Web カメラ", pc: "書斎の PC", time: "14:32", date: "2026/11/19" },
    Sample { tag: "zh-hans", speakers: "扬声器", headset: "耳机", tv: "电视", microphone: "麦克风", webcam: "网络摄像头", pc: "书房电脑", time: "14:32", date: "2026/11/19" },
];

/// The sample of `language` (English when it has none).
fn sample(language: Language) -> &'static Sample {
    SAMPLES.iter().find(|s| s.tag == language.tag()).unwrap_or(&SAMPLES[0])
}

/// Three outputs (speakers current), a microphone, three applications playing, the tray
/// icon large, Home Assistant connected.
fn sample_app(language: Language, sample: &Sample) -> App<FakeEngine> {
    let mut engine = FakeEngine::new()
        .with_device(
            Flow::Output,
            device(
                "spk",
                &format!("{} (Realtek(R) Audio)", sample.speakers),
                FormFactor::Speakers,
                Bus::Other,
            ),
        )
        .with_device(
            Flow::Output,
            device("hs", &format!("{} (USB Audio)", sample.headset), FormFactor::Headset, Bus::Usb),
        )
        .with_device(
            Flow::Output,
            device(
                "tv",
                &format!("{} (NVIDIA High Definition Audio)", sample.tv),
                FormFactor::Display,
                Bus::Other,
            ),
        )
        .with_device(
            Flow::Input,
            device(
                "mic",
                &format!("{} (USB Audio)", sample.microphone),
                FormFactor::Microphone,
                Bus::Usb,
            ),
        )
        .with_device(
            Flow::Input,
            device(
                "cam",
                &format!("{} (USB Camera)", sample.microphone),
                FormFactor::Microphone,
                Bus::Usb,
            ),
        );
    engine.levels.insert("spk".into(), (0.62, false));
    engine.levels.insert("hs".into(), (0.48, false));
    engine.levels.insert("tv".into(), (0.30, false));
    engine.levels.insert("mic".into(), (0.74, false));
    engine.levels.insert("cam".into(), (0.55, false));
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
    settings.general.tray_icon_size = TrayIconSize::Large;
    settings.general.tray_icon_style = TrayIconStyle::WithDeviceBadge;
    settings.general.tray_icon_look = TrayIconLook::Colourful;
    settings.devices.outputs = vec![
        known("spk", sample.speakers, DeviceIcon::BookshelfSpeakers),
        known("hs", sample.headset, DeviceIcon::GamingHeadset),
        known("tv", sample.tv, DeviceIcon::Tv),
    ];
    settings.devices.inputs = vec![
        known("mic", sample.microphone, DeviceIcon::DeskMicrophone),
        known("cam", sample.webcam, DeviceIcon::Webcam),
    ];
    let ha = &mut settings.home_assistant;
    ha.enabled = true;
    ha.host = "homeassistant.local".into();
    ha.username = "bettervolume".into();
    ha.pc_name = Some(sample.pc.into());
    ha.node_id = Some("k3v9q2xw".into());
    let mut app = App::new(engine, settings, language);
    app.ha.status = Some(Status::Connected);
    app.set_os_languages(vec![language.locale_name()]);
    app.set_process_list(|| vec!["firefox.exe".into(), "discord.exe".into()]);
    app.on_audio_signals();
    app
}

/// The element whose role is `role` and whose name is exactly `name`.
fn exact(node: &AccessNode, role: AccessRole, name: &str) -> Option<Rect> {
    if node.role == role && node.name == name {
        return Some(node.bounds);
    }
    node.children.iter().find_map(|child| exact(child, role, name))
}

/// The child with role `role` of the group named `group`.
fn in_group(node: &AccessNode, group: &str, role: AccessRole) -> Option<Rect> {
    if node.role == AccessRole::Group && node.name == group {
        if let Some(child) = node.children.iter().find(|c| c.role == role) {
            return Some(child.bounds);
        }
    }
    node.children.iter().find_map(|child| in_group(child, group, role))
}

fn point_json(p: Point) -> String {
    format!("[{:.2}, {:.2}]", p.x, p.y)
}

/// A popup surface on the translucent material (the panel), cut at the taskbar.
fn popup(layer: &str, at: Point) -> String {
    format!("{{\"kind\": \"popup\", \"layer\": \"{layer}\", \"at\": {}}}", point_json(at))
}

/// A surface drawing its own background (the bubble).
fn solid(layer: &str, at: Point) -> String {
    format!("{{\"kind\": \"solid\", \"layer\": \"{layer}\", \"at\": {}}}", point_json(at))
}

/// A window: its title bar above its client area, at `at` (title bar's top-left corner).
fn window(content: &str, caption: &str, at: Point, opacity: f32) -> String {
    format!(
        "{{\"kind\": \"window\", \"layer\": \"{content}\", \"caption\": \"{caption}\", \"at\": {}, \
         \"opacity\": {opacity:.3}}}",
        point_json(at)
    )
}

/// The drawing tools of one language.
struct Painter<'a> {
    out: &'a mut Output,
    sample: &'static Sample,
}

impl Painter<'_> {
    fn measure(&self) -> GraphicsMeasure<'_> {
        GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 }
    }

    /// The whole taskbar: hidden-icons arrow, BetterVolume's icon, network, clock (no
    /// Windows volume icon: BetterVolume replaces it). Returns the layer and the centre
    /// of BetterVolume's icon on the screen.
    fn taskbar(&mut self, app: &App<FakeEngine>, mode: ThemeMode, hover: bool) -> (String, Point) {
        let theme = shell_theme(mode);
        let ink = theme.text_primary;
        let width = DESKTOP.0;
        let (w, h) = Size::new(width, TASKBAR).to_pixels(SCALE);
        let icon = app.tray_icon();
        let clock = Rect::new(width - 8.0 - 78.0, 0.0, 78.0, TASKBAR);
        let network = Rect::new(clock.x - 40.0, 4.0, 40.0, TASKBAR - 8.0);
        let ours = Rect::new(network.x - 4.0 - 32.0, 4.0, 32.0, TASKBAR - 8.0);
        let chevron = Rect::new(ours.x - 28.0, 4.0, 28.0, TASKBAR - 8.0);
        let side = 16.0;
        let (iw, ih) = Size::new(side, side).to_pixels(SCALE);
        let glyph = self
            .out
            .gfx
            .rasterize(iw, ih, SCALE, 1.0, |c| tray::paint(c, &icon, &theme))
            .expect("icon");
        let (time, date) = (self.sample.time, self.sample.date);
        let image = self
            .out
            .gfx
            .rasterize(w, h, SCALE, 1.0, |c| {
                c.clear(Color::TRANSPARENT);
                if hover {
                    c.fill_rounded(ours, 4.0, theme.subtle_hover);
                }
                c.glyph('\u{E70E}', 12.0, chevron, ink);
                let slot = Rect::new(
                    ours.x + (ours.width - side) / 2.0,
                    ours.y + (ours.height - side) / 2.0,
                    side,
                    side,
                );
                c.image(&glyph, slot, 1.0);
                c.glyph('\u{E839}', 16.0, network, ink);
                let time_box = Rect::new(clock.x, 7.0, clock.width - 8.0, 16.0);
                let date_box = Rect::new(clock.x, 25.0, clock.width - 8.0, 16.0);
                c.text(time, CAPTION, time_box, TextAlign::Trailing, ink);
                c.text(date, CAPTION, date_box, TextAlign::Trailing, ink);
            })
            .expect("taskbar");
        let name = self.out.layer(&format!("taskbar-{}", mode_name(mode)), &image);
        let top = DESKTOP.1 - TASKBAR;
        (name, Point::new(ours.x + ours.width / 2.0, top + ours.y + ours.height / 2.0))
    }

    /// The panel as tall as `shown` (its height during a change of height).
    fn flyout(&mut self, view: &FlyoutView, mode: ThemeMode, shown: f32) -> String {
        let theme = shell_theme(mode);
        let mut view = view.clone();
        view.set_shown_height(shown);
        let (w, h) = Size::new(view.width(), shown).to_pixels(SCALE);
        let image =
            self.out.gfx.rasterize(w, h, SCALE, 1.0, |c| view.paint(c, &theme)).expect("flyout");
        self.out.layer(&format!("flyout-{}", mode_name(mode)), &image)
    }

    /// The settings window: its client area and the title bar Windows draws (icon,
    /// title, the three buttons).
    fn window(&mut self, view: &SettingsView, mode: ThemeMode) -> (String, String) {
        let mut theme = Theme::app(&ThemeInputs { apps_mode: mode, ..ThemeInputs::default() });
        theme.window.mica = false;
        let size = view.size();
        let (w, h) = size.to_pixels(SCALE);
        let measure = GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 };
        let image = self
            .out
            .gfx
            .rasterize(w, h, SCALE, 1.0, |c| view.paint(c, &theme, &measure))
            .expect("settings");
        let content = self.out.layer(&format!("settings-{}", mode_name(mode)), &image);
        let title = view.content().name.clone();
        let icon_file = concat!(env!("CARGO_MANIFEST_DIR"), r"\..\..\assets\icon\BetterVolume.ico");
        let icon = bv_shell::app_icon(icon_file, (16.0 * SCALE).round() as u32, &self.out.gfx);
        let ink = theme.text_primary;
        let (cw, ch) = Size::new(size.width, CAPTION_HEIGHT).to_pixels(SCALE);
        let caption = self
            .out
            .gfx
            .rasterize(cw, ch, SCALE, 1.0, |c| {
                c.clear(Color::TRANSPARENT);
                if let Some(icon) = &icon {
                    c.image(icon, Rect::new(12.0, 8.0, 16.0, 16.0), 1.0);
                }
                c.text(&title, CAPTION, Rect::new(40.0, 8.0, 400.0, 16.0), TextAlign::Leading, ink);
                for (i, glyph) in ['\u{E8BB}', '\u{E922}', '\u{E921}'].into_iter().enumerate() {
                    let x = size.width - 46.0 * (i as f32 + 1.0);
                    c.glyph(glyph, 10.0, Rect::new(x, 0.0, 46.0, CAPTION_HEIGHT), ink);
                }
            })
            .expect("caption");
        let caption = self.out.layer(&format!("caption-{}", mode_name(mode)), &caption);
        (content, caption)
    }

    /// The device bubble of `device` and where it sits (centred above the taskbar).
    fn bubble(
        &mut self,
        mode: ThemeMode,
        icon: DeviceIcon,
        name: &str,
        kind: &str,
    ) -> (String, Point) {
        let theme = bubble::theme(shell_theme(mode));
        let content = BubbleContent::Device { icon, name: name.to_owned(), kind: kind.to_owned() };
        let size = bubble::size(&content, &self.out.gfx, 1.0);
        let (w, h) = size.to_pixels(SCALE);
        let image = self
            .out
            .gfx
            .rasterize(w, h, SCALE, 1.0, |c| {
                c.clear(Color::TRANSPARENT);
                c.clipped(Rect::from_size(size), |c| bubble::paint(c, &theme, &content, size));
            })
            .expect("bubble");
        let layer = self.out.layer(&format!("bubble-{}", mode_name(mode)), &image);
        let at =
            Point::new((DESKTOP.0 - size.width) / 2.0, DESKTOP.1 - TASKBAR - 14.0 - size.height);
        (layer, at)
    }

    fn settings_view(
        &self,
        app: &mut App<FakeEngine>,
        page: Page,
        size: (f32, f32),
    ) -> SettingsView {
        app.settings_opening(true);
        app.settings_action(SettingsAction::SelectPage(page.key()));
        let measure = self.measure();
        SettingsView::new(app.settings_content(), Size::new(size.0, size.1), &measure, 1.0)
    }

    fn panel_view(&mut self, app: &mut App<FakeEngine>) -> FlyoutView {
        let content = self.out.with_icons(app.flyout_content());
        let measure = self.measure();
        FlyoutView::new(content, &measure, 1.0, DESKTOP.1 - TASKBAR - 2.0 * GAP)
    }

    fn panel_at(view: &FlyoutView) -> Point {
        Point::new(DESKTOP.0 - GAP - view.width(), DESKTOP.1 - TASKBAR - GAP - view.height())
    }

    /// One screenshot: the surfaces from bottom to top and the taskbar.
    fn shot(name: &str, mode: ThemeMode, items: &[String], taskbar: &str) -> String {
        format!(
            "{{\"name\": \"{name}\", \"mode\": \"{}\", \"items\": [{}], \"taskbar\": \"{taskbar}\"}}",
            mode_name(mode),
            items.join(", ")
        )
    }

    /// The six screenshots of a language, alternating dark and light.
    fn screenshots(&mut self, language: Language) -> Vec<String> {
        let tag = language.locale_name();
        let mut shots = Vec::new();
        let dark = ThemeMode::Dark;
        let light = ThemeMode::Light;

        // 1. The panel: outputs and applications.
        let mut app = sample_app(language, self.sample);
        app.flyout_opening();
        let view = self.panel_view(&mut app);
        let layer = self.flyout(&view, dark, view.height());
        let taskbar = self.taskbar(&app, dark, false).0;
        shots.push(Self::shot(
            &format!("{tag}-1"),
            dark,
            &[popup(&layer, Self::panel_at(&view))],
            &taskbar,
        ));

        // 2. The panel on the microphone view.
        let _ = app.flyout_action(FlyoutAction::SelectTab(FlyoutTab::Input));
        let view = self.panel_view(&mut app);
        let layer = self.flyout(&view, light, view.height());
        let taskbar = self.taskbar(&app, light, false).0;
        shots.push(Self::shot(
            &format!("{tag}-2"),
            light,
            &[popup(&layer, Self::panel_at(&view))],
            &taskbar,
        ));
        app.flyout_closed();

        let at = Point::new(WINDOW_AT.0, WINDOW_AT.1);
        // 3. Settings, General page.
        let mut app = sample_app(language, self.sample);
        let view = self.settings_view(&mut app, Page::General, WINDOW);
        let (content, caption) = self.window(&view, dark);
        let taskbar = self.taskbar(&app, dark, false).0;
        shots.push(Self::shot(
            &format!("{tag}-3"),
            dark,
            &[window(&content, &caption, at, 1.0)],
            &taskbar,
        ));

        // 4. Devices page, the icon picker of the second output open.
        let mut view = self.settings_view(&mut app, Page::Devices, WINDOW);
        let measure = GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 };
        let button =
            in_group(&view.access_tree(&measure), self.sample.headset, AccessRole::Expander)
                .expect("icon button of the second output");
        let _ = view.press(centre(button), &measure);
        let _ = view.release(centre(button), &measure);
        let _ = view.pointer_leave();
        let (content, caption) = self.window(&view, light);
        let taskbar = self.taskbar(&app, light, false).0;
        shots.push(Self::shot(
            &format!("{tag}-4"),
            light,
            &[window(&content, &caption, at, 1.0)],
            &taskbar,
        ));

        // 5. Home Assistant page, connected.
        let view = self.settings_view(&mut app, Page::HomeAssistant, WINDOW);
        let (content, caption) = self.window(&view, dark);
        let taskbar = self.taskbar(&app, dark, false).0;
        shots.push(Self::shot(
            &format!("{tag}-5"),
            dark,
            &[window(&content, &caption, at, 1.0)],
            &taskbar,
        ));

        // 6. The tray icon choices of the General page, the icon next to the clock and the
        // bubble shown when the output changes.
        let size = (WINDOW.0, WINDOW.1 - 70.0);
        let mut view = self.settings_view(&mut app, Page::General, size);
        let measure = GraphicsMeasure { graphics: &self.out.gfx, text_scale: 1.0 };
        let probe = Point::new(size.0 / 2.0, size.1 / 2.0);
        for _ in 0..40 {
            let tree = view.access_tree(&measure);
            let tiles = tree_list_bottom(&tree);
            if tiles.is_some_and(|bottom| bottom <= size.1 - 24.0) {
                break;
            }
            let _ = view.wheel(probe, -1, &measure);
        }
        let _ = view.pointer_leave();
        let (content, caption) = self.window(&view, light);
        let (bubble, bubble_at) = self.bubble(
            light,
            DeviceIcon::GamingHeadset,
            self.sample.headset,
            language.text(Msg::BubbleOutput),
        );
        app.engine.defaults[Flow::Output.index()] = [Some("hs".into()), Some("hs".into())];
        app.engine.sync();
        app.on_audio_signals();
        let taskbar = self.taskbar(&app, light, false).0;
        let top = Point::new(WINDOW_AT.0, 24.0);
        shots.push(Self::shot(
            &format!("{tag}-6"),
            light,
            &[window(&content, &caption, top, 1.0), solid(&bubble, bubble_at)],
            &taskbar,
        ));
        shots
    }
}

/// The bottom of the tray icon picture tiles (the last picture group of the General page).
fn tree_list_bottom(node: &AccessNode) -> Option<f32> {
    let mut bottom: Option<f32> = None;
    fn walk(node: &AccessNode, inside: bool, bottom: &mut Option<f32>) {
        let pictures = node.role == AccessRole::List && inside;
        for child in &node.children {
            if pictures && child.role == AccessRole::ListItem {
                let b = child.bounds.y + child.bounds.height;
                *bottom = Some(bottom.map_or(b, |x: f32| x.max(b)));
            }
            walk(child, inside || node.role == AccessRole::Group, bottom);
        }
    }
    walk(node, false, &mut bottom);
    bottom
}

/// One frame of the video: the surfaces of each theme shown, the pointer.
struct Frame {
    ms: u32,
    mix: f32,
    scenes: [Option<String>; 2],
    cursor: Point,
    down: bool,
}

/// Which surface the pointer is over.
#[derive(Clone, Copy, PartialEq)]
enum Under {
    Nothing,
    Panel,
    Window,
}

/// The English demonstration video being recorded.
struct Video<'a> {
    painter: Painter<'a>,
    app: App<FakeEngine>,
    panel: Option<FlyoutView>,
    /// Shown height of the panel (it changes while its content changes).
    shown: f32,
    /// Vertical offset of the panel from its resting place (entrance and exit).
    offset: f32,
    window: Option<SettingsView>,
    window_opacity: f32,
    /// 0 = dark, 1 = light (in between: the theme change fades).
    mix: f32,
    cursor: Point,
    down: bool,
    hover: bool,
    under: Under,
    frames: Vec<Frame>,
}

impl Video<'_> {
    fn max_height(&self) -> f32 {
        DESKTOP.1 - TASKBAR - 2.0 * GAP
    }

    fn panel_origin(&self) -> Point {
        let width = self.panel.as_ref().map_or(0.0, FlyoutView::width);
        Point::new(DESKTOP.0 - GAP - width, DESKTOP.1 - TASKBAR - GAP - self.shown + self.offset)
    }

    fn window_origin() -> Point {
        Point::new(WINDOW_AT.0, WINDOW_AT.1 + CAPTION_HEIGHT)
    }

    fn record(&mut self, ms: u32) {
        let wanted = [self.mix < 1.0, self.mix > 0.0];
        let modes = [ThemeMode::Dark, ThemeMode::Light];
        let mut scenes: [Option<String>; 2] = [None, None];
        for i in 0..2 {
            if !wanted[i] {
                continue;
            }
            let mode = modes[i];
            let mut items = Vec::new();
            if let Some(view) = self.window.clone() {
                let (content, caption) = self.painter.window(&view, mode);
                items.push(window(
                    &content,
                    &caption,
                    Point::new(WINDOW_AT.0, WINDOW_AT.1),
                    self.window_opacity,
                ));
            }
            if let Some(view) = self.panel.clone() {
                let layer = self.painter.flyout(&view, mode, self.shown);
                items.push(popup(&layer, self.panel_origin()));
            }
            let taskbar = self.painter.taskbar(&self.app, mode, self.hover).0;
            scenes[i] =
                Some(format!("{{\"items\": [{}], \"taskbar\": \"{taskbar}\"}}", items.join(", ")));
        }
        self.frames.push(Frame { ms, mix: self.mix, scenes, cursor: self.cursor, down: self.down });
    }

    fn hold(&mut self, ms: u32) {
        self.record(ms);
    }

    fn under(&self, p: Point) -> Under {
        if let Some(view) = &self.panel {
            let o = self.panel_origin();
            if Rect::new(o.x, o.y, view.width(), self.shown).contains(p) {
                return Under::Panel;
            }
        }
        if let Some(view) = &self.window {
            let o = Self::window_origin();
            let size = view.size();
            if Rect::new(o.x, o.y, size.width, size.height).contains(p) {
                return Under::Window;
            }
        }
        Under::Nothing
    }

    /// Moves the pointer to `to` (screen coordinates) in `ms`, telling the surfaces.
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
        let under = self.under(self.cursor);
        if under != self.under && self.under == Under::Window {
            if let Some(view) = self.window.as_mut() {
                let _ = view.pointer_leave();
            }
        }
        self.under = under;
        let p = self.cursor;
        match under {
            Under::Panel => {
                let o = self.panel_origin();
                if let Some(view) = self.panel.as_mut() {
                    let response = view.pointer_move(Point::new(p.x - o.x, p.y - o.y));
                    self.panel_act(response);
                }
            }
            Under::Window => {
                let o = Self::window_origin();
                let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
                if let Some(view) = self.window.as_mut() {
                    let response = view.pointer_move(Point::new(p.x - o.x, p.y - o.y), &measure);
                    if let Some(action) = response.action {
                        self.window_act(action);
                    }
                }
            }
            Under::Nothing => {}
        }
    }

    /// Applies a panel action; returns whether the settings window was asked for.
    fn panel_act(&mut self, response: Response) -> bool {
        let Some(action) = response.action else { return false };
        let effect = self.app.flyout_action(action);
        self.app.on_audio_signals();
        if self.refresh_panel() {
            self.resize();
        }
        matches!(effect, FlyoutEffect::OpenSettings)
    }

    fn window_act(&mut self, action: SettingsAction) {
        let _ = self.app.settings_action(action);
        let content = self.app.settings_content();
        let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
        if let Some(view) = self.window.as_mut() {
            view.set_content(content, &measure);
        }
    }

    fn refresh_panel(&mut self) -> bool {
        let content = self.painter.out.with_icons(self.app.flyout_content());
        let max = self.max_height();
        let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
        self.panel
            .as_mut()
            .is_some_and(|view| view.set_content(content, &measure, 1.0, max).resized)
    }

    /// A click where the pointer is; returns whether the settings window was asked for.
    fn click(&mut self) -> bool {
        let p = self.cursor;
        let mut settings = false;
        self.down = true;
        match self.under(p) {
            Under::Panel => {
                let o = self.panel_origin();
                let local = Point::new(p.x - o.x, p.y - o.y);
                if let Some(view) = self.panel.as_mut() {
                    let response = view.press(local);
                    settings |= self.panel_act(response);
                }
                self.record(FRAME_MS * 2);
                self.down = false;
                let o = self.panel_origin();
                let local = Point::new(p.x - o.x, p.y - o.y);
                if let Some(view) = self.panel.as_mut() {
                    let response = view.release(local);
                    settings |= self.panel_act(response);
                }
            }
            Under::Window => {
                let o = Self::window_origin();
                let local = Point::new(p.x - o.x, p.y - o.y);
                let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
                let mut actions = Vec::new();
                if let Some(view) = self.window.as_mut() {
                    actions.extend(view.press(local, &measure).action);
                }
                for action in actions.drain(..) {
                    self.window_act(action);
                }
                self.record(FRAME_MS * 2);
                self.down = false;
                let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
                if let Some(view) = self.window.as_mut() {
                    actions.extend(view.release(local, &measure).action);
                }
                for action in actions {
                    self.window_act(action);
                }
            }
            Under::Nothing => {
                self.record(FRAME_MS * 2);
                self.down = false;
            }
        }
        self.record(FRAME_MS);
        settings
    }

    /// The panel height follows its content (the "fits the content" mode).
    fn resize(&mut self) {
        let Some(to) = self.panel.as_ref().map(FlyoutView::height) else { return };
        let from = self.shown;
        let steps = FLYOUT_RESIZE.duration_ms / FRAME_MS;
        for i in 1..=steps {
            let t = FLYOUT_RESIZE.progress((i * FRAME_MS) as f32);
            self.shown = from + (to - from) * t;
            let shown = self.shown;
            if let Some(view) = self.panel.as_mut() {
                view.set_shown_height(shown);
            }
            self.record(FRAME_MS);
        }
        self.shown = to;
        if let Some(view) = self.panel.as_mut() {
            view.set_shown_height(to);
            view.end_transition();
        }
    }

    fn panel_element(&self, role: AccessRole, name: &str) -> Point {
        let view = self.panel.as_ref().expect("panel open");
        let bounds = find(&view.access_tree(), role, name)
            .unwrap_or_else(|| panic!("no {role:?} named {name}"));
        let o = self.panel_origin();
        let c = centre(bounds);
        Point::new(o.x + c.x, o.y + c.y)
    }

    fn slider(&self, name: &str, value: f32) -> Point {
        let view = self.panel.as_ref().expect("panel open");
        let bounds = find(&view.access_tree(), AccessRole::Slider, name)
            .unwrap_or_else(|| panic!("no slider named {name}"));
        let o = self.panel_origin();
        // The thumb travels the track between half a thumb from each end.
        let half = 10.0;
        let x = bounds.x + half + (bounds.width - 2.0 * half) * value / 100.0;
        Point::new(o.x + x, o.y + bounds.y + bounds.height / 2.0)
    }

    fn window_tree(&self) -> AccessNode {
        let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
        self.window.as_ref().expect("window open").access_tree(&measure)
    }

    fn window_point(r: Rect) -> Point {
        let o = Self::window_origin();
        let c = centre(r);
        Point::new(o.x + c.x, o.y + c.y)
    }

    fn window_element(&self, role: AccessRole, name: &str) -> Point {
        let bounds = exact(&self.window_tree(), role, name)
            .unwrap_or_else(|| panic!("no {role:?} named {name} in the settings window"));
        Self::window_point(bounds)
    }

    fn open_panel(&mut self) {
        self.app.flyout_opening();
        let content = self.painter.out.with_icons(self.app.flyout_content());
        let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
        let view = FlyoutView::new(content, &measure, 1.0, self.max_height());
        self.shown = view.height();
        self.panel = Some(view);
        self.slide(FLYOUT_OPEN, true);
    }

    fn close_panel(&mut self) {
        self.slide(FLYOUT_CLOSE, false);
        self.panel = None;
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

    fn open_window(&mut self) {
        let view = self.painter.settings_view(&mut self.app, Page::General, WINDOW);
        self.window = Some(view);
        let steps = WINDOW_FADE_MS / FRAME_MS;
        for i in 1..=steps {
            self.window_opacity = i as f32 / steps as f32;
            self.record(FRAME_MS);
        }
        self.window_opacity = 1.0;
    }

    fn fade_to_light(&mut self, ms: u32) {
        let steps = ms / FRAME_MS;
        for i in 1..=steps {
            self.mix = i as f32 / steps as f32;
            self.record(FRAME_MS);
        }
        self.mix = 1.0;
    }

    /// Turns the wheel down over the window until the tray icon tiles are in view.
    fn scroll_to_tiles(&mut self) {
        let measure_bottom = |video: &Self| tree_list_bottom(&video.window_tree());
        for _ in 0..40 {
            if measure_bottom(self).is_some_and(|bottom| bottom <= WINDOW.1 - 24.0) {
                break;
            }
            let o = Self::window_origin();
            let local = Point::new(self.cursor.x - o.x, self.cursor.y - o.y);
            let measure = GraphicsMeasure { graphics: &self.painter.out.gfx, text_scale: 1.0 };
            if let Some(view) = self.window.as_mut() {
                let _ = view.wheel(local, -1, &measure);
            }
            self.record(FRAME_MS * 2);
        }
    }
}

/// The video: the README demo (open, switch the output, lower an application, fold the
/// applications, dark to light), then the settings: the tray icon's picture and style
/// changed with the icon next to the clock following, a device's icon changed on the
/// Devices page, the Home Assistant page, and the panel showing the new device icon.
fn video(out: &mut Output) -> String {
    let language = Language::ENGLISH;
    let sample = sample(language);
    let mut app = sample_app(language, sample);
    // The tray icon starts as Windows draws it (large size all along).
    app.settings.general.tray_icon_style = TrayIconStyle::SameAsWindows;
    app.settings.general.tray_icon_look = TrayIconLook::Windows;
    let rest = Point::new(DESKTOP.0 * 0.42, DESKTOP.1 * 0.45);
    let mut v = Video {
        painter: Painter { out, sample },
        app,
        panel: None,
        shown: 0.0,
        offset: 0.0,
        window: None,
        window_opacity: 0.0,
        mix: 0.0,
        cursor: rest,
        down: false,
        hover: false,
        under: Under::Nothing,
        frames: Vec::new(),
    };
    let icon = v.painter.taskbar(&v.app, ThemeMode::Dark, false).1;
    let headset = v.painter.sample.headset;
    v.hold(900);
    v.point(icon, 720);
    v.hover = true;
    v.hold(240);
    v.click();
    v.open_panel();
    v.hover = false;
    v.hold(700);
    // Switch the output.
    let tile = v.panel_element(AccessRole::ListItem, headset);
    v.point(tile, 640);
    v.hold(200);
    v.click();
    v.hold(800);
    // Lower Firefox.
    let from = v.slider("Firefox", 85.0);
    v.point(from, 680);
    v.hold(160);
    v.down = true;
    let o = v.panel_origin();
    if let Some(view) = v.panel.as_mut() {
        let response = view.press(Point::new(from.x - o.x, from.y - o.y));
        v.panel_act(response);
    }
    v.record(FRAME_MS * 2);
    let to = v.slider("Firefox", 35.0);
    v.point(to, 640);
    v.down = false;
    let o = v.panel_origin();
    if let Some(view) = v.panel.as_mut() {
        let response = view.release(Point::new(to.x - o.x, to.y - o.y));
        v.panel_act(response);
    }
    v.record(FRAME_MS);
    v.hold(700);
    // Fold the applications.
    let applications = Language::ENGLISH.text(Msg::FlyoutApps);
    let fold = v.panel_element(AccessRole::Expander, applications);
    v.point(fold, 600);
    v.hold(160);
    v.click();
    v.hold(700);
    // Dark to light.
    v.fade_to_light(320);
    v.hold(900);
    // Open the settings from the panel's button.
    let gear = v.panel_element(AccessRole::Button, Language::ENGLISH.text(Msg::MenuSettings));
    v.point(gear, 640);
    v.hold(200);
    let asked = v.click();
    assert!(asked, "the panel's settings button opens the settings");
    v.close_panel();
    v.open_window();
    v.hold(500);
    // The tray icon's picture and style, the icon next to the clock following.
    v.point(Point::new(WINDOW_AT.0 + WINDOW.0 * 0.6, WINDOW_AT.1 + WINDOW.1 * 0.6), 600);
    v.pointer_moved();
    v.scroll_to_tiles();
    v.hold(500);
    let picture = v.window_element(
        AccessRole::ListItem,
        Language::ENGLISH.text(Msg::SettingsTrayPictureDevice),
    );
    v.point(picture, 640);
    v.hold(200);
    v.click();
    v.hold(500);
    v.point(icon, 700);
    v.hold(900);
    let style = v.window_element(
        AccessRole::ListItem,
        Language::ENGLISH.text(Msg::SettingsTrayLookColourful),
    );
    v.point(style, 700);
    v.hold(200);
    v.click();
    v.hold(500);
    v.point(icon, 700);
    v.hold(900);
    // A device's icon, on the Devices page.
    let devices =
        v.window_element(AccessRole::ListItem, Language::ENGLISH.text(Msg::SettingsPageDevices));
    v.point(devices, 700);
    v.hold(200);
    v.click();
    v.hold(700);
    let button = in_group(&v.window_tree(), headset, AccessRole::Expander)
        .map(Video::window_point)
        .expect("icon button of the headset");
    v.point(button, 640);
    v.hold(200);
    v.click();
    v.hold(800);
    let choice = v
        .window_element(AccessRole::ListItem, Language::ENGLISH.text(Msg::IconBluetoothHeadphones));
    v.point(choice, 700);
    v.hold(300);
    v.click();
    v.hold(800);
    // The Home Assistant page.
    let ha = v.window_element(AccessRole::ListItem, Language::ENGLISH.text(Msg::SettingsPageHa));
    v.point(ha, 700);
    v.hold(200);
    v.click();
    v.hold(1800);
    // The panel shows the headset's new icon.
    v.point(icon, 800);
    v.hover = true;
    v.hold(240);
    v.click();
    v.open_panel();
    v.hover = false;
    v.hold(400);
    let tile = v.panel_element(AccessRole::ListItem, headset);
    v.point(Point::new(tile.x - 40.0, tile.y + 30.0), 600);
    v.hold(2200);
    frames_json(&v.frames)
}

fn frames_json(frames: &[Frame]) -> String {
    let mut json = String::new();
    for (i, f) in frames.iter().enumerate() {
        let scene = |s: &Option<String>| s.clone().unwrap_or_else(|| "null".into());
        let _ = write!(
            json,
            "{}{{\"ms\": {}, \"mix\": {:.3}, \"dark\": {}, \"light\": {}, \"cursor\": {}, \"down\": {}}}",
            if i == 0 { "" } else { ",\n    " },
            f.ms,
            f.mix,
            scene(&f.scenes[0]),
            scene(&f.scenes[1]),
            point_json(f.cursor),
            f.down
        );
    }
    json
}

#[test]
#[ignore = "writes the Store images' layers; run on demand (see the module documentation)"]
fn store_images() {
    let out = PathBuf::from(std::env::var("BV_STORE").expect("BV_STORE = output folder"));
    std::fs::create_dir_all(&out).expect("output folder");
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let mut output =
        Output { gfx, out, layers: HashMap::new(), sizes: String::new(), icons: HashMap::new() };
    let mut shots = Vec::new();
    for language in Language::all() {
        output.gfx.set_text_locale(&language.locale_name());
        let mut painter = Painter { out: &mut output, sample: sample(language) };
        shots.extend(painter.screenshots(language));
    }
    output.gfx.set_text_locale("en-US");
    let frames = video(&mut output);
    let json = format!(
        "{{\n  \"scale\": {SCALE},\n  \"desktop\": [{}, {}],\n  \"taskbar\": {TASKBAR},\n  \
         \"layers\": {{{}}},\n  \"shots\": [\n    {}\n  ],\n  \"frames\": [\n    {frames}\n  ]\n}}\n",
        DESKTOP.0,
        DESKTOP.1,
        output.sizes,
        shots.join(",\n    ")
    );
    std::fs::write(output.out.join("scenes.json"), json).expect("scenes");
}
