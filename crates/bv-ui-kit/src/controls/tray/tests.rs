use super::*;
use crate::backend::{DeviceKind, Graphics, Pixels};
use crate::theme::{AccentPalette, HighContrastColors, ThemeMode};
use crate::ThemeInputs;

/// Small-icon sizes of every Windows scaling step (100 % to 300 %) and a larger one.
const SIZES: [u32; 9] = [16, 20, 24, 28, 32, 36, 40, 48, 64];
const DEVICES: [DeviceType; 4] =
    [DeviceType::Speakers, DeviceType::Headphones, DeviceType::Headset, DeviceType::Display];
const LOOKS: [TrayLook; 2] = [TrayLook::Windows, TrayLook::Colourful];
const LEVELS: [VolumeLevel; 5] = [
    VolumeLevel::Zero,
    VolumeLevel::Low,
    VolumeLevel::Medium,
    VolumeLevel::High,
    VolumeLevel::Muted,
];

fn render(gfx: &Graphics, px: u32, paint: impl FnOnce(&mut Canvas<'_>)) -> Pixels {
    gfx.rasterize(px, px, scale_for(px), 1.0, paint).expect("rasterize")
}

fn icon(
    level: Option<VolumeLevel>,
    badge: Option<DeviceType>,
    large: bool,
    look: TrayLook,
) -> TrayIcon {
    TrayIcon { level, badge, large, look }
}

/// Runs `draw` stretched like the large icon when `large`.
fn sized(c: &mut Canvas<'_>, large: bool, draw: impl FnOnce(&mut Canvas<'_>)) {
    if large {
        c.stretched_vertically(LARGE_STRETCH, MIDDLE, draw);
    } else {
        draw(c);
    }
}

/// Pixels whose centre lies right of the cut at `cut` DIPs.
fn first_column_right_of(cut: f32, px: u32) -> u32 {
    (cut * scale_for(px) - 0.5).ceil() as u32
}

/// The device drawing alone, as the icon draws it in `look`.
fn device_only(c: &mut Canvas<'_>, device: DeviceType, look: TrayLook, colors: &TrayColors) {
    let ink = Ink { colors, look, dim: false };
    let w = match look {
        TrayLook::Windows => SPEAKER_RIGHT,
        TrayLook::Colourful => Waves::DEVICE.room(),
    };
    ink.paint(c, TOP, BOTTOM, |c, color| device_drawing(c, device, w, color));
}

/// Where the picture ends and the level part starts, by look.
fn cut(look: TrayLook, badge: Option<DeviceType>) -> f32 {
    match (look, badge) {
        (TrayLook::Windows, _) => LEVEL_START,
        (TrayLook::Colourful, None) => Waves::SPEAKER.left() - waves::PICTURE_GAP / 2.0,
        (TrayLook::Colourful, Some(_)) => Waves::DEVICE.left() - waves::PICTURE_GAP / 2.0,
    }
}

/// Columns holding something (alpha above `min`) in `columns`, first and last.
fn inked_columns(img: &Pixels, columns: std::ops::Range<u32>, min: u8) -> Option<(u32, u32)> {
    let inked: Vec<u32> =
        columns.filter(|&x| (0..img.height).any(|y| img.pixel(x, y)[3] > min)).collect();
    Some((*inked.first()?, *inked.last()?))
}

/// Rows holding something (alpha above `min`) in `columns`, first and last.
fn inked_rows(img: &Pixels, columns: std::ops::Range<u32>, min: u8) -> Option<(u32, u32)> {
    let inked: Vec<u32> =
        (0..img.height).filter(|&y| columns.clone().any(|x| img.pixel(x, y)[3] > min)).collect();
    Some((*inked.first()?, *inked.last()?))
}

