//! Language sheets: every window of the application drawn in every shipped language by
//! the application's own views and drawing code, for a visual check of the translations
//! (nothing cut, no missing character). Not part of the normal test run:
//!
//!   set BV_SHEETS=<output folder>
//!   cargo test -p bv-app language_sheets -- --ignored
//!
//! Writes one 32-bit BMP per window and language (`<tag>-<nn>-<window>.bmp`) and the tray
//! tooltips (`tooltips.txt`, drawn by Windows, so given as text).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use bv_audio::fake::FakeEngine;
use bv_audio::{AppKey, Bus, DeviceInfo, DeviceState, Flow, FormFactor, HeardApp, Stream};
use bv_core::devices::KnownDevice;
use bv_core::i18n::{Language, Msg};
use bv_core::settings::{FlyoutHeight, Settings};
use bv_shell::MenuLine;
use bv_ui_kit::backend::{DeviceKind, Graphics, Pixels};
use bv_ui_kit::controls::bubble::{self, BubbleContent};
use bv_ui_kit::controls::device_icon::DeviceIcon;
use bv_ui_kit::controls::flyout::{self, FlyoutTab, FlyoutView, GraphicsMeasure};
use bv_ui_kit::controls::menu::{Menu, MenuEntry};
use bv_ui_kit::controls::settings::{SettingsContent, SettingsKey, SettingsView};
use bv_ui_kit::theme::{Material, Theme, ThemeInputs, ThemeMode};
use bv_ui_kit::{Color, Rect, Size};

use super::settings_window::Page;
use super::App;
use crate::setup::{Mode, Setup};

/// Width of the settings window (its default) and a height tall enough for every page.
const SETTINGS: (f32, f32) = (980.0, 1500.0);
/// The narrowest settings window, for the longest texts.
const NARROW: (f32, f32) = (680.0, 1900.0);

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
    Stream {
        app: AppKey::executable(&path),
        process: 7,
        name: name.into(),
        icon: None,
        path: Some(path),
        volume,
        muted: false,
        heard_here: true,
    }
}

fn running() -> Vec<String> {
    vec!["firefox.exe".into(), "discord.exe".into()]
}

/// Two outputs, a disconnected one remembered, a microphone; three applications playing
/// and one remembered; Home Assistant on but not set up.
fn app(language: Language) -> App<FakeEngine> {
    let mut engine = FakeEngine::new()
        .with_device(
            Flow::Output,
            device("spk", "Speakers (Realtek(R) Audio)", FormFactor::Speakers, Bus::Other),
        )
        .with_device(
            Flow::Output,
            device("hp", "Headphones (WH-1000XM4)", FormFactor::Headphones, Bus::Bluetooth),
        )
        .with_device(
            Flow::Input,
            device("mic", "Microphone (Yeti Stereo Microphone)", FormFactor::Microphone, Bus::Usb),
        );
    engine.levels.insert("spk".into(), (0.42, false));
    engine.streams = vec![
        stream("firefox.exe", "Firefox", 0.8),
        stream("discord.exe", "Discord", 0.6),
        stream("spotify.exe", "Spotify", 0.35),
        Stream { app: AppKey::System, name: String::new(), ..stream("x", "", 1.0) },
    ];
    engine.sync();
    let mut settings = Settings::default();
    settings.general.flyout_height = FlyoutHeight::Content;
    settings.devices.outputs.push(KnownDevice::new("spk"));
    settings.devices.outputs.push(KnownDevice::new("hp"));
    settings.devices.outputs.push(KnownDevice {
        windows_name: "LG TV SSCR2 (NVIDIA High Definition Audio)".into(),
        ..KnownDevice::new("tv")
    });
    settings.home_assistant.enabled = true;
    let mut app = App::new(engine, settings, language);
    app.set_process_list(running);
    let path = r"C:\Program Files\VLC\vlc.exe".to_owned();
    app.engine.heard_apps =
        vec![HeardApp { app: AppKey::executable(&path), name: "VLC".into(), path: Some(path) }];
    app.on_audio_signals();
    app
}

fn shell_theme() -> Theme {
    Theme::shell(&ThemeInputs { system_mode: ThemeMode::Dark, ..ThemeInputs::default() })
}

fn window_theme() -> Theme {
    let mut theme =
        Theme::app(&ThemeInputs { apps_mode: ThemeMode::Light, ..ThemeInputs::default() });
    theme.window.mica = false;
    theme
}

struct Sheets {
    gfx: Graphics,
    out: PathBuf,
}

impl Sheets {
    fn write(&self, language: Language, index: &mut u32, name: &str, image: &Pixels) {
        *index += 1;
        let file = format!("{}-{:02}-{name}.bmp", language.tag(), *index);
        write_bmp(&self.out.join(file), image);
    }

