//! Renders design-system samples to PNG files, for visual checks.
//!
//!   cargo run -p bv-ui-kit --example gallery -- <output folder>
//!
//! Produces real renderings from the backend (same code paths as the application): tray
//! icons at every Windows scaling on light and dark taskbars, icon-font candidates, and the
//! context menu in both modes over a sample background.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use bv_ui_kit::backend::{DeviceKind, Graphics, Pixels, TextAlign};
use bv_ui_kit::controls::bubble::{self, BubbleContent};
use bv_ui_kit::controls::device_icon::{self, DeviceIcon};
use bv_ui_kit::controls::flyout;
use bv_ui_kit::controls::menu::{Menu, MenuEntry};
use bv_ui_kit::controls::tray::{self, DeviceType, TrayIcon};
use bv_ui_kit::icons::{Icon, VolumeLevel};
use bv_ui_kit::theme::{Material, Theme, ThemeInputs, ThemeMode};
use bv_ui_kit::tokens::typography::{BODY, CAPTION};
use bv_ui_kit::{Color, Point, Rect};

fn main() {
    let out: PathBuf = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&out).expect("output folder");
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");

    tray_icons(&gfx, &out);
    badges(&gfx, &out);
    glyph_sheet(&gfx, &out);
    menus(&gfx, &out);
    device_icons(&gfx, &out);
    flyouts(&gfx, &out);
    settings_windows(&gfx, &out);
    bubbles(&gfx, &out);
}

fn theme(mode: ThemeMode) -> Theme {
    Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() })
}

/// Tray icons at the small-icon sizes of each Windows scaling, on a taskbar background,
/// at both sizes (`-large`: FR-083).
fn tray_icons(gfx: &Graphics, out: &Path) {
    let levels = [
        None,
        Some(VolumeLevel::Muted),
        Some(VolumeLevel::Zero),
        Some(VolumeLevel::Low),
        Some(VolumeLevel::Medium),
        Some(VolumeLevel::High),
    ];
    for (mode, name, background) in [
        (ThemeMode::Dark, "dark", Color::rgb(0x1C1C1C)),
        (ThemeMode::Light, "light", Color::rgb(0xEEEEEE)),
    ] {
        let theme = theme(mode);
        for (pixels, large) in [16u32, 20, 24, 28, 32, 36, 40, 48]
            .into_iter()
            .flat_map(|pixels| [(pixels, false), (pixels, true)])
        {
            let cell = pixels + pixels / 2;
            let width = cell * levels.len() as u32;
            let mut sheet = Pixels {
                width,
                height: cell,
                data: [background_bytes(background)].repeat((width * cell) as usize).concat(),
            };
            for (i, level) in levels.iter().enumerate() {
                let icon = gfx
                    .rasterize(pixels, pixels, tray::scale_for(pixels), 1.0, |c| {
                        tray::paint(c, &TrayIcon { level: *level, badge: None, large }, &theme)
                    })
                    .expect("icon");
                blend(&mut sheet, &icon, i as u32 * cell + pixels / 4, pixels / 4);
            }
            let size = if large { "-large" } else { "" };
            write_png(&out.join(format!("tray-{name}-{pixels}px{size}.png")), &sheet);
        }
    }
}

/// Candidate glyphs of the icon font with their code, to choose menu and device icons.
fn glyph_sheet(gfx: &Graphics, out: &Path) {
    let codes: &[u32] = &[
        0xE767, 0xE768, 0xE7F3, 0xE7F4, 0xE7F5, 0xE7F6, 0xE95B, 0xE9E9, 0xE9F5, 0xEA8F, 0xE8D6,
        0xE189, 0xE713, 0xE711, 0xE8BB, 0xE7E8, 0xF4C3, 0xE720, 0xE1D6, 0xE7FC, 0xE702, 0xEBDE,
        0xE7F7, 0xE7F8, 0xE8EA, 0xE909, 0xEC4F, 0xED1F, 0xF0B9, 0xF13C, 0xF4C0, 0xF4C1, 0xE9E8,
        0xE15D, 0xE994, 0xE74F, 0xE992, 0xF3B1, 0xF5B0, 0xEA37, 0xE95A, 0xEC80, 0xE80F, 0xE946,
    ];
    let theme = theme(ThemeMode::Dark);
    let (cols, cell_w, cell_h) = (11u32, 88.0f32, 76.0f32);
    let rows = (codes.len() as u32).div_ceil(cols);
    let (w, h) = (cols * cell_w as u32, rows * cell_h as u32);
    let sheet = gfx
        .rasterize(w, h, 1.0, 1.0, |c| {
            c.clear(Color::rgb(0x202020));
            for (i, &code) in codes.iter().enumerate() {
                let (x, y) = ((i as u32 % cols) as f32 * cell_w, (i as u32 / cols) as f32 * cell_h);
                let glyph = char::from_u32(code).unwrap_or(' ');
                c.glyph(glyph, 32.0, Rect::new(x, y + 4.0, cell_w, 44.0), theme.text_primary);
                c.text(
                    &format!("{code:X}"),
                    CAPTION,
                    Rect::new(x, y + 50.0, cell_w, 20.0),
                    TextAlign::Center,
                    theme.text_secondary,
                );
            }
        })
        .expect("sheet");
    write_png(&out.join("glyphs.png"), &sheet);
}