#[test]
fn nothing_of_the_speaker_remains_next_to_the_device() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    for (px, large) in SIZES.into_iter().flat_map(|px| [(px, false), (px, true)]) {
        for look in LOOKS {
            for device in DEVICES {
                // Volume 0: the Windows glyph is the speaker alone, so left of the cut only
                // the device drawing may remain.
                let with_glyph = render(&gfx, px, |c| {
                    paint(c, &icon(Some(VolumeLevel::Zero), Some(device), large, look), &theme)
                });
                let alone = render(&gfx, px, |c| {
                    c.clear(Color::TRANSPARENT);
                    sized(c, large, |c| device_only(c, device, look, &theme.tray));
                });
                for y in 0..px {
                    for x in 0..first_column_right_of(cut(look, Some(device)), px) {
                        assert_eq!(
                            with_glyph.pixel(x, y),
                            alone.pixel(x, y),
                            "{look:?} {device:?} at {px} px (large {large}), pixel ({x}, \
                             {y}): speaker remnant"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_picture_keeps_a_gap_before_the_waves() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    let pictures =
        [None, Some(DeviceType::Speakers), Some(DeviceType::Headphones), Some(DeviceType::Display)];
    for (px, large) in SIZES.into_iter().flat_map(|px| [(px, false), (px, true)]) {
        for look in LOOKS {
            for badge in pictures {
                for level in LEVELS {
                    let img = render(&gfx, px, |c| {
                        paint(c, &icon(Some(level), badge, large, look), &theme)
                    });
                    let split = first_column_right_of(cut(look, badge), px);
                    let picture = inked_columns(&img, 0..split, 8).expect("picture");
                    let waves = inked_columns(&img, split..px, 8).expect("waves");
                    // No column holds both (from 150 %, a column of nothing between them).
                    let gap = if px >= 24 { 1 } else { 0 };
                    assert!(
                        waves.0 > picture.1 + gap,
                        "{look:?} {badge:?} {level:?} at {px} px (large {large}): picture to \
                         {}, waves from {}",
                        picture.1,
                        waves.0
                    );
                }
            }
        }
    }
}

#[test]
fn our_waves_stay_distinct_at_the_real_size() {
    // Across the middle row, the three reached arcs are three separate strokes (they
    // merged into one when 1.4 px apart).
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let theme = Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() });
        for px in [24, 28, 32, 36, 40, 48, 64] {
            for (badge, waves) in
                [(None, Waves::SPEAKER), (Some(DeviceType::Headphones), Waves::DEVICE)]
            {
                for large in [false, true] {
                    let i = icon(Some(VolumeLevel::High), badge, large, TrayLook::Colourful);
                    let img = render(&gfx, px, |c| paint(c, &i, &theme));
                    let y = px / 2;
                    let from = first_column_right_of(waves.left() - 0.3, px);
                    let mut strokes = 0;
                    let mut inside = false;
                    for x in from..px {
                        let a = img.pixel(x, y)[3];
                        if !inside && a > 160 {
                            strokes += 1;
                            inside = true;
                        } else if inside && a < 100 {
                            inside = false;
                        }
                    }
                    assert_eq!(strokes, 3, "{mode:?} {badge:?} at {px} px (large {large})");
                }
            }
        }
    }
}

#[test]
fn devices_keep_their_natural_proportions() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    let px = 64;
    for look in LOOKS {
        for (device, ratio) in [
            (DeviceType::Speakers, proportions::SPEAKERS),
            (DeviceType::Headphones, proportions::HEADPHONES),
            (DeviceType::Headset, proportions::HEADPHONES),
            (DeviceType::Display, proportions::SCREEN + proportions::STAND),
        ] {
            let img = render(&gfx, px, |c| {
                c.clear(Color::TRANSPARENT);
                device_only(c, device, look, &theme.tray);
            });
            let (x0, x1) = inked_columns(&img, 0..px, 127).expect("drawn");
            let (y0, y1) = inked_rows(&img, 0..px, 127).expect("drawn");
            let measured = (y1 - y0 + 1) as f32 / (x1 - x0 + 1) as f32;
            assert!(
                (measured / ratio - 1.0).abs() < 0.08,
                "{look:?} {device:?}: height / width {measured}, expected {ratio}"
            );
        }
    }
}

#[test]
fn the_bettervolume_device_takes_more_than_half_the_width() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    assert!(Waves::DEVICE.room() > LOGICAL_SIZE / 2.0, "{}", Waves::DEVICE.room());
    for px in [32, 36, 48] {
        for device in [DeviceType::Headphones, DeviceType::Display] {
            let ours = render(&gfx, px, |c| {
                c.clear(Color::TRANSPARENT);
                device_only(c, device, TrayLook::Colourful, &theme.tray);
            });
            let (x0, x1) = inked_columns(&ours, 0..px, 127).expect("drawn");
            assert!((x1 - x0 + 1) * 2 > px, "{device:?} at {px} px: {} columns", x1 - x0 + 1);
        }
    }
}

