//! The tray icon (FR-010, FR-011, FR-083, FR-084).
//!
//! Three independent choices. Picture: the speaker, or the device now playing (its type
//! drawn in place of the speaker). Look: Windows (the Windows volume glyph, one colour;
//! with a device, only its speaker is replaced) or BetterVolume (the speaker of the
//! application icon and our own waves, in a gradient of the accent colour; with a device,
//! a larger device left of the waves). Size: the Windows one, or large (stretched to the
//! height of the icon). The waves (or the mute cross) always show the level, and every
//! drawing keeps its natural proportions.

use crate::backend::{Canvas, Gradient};
use crate::icons::VolumeLevel;
use crate::path::Path;
use crate::theme::TrayColors;
use crate::{Color, Point, Rect, Theme};

/// Size of the tray icon in DIPs (Windows small-icon size at 100 %).
pub const LOGICAL_SIZE: f32 = 16.0;
/// Where the level part of the Windows volume glyphs is cut from: the middle of the gap
/// between the speaker (its edge ends at [`SPEAKER_RIGHT`]) and the first wave or the
/// mute cross (they start at 8.1), measured. The clip keeps whole pixels whose centre lies
/// right of this line, so at every Windows scaling (from 100 %) the pixel holding the
/// speaker's anti-aliased edge is dropped and the first wave pixel is kept.
const LEVEL_START: f32 = 7.55;
/// Right edge of the speaker of the Windows volume glyphs (measured).
const SPEAKER_RIGHT: f32 = 7.0;
/// Height of the speaker of the Windows volume glyphs (2.38 to 13.62 DIPs, measured).
const SPEAKER_HEIGHT: f32 = 11.24;
/// Size "Large" (FR-083): the drawing stretched vertically about the middle of the icon.
/// The Windows glyphs already span the whole width of the icon (the loudest step touches
/// both sides); only their height leaves a margin. Their tallest part, the loudest step's
/// outer wave (1.25 to 14.75 DIPs, measured), then fills the icon's height, 0.06 DIP from
/// each edge.
pub const LARGE_STRETCH: f32 = 20.0 / 17.0;
/// Vertical middle of the icon: the line the large size is stretched about, and the
/// centre of the speaker, the devices and the waves.
const MIDDLE: f32 = LOGICAL_SIZE / 2.0;

/// Our waves (BetterVolume look): three arcs about a centre on the middle line, against
/// the icon's right edge. Their stroke is the Windows one; they are farther apart than a
/// mere scaled-down Windows glyph would leave them, so that they stay distinct at the real
/// size (they merged into one stroke at 1.4 pixels apart).
pub mod waves {
    /// Stroke width (the Windows glyphs' one).
    pub const STROKE: f32 = 1.0;
    /// Empty space between two arcs.
    pub const GAP: f32 = 1.45;
    /// Half the angle each arc spans, in degrees.
    pub const ANGLE: f32 = 56.0;
    /// Room left between the outer arc and the icon's right edge.
    pub const MARGIN: f32 = 0.2;
    /// Gap between the waves and the picture left of them.
    pub const PICTURE_GAP: f32 = 1.0;
}

/// Where our waves are: the radius of the first arc sets how much room the picture left
/// of them gets.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Waves {
    r1: f32,
}

impl Waves {
    /// Next to the application speaker.
    const SPEAKER: Waves = Waves { r1: 1.9 };
    /// Next to a device: tighter, so the device gets more than half the width.
    const DEVICE: Waves = Waves { r1: 1.4 };

    /// Radius of arc `i` (0 = the innermost).
    fn r(self, i: usize) -> f32 {
        self.r1 + i as f32 * (waves::STROKE + waves::GAP)
    }

    /// Centre of the arcs (on the middle line).
    fn cx(self) -> f32 {
        LOGICAL_SIZE - waves::MARGIN - self.r(2) - waves::STROKE / 2.0
    }

    /// Left edge of the innermost arc (its ends).
    fn left(self) -> f32 {
        self.cx() + self.r1 * waves::ANGLE.to_radians().cos() - waves::STROKE / 2.0
    }

    /// Width left of the waves for the picture.
    fn room(self) -> f32 {
        self.left() - waves::PICTURE_GAP
    }

