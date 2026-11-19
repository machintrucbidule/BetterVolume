//! The tray icon (FR-010, FR-011, FR-083, FR-084).
//!
//! Three independent choices. Picture: the speaker, or the device now playing (its type
//! drawn in place of the speaker, larger, the waves made smaller to give it room). Look:
//! Windows (the Windows volume glyph, one colour) or BetterVolume (the speaker of the
//! application icon and the waves in a gradient of the accent colour, the devices as
//! filled drawings in that gradient). Size: the Windows one, or large (stretched to the
//! height of the icon). The waves (or the mute cross) always show the level.

use crate::backend::{Canvas, Gradient};
use crate::icons::VolumeLevel;
use crate::path::Path;
use crate::theme::TrayColors;
use crate::{Color, Point, Rect, Theme};

/// Size of the tray icon in DIPs (Windows small-icon size at 100 %).
pub const LOGICAL_SIZE: f32 = 16.0;
/// Where the level part of the Windows volume glyphs is cut from: the middle of the gap
/// between the speaker (its edge ends at [`SPEAKER_RIGHT`]) and the first wave or the
/// mute cross (they start at [`FIRST_WAVE`]), measured. The clip keeps whole pixels
/// whose centre lies right of this line, so at every Windows scaling (from 100 %) the
/// pixel holding the speaker's anti-aliased edge is dropped and the first wave pixel is
/// kept.
const LEVEL_START: f32 = 7.55;
/// Right edge of the speaker of the Windows volume glyphs (measured).
const SPEAKER_RIGHT: f32 = 7.0;
/// Left edge of the first wave and of the mute cross of the Windows glyphs (measured).
const FIRST_WAVE: f32 = 8.1;
/// Stroke width of the Windows volume glyphs at 16 DIPs (measured).
const STROKE: f32 = 1.0;
/// Size "Large" (FR-083): the drawing stretched vertically about the middle of the icon.
/// The Windows glyphs already span the whole width of the icon (the loudest step touches
/// both sides); only their height leaves a margin. Their tallest part, the loudest step's
/// outer wave (1.25 to 14.75 DIPs, measured), then fills the icon's height, 0.06 DIP from
/// each edge.
pub const LARGE_STRETCH: f32 = 20.0 / 17.0;
/// Vertical middle of the icon, the line the large size is stretched about.
const MIDDLE: f32 = LOGICAL_SIZE / 2.0;
/// Picture "device now playing": the waves at this scale, about the middle of the icon's
/// right edge, so the device gets more room than the Windows speaker.
pub const DEVICE_WAVES_SCALE: f32 = 0.75;
/// Picture "device now playing": the width of the device drawing, from the left edge to
/// the Windows gap before the (smaller) waves.
pub const DEVICE_WIDTH: f32 =
    LOGICAL_SIZE - (LOGICAL_SIZE - FIRST_WAVE) * DEVICE_WAVES_SCALE - (FIRST_WAVE - SPEAKER_RIGHT);
/// Vertical centre of the speaker in the Windows glyphs, and of the device drawings.
const DEVICE_CENTER_Y: f32 = 8.0;

/// Type of the current output, shown by the picture "device now playing".
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

/// Look of the tray icon (FR-084).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrayLook {
    /// The Windows volume glyph, in the taskbar's text colour.
    #[default]
    Windows,
    /// The speaker of the application icon and the waves in the accent gradient.
    BetterVolume,
}

/// What the tray icon shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrayIcon {
    /// Volume step of the current output. `None`: no output device.
    pub level: Option<VolumeLevel>,
    /// Picture "device now playing": the device drawn in place of the speaker. `None`:
    /// the speaker (picture "speaker", or a device of no particular type).
    pub badge: Option<DeviceType>,
    /// Size "Large" (FR-083): stretched to the icon's height ([`LARGE_STRETCH`]).
    pub large: bool,
    /// The look (FR-084).
    pub look: TrayLook,
}

/// The scale to rasterise an icon of `pixels` × `pixels` at.
pub fn scale_for(pixels: u32) -> f32 {
    pixels as f32 / LOGICAL_SIZE
}

/// Paints the icon on a transparent canvas of [`LOGICAL_SIZE`] DIPs.
pub fn paint(canvas: &mut Canvas<'_>, icon: &TrayIcon, theme: &Theme) {
    canvas.clear(Color::TRANSPARENT);
    draw(canvas, icon, &theme.tray);
}

/// Paints the icon over what is drawn, filling the square `square` (a picture of the
/// icon, such as the choices of the settings window).
pub fn paint_in(canvas: &mut Canvas<'_>, square: Rect, icon: &TrayIcon, colors: &TrayColors) {
    let scale = square.width.min(square.height) / LOGICAL_SIZE;
    canvas.placed(Point::new(square.x, square.y), scale, |c| draw(c, icon, colors));
}