#[test]
fn waves_not_reached_are_dimmed_like_windows_11() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let high_contrast = HighContrastColors {
        window: Color::BLACK,
        window_text: Color::WHITE,
        highlight: Color::rgb(0x1AEBFF),
        highlight_text: Color::BLACK,
        gray_text: Color::rgb(0x3FF23F),
        button_face: Color::BLACK,
        button_text: Color::WHITE,
        hotlight: Color::rgb(0xFFFF00),
    };
    let themes = [
        ("dark", ThemeInputs::default()),
        ("light", ThemeInputs { system_mode: ThemeMode::Light, ..ThemeInputs::default() }),
        (
            "high contrast",
            ThemeInputs { high_contrast: Some(high_contrast), ..ThemeInputs::default() },
        ),
    ];
    for (name, inputs) in themes {
        let theme = Theme::shell(&inputs);
        let colors = theme.tray;
        // Large enough for the 1-DIP wave stroke to cover whole pixels.
        for (px, large) in [(32, false), (64, false), (32, true), (64, true)] {
            let name = format!("{name} (large {large})");
            let draw = |level| {
                render(&gfx, px, |c| {
                    paint(c, &icon(Some(level), None, large, TrayLook::Windows), &theme)
                })
            };
            let alpha = |img: &Pixels| {
                (0..px * px).map(|i| u32::from(img.pixel(i % px, i / px)[3])).sum::<u32>()
            };
            let (zero, low, medium, high) = (
                draw(VolumeLevel::Zero),
                draw(VolumeLevel::Low),
                draw(VolumeLevel::Medium),
                draw(VolumeLevel::High),
            );
            // Every level shows the three waves: more of them solid as the level rises
            // (in high contrast GrayText is opaque: the colour check below covers it).
            assert!(
                colors.high_contrast
                    || (alpha(&zero) < alpha(&low)
                        && alpha(&low) < alpha(&medium)
                        && alpha(&medium) < alpha(&high)),
                "{name} at {px} px"
            );
            // Full volume and mute (also "no device") are the Windows glyphs alone.
            let glyph_only = |level: VolumeLevel, color: Color| {
                render(&gfx, px, |c| {
                    c.clear(Color::TRANSPARENT);
                    sized(c, large, |c| glyph(c, level.icon().glyph(), color));
                })
            };
            // Where only the outer wave of the Windows glyphs is (fully covered), a low
            // level shows it in the "disabled text" colour exactly.
            let raw_high = glyph_only(VolumeLevel::High, colors.glyph);
            let raw_medium = glyph_only(VolumeLevel::Medium, colors.glyph);
            let full_alpha = colors.glyph.a * 255.0;
            let outer: Vec<(u32, u32, f32)> = (0..px)
                .flat_map(|y| (first_column_right_of(LEVEL_START, px)..px).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    f32::from(raw_high.pixel(x, y)[3]) >= 0.98 * full_alpha
                        && raw_medium.pixel(x, y)[3] == 0
                })
                .map(|(x, y)| (x, y, f32::from(raw_high.pixel(x, y)[3]) / full_alpha))
                .collect();
            assert!(!outer.is_empty(), "{name} at {px} px: no outer wave pixel");
            for &(x, y, cover) in &outer {
                assert_colour(&low, x, y, colors.unreached, cover, &name);
                assert_eq!(
                    medium.pixel(x, y),
                    low.pixel(x, y),
                    "{name}: the outer wave is dim at medium too"
                );
            }
            assert_eq!(high, glyph_only(VolumeLevel::High, colors.glyph), "{name}");
            assert_eq!(
                draw(VolumeLevel::Muted),
                glyph_only(VolumeLevel::Muted, colors.glyph),
                "{name}"
            );
            let none =
                render(&gfx, px, |c| paint(c, &icon(None, None, large, TrayLook::Windows), &theme));
            let dimmed = colors.glyph.with_alpha(colors.glyph.a * 0.6);
            assert_eq!(none, glyph_only(VolumeLevel::Muted, dimmed), "{name}: no device");
        }
    }
}

