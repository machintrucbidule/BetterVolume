//! The tray icon (FR-010, FR-011).
//!
//! Style "same as Windows": the Windows volume glyph itself. Style "with device badge":
//! the speaker part of that glyph is replaced by a drawing of the device type, in the
//! same place, size, stroke and colour; the waves (or the mute cross) stay on the right
//! and still show the level.

use crate::backend::Canvas;
use crate::icons::VolumeLevel;
use crate::path::Path;
use crate::{Color, Point, Rect, Theme};

/// Size of the tray icon in DIPs (Windows small-icon size at 100 %).
pub const LOGICAL_SIZE: f32 = 16.0;
/// Where the level part of the Windows volume glyphs is cut from: the middle of the gap
/// between the speaker (its edge ends at 7.0) and the first wave or the mute cross (they
/// start at 8.1), measured. The clip keeps whole pixels whose centre lies right of this
/// line, so at every Windows scaling (from 100 %) the pixel holding the speaker's
/// anti-aliased edge is dropped and the first wave pixel is kept.
const LEVEL_START: f32 = 7.55;
/// Stroke width of the Windows volume glyphs at 16 DIPs (measured).
const STROKE: f32 = 1.0;

/// Type of the current output, shown by the "with device badge" style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceType {
    /// Speaker enclosure.
    Speakers,
    /// Headphones.
    Headphones,
    /// Headset (headphones with a microphone).
    Headset,
    /// Display or TV.
    Display,
}

/// What the tray icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrayIcon {
    /// Volume step of the current output. `None`: no output device.
    pub level: Option<VolumeLevel>,
    /// Style "with device badge": the device drawn in place of the speaker. `None`: the
    /// Windows speaker (style "same as Windows", or a device of no particular type).
    pub badge: Option<DeviceType>,
}

/// The scale to rasterise an icon of `pixels` × `pixels` at.
pub fn scale_for(pixels: u32) -> f32 {
    pixels as f32 / LOGICAL_SIZE
}

/// Paints the icon on a transparent canvas of [`LOGICAL_SIZE`] DIPs.
pub fn paint(canvas: &mut Canvas<'_>, icon: &TrayIcon, theme: &Theme) {
    canvas.clear(Color::TRANSPARENT);
    let full = Rect::new(0.0, 0.0, LOGICAL_SIZE, LOGICAL_SIZE);
    // No device: Windows shows the muted speaker, dimmed.
    let (level, color) = match icon.level {
        Some(level) => (level, theme.tray_glyph),
        None => (VolumeLevel::Muted, theme.tray_glyph.with_alpha(theme.tray_glyph.a * 0.6)),
    };
    let glyph = level.icon().glyph();
    let level_part = Rect::new(LEVEL_START, 0.0, LOGICAL_SIZE - LEVEL_START, LOGICAL_SIZE);
    // Windows 11 draws the waves not reached in its "disabled text" colour under the
    // reached ones: #5D white on a dark taskbar, #5C black on a light one, the
    // GrayText system colour in high contrast. The muted icon (also the "no device" one)
    // has a cross instead, and no waves.
    if level != VolumeLevel::Muted && level != VolumeLevel::High {
        let all_waves = VolumeLevel::High.icon().glyph();
        canvas.clipped(level_part, |c| {
            c.glyph(all_waves, LOGICAL_SIZE, full, theme.text_disabled);
        });
    }
    match icon.badge {
        None => canvas.glyph(glyph, LOGICAL_SIZE, full, color),
        Some(device) => {
            canvas.clipped(level_part, |c| c.glyph(glyph, LOGICAL_SIZE, full, color));
            paint_device(canvas, device, color);
        }
    }
}

/// Width of the device drawing (outer stroke edges): exactly the speaker's, so the gap
/// Windows leaves before the first wave is kept at every scaling.
const DEVICE_WIDTH: f32 = 7.0;
/// Vertical centre of the speaker in the Windows glyphs.
const DEVICE_CENTER_Y: f32 = 8.0;

/// Draws a device in place of the speaker of the Windows glyph, in its natural
/// proportions (never stretched to the speaker's tall shape): at most [`DEVICE_WIDTH`]
/// wide, centred on the speaker's middle, stroke 1 DIP like the speaker it replaces.
fn paint_device(canvas: &mut Canvas<'_>, device: DeviceType, color: Color) {
    let half = STROKE / 2.0;
    let cy = DEVICE_CENTER_Y;
    match device {
        DeviceType::Speakers => {
            // A cabinet (about 5 wide for 7 high), tweeter and woofer.
            let (w, h) = (6.25, 8.75);
            let x = (DEVICE_WIDTH - w) / 2.0;
            canvas.stroke_rounded(
                Rect::new(x + half, cy - h / 2.0 + half, w - STROKE, h - STROKE),
                1.25,
                STROKE,
                color,
            );
            let mid = x + w / 2.0;
            canvas.fill_circle(Point::new(mid, cy - 2.2), 0.7, color);
            canvas.stroke_circle(Point::new(mid, cy + 1.25), 1.35, STROKE, color);
        }
        DeviceType::Headphones => headphones(canvas, color, false),
        DeviceType::Headset => headphones(canvas, color, true),
        DeviceType::Display => {
            // A 16:10 screen on a short stand.
            let (w, screen_h) = (DEVICE_WIDTH, 4.75);
            let top = cy - 3.6;
            canvas.stroke_rounded(
                Rect::new(half, top + half, w - STROKE, screen_h - STROKE),
                0.9,
                STROKE,
                color,
            );
            let mid = w / 2.0;
            let foot = top + screen_h + 1.9;
            let stand = Path::new()
                .move_to(mid, top + screen_h)
                .line_to(mid, foot)
                .move_to(mid - 1.75, foot)
                .line_to(mid + 1.75, foot);
            canvas.stroke_path(&stand, STROKE, color);
        }
    }
}