fn draw(canvas: &mut Canvas<'_>, icon: &TrayIcon, colors: &TrayColors) {
    if icon.large {
        canvas.stretched_vertically(LARGE_STRETCH, MIDDLE, |c| paint_icon(c, icon, colors));
    } else {
        paint_icon(canvas, icon, colors);
    }
}

/// What one painting needs: the look actually drawn and its colours.
struct Ink<'a> {
    colors: &'a TrayColors,
    look: TrayLook,
    /// No output device: everything in the dimmed glyph colour, as Windows.
    dim: bool,
}

impl Ink<'_> {
    /// The one colour of the Windows look (dimmed without a device).
    fn plain(&self) -> Color {
        let glyph = self.colors.glyph;
        if self.dim {
            glyph.with_alpha(glyph.a * 0.6)
        } else {
            glyph
        }
    }

    /// Whether to paint with the accent gradient.
    fn gradient(&self) -> bool {
        self.look == TrayLook::BetterVolume && !self.dim
    }

    /// The accent gradient from `from` (top) to `to` (bottom).
    fn accent(&self, from: Point, to: Point) -> Gradient {
        Gradient { from, to, colors: self.colors.accent }
    }

    /// Runs `draw` in the colour of this look: `draw` gets the colour to use, and inside
    /// the gradient that colour only gives the opacity.
    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        from: Point,
        to: Point,
        draw: impl FnOnce(&mut Canvas<'_>, Color),
    ) {
        if self.gradient() {
            canvas.with_gradient(&self.accent(from, to), |c| draw(c, Color::WHITE));
        } else {
            draw(canvas, self.plain());
        }
    }
}

/// The icon at the Windows size (the stretch, if any, is already set).
fn paint_icon(canvas: &mut Canvas<'_>, icon: &TrayIcon, colors: &TrayColors) {
    // No device: Windows shows the muted speaker, dimmed.
    let (level, dim) = match icon.level {
        Some(level) => (level, false),
        None => (VolumeLevel::Muted, true),
    };
    // High contrast allows only the system colours: the Windows look then.
    let look = if colors.high_contrast { TrayLook::Windows } else { icon.look };
    let ink = Ink { colors, look, dim };
    match icon.badge {
        None => {
            level_part(canvas, level, &ink);
            match look {
                // The Windows speaker: the glyph's own part left of the cut.
                TrayLook::Windows => canvas
                    .clipped(Rect::new(0.0, 0.0, LEVEL_START, LOGICAL_SIZE), |c| {
                        glyph(c, level.icon().glyph(), ink.plain())
                    }),
                TrayLook::BetterVolume => {
                    let speaker = app_speaker();
                    ink.paint(canvas, Point::new(1.0, 3.0), Point::new(6.0, 13.0), |c, color| {
                        c.fill_path(&speaker, color)
                    });
                }
            }
        }
        Some(device) => {
            let anchor = Point::new(LOGICAL_SIZE, MIDDLE);
            canvas.scaled(DEVICE_WAVES_SCALE, DEVICE_WAVES_SCALE, anchor, |c| {
                level_part(c, level, &ink)
            });
            let (from, to) =
                (Point::new(DEVICE_WIDTH * 0.2, 2.5), Point::new(DEVICE_WIDTH * 0.8, 13.5));
            match look {
                TrayLook::Windows => outline_device(canvas, device, ink.plain()),
                TrayLook::BetterVolume => {
                    ink.paint(canvas, from, to, |c, color| filled_device(c, device, color))
                }
            }
        }
    }
}

fn glyph(canvas: &mut Canvas<'_>, glyph: char, color: Color) {
    canvas.glyph(glyph, LOGICAL_SIZE, Rect::new(0.0, 0.0, LOGICAL_SIZE, LOGICAL_SIZE), color);
}

/// The waves (or the mute cross) of the Windows glyph of `level`, right of the cut.
/// Windows 11 draws the waves not reached in its "disabled text" colour under the
/// reached ones (#5D white on a dark taskbar, #5C black on a light one, GrayText in high
/// contrast); the BetterVolume look draws them in a pale accent. The muted icon (also the
/// "no device" one) has a cross instead, and no waves.
fn level_part(canvas: &mut Canvas<'_>, level: VolumeLevel, ink: &Ink<'_>) {
    let part = Rect::new(LEVEL_START, 0.0, LOGICAL_SIZE - LEVEL_START, LOGICAL_SIZE);
    if level != VolumeLevel::Muted && level != VolumeLevel::High {
        let unreached = match ink.look {
            TrayLook::Windows => ink.colors.unreached,
            TrayLook::BetterVolume => ink.colors.accent_unreached,
        };
        let all = VolumeLevel::High.icon().glyph();
        canvas.clipped(part, |c| glyph(c, all, unreached));
    }
    let reached = level.icon().glyph();
    ink.paint(canvas, Point::new(9.0, 2.0), Point::new(14.0, 14.0), |c, color| {
        c.clipped(part, |c| glyph(c, reached, color))
    });
}