#[test]
fn the_windows_look_with_a_device_keeps_the_windows_waves() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    for px in SIZES {
        for level in LEVELS {
            let plain = render(&gfx, px, |c| {
                paint(c, &icon(Some(level), None, false, TrayLook::Windows), &theme)
            });
            for device in DEVICES {
                let badged = render(&gfx, px, |c| {
                    paint(c, &icon(Some(level), Some(device), false, TrayLook::Windows), &theme)
                });
                for y in 0..px {
                    for x in first_column_right_of(LEVEL_START, px)..px {
                        assert_eq!(
                            badged.pixel(x, y),
                            plain.pixel(x, y),
                            "{device:?} {level:?} at {px} px, pixel ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

fn assert_colour(img: &Pixels, x: u32, y: u32, c: Color, cover: f32, what: &str) {
    let expected =
        [c.b * c.a, c.g * c.a, c.r * c.a, c.a].map(|v| (v * cover * 255.0).round() as i32);
    let got = img.pixel(x, y).map(i32::from);
    let close = got.iter().zip(expected).all(|(g, e)| (g - e).abs() <= 3);
    assert!(close, "{what}, pixel ({x}, {y}): {got:?}, expected {expected:?}");
}

/// Average colour (straight, 0 to 255) of the pixels at least half covered.
fn average(img: &Pixels, px: u32, columns: std::ops::Range<u32>) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0.0;
    for y in 0..px {
        for x in columns.clone() {
            let [b, g, r, a] = img.pixel(x, y);
            if a >= 128 {
                let a = f32::from(a);
                sum[0] += f32::from(r) * 255.0 / a;
                sum[1] += f32::from(g) * 255.0 / a;
                sum[2] += f32::from(b) * 255.0 / a;
                count += 1.0;
            }
        }
    }
    assert!(count > 0.0, "nothing drawn");
    sum.map(|s| s / count)
}

#[test]
fn the_bettervolume_look_takes_the_accent_colour() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let red = AccentPalette {
        light3: Color::rgb(0xFFB3B3),
        light2: Color::rgb(0xFF8080),
        light1: Color::rgb(0xFF4040),
        base: Color::rgb(0xE00000),
        dark1: Color::rgb(0xB00000),
        dark2: Color::rgb(0x800000),
        dark3: Color::rgb(0x500000),
    };
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let blue = Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() });
        let reddish = Theme::shell(&ThemeInputs {
            system_mode: mode,
            accent: Some(red),
            ..ThemeInputs::default()
        });
        for badge in [None, Some(DeviceType::Headphones), Some(DeviceType::Speakers)] {
            let draw = |theme: &Theme| {
                render(&gfx, 32, |c| {
                    paint(
                        c,
                        &icon(Some(VolumeLevel::High), badge, false, TrayLook::Colourful),
                        theme,
                    )
                })
            };
            // The speaker or device, and the waves: both coloured.
            for columns in [0..14, 20..32] {
                let [r, _, b] = average(&draw(&blue), 32, columns.clone());
                assert!(b > r + 100.0, "{mode:?} {badge:?} {columns:?}: blue {r} {b}");
                let [r, _, b] = average(&draw(&reddish), 32, columns.clone());
                assert!(r > b + 80.0, "{mode:?} {badge:?} {columns:?}: red {r} {b}");
            }
            // A gradient: the top lighter than the bottom.
            let img = draw(&blue);
            let light = |y: std::ops::Range<u32>| {
                let mut sum = 0.0;
                let mut n = 0.0;
                for y in y {
                    for x in 0..14 {
                        let p = img.pixel(x, y);
                        if p[3] == 255 {
                            sum += f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2]);
                            n += 1.0;
                        }
                    }
                }
                sum / n
            };
            assert!(light(10..13) > light(19..22) + 30.0, "{mode:?} {badge:?}: no gradient");
        }
    }
}

#[test]
fn the_bettervolume_waves_not_reached_are_a_pale_accent() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let theme = Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() });
        let px = 64;
        let low = render(&gfx, px, |c| {
            paint(c, &icon(Some(VolumeLevel::Low), None, false, TrayLook::Colourful), &theme)
        });
        // The middle of the outer arc (not reached at a low level), fully covered.
        let w = Waves::SPEAKER;
        let x = ((w.cx() + w.r(2)) * scale_for(px)) as u32;
        assert_colour(&low, x, px / 2, theme.tray.accent_unreached, 1.0, &format!("{mode:?}"));
    }
}

