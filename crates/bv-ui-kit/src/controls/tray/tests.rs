use super::*;
use crate::backend::{DeviceKind, Graphics, Pixels};
use crate::theme::{AccentPalette, HighContrastColors, ThemeMode};
use crate::ThemeInputs;

/// Small-icon sizes of every Windows scaling step (100 % to 300 %) and a larger one.
const SIZES: [u32; 9] = [16, 20, 24, 28, 32, 36, 40, 48, 64];
const DEVICES: [DeviceType; 4] =
    [DeviceType::Speakers, DeviceType::Headphones, DeviceType::Headset, DeviceType::Display];
const LOOKS: [TrayLook; 2] = [TrayLook::Windows, TrayLook::BetterVolume];
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

fn first_level_column(px: u32) -> u32 {
    first_column_right_of(LEVEL_START, px)
}

/// Where the cut of the level part lands once the waves are made smaller for a device.
fn device_cut() -> f32 {
    LOGICAL_SIZE - (LOGICAL_SIZE - LEVEL_START) * DEVICE_WAVES_SCALE
}

/// The device drawing alone, as the icon draws it.
fn device_only(c: &mut Canvas<'_>, device: DeviceType, look: TrayLook, colors: &TrayColors) {
    let ink = Ink { colors, look, dim: false };
    let (from, to) = (Point::new(DEVICE_WIDTH * 0.2, 2.5), Point::new(DEVICE_WIDTH * 0.8, 13.5));
    match look {
        TrayLook::Windows => outline_device(c, device, ink.plain()),
        TrayLook::BetterVolume => {
            ink.paint(c, from, to, |c, color| filled_device(c, device, color))
        }
    }
}

/// Columns holding something (alpha above `min`), first and last.
fn inked_columns(
    img: &Pixels,
    px: u32,
    columns: std::ops::Range<u32>,
    min: u8,
) -> Option<(u32, u32)> {
    let inked: Vec<u32> = columns.filter(|&x| (0..px).any(|y| img.pixel(x, y)[3] > min)).collect();
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
                    for x in 0..first_column_right_of(device_cut(), px) {
                        assert_eq!(
                            with_glyph.pixel(x, y),
                            alone.pixel(x, y),
                            "{look:?} {device:?} at {px} px (large {large}), pixel ({x}, {y}): \
                             speaker remnant"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn the_device_gets_more_room_and_keeps_a_gap_before_the_smaller_waves() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    const { assert!(DEVICE_WIDTH > 8.9 && DEVICE_WIDTH < 9.0) };
    for (px, large) in SIZES.into_iter().flat_map(|px| [(px, false), (px, true)]) {
        for look in LOOKS {
            let speaker = render(&gfx, px, |c| {
                paint(c, &icon(Some(VolumeLevel::Zero), None, large, look), &theme)
            });
            let speaker_last = inked_columns(&speaker, px, 0..first_level_column(px), 8)
                .expect("nothing drawn: the Windows icon font (Segoe Fluent Icons) is missing")
                .1;
            for device in DEVICES {
                let alone = render(&gfx, px, |c| {
                    c.clear(Color::TRANSPARENT);
                    sized(c, large, |c| device_only(c, device, look, &theme.tray));
                });
                let (_, device_last) = inked_columns(&alone, px, 0..px, 8).expect("device drawn");
                assert!(
                    device_last > speaker_last || px < 24,
                    "{look:?} {device:?} at {px} px: not larger than the speaker \
                     ({device_last}, speaker {speaker_last})"
                );
                for level in LEVELS {
                    let full = render(&gfx, px, |c| {
                        paint(c, &icon(Some(level), Some(device), large, look), &theme)
                    });
                    let waves =
                        inked_columns(&full, px, first_column_right_of(device_cut(), px)..px, 8)
                            .expect("waves drawn");
                    // The Windows gap (1.1 DIP) between the device and the waves: no column
                    // holds both (from 150 %, a column of nothing), and the waves end at the
                    // right edge.
                    let gap = if px >= 24 { 1 } else { 0 };
                    assert!(
                        waves.0 > device_last + gap,
                        "{look:?} {device:?} {level:?} at {px} px (large {large}): device to \
                         column {device_last}, waves from {}",
                        waves.0
                    );
                    assert!(
                        waves.1 + 2 >= px || level == VolumeLevel::Muted,
                        "{level:?} at {px} px"
                    );
                }
            }
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
            let outer = outer_wave(&raw_high, &raw_medium, px, colors.glyph);
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

/// The pixels of the outer wave only (fully covered in `high`, empty in `medium`), with
/// their coverage.
fn outer_wave(high: &Pixels, medium: &Pixels, px: u32, color: Color) -> Vec<(u32, u32, f32)> {
    // The glyph colour may be partly transparent (light taskbar: #E4000000).
    let full_alpha = color.a * 255.0;
    (0..px)
        .flat_map(|y| (first_level_column(px)..px).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            f32::from(high.pixel(x, y)[3]) >= 0.98 * full_alpha && medium.pixel(x, y)[3] == 0
        })
        .map(|(x, y)| (x, y, f32::from(high.pixel(x, y)[3]) / full_alpha))
        .collect()
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
                        &icon(Some(VolumeLevel::High), badge, false, TrayLook::BetterVolume),
                        theme,
                    )
                })
            };
            // The speaker or device, and the waves: both coloured.
            for columns in [0..14, 17..32] {
                let [r, _, b] = average(&draw(&blue), 32, columns.clone());
                assert!(b > r + 100.0, "{mode:?} {badge:?} {columns:?}: blue {r} {b}");
                let [r, _, b] = average(&draw(&reddish), 32, columns.clone());
                assert!(r > b + 100.0, "{mode:?} {badge:?} {columns:?}: red {r} {b}");
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
            assert!(light(8..13) > light(19..24) + 30.0, "{mode:?} {badge:?}: no gradient");
        }
    }
}