/// The speaker of the application icon (its small drawing, `BetterVolume-small.svg`,
/// without the two roads), in the place of the Windows speaker: from the left edge to
/// [`SPEAKER_RIGHT`], centred on the speaker's middle.
fn app_speaker() -> Path {
    // The drawing spans x 2 to 19.8 and is centred on y 24 in its 48-unit square.
    let s = SPEAKER_RIGHT / 17.8;
    let p = |x: f32, y: f32| ((x - 2.0) * s, DEVICE_CENTER_Y + (y - 24.0) * s);
    let at = |path: Path, x: f32, y: f32| {
        let (x, y) = p(x, y);
        path.line_to(x, y)
    };
    let (x, y) = p(2.0, 18.4);
    let r = 2.4 * s;
    let path = Path::new().move_to(x, y);
    let (x, y) = p(4.4, 16.0);
    let path = path.arc_to(x, y, r, true);
    let path = at(at(path, 8.8, 16.0), 16.3, 9.6);
    let path = path.cubic_to(p(17.7, 8.4), p(19.8, 9.4), p(19.8, 11.2));
    let path = at(path, 19.8, 36.8);
    let path = path.cubic_to(p(19.8, 38.6), p(17.7, 39.6), p(16.3, 38.4));
    let path = at(at(path, 8.8, 32.0), 4.4, 32.0);
    let (x, y) = p(2.0, 29.6);
    path.arc_to(x, y, r, true).close()
}

/// A closed rounded rectangle figure added to `path`.
fn rounded(path: Path, r: Rect, radius: f32) -> Path {
    let (x, y, w, h) = (r.x, r.y, r.width, r.height);
    path.move_to(x + radius, y)
        .line_to(x + w - radius, y)
        .arc_to(x + w, y + radius, radius, true)
        .line_to(x + w, y + h - radius)
        .arc_to(x + w - radius, y + h, radius, true)
        .line_to(x + radius, y + h)
        .arc_to(x, y + h - radius, radius, true)
        .line_to(x, y + radius)
        .arc_to(x + radius, y, radius, true)
        .close()
}

/// A closed circle figure added to `path`.
fn circle(path: Path, cx: f32, cy: f32, r: f32) -> Path {
    path.move_to(cx - r, cy).arc_to(cx + r, cy, r, true).arc_to(cx - r, cy, r, true).close()
}

/// The device, Windows look: 1-DIP outlines like the Windows speaker it replaces, in its
/// natural proportions (never stretched to the speaker's tall shape), [`DEVICE_WIDTH`]
/// wide at most, centred on the speaker's middle. Drawn at 7 DIPs, then enlarged.
fn outline_device(canvas: &mut Canvas<'_>, device: DeviceType, color: Color) {
    let f = DEVICE_WIDTH / 7.0;
    let w = DEVICE_WIDTH;
    let half = STROKE / 2.0;
    let cy = DEVICE_CENTER_Y;
    match device {
        DeviceType::Speakers => {
            // A cabinet (about 5 wide for 7 high), tweeter and woofer.
            let (cw, h) = (6.25 * f, 8.75 * f);
            let x = (w - cw) / 2.0;
            canvas.stroke_rounded(
                Rect::new(x + half, cy - h / 2.0 + half, cw - STROKE, h - STROKE),
                1.25 * f,
                STROKE,
                color,
            );
            let mid = x + cw / 2.0;
            canvas.fill_circle(Point::new(mid, cy - 2.2 * f), 0.7 * f, color);
            canvas.stroke_circle(Point::new(mid, cy + 1.25 * f), 1.35 * f, STROKE, color);
        }
        DeviceType::Headphones | DeviceType::Headset => {
            // Round headband, filled ear cups; a microphone boom for a headset.
            let microphone = device == DeviceType::Headset;
            let top = cy - w / 2.0;
            let bottom = top + w;
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
            let cup_w = 2.3 * f;
            let cup_bottom = if microphone { bottom - 1.2 * f } else { bottom };
            let cup_top = band_y - 0.4 * f;
            let cup = |x: f32| Rect::new(x, cup_top, cup_w, cup_bottom - cup_top);
            canvas.fill_rounded(cup(0.0), 0.9 * f, color);
            canvas.fill_rounded(cup(w - cup_w), 0.9 * f, color);
            if microphone {
                // Boom from under the left cup to the front, ending in the capsule.
                let y = bottom - half;
                let boom = Path::new()
                    .move_to(cup_w / 2.0, cup_bottom - 0.2)
                    .arc_to(cup_w / 2.0 + 1.1 * f, y, 1.1 * f, false)
                    .line_to(w / 2.0 + 0.6 * f, y);
                canvas.stroke_path(&boom, STROKE, color);
                canvas.fill_circle(Point::new(w / 2.0 + 1.0 * f, y), 0.8 * f, color);
            }
        }
        DeviceType::Display => {
            // A 16:10 screen on a short stand.
            let screen_h = 4.75 * f;
            let top = cy - 3.6 * f;
            canvas.stroke_rounded(
                Rect::new(half, top + half, w - STROKE, screen_h - STROKE),
                0.9 * f,
                STROKE,
                color,
            );
            let mid = w / 2.0;
            let foot = top + screen_h + 1.9 * f;
            let stand = Path::new()
                .move_to(mid, top + screen_h)
                .line_to(mid, foot)
                .move_to(mid - 1.75 * f, foot)
                .line_to(mid + 1.75 * f, foot);
            canvas.stroke_path(&stand, STROKE, color);
        }
    }
}