    fn flyout(
        &self,
        language: Language,
        index: &mut u32,
        name: &str,
        content: flyout::FlyoutContent,
    ) {
        let backdrop = Color::rgb(0x1C1C1C);
        let theme = Theme { material: Material::Solid(backdrop), ..shell_theme() };
        let measure = GraphicsMeasure { graphics: &self.gfx, text_scale: 1.0 };
        let view = FlyoutView::new(content, &measure, 1.0, 1008.0);
        let (w, h) = view.size().to_pixels(1.0);
        let image = self.gfx.rasterize(w, h, 1.0, 1.0, |c| view.paint(c, &theme)).expect("flyout");
        self.write(language, index, name, &image);
    }

    fn settings(
        &self,
        language: Language,
        index: &mut u32,
        name: &str,
        content: &SettingsContent,
        size: (f32, f32),
        then: impl Fn(&mut SettingsView, &GraphicsMeasure<'_>),
    ) {
        let theme = window_theme();
        let measure = GraphicsMeasure { graphics: &self.gfx, text_scale: 1.0 };
        let size = Size::new(size.0, size.1);
        let mut view = SettingsView::new(content.clone(), size, &measure, 1.0);
        then(&mut view, &measure);
        let (w, h) = size.to_pixels(1.0);
        let image = self
            .gfx
            .rasterize(w, h, 1.0, 1.0, |c| view.paint(c, &theme, &measure))
            .expect("settings");
        self.write(language, index, name, &image);
    }

    /// The first forget confirmation a "Tab × n, Enter" sequence opens.
    fn first_dialog(
        &self,
        language: Language,
        index: &mut u32,
        name: &str,
        content: &SettingsContent,
    ) {
        let measure = GraphicsMeasure { graphics: &self.gfx, text_scale: 1.0 };
        let size = Size::new(SETTINGS.0, 900.0);
        let tabs = (1..60).find(|&n| {
            let mut view = SettingsView::new(content.clone(), size, &measure, 1.0);
            for _ in 0..n {
                view.key(SettingsKey::Tab { back: false }, &measure);
            }
            view.key(SettingsKey::Enter, &measure);
            view.confirming()
        });
        let tabs = tabs.unwrap_or_else(|| panic!("{name}: no confirmation reachable"));
        self.settings(language, index, name, content, (SETTINGS.0, 900.0), |view, measure| {
            for _ in 0..tabs {
                view.key(SettingsKey::Tab { back: false }, measure);
            }
            view.key(SettingsKey::Enter, measure);
        });
    }
}

fn menu_entries(language: Language) -> Vec<MenuEntry> {
    crate::menu(language)
        .into_iter()
        .map(|line| match line {
            MenuLine::Item { icon, label, enabled, .. } => MenuEntry::Item { icon, label, enabled },
            MenuLine::Separator => MenuEntry::Separator,
        })
        .collect()
}

fn render_language(sheets: &Sheets, language: Language, tooltips: &mut String) {
    let mut n = 0;
    let n = &mut n;
    let mut app = app(language);
    let _ = writeln!(tooltips, "{}\t{}", language.tag(), app.tooltip());

    // Flyout: outputs with the applications, the update line; the microphone tab.
    app.flyout_opening();
    let mut content = app.flyout_content();
    content.update = Some(flyout::UpdateLine {
        text: language.text(Msg::UpdateFailed).to_owned(),
        action: Some(language.text(Msg::UpdateRetry).to_owned()),
        close: Some(language.text(Msg::UpdateClose).to_owned()),
    });
    sheets.flyout(language, n, "flyout", content);
    app.flyout_action(bv_ui_kit::controls::flyout::FlyoutAction::SelectTab(FlyoutTab::Input));
    sheets.flyout(language, n, "flyout-microphone", app.flyout_content());
    app.flyout_closed();

    // Tray menu.
    let theme = shell_theme();
    let menu = Menu::measured(menu_entries(language), &sheets.gfx, 1.0);
    let (w, h) = menu.size().to_pixels(1.0);
    let image = sheets
        .gfx
        .rasterize(w, h, 1.0, 1.0, |c| {
            if let Material::Acrylic { tint, .. } = theme.material {
                c.fill_rect(Rect::from_size(menu.size()), tint);
            }
            let solid = Theme { material: Material::Solid(Color::TRANSPARENT), ..theme };
            menu.paint_items(c, &solid);
        })
        .expect("menu");
    sheets.write(language, n, "menu", &image);

    // Bubbles above the taskbar.
    let bubble_theme = bubble::theme(shell_theme());
    for (name, icon, device, kind) in [
        ("bubble", DeviceIcon::WiredHeadphones, "Headphones", Msg::BubbleOutput),
        ("bubble-calls", DeviceIcon::DeskMicrophone, "Microphone", Msg::BubbleInputCalls),
    ] {
        let content = BubbleContent::Device {
            icon,
            name: device.into(),
            kind: language.text(kind).to_owned(),
        };
        let size = bubble::size(&content, &sheets.gfx, 1.0);
        let (w, h) = size.to_pixels(1.0);
        let image = sheets
            .gfx
            .rasterize(w, h, 1.0, 1.0, |c| {
                c.clear(Color::rgb(0x181818));
                c.clipped(Rect::from_size(size), |c| {
                    bubble::paint(c, &bubble_theme, &content, size)
                });
            })
            .expect("bubble");
        sheets.write(language, n, name, &image);
    }

    // Settings window: the first-run question, every page, the confirmations.
    app.settings_opening(false);
    app.start_window_opening();
    sheets.settings(language, n, "question", &app.settings_content(), SETTINGS, |_, _| {});
    app.volume_icon_window_closed();
    app.volume_icon_incident();
    let general = app.settings_content();
    sheets.settings(language, n, "general", &general, SETTINGS, |_, _| {});
    sheets.settings(language, n, "general-narrow", &general, NARROW, |_, _| {});
    for (page, name) in [
        (Page::Devices, "devices"),
        (Page::Applications, "applications"),
        (Page::HomeAssistant, "home-assistant"),
        (Page::About, "about"),
    ] {
        app.settings_action(bv_ui_kit::controls::settings::SettingsAction::SelectPage(page.key()));
        let content = app.settings_content();
        sheets.settings(language, n, name, &content, SETTINGS, |_, _| {});
        if matches!(page, Page::Devices | Page::Applications | Page::HomeAssistant) {
            sheets.settings(language, n, &format!("{name}-narrow"), &content, NARROW, |_, _| {});
        }
        match page {
            Page::Devices | Page::Applications => {
                sheets.first_dialog(language, n, &format!("{name}-forget"), &content);
            }
            Page::HomeAssistant => {
                sheets.settings(language, n, "home-assistant-close", &content, SETTINGS, |v, m| {
                    v.request_close(m);
                });
                // The certificate notices, with their fingerprint (the longest notices).
                for (name, problem) in [
                    ("certificate-unknown", bv_mqtt::Problem::CertificateUnknown([0xA7; 32])),
                    ("certificate-changed", bv_mqtt::Problem::CertificateChanged([0x3C; 32])),
                ] {
                    app.settings_window.ha.outcome =
                        Some(super::ha_page::Outcome::Tested(Err(problem)));
                    let content = app.settings_content();
                    sheets.settings(language, n, name, &content, SETTINGS, |_, _| {});
                    sheets.settings(
                        language,
                        n,
                        &format!("{name}-narrow"),
                        &content,
                        NARROW,
                        |_, _| {},
                    );
                }
                app.settings_window.ha.outcome = None;
            }
            _ => {}
        }
    }
    app.settings_action(bv_ui_kit::controls::settings::SettingsAction::SelectPage(
        Page::Applications.key(),
    ));
    app.settings_action(bv_ui_kit::controls::settings::SettingsAction::Invoke(
        super::apps_page::SORT,
    ));
    sheets.settings(language, n, "applications-sort", &app.settings_content(), SETTINGS, |_, _| {});

    // Installer and uninstaller windows.
    let compact = bv_ui_kit::tokens::settings::COMPACT_SIZE;
    for (name, mode) in [
        ("setup", Mode::Install { installed: None }),
        ("setup-update", Mode::Install { installed: Some("2.0.0".into()) }),
        ("uninstall", Mode::Uninstall),
    ] {
        let setup = Setup::new(mode, language, "2.1.0", true);
        sheets.settings(language, n, name, &setup.content(), compact, |_, _| {});
    }
}

#[test]
#[ignore = "writes images for a visual check; run on demand (see the module documentation)"]
fn language_sheets() {
    let out = PathBuf::from(std::env::var("BV_SHEETS").expect("BV_SHEETS = output folder"));
    std::fs::create_dir_all(&out).expect("output folder");
    let sheets = Sheets { gfx: Graphics::new(DeviceKind::Software).expect("graphics"), out };
    let mut tooltips = String::new();
    for language in Language::in_display_order() {
        sheets.gfx.set_text_locale(&language.locale_name());
        render_language(&sheets, language, &mut tooltips);
    }
    std::fs::write(sheets.out.join("tooltips.txt"), tooltips).expect("tooltips");
}

/// A 32-bit top-down BMP (premultiplied pixels over opaque backgrounds: stored as is).
fn write_bmp(path: &Path, image: &Pixels) {
    let size = image.data.len() as u32;
    let mut bmp = Vec::with_capacity(54 + image.data.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(54 + size).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(image.width as i32).to_le_bytes());
    bmp.extend_from_slice(&(-(image.height as i32)).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 16]);
    bmp.extend_from_slice(&image.data);
    std::fs::write(path, bmp).expect("write image");
}