/// The context menu in both modes, over a sample backdrop standing in for the blur.
fn menus(gfx: &Graphics, out: &Path) {
    let entries = || {
        let item =
            |icon, label: &str, enabled| MenuEntry::Item { icon, label: label.into(), enabled };
        vec![
            item(Icon::SoundSettings, "Paramètres du son", true),
            item(Icon::Mixer, "Mélangeur de volume", true),
            item(Icon::SoundPanel, "Panneau Son classique", true),
            MenuEntry::Separator,
            item(Icon::Settings, "Paramètres de BetterVolume", false),
            MenuEntry::Separator,
            item(Icon::Quit, "Quitter", true),
        ]
    };
    for (mode, name, backdrop) in [
        (ThemeMode::Dark, "dark", Color::rgb(0x2C2C2C)),
        (ThemeMode::Light, "light", Color::rgb(0xFCFCFC)),
    ] {
        let theme = theme(mode);
        for scale in [1.0f32, 1.5, 2.25] {
            let mut menu = Menu::new(entries(), 1.0, |label| {
                gfx.measure_text(label, BODY, 1.0).map(|s| s.width).unwrap_or(0.0)
            });
            menu.key(bv_ui_kit::controls::menu::MenuKey::Down);
            menu.key(bv_ui_kit::controls::menu::MenuKey::Down);
            let size = menu.size();
            let (w, h) = size.to_pixels(scale);
            let image = gfx
                .rasterize(w, h, scale, 1.0, |c| {
                    // Stand-in for the system's blurred backdrop.
                    c.clear(backdrop);
                    if let Material::Acrylic { tint, .. } = theme.material {
                        c.fill_rect(Rect::from_size(size), tint);
                    }
                    let solid = Theme { material: Material::Solid(Color::TRANSPARENT), ..theme };
                    menu.paint_items(c, &solid);
                })
                .expect("menu");
            write_png(&out.join(format!("menu-{name}-{}.png", (scale * 100.0) as u32)), &image);
        }
    }
}

fn background_bytes(c: Color) -> [u8; 4] {
    let byte = |v: f32| (v * 255.0).round() as u8;
    [byte(c.b), byte(c.g), byte(c.r), 255]
}

/// Draws premultiplied `src` over opaque `dst` at (x, y).
fn blend(dst: &mut Pixels, src: &Pixels, x: u32, y: u32) {
    for sy in 0..src.height {
        for sx in 0..src.width {
            let s = src.pixel(sx, sy);
            let i = (((y + sy) * dst.width + x + sx) * 4) as usize;
            let a = u32::from(s[3]);
            for k in 0..3 {
                let d = u32::from(dst.data[i + k]);
                dst.data[i + k] = (u32::from(s[k]) + d * (255 - a) / 255).min(255) as u8;
            }
        }
    }
}