/// Headphones (round headband, filled ear cups), square overall; with a microphone boom
/// for a headset.
fn headphones(canvas: &mut Canvas<'_>, color: Color, microphone: bool) {
    let half = STROKE / 2.0;
    let (w, h) = (DEVICE_WIDTH, DEVICE_WIDTH);
    let top = DEVICE_CENTER_Y - h / 2.0;
    let bottom = top + h;
    let radius = w / 2.0 - half;
    let band_y = top + half + radius;
    let band = Path::new()
        .move_to(half, band_y)
        .arc_to(w - half, band_y, radius, true)
        .move_to(half, band_y)
        .line_to(half, band_y + 0.5)
        .move_to(w - half, band_y)
        .line_to(w - half, band_y + 0.5);
    canvas.stroke_path(&band, STROKE, color);
    let cup_w = 2.3;
    let cup_bottom = if microphone { bottom - 1.2 } else { bottom };
    let cup_top = band_y - 0.4;
    canvas.fill_rounded(Rect::new(0.0, cup_top, cup_w, cup_bottom - cup_top), 0.9, color);
    canvas.fill_rounded(Rect::new(w - cup_w, cup_top, cup_w, cup_bottom - cup_top), 0.9, color);
    if microphone {
        // Boom from under the left cup to the front, ending in the capsule.
        let y = bottom - half;
        let boom = Path::new()
            .move_to(cup_w / 2.0, cup_bottom - 0.2)
            .arc_to(cup_w / 2.0 + 1.1, y, 1.1, false)
            .line_to(w / 2.0 + 0.6, y);
        canvas.stroke_path(&boom, STROKE, color);
        canvas.fill_circle(Point::new(w / 2.0 + 1.0, y), 0.8, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{DeviceKind, Graphics, Pixels};
    use crate::ThemeInputs;

    /// Small-icon sizes of every Windows scaling step (100 % to 225 %) and a large one.
    const SIZES: [u32; 7] = [16, 20, 24, 28, 32, 36, 64];
    const DEVICES: [DeviceType; 4] =
        [DeviceType::Speakers, DeviceType::Headphones, DeviceType::Headset, DeviceType::Display];

    fn render(gfx: &Graphics, px: u32, paint: impl FnOnce(&mut Canvas<'_>)) -> Pixels {
        gfx.rasterize(px, px, scale_for(px), 1.0, paint).expect("rasterize")
    }

    fn first_level_column(px: u32) -> u32 {
        // Pixels whose centre lies right of LEVEL_START.
        (LEVEL_START * scale_for(px) - 0.5).ceil() as u32
    }

    #[test]
    fn nothing_of_the_windows_speaker_remains_next_to_the_device() {
        let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
        let theme = Theme::shell(&ThemeInputs::default());
        for px in SIZES {
            for device in DEVICES {
                // Volume 0: the Windows glyph is the speaker alone, so after the cut only
                // the device drawing may remain.
                let icon = TrayIcon { level: Some(VolumeLevel::Zero), badge: Some(device) };
                let with_glyph = render(&gfx, px, |c| paint(c, &icon, &theme));
                let device_only = render(&gfx, px, |c| {
                    c.clear(Color::TRANSPARENT);
                    paint_device(c, device, theme.tray_glyph);
                });
                // Left of the cut only the device; right of it, the dimmed waves.
                for y in 0..px {
                    for x in 0..first_level_column(px) {
                        assert_eq!(
                            with_glyph.pixel(x, y),
                            device_only.pixel(x, y),
                            "{device:?} at {px} px, pixel ({x}, {y}): speaker remnant"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn waves_not_reached_are_dimmed_like_windows_11() {
        use crate::theme::{HighContrastColors, ThemeMode};
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
            // Large enough for the 1-DIP wave stroke to cover whole pixels.
            for px in [32, 64] {
                let draw = |level| {
                    render(&gfx, px, |c| {
                        paint(c, &TrayIcon { level: Some(level), badge: None }, &theme)
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
                    theme.high_contrast
                        || (alpha(&zero) < alpha(&low)
                            && alpha(&low) < alpha(&medium)
                            && alpha(&medium) < alpha(&high)),
                    "{name} at {px} px"
                );
                // Full volume and mute (also "no device") are the Windows glyphs alone.
                let glyph_only = |level: VolumeLevel, color: Color| {
                    render(&gfx, px, |c| {
                        c.clear(Color::TRANSPARENT);
                        c.glyph(
                            level.icon().glyph(),
                            LOGICAL_SIZE,
                            Rect::new(0.0, 0.0, LOGICAL_SIZE, LOGICAL_SIZE),
                            color,
                        );
                    })
                };
                // Where only the outer wave of the Windows glyphs is (fully covered), a low
                // level shows it in the "disabled text" colour exactly.
                let raw_high = glyph_only(VolumeLevel::High, theme.tray_glyph);
                let raw_medium = glyph_only(VolumeLevel::Medium, theme.tray_glyph);
                // The glyph colour may be partly transparent (light taskbar: #E4000000).
                let full_alpha = theme.tray_glyph.a * 255.0;
                let outer: Vec<(u32, u32)> = (0..px)
                    .flat_map(|y| (first_level_column(px)..px).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        f32::from(raw_high.pixel(x, y)[3]) >= 0.98 * full_alpha
                            && raw_medium.pixel(x, y)[3] == 0
                    })
                    .collect();
                assert!(!outer.is_empty(), "{name} at {px} px: no outer wave pixel");
                let d = theme.text_disabled;
                for (x, y) in outer {
                    let cover = f32::from(raw_high.pixel(x, y)[3]) / full_alpha;
                    let expected = [d.b * d.a, d.g * d.a, d.r * d.a, d.a]
                        .map(|v| (v * cover * 255.0).round() as i32);
                    let got = low.pixel(x, y).map(i32::from);
                    let close = got.iter().zip(expected).all(|(g, e)| (g - e).abs() <= 3);
                    assert!(
                        close,
                        "{name} at {px} px, pixel ({x}, {y}): {got:?}, expected {expected:?}"
                    );
                    assert_eq!(
                        medium.pixel(x, y),
                        low.pixel(x, y),
                        "{name}: the outer wave is dim at medium too"
                    );
                }
                assert_eq!(high, glyph_only(VolumeLevel::High, theme.tray_glyph), "{name}");
                assert_eq!(
                    draw(VolumeLevel::Muted),
                    glyph_only(VolumeLevel::Muted, theme.tray_glyph),
                    "{name}"
                );
                let none =
                    render(&gfx, px, |c| paint(c, &TrayIcon { level: None, badge: None }, &theme));
                let dimmed = theme.tray_glyph.with_alpha(theme.tray_glyph.a * 0.6);
                assert_eq!(none, glyph_only(VolumeLevel::Muted, dimmed), "{name}: no device");
            }
        }
    }

    #[test]
    fn the_level_part_is_the_windows_one_and_the_device_stays_left_of_it() {
        let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
        let theme = Theme::shell(&ThemeInputs::default());
        let levels = [
            VolumeLevel::Zero,
            VolumeLevel::Low,
            VolumeLevel::Medium,
            VolumeLevel::High,
            VolumeLevel::Muted,
        ];
        for px in SIZES {
            let first = first_level_column(px);
            for level in levels {
                let plain = render(&gfx, px, |c| {
                    paint(c, &TrayIcon { level: Some(level), badge: None }, &theme)
                });
                for device in DEVICES {
                    let icon = TrayIcon { level: Some(level), badge: Some(device) };
                    let badged = render(&gfx, px, |c| paint(c, &icon, &theme));
                    for y in 0..px {
                        for x in first..px {
                            assert_eq!(
                                badged.pixel(x, y),
                                plain.pixel(x, y),
                                "{device:?} {level:?} at {px} px, pixel ({x}, {y})"
                            );
                        }
                    }
                    // The device never reaches further right than the Windows speaker it
                    // replaces, so the gap before the first wave stays the Windows one.
                    let device_only = render(&gfx, px, |c| {
                        c.clear(Color::TRANSPARENT);
                        paint_device(c, device, theme.tray_glyph);
                    });
                    let last = |img: &Pixels| {
                        (0..first)
                            .filter(|&x| (0..px).any(|y| img.pixel(x, y)[3] > 8))
                            .max()
                            .unwrap_or(0)
                    };
                    let speaker = render(&gfx, px, |c| {
                        paint(c, &TrayIcon { level: Some(VolumeLevel::Zero), badge: None }, &theme)
                    });
                    assert!(
                        last(&speaker) > 0,
                        "nothing drawn: the Windows icon font (Segoe Fluent Icons) is missing"
                    );
                    assert!(
                        last(&device_only) <= last(&speaker),
                        "{device:?} at {px} px reaches column {}, the speaker {}",
                        last(&device_only),
                        last(&speaker)
                    );
                }
            }
        }
    }
}