#[test]
fn the_bettervolume_speaker_is_at_least_as_large_as_the_windows_one() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    for (px, large) in SIZES.into_iter().flat_map(|px| [(px, false), (px, true)]) {
        let draw = |look| {
            render(&gfx, px, |c| {
                paint(c, &icon(Some(VolumeLevel::Zero), None, large, look), &theme)
            })
        };
        let (windows, ours) = (draw(TrayLook::Windows), draw(TrayLook::Colourful));
        let w_split = first_column_right_of(LEVEL_START, px);
        let o_split = first_column_right_of(cut(TrayLook::Colourful, None), px);
        let wc = inked_columns(&windows, 0..w_split, 127).expect("Windows speaker");
        let oc = inked_columns(&ours, 0..o_split, 127).expect("our speaker");
        let wr = inked_rows(&windows, 0..w_split, 127).expect("Windows speaker");
        let or = inked_rows(&ours, 0..o_split, 127).expect("our speaker");
        assert!(oc.1 - oc.0 >= wc.1 - wc.0, "at {px} px (large {large}): {oc:?}, Windows {wc:?}");
        assert!(
            or.1 - or.0 + 1 >= wr.1 - wr.0,
            "at {px} px (large {large}): {or:?}, Windows {wr:?}"
        );
    }
}

#[test]
fn high_contrast_draws_the_bettervolume_look_as_windows() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let hc = HighContrastColors {
        window: Color::BLACK,
        window_text: Color::WHITE,
        highlight: Color::rgb(0x1AEBFF),
        highlight_text: Color::BLACK,
        gray_text: Color::rgb(0x3FF23F),
        button_face: Color::BLACK,
        button_text: Color::WHITE,
        hotlight: Color::rgb(0xFFFF00),
    };
    let theme = Theme::shell(&ThemeInputs { high_contrast: Some(hc), ..ThemeInputs::default() });
    for badge in [None, Some(DeviceType::Headset)] {
        for level in LEVELS {
            let draw = |look| {
                render(&gfx, 32, |c| paint(c, &icon(Some(level), badge, true, look), &theme))
            };
            assert_eq!(draw(TrayLook::Colourful), draw(TrayLook::Windows), "{badge:?} {level:?}");
        }
    }
}

#[test]
fn a_picture_of_the_icon_is_the_icon() {
    // The settings window shows the icon with `paint_in`: the same drawing, placed.
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    for look in LOOKS {
        let i = icon(Some(VolumeLevel::Medium), Some(DeviceType::Speakers), true, look);
        let direct = render(&gfx, 32, |c| paint(c, &i, &theme));
        let placed = gfx
            .rasterize(48, 48, 1.0, 1.0, |c| {
                c.clear(Color::TRANSPARENT);
                paint_in(c, Rect::new(8.0, 8.0, 32.0, 32.0), &i, &theme.tray);
            })
            .expect("rasterize");
        // The same drawing at the same place: every solid pixel of one is inked in the
        // other, nothing is drawn outside the square, and as much ink overall. Glyph edges
        // may differ (text is anti-aliased for the transform of the target).
        let (mut ink_placed, mut ink_direct) = (0u32, 0u32);
        for y in 0..48 {
            for x in 0..48 {
                let p = placed.pixel(x, y)[3];
                let inside = (8..40).contains(&x) && (8..40).contains(&y);
                if !inside {
                    assert_eq!(p, 0, "{look:?}: drawn outside at ({x}, {y})");
                    continue;
                }
                let d = direct.pixel(x - 8, y - 8)[3];
                assert!(
                    !(p >= 230 && d < 40) && !(d >= 230 && p < 40),
                    "{look:?} ({x}, {y}): {p}, {d}"
                );
                ink_placed += u32::from(p);
                ink_direct += u32::from(d);
            }
        }
        assert!(
            ink_placed.abs_diff(ink_direct) * 20 < ink_direct,
            "{look:?}: {ink_placed}, {ink_direct}"
        );
    }
}