/// Writes an opaque-or-not BGRA image as a PNG (uncompressed deflate: simple and exact).
fn write_png(path: &Path, image: &Pixels) {
    let mut raw = Vec::with_capacity((image.width * 4 + 1) as usize * image.height as usize);
    for y in 0..image.height {
        raw.push(0); // no filter
        for x in 0..image.width {
            let [b, g, r, a] = image.pixel(x, y);
            // Back to straight alpha for PNG.
            let un = |c: u8| {
                if a == 0 {
                    0
                } else {
                    ((u32::from(c) * 255) / u32::from(a)).min(255) as u8
                }
            };
            raw.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(chunk) = chunks.next() {
        zlib.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&image.width.to_be_bytes());
    ihdr.extend_from_slice(&image.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib);
    chunk(&mut png, b"IEND", &[]);
    if let Err(error) = std::fs::write(path, png) {
        panic!("write {}: {error}", path.display());
    }
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in bytes {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Style "with device badge" (FR-011): candidate renderings next to the plain icon.
/// Rows: headphones, speakers, display, earbuds (medium volume), headphones muted.
/// Columns: plain, with the device glyph in place of the speaker.
fn badges(gfx: &Graphics, out: &Path) {
    let rows = [
        (DeviceType::Headphones, VolumeLevel::Medium),
        (DeviceType::Speakers, VolumeLevel::Medium),
        (DeviceType::Display, VolumeLevel::Medium),
        (DeviceType::Headset, VolumeLevel::High),
        (DeviceType::Headphones, VolumeLevel::Low),
        (DeviceType::Speakers, VolumeLevel::Zero),
        (DeviceType::Headphones, VolumeLevel::Muted),
    ];
    // Without and with the badge, at the Windows size then large (FR-083).
    let columns = [(false, false), (true, false), (false, true), (true, true)];
    for (mode, name, background) in [
        (ThemeMode::Dark, "dark", Color::rgb(0x1C1C1C)),
        (ThemeMode::Light, "light", Color::rgb(0xEEEEEE)),
    ] {
        let theme = theme(mode);
        for pixels in [16u32, 20, 24, 32, 160] {
            let cell = pixels * 2;
            let (width, height) = (cell * columns.len() as u32, cell * rows.len() as u32);
            let mut sheet = Pixels {
                width,
                height,
                data: [background_bytes(background)].repeat((width * height) as usize).concat(),
            };
            for (r, (device, level)) in rows.iter().enumerate() {
                for (c, &(badge, large)) in columns.iter().enumerate() {
                    let icon =
                        TrayIcon { level: Some(*level), badge: badge.then_some(*device), large };
                    let image = gfx
                        .rasterize(pixels, pixels, tray::scale_for(pixels), 1.0, |canvas| {
                            tray::paint(canvas, &icon, &theme)
                        })
                        .expect("badge");
                    blend(
                        &mut sheet,
                        &image,
                        c as u32 * cell + pixels / 2,
                        r as u32 * cell + pixels / 2,
                    );
                }
            }
            write_png(&out.join(format!("badge-{name}-{pixels}px.png")), &sheet);
        }
    }
}

/// Opaque stand-ins for the flyout material and for a tile on it, per mode.
fn icon_backgrounds(mode: ThemeMode) -> (Color, Color) {
    match mode {
        ThemeMode::Dark => (Color::rgb(0x2C2C2C), Color::rgb(0x373737)),
        ThemeMode::Light => (Color::rgb(0xF3F3F3), Color::rgb(0xFBFBFB)),
    }
}

/// The device icon catalogue (FR-068): every icon at the small-icon sizes of the common
/// scalings, on a tile (normal and current) at 100/150/225 %, and large; plus one sheet per
/// size with every icon, real size and zoomed (nearest neighbour) for inspection.
fn device_icons(gfx: &Graphics, out: &Path) {
    let dir = out.join("icons");
    std::fs::create_dir_all(&dir).expect("icons folder");
    for (mode, name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        let theme = theme(mode);
        let (flyout, tile) = icon_backgrounds(mode);
        for icon in DeviceIcon::ALL {
            let id = icon.id();
            for px in [16u32, 20, 24, 32, 160] {
                let image = gfx
                    .rasterize(px, px, px as f32 / 16.0, 1.0, |c| {
                        c.clear(flyout);
                        device_icon::paint(c, icon, Point::new(0.0, 0.0), 16.0, theme.text_primary);
                    })
                    .expect("icon");
                write_png(&dir.join(format!("{id}-{name}-{px}px.png")), &image);
            }
            // A square piece of tile, 48 DIPs, with the 24-DIP icon in its middle.
            for scale in [1.0f32, 1.5, 2.25] {
                for current in [false, true] {
                    let px = (48.0 * scale) as u32;
                    let image = gfx
                        .rasterize(px, px, scale, 1.0, |c| {
                            c.clear(flyout);
                            let r = Rect::new(0.5, 0.5, 47.0, 47.0);
                            let (fill, ink) = if current {
                                (theme.accent_fill, theme.text_on_accent)
                            } else {
                                (tile, theme.text_primary)
                            };
                            c.fill_rounded(r, 6.0, fill);
                            if !current {
                                c.stroke_rounded(r, 6.0, 1.0, theme.surface_stroke);
                            }
                            device_icon::paint(c, icon, Point::new(12.0, 12.0), 24.0, ink);
                        })
                        .expect("tile");
                    let state = if current { "current" } else { "tile" };
                    let pct = (scale * 100.0) as u32;
                    write_png(&dir.join(format!("{id}-{name}-{state}-{pct}.png")), &image);
                }
            }
        }
        // Large grid, 7 per row, for inspection.
        let (cols, cell) = (7u32, 140u32);
        let rows = (DeviceIcon::ALL.len() as u32).div_ceil(cols);
        let grid = gfx
            .rasterize(cols * cell, rows * cell, 1.0, 1.0, |c| {
                c.clear(flyout);
                for (i, icon) in DeviceIcon::ALL.into_iter().enumerate() {
                    let (x, y) = (
                        (i as u32 % cols * cell + 10) as f32,
                        (i as u32 / cols * cell + 10) as f32,
                    );
                    c.stroke_rounded(
                        Rect::new(x - 0.5, y - 0.5, 121.0, 121.0),
                        0.0,
                        1.0,
                        theme.surface_stroke,
                    );
                    device_icon::paint(c, icon, Point::new(x, y), 120.0, theme.text_primary);
                }
            })
            .expect("grid");
        write_png(&dir.join(format!("grid-{name}.png")), &grid);
        // Inspection sheets: all icons in two rows of 13, 4 px apart.
        for px in [16u32, 20, 24, 32, 36, 54] {
            let cell = px + 4;
            let (w, h) = (cell * 13, cell * 2);
            let sheet = gfx
                .rasterize(w, h, 1.0, 1.0, |c| {
                    c.clear(flyout);
                    for (i, icon) in DeviceIcon::ALL.into_iter().enumerate() {
                        let x = (i as u32 % 13 * cell + 2) as f32;
                        let y = (i as u32 / 13 * cell + 2) as f32;
                        device_icon::paint(
                            c,
                            icon,
                            Point::new(x, y),
                            px as f32,
                            theme.text_primary,
                        );
                    }
                })
                .expect("sheet");
            write_png(&dir.join(format!("strip-{name}-{px}px.png")), &sheet);
            write_png(&dir.join(format!("strip-{name}-{px}px-zoom.png")), &zoom(&sheet, 8));
        }
    }
}

/// Enlarges an image by `k` (nearest neighbour).
fn zoom(image: &Pixels, k: u32) -> Pixels {
    let (width, height) = (image.width * k, image.height * k);
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&image.pixel(x / k, y / k));
        }
    }
    Pixels { width, height, data }
}

/// Sample flyout content (in French).
fn flyout_content(tab: flyout::FlyoutTab, expanded: bool, tiles: usize) -> flyout::FlyoutContent {
    use flyout::*;
    let names = [
        ("Haut-parleurs", DeviceIcon::DesktopSpeakers),
        ("Casque sans fil de jeu avec un nom très long", DeviceIcon::GamingHeadset),
        ("Écran", DeviceIcon::Monitor),
        ("Barre de son", DeviceIcon::Soundbar),
        ("Écouteurs", DeviceIcon::WirelessEarbuds),
        ("TV salon", DeviceIcon::Tv),
        ("Ampli", DeviceIcon::AvReceiver),
    ];
    let mics = [
        ("Microphone USB", DeviceIcon::DeskMicrophone),
        ("Micro du casque", DeviceIcon::Headset),
        ("Webcam", DeviceIcon::Webcam),
    ];
    let list: &[(&str, DeviceIcon)] =
        if tab == FlyoutTab::Output { &names[..tiles] } else { &mics };
    let row = |target, label: &str, icon, volume, muted| VolumeControl {
        target,
        label: label.into(),
        icon,
        icon_source: None,
        volume,
        muted,
        enabled: true,
        mute_name: format!("Couper le son de {label}"),
        mute_tooltip: "Couper le son".into(),
        slider_name: format!("Volume de {label}"),
    };
    let output = tab == FlyoutTab::Output;
    FlyoutContent {
        name: "BetterVolume".into(),
        tabs: Some(Tabs {
            name: "Vue".into(),
            selected: tab,
            output: "Sortie".into(),
            input: "Micro".into(),
        }),
        devices: Devices {
            name: "Sorties".into(),
            tiles: list
                .iter()
                .enumerate()
                .map(|(i, (name, icon))| DeviceTile {
                    key: ItemKey(i as u64 + 1),
                    name: (*name).into(),
                    icon: *icon,
                    current: i == 0,
                    calls: (output && i == 1).then(|| "Utilisé pour les appels".into()),
                })
                .collect(),
            empty: "Aucun périphérique".into(),
        },
        apps: output.then(|| AppsSection {
            title: "Applications".into(),
            expanded,
            rows: vec![
                row(
                    SliderTarget::App(ItemKey(10)),
                    "Sons système",
                    VolumeIcon::SystemSounds,
                    45,
                    false,
                ),
                row(
                    SliderTarget::App(ItemKey(11)),
                    "Navigateur",
                    VolumeIcon::App(None),
                    100,
                    false,
                ),
                row(
                    SliderTarget::App(ItemKey(12)),
                    "Lecteur de musique",
                    VolumeIcon::App(None),
                    62,
                    true,
                ),
            ],
        }),
        main: if output {
            row(
                SliderTarget::Main,
                "Haut-parleurs",
                VolumeIcon::Level(VolumeLevel::Medium),
                54,
                false,
            )
        } else {
            row(SliderTarget::Main, "Microphone USB", VolumeIcon::Microphone, 80, false)
        },
        footer: Footer {
            sound_settings: "Paramètres du son Windows".into(),
            settings: "Paramètres de BetterVolume".into(),
            settings_enabled: false,
        },
        update: None,
        wheel_step: 2,
        height: flyout::PanelHeight::Full,
    }
}

/// The flyout in its main states, both modes, at 100 % and 150 %, as tall as a 1080p screen
/// allows.
fn flyouts(gfx: &Graphics, out: &Path) {
    std::fs::create_dir_all(out.join("flyout")).expect("flyout folder");
    use flyout::{FlyoutKey, FlyoutTab, FlyoutView, GraphicsMeasure};
    let cases = [
        ("sortie", FlyoutTab::Output, true, 4, false),
        ("replie", FlyoutTab::Output, false, 7, false),
        ("micro", FlyoutTab::Input, true, 3, false),
        ("clavier", FlyoutTab::Output, true, 2, true),
        ("mise-a-jour", FlyoutTab::Output, true, 4, false),
    ];
    for (mode, mode_name, backdrop) in [
        (ThemeMode::Dark, "dark", Color::rgb(0x1C1C1C)),
        (ThemeMode::Light, "light", Color::rgb(0xF2F2F2)),
    ] {
        let theme = Theme { material: Material::Solid(backdrop), ..theme(mode) };
        for (name, tab, expanded, tiles, keyboard) in cases {
            for (scale, text_scale) in [(1.0f32, 1.0f32), (1.5, 1.0), (1.0, 1.5)] {
                let measure = GraphicsMeasure { graphics: gfx, text_scale };
                let mut content = flyout_content(tab, expanded, tiles);
                if name == "mise-a-jour" {
                    // The update line above the footer.
                    content.update = Some(flyout::UpdateLine {
                        text: "Version 2.1.0 disponible".into(),
                        action: Some("Installer".into()),
                        close: Some("Fermer".into()),
                    });
                }
                let mut view = FlyoutView::new(
                    content, &measure, text_scale,
                    // A 1920 x 1080 screen at 100 %: work area 1032 high, 12 above and below.
                    1008.0,
                );
                if keyboard {
                    view.key(FlyoutKey::Tab { back: false });
                    view.key(FlyoutKey::Tab { back: false });
                }
                let size = view.size();
                let (w, h) = size.to_pixels(scale);
                let image = gfx
                    .rasterize(w, h, scale, text_scale, |c| view.paint(c, &theme))
                    .expect("flyout");
                let file = format!(
                    "flyout-{name}-{mode_name}-{}-t{}.png",
                    (scale * 100.0) as u32,
                    (text_scale * 100.0) as u32
                );
                write_png(&out.join("flyout").join(file), &image);
            }
        }
    }
}

/// The settings window: General page at rest, with the keyboard focus, with a list open,
/// and the About page, in both modes, at 100 % and 150 % and with larger text.
fn settings_windows(gfx: &Graphics, out: &Path) {
    use bv_ui_kit::controls::settings::{
        Block, Button, Card, CardControl, CloseQuestion, DeviceRow, DeviceTexts, NavEntry, Notice,
        PageKey, Question, SettingKey, SettingsContent, SettingsKey, SettingsView, Severity,
        TextField,
    };
    std::fs::create_dir_all(out.join("settings")).expect("settings folder");
    let toggle =
        |on| CardControl::Toggle {
            on, on_label: "Activé".into(), off_label: "Désactivé".into()
        };
    let choice = |options: &[&str], selected| CardControl::Choice {
        options: options.iter().map(|o| (*o).to_owned()).collect(),
        selected,
    };
    let card = |key, icon, title: &str, help: &str, control| {
        Block::Card(Card {
            key: SettingKey(key),
            icon: Some(icon),
            title: title.into(),
            description: Some(help.into()),
            control,
            enabled: true,
        })
    };
    let nav = vec![
        NavEntry { key: PageKey(0), icon: Icon::Settings, label: "Général".into() },
        NavEntry { key: PageKey(1), icon: Icon::Info, label: "À propos".into() },
    ];
    let general = SettingsContent {
        name: "Paramètres de BetterVolume".into(),
        nav_name: "Navigation".into(),
        nav: nav.clone(),
        page: PageKey(0),
        title: "Général".into(),
        devices: DeviceTexts::default(),
        close_question: None,
        question: None,
        reveal: None,
        blocks: vec![
            card(1, Icon::Power, "Lancer au démarrage de Windows", "BetterVolume démarre discrètement près de l’horloge", toggle(true)),
            card(2, Icon::Language, "Langue", "Automatique suit la langue d’affichage de Windows, avec l’anglais si elle n’est pas disponible", choice(&["Automatique (Français)", "English", "Français"], 0)),
            card(3, Icon::Microphone, "Gestion des micros", "Affiche la vue des micros dans le panneau", toggle(false)),
            card(4, Icon::Speaker, "Changer de périphérique pour", "Windows distingue le son des médias et celui des appels", choice(&["Médias et appels", "Médias seulement", "Appels seulement"], 0)),
            card(5, Icon::Mouse, "Pas de la molette", "Variation du volume à chaque cran de la molette de la souris", choice(&["1 %", "2 %", "3 %", "4 %", "5 %", "6 %", "7 %", "8 %", "9 %", "10 %"], 1)),
            card(6, Icon::Taskbar, "Icône près de l’horloge", "L’icône du volume, seule ou avec le type du périphérique actuel", choice(&["Identique à Windows", "Avec le type de périphérique"], 0)),
        ],
    };
    let about = SettingsContent {
        page: PageKey(1),
        title: "À propos".into(),
        blocks: vec![
            card(7, Icon::Info, "BetterVolume", "Version 0.1.0", CardControl::None),
            card(8, Icon::Document, "Licence", "Licence MIT : logiciel libre et open source", CardControl::None),
            card(9, Icon::Folder, "Journal", "Un journal technique qui aide à diagnostiquer les problèmes ; il ne contient aucune donnée personnelle", CardControl::Link { label: "Ouvrir le dossier".into() }),
        ],
        ..general.clone()
    };
    let device = |key: u64,
                  list: u32,
                  icon: DeviceIcon,
                  name: &str,
                  detail: &str,
                  hidden: bool,
                  connected: bool| {
        Block::Device(DeviceRow {
            key: SettingKey(key),
            list,
            icon,
            chosen_icon: None,
            automatic_icon: icon,
            name: name.into(),
            detail: detail.into(),
            hidden,
            connected,
        })
    };
    let devices = SettingsContent {
        nav: vec![
            nav[0].clone(),
            NavEntry { key: PageKey(2), icon: Icon::Devices, label: "Périphériques".into() },
            nav[1].clone(),
        ],
        page: PageKey(2),
        title: "Périphériques".into(),
        devices: DeviceTexts {
            change_icon: "Changer l’icône".into(),
            rename: "Renommer".into(),
            hide: "Masquer".into(),
            show: "Afficher".into(),
            forget: "Oublier".into(),
            move_help: String::new(),
            name_box: "Nom".into(),
            duplicate: "Ce nom est déjà utilisé par un autre périphérique".into(),
            automatic: "Automatique".into(),
            icon_names: DeviceIcon::ALL.iter().map(|i| i.id().replace('-', " ")).collect(),
            forget_question: "Oublier ce périphérique ?".into(),
            forget_detail: "Son nom, son icône et sa place seront perdus. S’il revient, il réapparaîtra en fin de liste.".into(),
            forget_confirm: "Oublier".into(),
            cancel: "Annuler".into(),
        },
        blocks: vec![
            Block::Text("Faites glisser une ligne pour changer l’ordre (ou Alt+Haut / Alt+Bas). Cliquez sur l’icône pour la changer, sur le crayon pour renommer, sur l’œil pour masquer.".into()),
            Block::Header("Sorties".into()),
            device(10, 0, DeviceIcon::DesktopSpeakers, "Enceintes bureau", "Haut-parleurs (Realtek USB Audio) · connecté", false, true),
            device(11, 0, DeviceIcon::Headset, "Casque", "Casque (Arctis Nova 7) · connecté", false, true),
            device(12, 0, DeviceIcon::Tv, "TV salon", "LG TV (NVIDIA High Definition Audio) · déconnecté", false, false),
            device(13, 0, DeviceIcon::Monitor, "Écran droit", "DELL U2723QE (NVIDIA High Definition Audio) · connecté", true, true),
            Block::Header("Micros".into()),
            device(20, 1, DeviceIcon::DeskMicrophone, "Micro USB", "Microphone (Yeti Stereo Microphone) · connecté", false, true),
        ],
        ..general.clone()
    };
    let field = |value: &str, placeholder: &str, password: bool, digits_only: bool| {
        CardControl::TextField(TextField {
            value: value.into(),
            placeholder: placeholder.into(),
            password,
            digits_only,
            max_chars: 255,
        })
    };
    let home = SettingsContent {
        nav: vec![
            nav[0].clone(),
            NavEntry { key: PageKey(2), icon: Icon::Devices, label: "Périphériques".into() },
            NavEntry { key: PageKey(3), icon: Icon::Home, label: "Home Assistant".into() },
            nav[1].clone(),
        ],
        page: PageKey(3),
        title: "Home Assistant".into(),
        close_question: Some(CloseQuestion {
            title: "Enregistrer les modifications ?".into(),
            text: "Les paramètres Home Assistant ont été modifiés mais pas enregistrés.".into(),
            save: "Enregistrer".into(),
            discard: "Ne pas enregistrer".into(),
            cancel: "Annuler".into(),
        }),
        blocks: vec![
            card(30, Icon::Home, "Connexion à Home Assistant (MQTT)", "Non connecté : nouvel essai automatique", toggle(true)),
            Block::Notice(Notice {
                key: SettingKey(40),
                severity: Severity::Caution,
                title: "Certificat inconnu".into(),
                message: "Windows ne connaît pas le certificat de ce serveur. Si c’est bien votre serveur, vous pouvez lui faire confiance.".into(),
                action: Some("Faire confiance à ce serveur".into()),
            }),
            Block::Header("Serveur MQTT".into()),
            card(31, Icon::Globe, "Adresse", "Nom ou adresse IP du serveur MQTT (souvent la machine Home Assistant)", field("192.0.2.10", "", false, false)),
            card(32, Icon::Ethernet, "Port", "En général 1883, ou 8883 pour une connexion chiffrée", field("8883", "", false, true)),
            card(33, Icon::Lock, "Connexion chiffrée", "À activer si votre serveur MQTT l’exige", toggle(true)),
            card(34, Icon::Contact, "Identifiant", "À laisser vide si le serveur n’en demande pas", field("maison", "", false, false)),
            card(35, Icon::Key, "Mot de passe", "Enregistré chiffré pour votre compte Windows, jamais affiché", field("", "Enregistré, tapez pour le remplacer", true, false)),
            Block::Header("Ce PC dans Home Assistant".into()),
            card(36, Icon::Pc, "Nom affiché", "Nom de l’appareil qui regroupe les contrôles de ce PC", field("", "PC-SALON", false, false)),
            card(37, Icon::Tag, "Préfixe de découverte", "À changer seulement si vous l’avez changé dans Home Assistant", field("homeassistant", "", false, false)),
            Block::Buttons(vec![
                Button { key: SettingKey(38), label: "Tester la connexion".into(), accent: true, enabled: true },
                Button { key: SettingKey(39), label: "Enregistrer".into(), accent: false, enabled: true },
            ]),
        ],
        ..general.clone()
    };
    let success = SettingsContent {
        close_question: None,
        blocks: home
            .blocks
            .iter()
            .map(|b| match b {
                Block::Notice(_) => Block::Notice(Notice {
                    key: SettingKey(40),
                    severity: Severity::Success,
                    title: "La connexion fonctionne".into(),
                    message: "Le serveur MQTT accepte la connexion.".into(),
                    action: None,
                }),
                other => other.clone(),
            })
            .collect(),
        ..home.clone()
    };
    // Hiding the Windows volume icon: the switch with the notice of a guard-rail
    // incident, scrolled into view, and the first-run question.
    let hiding = SettingsContent {
        blocks: {
            let mut blocks = general.blocks.clone();
            blocks.push(card(
                20,
                Icon::Hidden,
                "Masquer l’icône volume de Windows",
                "BetterVolume s’insère pour cela dans la barre des tâches. Une mise à jour de Windows peut l’empêcher de fonctionner (l’icône de Windows revient alors simplement), et certains antivirus peuvent s’en méfier.",
                toggle(false),
            ));
            blocks.push(Block::Notice(Notice {
                key: SettingKey(21),
                severity: Severity::Caution,
                title: "Icône volume de Windows réaffichée".into(),
                message: "La barre des tâches s’est arrêtée peu après que BetterVolume a masqué l’icône volume de Windows\u{a0}: par précaution, l’option a été désactivée. Vous pouvez la réactiver.".into(),
                action: None,
            }));
            blocks
        },
        reveal: Some(SettingKey(21)),
        ..general.clone()
    };
    let asking = SettingsContent {
        question: Some(Question {
            title: "Masquer l’icône volume de Windows\u{a0}?".into(),
            text: "BetterVolume ajoute sa propre icône de volume dans la barre des tâches, qui remplace celle de Windows. Pour éviter deux icônes de volume côte à côte, BetterVolume peut masquer celle de Windows. Vous pourrez changer d’avis à tout moment dans les paramètres (page Général).".into(),
            primary: "Masquer l’icône de Windows".into(),
            secondary: "Garder les deux".into(),
        }),
        ..general.clone()
    };
    for (mode, mode_name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        let mut theme = theme(mode);
        theme.window.mica = false;
        for (name, content, keys) in [
            ("masquer-incident", &hiding, 0),
            ("question-icone", &asking, 0),
            ("homeassistant", &home, 0),
            ("homeassistant-saisie", &home, 10),
            ("homeassistant-succes", &success, 0),
            ("homeassistant-fermer", &home, 11),
            ("general", &general, 0),
            ("clavier", &general, 2),
            ("liste", &general, 6),
            ("apropos", &about, 0),
            ("peripheriques", &devices, 0),
            ("renommer", &devices, 7),
            ("icones", &devices, 8),
            ("oublier", &devices, 9),
        ] {
            for (scale, text_scale) in [(1.0f32, 1.0f32), (1.5, 1.0), (1.0, 1.5)] {
                let measure = flyout::GraphicsMeasure { graphics: gfx, text_scale };
                let size = bv_ui_kit::Size::new(980.0, 700.0);
                let mut view = SettingsView::new(content.clone(), size, &measure, text_scale);
                let tab = |view: &mut SettingsView, n: usize| {
                    for _ in 0..n {
                        view.key(SettingsKey::Tab { back: false }, &measure);
                    }
                };
                match keys {
                    // General page: keyboard focus on the first control, an open list.
                    2 => tab(&mut view, 2),
                    6 => {
                        tab(&mut view, 5);
                        view.key(SettingsKey::Open, &measure);
                        view.key(SettingsKey::Down, &measure);
                    }
                    // Devices page: a refused rename, the icon picker, the forget question.
                    7 => {
                        tab(&mut view, 3);
                        view.key(SettingsKey::Enter, &measure);
                        view.text_input("Casque", &measure);
                        view.key(SettingsKey::Enter, &measure);
                    }
                    8 => {
                        tab(&mut view, 2);
                        view.key(SettingsKey::Enter, &measure);
                    }
                    9 => {
                        tab(&mut view, 11);
                        view.key(SettingsKey::Enter, &measure);
                    }
                    // Home Assistant page: a password being typed (keyboard focus), the
                    // close question.
                    10 => {
                        tab(&mut view, 8);
                        view.text_input("secret", &measure);
                    }
                    11 => {
                        view.request_close(&measure);
                    }
                    _ => {}
                }
                let (w, h) = size.to_pixels(scale);
                let image = gfx
                    .rasterize(w, h, scale, text_scale, |c| view.paint(c, &theme, &measure))
                    .expect("settings");
                let file = format!(
                    "settings-{name}-{mode_name}-{}-t{}.png",
                    (scale * 100.0) as u32,
                    (text_scale * 100.0) as u32
                );
                write_png(&out.join("settings").join(file), &image);
            }
        }
    }
}

/// The bubble above the taskbar (FR-017, FR-105): volume and device-switch contents, both
/// modes, 100/150/225 % and text 150 %. The rounded corners come from the window frame in
/// the application; here they are cut over a desktop-like background.
fn bubbles(gfx: &Graphics, out: &Path) {
    let folder = out.join("bulle");
    std::fs::create_dir_all(&folder).expect("bubble folder");
    let contents = [
        ("volume-34", BubbleContent::Volume { level: VolumeLevel::Medium, percent: 34 }),
        ("volume-100", BubbleContent::Volume { level: VolumeLevel::High, percent: 100 }),
        ("volume-muet", BubbleContent::Volume { level: VolumeLevel::Muted, percent: 36 }),
        (
            "sortie",
            BubbleContent::Device {
                icon: DeviceIcon::WiredHeadphones,
                name: "Casque".into(),
                kind: "Sortie audio".into(),
            },
        ),
        (
            "micro-appels",
            BubbleContent::Device {
                icon: DeviceIcon::DeskMicrophone,
                name: "Micro de bureau".into(),
                kind: "Microphone · appels".into(),
            },
        ),
        (
            "nom-long",
            BubbleContent::Device {
                icon: DeviceIcon::DesktopSpeakers,
                name: "Haut-parleurs Realtek High Definition Audio du bureau".into(),
                kind: "Sortie audio".into(),
            },
        ),
    ];
    for (mode, name, backdrop) in [
        (ThemeMode::Dark, "dark", Color::rgb(0x181818)),
        (ThemeMode::Light, "light", Color::rgb(0xE8E8E8)),
    ] {
        let theme = bubble::theme(theme(mode));
        for (scale, text_scale, tag) in
            [(1.0f32, 1.0f32, "100"), (1.5, 1.0, "150"), (2.25, 1.0, "225"), (1.0, 1.5, "texte150")]
        {
            for (label, content) in &contents {
                let size = bubble::size(content, gfx, text_scale);
                let (w, h) = size.to_pixels(scale);
                let image = gfx
                    .rasterize(w, h, scale, text_scale, |c| {
                        c.clear(backdrop);
                        c.clipped(Rect::from_size(size), |c| {
                            bubble::paint(c, &theme, content, size);
                        });
                    })
                    .expect("bubble");
                write_png(&folder.join(format!("bulle-{label}-{name}-{tag}.png")), &image);
            }
        }
    }
}