#[test]
fn the_bettervolume_waves_not_reached_are_a_pale_accent() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let theme = Theme::shell(&ThemeInputs { system_mode: mode, ..ThemeInputs::default() });
        let colors = theme.tray;
        for px in [32, 64] {
            let low = render(&gfx, px, |c| {
                paint(c, &icon(Some(VolumeLevel::Low), None, false, TrayLook::BetterVolume), &theme)
            });
            let raw = |level: VolumeLevel| {
                render(&gfx, px, |c| {
                    c.clear(Color::TRANSPARENT);
                    glyph(c, level.icon().glyph(), colors.glyph);
                })
            };
            let outer =
                outer_wave(&raw(VolumeLevel::High), &raw(VolumeLevel::Medium), px, colors.glyph);
            assert!(!outer.is_empty());
            for (x, y, cover) in outer {
                assert_colour(
                    &low,
                    x,
                    y,
                    colors.accent_unreached,
                    cover,
                    &format!("{mode:?} at {px} px"),
                );
            }
        }
    }
}

#[test]
fn the_bettervolume_speaker_takes_the_place_of_the_windows_one() {
    let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
    let theme = Theme::shell(&ThemeInputs::default());
    for (px, large) in SIZES.into_iter().flat_map(|px| [(px, false), (px, true)]) {
        let draw = |look| {
            render(&gfx, px, |c| {
                paint(c, &icon(Some(VolumeLevel::Zero), None, large, look), &theme)
            })
        };
        let (windows, ours) = (draw(TrayLook::Windows), draw(TrayLook::BetterVolume));
        let left = 0..first_level_column(px);
        let w = inked_columns(&windows, px, left.clone(), 8).expect("Windows speaker");
        let o = inked_columns(&ours, px, left, 8).expect("our speaker");
        assert!(
            o.0 <= w.0 + 1 && o.1 <= w.1,
            "at {px} px (large {large}): columns {o:?}, Windows {w:?}"
        );
        let rows = |img: &Pixels| {
            let inked: Vec<u32> = (0..px)
                .filter(|&y| (0..first_level_column(px)).any(|x| img.pixel(x, y)[3] > 8))
                .collect();
            (inked[0], inked[inked.len() - 1])
        };
        let (wr, or) = (rows(&windows), rows(&ours));
        assert!(
            or.0 + 1 >= wr.0 && or.1 <= wr.1 + 1,
            "at {px} px (large {large}): rows {or:?}, Windows {wr:?}"
        );
        // The waves are the same shapes as the Windows ones (only their colour differs).
        let right = first_level_column(px)..px;
        for y in 0..px {
            for x in right.clone() {
                assert_eq!(
                    windows.pixel(x, y)[3] > 0,
                    ours.pixel(x, y)[3] > 0,
                    "({x}, {y}) at {px} px"
                );
            }
        }
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
            assert_eq!(
                draw(TrayLook::BetterVolume),
                draw(TrayLook::Windows),
                "{badge:?} {level:?}"
            );
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
                        check_large(
                            &gfx,
                            &theme,
                            icon(level, badge, false, look),
                            px,
                            &mut problems,
                        );
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
        (Some(_), TrayLook::BetterVolume) => 1.0,
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