    fn arc(self, i: usize) -> Path {
        let r = self.r(i);
        let a = waves::ANGLE.to_radians();
        let (x, dy) = (self.cx() + r * a.cos(), r * a.sin());
        Path::new().move_to(x, MIDDLE - dy).arc_to(x, MIDDLE + dy, r, true)
    }
}

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
    /// The speaker of the application icon and our waves in the accent gradient.
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

    /// Runs `draw` in the colour of this look: `draw` gets the colour to use; inside the
    /// accent gradient (from `from`, top, to `to`, bottom) that colour only gives the
    /// opacity. A translucent colour (the light taskbar's glyph colour, the dimmed one)
    /// is applied to the drawing as a whole, so its overlapping parts stay even.
    fn paint(
        &self,
        canvas: &mut Canvas<'_>,
        from: Point,
        to: Point,
        draw: impl FnOnce(&mut Canvas<'_>, Color),
    ) {
        if self.look == TrayLook::BetterVolume && !self.dim {
            let gradient = Gradient { from, to, colors: self.colors.accent };
            canvas.with_gradient(&gradient, |c| draw(c, Color::WHITE));
        } else {
            let plain = self.plain();
            if plain.a < 1.0 {
                canvas.with_opacity(plain.a, |c| draw(c, plain.with_alpha(1.0)));
            } else {
                draw(canvas, plain);
            }
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
    match (look, icon.badge) {
        // The Windows glyph itself: its speaker is the part left of the cut.
        (TrayLook::Windows, None) => {
            windows_level(canvas, level, &ink);
            canvas.clipped(Rect::new(0.0, 0.0, LEVEL_START, LOGICAL_SIZE), |c| {
                glyph(c, level.icon().glyph(), ink.plain())
            });
        }
        // The Windows glyph with our device in place of its speaker.
        (TrayLook::Windows, Some(device)) => {
            windows_level(canvas, level, &ink);
            ink.paint(canvas, TOP, BOTTOM, |c, color| {
                device_drawing(c, device, SPEAKER_RIGHT, color)
            });
        }
        (TrayLook::BetterVolume, None) => {
            let waves = Waves::SPEAKER;
            our_level(canvas, waves, level, &ink);
            let speaker = app_speaker(waves.room().min(SPEAKER_WIDTH));
            ink.paint(canvas, TOP, BOTTOM, |c, color| c.fill_path(&speaker, color));
        }
        (TrayLook::BetterVolume, Some(device)) => {
            let waves = Waves::DEVICE;
            our_level(canvas, waves, level, &ink);
            let w = waves.room();
            ink.paint(canvas, TOP, BOTTOM, |c, color| device_drawing(c, device, w, color));
        }
    }
}

/// The accent gradient runs from top to bottom of the icon, the same for every part.
const TOP: Point = Point::new(0.0, 2.0);
/// See [`TOP`].
const BOTTOM: Point = Point::new(0.0, 14.0);

/// Width of the application speaker in the BetterVolume look (a little wider than the
/// Windows speaker, which its filled shape replaces).
const SPEAKER_WIDTH: f32 = 7.6;

fn glyph(canvas: &mut Canvas<'_>, glyph: char, color: Color) {
    canvas.glyph(glyph, LOGICAL_SIZE, Rect::new(0.0, 0.0, LOGICAL_SIZE, LOGICAL_SIZE), color);
}

/// The waves (or the mute cross) of the Windows glyph of `level`, right of the cut.
/// Windows 11 draws the waves not reached in its "disabled text" colour under the
/// reached ones (#5D white on a dark taskbar, #5C black on a light one, GrayText in high
/// contrast). The muted icon (also the "no device" one) has a cross instead, and no waves.
fn windows_level(canvas: &mut Canvas<'_>, level: VolumeLevel, ink: &Ink<'_>) {
    let part = Rect::new(LEVEL_START, 0.0, LOGICAL_SIZE - LEVEL_START, LOGICAL_SIZE);
    if level != VolumeLevel::Muted && level != VolumeLevel::High {
        let all = VolumeLevel::High.icon().glyph();
        canvas.clipped(part, |c| glyph(c, all, ink.colors.unreached));
    }
    let reached = level.icon().glyph();
    canvas.clipped(part, |c| glyph(c, reached, ink.plain()));
}

/// Our waves for `level` (BetterVolume look): as many arcs reached as the Windows glyph
/// has waves (none at 0, one when low, two at medium, three when high), the others in a
/// pale accent; a cross when muted (or without a device).
fn our_level(canvas: &mut Canvas<'_>, w: Waves, level: VolumeLevel, ink: &Ink<'_>) {
    let reached = match level {
        VolumeLevel::Muted => {
            let size = w.r(2) * 0.55;
            let (x, y) = (w.cx() + w.r(1), MIDDLE);
            let cross = Path::new()
                .move_to(x - size / 2.0, y - size / 2.0)
                .line_to(x + size / 2.0, y + size / 2.0)
                .move_to(x + size / 2.0, y - size / 2.0)
                .line_to(x - size / 2.0, y + size / 2.0);
            ink.paint(canvas, TOP, BOTTOM, |c, color| c.stroke_path(&cross, waves::STROKE, color));
            return;
        }
        VolumeLevel::Zero => 0,
        VolumeLevel::Low => 1,
        VolumeLevel::Medium => 2,
        VolumeLevel::High => 3,
    };
    for i in reached..3 {
        canvas.stroke_path(&w.arc(i), waves::STROKE, ink.colors.accent_unreached);
    }
    for i in 0..reached {
        let arc = w.arc(i);
        ink.paint(canvas, TOP, BOTTOM, |c, color| c.stroke_path(&arc, waves::STROKE, color));
    }
}

/// The speaker of the application icon (its small drawing, `BetterVolume-small.svg`,
/// without the two roads), `width` wide from the left edge, centred on the middle line.
fn app_speaker(width: f32) -> Path {
    // The drawing spans x 2 to 19.8 and is centred on y 24 in its 48-unit square.
    let s = width / 17.8;
    let p = |x: f32, y: f32| ((x - 2.0) * s, MIDDLE + (y - 24.0) * s);
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

/// Proportions of the device drawings (height for a width of 1), kept whatever their
/// size: never stretched (except by the large size, like everything).
pub mod proportions {
    /// Speaker cabinet: height for its width (a bookshelf speaker).
    pub const SPEAKERS: f32 = 1.4;
    /// Headphones: as tall as wide.
    pub const HEADPHONES: f32 = 1.0;
    /// Screen: height for its width (16:10), plus the stand.
    pub const SCREEN: f32 = 0.62;
    /// The stand under the screen, for the screen's width.
    pub const STAND: f32 = 0.28;
}

/// The device as a filled drawing (the speaker cabinet with its drivers cut out, a thick
/// headband and filled cups, a full screen on its stand), at most `w` wide and never
/// taller than the Windows speaker, in its natural proportions, centred on the middle
/// line.
fn device_drawing(canvas: &mut Canvas<'_>, device: DeviceType, w: f32, color: Color) {
    let cy = MIDDLE;
    match device {
        DeviceType::Speakers => {
            let cw = (w * 0.9).min(SPEAKER_HEIGHT / proportions::SPEAKERS);
            let ch = cw * proportions::SPEAKERS;
            let x = (w - cw) / 2.0;
            let mid = x + cw / 2.0;
            // Even-odd filling: the drivers are holes in the cabinet.
            let cabinet = rounded(Path::new(), Rect::new(x, cy - ch / 2.0, cw, ch), cw * 0.2);
            let cabinet = circle(cabinet, mid, cy - ch * 0.25, cw * 0.12);
            let cabinet = circle(cabinet, mid, cy + ch * 0.15, cw * 0.28);
            canvas.fill_path(&cabinet, color);
            canvas.fill_circle(Point::new(mid, cy + ch * 0.15), cw * 0.09, color);
        }
        DeviceType::Headphones | DeviceType::Headset => {
            let microphone = device == DeviceType::Headset;
            let h = w * proportions::HEADPHONES;
            let band_w = w * 0.15;
            let top = cy - h / 2.0;
            let bottom = top + h;
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
                // Boom from under the left cup to the front, ending in the capsule.
                let y = bottom - band_w / 2.0;
                let r = w * 0.16;
                let boom = Path::new()
                    .move_to(cup_w / 2.0, cup_bottom - 0.2)
                    .arc_to(cup_w / 2.0 + r, y, r, false)
                    .line_to(w / 2.0 + w * 0.08, y);
                canvas.stroke_path(&boom, band_w * 0.85, color);
                canvas.fill_circle(Point::new(w / 2.0 + w * 0.14, y), band_w * 0.75, color);
            }
        }
        DeviceType::Display => {
            let screen_h = w * proportions::SCREEN;
            let total = screen_h + w * proportions::STAND;
            let top = cy - total / 2.0;
            let screen = rounded(Path::new(), Rect::new(0.0, top, w, screen_h), w * 0.1);
            canvas.fill_path(&screen, color);
            let neck = w * 0.13;
            let foot_h = w * 0.11;
            let neck_h = total - screen_h - foot_h + 0.2;
            canvas.fill_rect(
                Rect::new(w / 2.0 - neck / 2.0, top + screen_h - 0.1, neck, neck_h),
                color,
            );
            let foot_w = w * 0.55;
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