/// The device, BetterVolume look: filled drawings (the speaker cabinet with its drivers
/// cut out, a thick headband, a full screen), [`DEVICE_WIDTH`] wide at most, centred on
/// the speaker's middle.
fn filled_device(canvas: &mut Canvas<'_>, device: DeviceType, color: Color) {
    let w = DEVICE_WIDTH;
    let cy = DEVICE_CENTER_Y;
    match device {
        DeviceType::Speakers => {
            let cw = w * 0.86;
            let ch = cw * 1.38;
            let x = (w - cw) / 2.0;
            let mid = x + cw / 2.0;
            // Even-odd filling: the drivers are holes in the cabinet.
            let cabinet = rounded(Path::new(), Rect::new(x, cy - ch / 2.0, cw, ch), cw * 0.2);
            let cabinet = circle(cabinet, mid, cy - ch * 0.26, cw * 0.11);
            let cabinet = circle(cabinet, mid, cy + ch * 0.15, cw * 0.27);
            canvas.fill_path(&cabinet, color);
            canvas.fill_circle(Point::new(mid, cy + ch * 0.15), cw * 0.09, color);
        }
        DeviceType::Headphones | DeviceType::Headset => {
            let microphone = device == DeviceType::Headset;
            let band_w = w * 0.15;
            let top = cy - w / 2.0;
            let bottom = top + w;
            let radius = (w - band_w) / 2.0;
            let band_y = top + band_w / 2.0 + radius;
            let band = Path::new()
                .move_to(band_w / 2.0, band_y + 0.3)
                .line_to(band_w / 2.0, band_y)
                .arc_to(w - band_w / 2.0, band_y, radius, true)
                .line_to(w - band_w / 2.0, band_y + 0.3);
            canvas.stroke_path(&band, band_w, color);
            let cup_w = w * 0.36;
            let cup_bottom = if microphone { bottom - w * 0.17 } else { bottom };
            let cup_top = band_y - w * 0.08;
            let cup = |x: f32| Rect::new(x, cup_top, cup_w, cup_bottom - cup_top);
            canvas.fill_rounded(cup(0.0), cup_w * 0.38, color);
            canvas.fill_rounded(cup(w - cup_w), cup_w * 0.38, color);
            if microphone {
                let y = bottom - band_w / 2.0;
                let boom = Path::new()
                    .move_to(cup_w / 2.0, cup_bottom - 0.2)
                    .arc_to(cup_w / 2.0 + w * 0.16, y, w * 0.16, false)
                    .line_to(w / 2.0 + w * 0.08, y);
                canvas.stroke_path(&boom, band_w * 0.85, color);
                canvas.fill_circle(Point::new(w / 2.0 + w * 0.14, y), band_w * 0.75, color);
            }
        }
        DeviceType::Display => {
            let screen_h = w * 0.66;
            let total = screen_h + w * 0.3;
            let top = cy - total / 2.0;
            let screen = rounded(Path::new(), Rect::new(0.0, top, w, screen_h), w * 0.11);
            canvas.fill_path(&screen, color);
            let neck = w * 0.14;
            canvas.fill_rect(
                Rect::new(w / 2.0 - neck / 2.0, top + screen_h - 0.1, neck, w * 0.2),
                color,
            );
            let (foot_w, foot_h) = (w * 0.56, w * 0.12);
            canvas.fill_rounded(
                Rect::new(w / 2.0 - foot_w / 2.0, top + total - foot_h, foot_w, foot_h),
                foot_h / 2.0,
                color,
            );
        }
    }
}

#[cfg(test)]
mod tests;