#[test]
fn the_large_icon_is_taller_whole_and_keeps_the_windows_widths() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let levels = [
        Some(VolumeLevel::Muted),
        Some(VolumeLevel::Zero),
        Some(VolumeLevel::Low),
        Some(VolumeLevel::Medium),
        Some(VolumeLevel::High),
        None,
    ];
    let badges = [None, Some(DeviceType::Headphones), Some(DeviceType::Speakers)];
    let mut problems = Vec::new();
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let theme = Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() });
        for look in LOOKS {
            for px in SIZES {
                for level in levels {
                    for badge in badges {
                        let normal = icon(level, badge, false, look);
                        check_large(&gfx, &theme, normal, px, &mut problems);
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn check_large(
    gfx: &Graphics,
    theme: &Theme,
    normal: TrayIcon,
    px: u32,
    problems: &mut Vec<String>,
) {
    let draw = |large| render(gfx, px, |c| paint(c, &TrayIcon { large, ..normal }, theme));
    let (windows, large) = (draw(false), draw(true));
    let what = format!(
        "{:?} {:?} {:?} {:?} at {px} px",
        theme.mode, normal.look, normal.level, normal.badge
    );
    // Half covered at least, whatever the colour's own opacity (the gradient is opaque).
    let colour = match (normal.level, normal.look) {
        (None, _) => theme.tray.glyph.a * 0.6,
        (Some(_), TrayLook::Windows) => theme.tray.glyph.a,
        (Some(_), TrayLook::Colourful) => 1.0,
    };
    let inked = |img: &Pixels, x: u32, y: u32| f32::from(img.pixel(x, y)[3]) > 0.5 * colour * 255.0;
    let span = |img: &Pixels, columns: bool| {
        let lines: Vec<u32> = (0..px)
            .filter(|&a| (0..px).any(|b| if columns { inked(img, a, b) } else { inked(img, b, a) }))
            .collect();
        (lines.first().copied(), lines.last().copied())
    };
    // Same width as at the Windows size: the same first and last columns, give or take
    // the half-covered edge pixel.
    let (w, l) = (span(&windows, true), span(&large, true));
    let near = |a: Option<u32>, b: Option<u32>| match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= 1,
        _ => a == b,
    };
    if !near(w.0, l.0) || !near(w.1, l.1) {
        problems.push(format!("{what}: columns {l:?}, Windows {w:?}"));
    }
    // Taller by the stretch (within a pixel of rounding), inside the icon.
    let height = |s: (Option<u32>, Option<u32>)| match s {
        (Some(first), Some(last)) => last - first + 1,
        _ => 0,
    };
    let (w, l) = (height(span(&windows, false)), height(span(&large, false)));
    let expected = f64::from(w) * f64::from(LARGE_STRETCH);
    if f64::from(l) < expected - 2.5 || l > px {
        problems.push(format!("{what}: {l} rows, Windows {w}"));
    }
    // Not cut: drawn with room above and below, nothing goes past the icon's square, and
    // inside it the picture is the same.
    let room = px / 4;
    let roomy = gfx
        .rasterize(px, px + 2 * room, scale_for(px), 1.0, |c| {
            c.shifted_down(room as f32 / scale_for(px), |c| {
                paint(c, &TrayIcon { large: true, ..normal }, theme)
            })
        })
        .expect("rasterize");
    for y in 0..px + 2 * room {
        for x in 0..px {
            let got = roomy.pixel(x, y);
            let inside = (room..room + px).contains(&y);
            if inside && got != large.pixel(x, y - room) {
                problems.push(format!("{what}: ({x}, {y}) differs with room"));
            } else if !inside && got[3] != 0 {
                problems.push(format!("{what}: cut at ({x}, {y}): {got:?}"));
            }
        }
    }
    let full = colour * 255.0;
    // Drawn, not an enlarged picture: about as many fully covered pixels as at the
    // Windows size at least (stretching a picture blurs them).
    let solid = |img: &Pixels| {
        (0..px * px).filter(|i| f32::from(img.pixel(i % px, i / px)[3]) >= 0.98 * full).count()
    };
    if solid(&large) * 10 < solid(&windows) * 8 {
        problems.push(format!(
            "{what}: {} solid pixels, Windows {}",
            solid(&large),
            solid(&windows)
        ));
    }
}

#[test]
fn overlapping_parts_of_a_device_are_not_darker() {
    // The light taskbar's glyph colour is translucent (#E4000000): the parts of a device
    // that overlap (headband and cups, screen and stand) must not add up.
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme =
        Theme::shell(&ThemeInputs { system_mode: ThemeMode::Light, ..ThemeInputs::default() });
    let most = (theme.tray.glyph.a * 255.0).round() as u8 + 1;
    for px in [32, 64] {
        for device in DEVICES {
            let i = icon(Some(VolumeLevel::High), Some(device), false, TrayLook::Windows);
            let img = render(&gfx, px, |c| paint(c, &i, &theme));
            for y in 0..px {
                for x in 0..px {
                    let a = img.pixel(x, y)[3];
                    assert!(a <= most, "{device:?} at {px} px, ({x}, {y}): alpha {a}");
                }
            }
        }
    }
}
