//! The device icon catalogue (FR-067, FR-068).
//!
//! Every icon is drawn by the design system on a 16-DIP design grid, in the style of the
//! Windows 11 Fluent "regular" icons and of the tray badge drawings: 1-DIP strokes with
//! round caps and joins, small filled details (ear cups, capsules), natural proportions,
//! strokes on half-DIP coordinates so they are crisp at 100 %. An icon is painted at any
//! size by scaling that grid uniformly (never stretched).

use crate::backend::Canvas;
use crate::path::{Path, Segment};
use crate::{Color, Point, Rect, Size};

/// Size of the design grid, in DIPs.
pub const DESIGN_SIZE: f32 = 16.0;
/// Stroke width on the design grid.
const STROKE: f32 = 1.0;

/// One icon of the catalogue. The order is the order shown to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceIcon {
    /// A pair of desktop speakers.
    DesktopSpeakers,
    /// A bookshelf speaker on its shelf.
    BookshelfSpeakers,
    /// A soundbar.
    Soundbar,
    /// An AV receiver or amplifier.
    AvReceiver,
    /// A subwoofer.
    Subwoofer,
    /// Portable (often Bluetooth) speaker.
    PortableSpeaker,
    /// Wired over-ear headphones.
    WiredHeadphones,
    /// Bluetooth headphones.
    BluetoothHeadphones,
    /// A gaming headset (suspension headband, microphone boom).
    GamingHeadset,
    /// A headset with a microphone.
    Headset,
    /// Wired earbuds.
    WiredEarbuds,
    /// True-wireless earbuds.
    WirelessEarbuds,
    /// Bone-conduction headphones.
    BoneConduction,
    /// A computer monitor.
    Monitor,
    /// A television.
    Tv,
    /// A video projector.
    Projector,
    /// A laptop.
    Laptop,
    /// A phone.
    Phone,
    /// A virtual-reality headset.
    VrHeadset,
    /// A desk (gooseneck) microphone.
    DeskMicrophone,
    /// A studio microphone.
    StudioMicrophone,
    /// A lavalier (clip-on) microphone.
    LavalierMicrophone,
    /// A webcam.
    Webcam,
    /// An audio interface.
    AudioInterface,
    /// A virtual (software) device.
    VirtualDevice,
    /// Any output device.
    GenericOutput,
    /// Any input device.
    GenericInput,
}

impl DeviceIcon {
    /// Every icon, in catalogue order.
    pub const ALL: [DeviceIcon; 27] = [
        DeviceIcon::DesktopSpeakers,
        DeviceIcon::BookshelfSpeakers,
        DeviceIcon::Soundbar,
        DeviceIcon::AvReceiver,
        DeviceIcon::Subwoofer,
        DeviceIcon::PortableSpeaker,
        DeviceIcon::WiredHeadphones,
        DeviceIcon::BluetoothHeadphones,
        DeviceIcon::GamingHeadset,
        DeviceIcon::Headset,
        DeviceIcon::WiredEarbuds,
        DeviceIcon::WirelessEarbuds,
        DeviceIcon::BoneConduction,
        DeviceIcon::Monitor,
        DeviceIcon::Tv,
        DeviceIcon::Projector,
        DeviceIcon::Laptop,
        DeviceIcon::Phone,
        DeviceIcon::VrHeadset,
        DeviceIcon::DeskMicrophone,
        DeviceIcon::StudioMicrophone,
        DeviceIcon::LavalierMicrophone,
        DeviceIcon::Webcam,
        DeviceIcon::AudioInterface,
        DeviceIcon::VirtualDevice,
        DeviceIcon::GenericOutput,
        DeviceIcon::GenericInput,
    ];

    /// Stable identifier, stored in the settings file (never translated, never changed).
    pub fn id(self) -> &'static str {
        match self {
            DeviceIcon::DesktopSpeakers => "desktop-speakers",
            DeviceIcon::BookshelfSpeakers => "bookshelf-speakers",
            DeviceIcon::Soundbar => "soundbar",
            DeviceIcon::AvReceiver => "av-receiver",
            DeviceIcon::Subwoofer => "subwoofer",
            DeviceIcon::PortableSpeaker => "portable-speaker",
            DeviceIcon::WiredHeadphones => "wired-headphones",
            DeviceIcon::BluetoothHeadphones => "bluetooth-headphones",
            DeviceIcon::GamingHeadset => "gaming-headset",
            DeviceIcon::Headset => "headset",
            DeviceIcon::WiredEarbuds => "wired-earbuds",
            DeviceIcon::WirelessEarbuds => "wireless-earbuds",
            DeviceIcon::BoneConduction => "bone-conduction",
            DeviceIcon::Monitor => "monitor",
            DeviceIcon::Tv => "tv",
            DeviceIcon::Projector => "projector",
            DeviceIcon::Laptop => "laptop",
            DeviceIcon::Phone => "phone",
            DeviceIcon::VrHeadset => "vr-headset",
            DeviceIcon::DeskMicrophone => "desk-microphone",
            DeviceIcon::StudioMicrophone => "studio-microphone",
            DeviceIcon::LavalierMicrophone => "lavalier-microphone",
            DeviceIcon::Webcam => "webcam",
            DeviceIcon::AudioInterface => "audio-interface",
            DeviceIcon::VirtualDevice => "virtual-device",
            DeviceIcon::GenericOutput => "generic-output",
            DeviceIcon::GenericInput => "generic-input",
        }
    }

    /// The icon with this identifier, if any.
    pub fn from_id(id: &str) -> Option<DeviceIcon> {
        DeviceIcon::ALL.into_iter().find(|icon| icon.id() == id)
    }
}

/// Paints `icon` as a square of `size` DIPs whose top-left corner is `origin`.
pub fn paint(canvas: &mut Canvas<'_>, icon: DeviceIcon, origin: Point, size: f32, color: Color) {
    let mut pen = Pen { canvas, origin, scale: size / DESIGN_SIZE, color };
    match icon {
        DeviceIcon::DesktopSpeakers => desktop_speakers(&mut pen),
        DeviceIcon::BookshelfSpeakers => bookshelf_speakers(&mut pen),
        DeviceIcon::Soundbar => soundbar(&mut pen),
        DeviceIcon::AvReceiver => av_receiver(&mut pen),
        DeviceIcon::Subwoofer => subwoofer(&mut pen),
        DeviceIcon::PortableSpeaker => portable_speaker(&mut pen),
        DeviceIcon::WiredHeadphones => headphones(&mut pen, Phones::Wired),
        DeviceIcon::BluetoothHeadphones => headphones(&mut pen, Phones::Bluetooth),
        DeviceIcon::GamingHeadset => headphones(&mut pen, Phones::Gaming),
        DeviceIcon::Headset => headphones(&mut pen, Phones::Headset),
        DeviceIcon::WiredEarbuds => wired_earbuds(&mut pen),
        DeviceIcon::WirelessEarbuds => wireless_earbuds(&mut pen),
        DeviceIcon::BoneConduction => bone_conduction(&mut pen),
        DeviceIcon::Monitor => monitor(&mut pen),
        DeviceIcon::Tv => tv(&mut pen),
        DeviceIcon::Projector => projector(&mut pen),
        DeviceIcon::Laptop => laptop(&mut pen),
        DeviceIcon::Phone => phone(&mut pen),
        DeviceIcon::VrHeadset => vr_headset(&mut pen),
        DeviceIcon::DeskMicrophone => desk_microphone(&mut pen),
        DeviceIcon::StudioMicrophone => studio_microphone(&mut pen),
        DeviceIcon::LavalierMicrophone => lavalier_microphone(&mut pen),
        DeviceIcon::Webcam => webcam(&mut pen),
        DeviceIcon::AudioInterface => audio_interface(&mut pen),
        DeviceIcon::VirtualDevice => virtual_device(&mut pen),
        DeviceIcon::GenericOutput => generic_output(&mut pen),
        DeviceIcon::GenericInput => generic_input(&mut pen),
    }
}

/// Draws on the design grid: maps its coordinates to the canvas.
struct Pen<'c, 'a> {
    canvas: &'c mut Canvas<'a>,
    origin: Point,
    scale: f32,
    color: Color,
}

impl Pen<'_, '_> {
    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(self.origin.x + x * self.scale, self.origin.y + y * self.scale)
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        let p = self.point(x, y);
        Rect::new(p.x, p.y, w * self.scale, h * self.scale)
    }

    /// Outline of a rounded rectangle given by the centre line of its stroke.
    fn frame(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let (r, s, c) = (self.rect(x, y, w, h), self.scale, self.color);
        self.canvas.stroke_rounded(r, radius * s, STROKE * s, c);
    }

    /// Filled rounded rectangle.
    fn block(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let (r, s, c) = (self.rect(x, y, w, h), self.scale, self.color);
        self.canvas.fill_rounded(r, radius * s, c);
    }

    fn ring(&mut self, cx: f32, cy: f32, radius: f32) {
        let (p, s, c) = (self.point(cx, cy), self.scale, self.color);
        self.canvas.stroke_circle(p, radius * s, STROKE * s, c);
    }

    fn dot(&mut self, cx: f32, cy: f32, radius: f32) {
        let (p, s, c) = (self.point(cx, cy), self.scale, self.color);
        self.canvas.fill_circle(p, radius * s, c);
    }

    fn map(&self, path: &Path) -> Path {
        let segments = path
            .segments
            .iter()
            .map(|segment| match *segment {
                Segment::MoveTo(p) => Segment::MoveTo(self.point(p.x, p.y)),
                Segment::LineTo(p) => Segment::LineTo(self.point(p.x, p.y)),
                Segment::ArcTo { to, radii, clockwise, large } => Segment::ArcTo {
                    to: self.point(to.x, to.y),
                    radii: Size::new(radii.width * self.scale, radii.height * self.scale),
                    clockwise,
                    large,
                },
                Segment::CubicTo { c1, c2, to } => Segment::CubicTo {
                    c1: self.point(c1.x, c1.y),
                    c2: self.point(c2.x, c2.y),
                    to: self.point(to.x, to.y),
                },
                Segment::Close => Segment::Close,
            })
            .collect();
        Path { segments }
    }

    fn stroke(&mut self, path: Path) {
        let (mapped, s, c) = (self.map(&path), self.scale, self.color);
        self.canvas.stroke_path(&mapped, STROKE * s, c);
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32) {
        self.stroke(Path::new().move_to(x1, y1).line_to(x2, y2));
    }
}

// --- Speakers ------------------------------------------------------------------------

/// A speaker cabinet (outline) with its tweeter (dot) and woofer (ring).
fn cabinet(pen: &mut Pen<'_, '_>, x: f32, y: f32, w: f32, h: f32, woofer: f32) {
    pen.frame(x, y, w, h, 1.5);
    let mid = x + w / 2.0;
    pen.dot(mid, y + 2.5, 0.85);
    pen.ring(mid, y + h - woofer - 1.75, woofer);
}

fn desktop_speakers(pen: &mut Pen<'_, '_>) {
    // A pair of narrow cabinets; at this size the woofers are filled.
    for x in [1.5, 9.5] {
        pen.frame(x, 3.5, 5.0, 10.0, 1.5);
        pen.dot(x + 2.5, 6.0, 0.75);
        pen.dot(x + 2.5, 10.25, 1.5);
    }
}

fn bookshelf_speakers(pen: &mut Pen<'_, '_>) {
    // One cabinet standing on a shelf.
    cabinet(pen, 3.5, 1.5, 9.0, 11.0, 2.25);
    pen.line(1.5, 14.5, 14.5, 14.5);
}

fn soundbar(pen: &mut Pen<'_, '_>) {
    // A long, low bar with its row of drivers.
    pen.frame(1.5, 5.5, 13.0, 5.0, 2.5);
    for x in [3.5, 6.5, 9.5, 12.5] {
        pen.dot(x, 8.0, 0.85);
    }
}

fn av_receiver(pen: &mut Pen<'_, '_>) {
    // A wide box: display and buttons on the left, the volume knob on the right, feet.
    pen.frame(1.5, 3.5, 13.0, 8.0, 1.5);
    pen.line(4.0, 6.5, 7.0, 6.5);
    pen.dot(4.25, 9.0, 0.75);
    pen.dot(6.75, 9.0, 0.75);
    pen.ring(10.5, 7.5, 2.0);
    pen.line(3.5, 11.5, 3.5, 13.0);
    pen.line(12.5, 11.5, 12.5, 13.0);
}

fn subwoofer(pen: &mut Pen<'_, '_>) {
    // A cube with one big driver.
    pen.frame(1.5, 1.5, 13.0, 13.0, 2.0);
    pen.ring(8.0, 8.0, 4.0);
    pen.dot(8.0, 8.0, 1.25);
}

fn portable_speaker(pen: &mut Pen<'_, '_>) {
    // A capsule lying down (JBL Flip, Charge...) with its grille: two rows of holes.
    // Taller than the soundbar, whose drivers make a single row.
    pen.frame(1.5, 4.0, 13.0, 8.0, 4.0);
    for y in [6.75, 9.25] {
        for x in [5.0, 7.0, 9.0, 11.0] {
            pen.dot(x, y, 0.7);
        }
    }
}

// --- Headphones ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phones {
    Wired,
    Bluetooth,
    Gaming,
    Headset,
}

fn headphones(pen: &mut Pen<'_, '_>, kind: Phones) {
    // Centre of the headband arc. The variants with something below the cups sit higher.
    let cy = match kind {
        Phones::Wired | Phones::Bluetooth => 9.0,
        _ => 8.0,
    };
    let radius = 5.5;
    // Both halves are drawn from the top outwards, so they rasterise as mirror images.
    let top = cy - radius;
    pen.stroke(
        Path::new()
            .move_to(8.0, top)
            .arc_to(2.5, cy, radius, false)
            .line_to(2.5, cy + 0.5)
            .move_to(8.0, top)
            .arc_to(13.5, cy, radius, true)
            .line_to(13.5, cy + 0.5),
    );
    if kind == Phones::Gaming {
        // Suspension band under the headband.
        let inner = cy - 3.5;
        pen.stroke(
            Path::new()
                .move_to(8.0, inner)
                .arc_to(4.5, cy, 3.5, false)
                .move_to(8.0, inner)
                .arc_to(11.5, cy, 3.5, true),
        );
    }
    // Ear cups.
    // Ear cups (shorter when a microphone boom must fit below them).
    let boom = matches!(kind, Phones::Gaming | Phones::Headset);
    let (cup_top, bottom) = (cy - 1.0, if boom { cy + 4.0 } else { cy + 5.0 });
    pen.block(1.0, cup_top, 3.0, bottom - cup_top, 1.0);
    pen.block(12.0, cup_top, 3.0, bottom - cup_top, 1.0);
    match kind {
        // Plain headphones: the wired ones (Fluent draws headphones this way).
        Phones::Wired => {}
        Phones::Bluetooth => bluetooth_rune(pen, 8.0, cy + 1.5, 6.0),
        Phones::Gaming | Phones::Headset => {
            // Microphone boom from under the left cup to the mouth, ending in a capsule.
            let y = bottom + 1.5;
            pen.stroke(Path::new().move_to(2.5, bottom).arc_to(4.0, y, 1.5, false).line_to(7.0, y));
            pen.dot(7.75, y, 1.25);
        }
    }
}

/// The Bluetooth rune, `height` DIPs tall, centred on (`cx`, `cy`).
fn bluetooth_rune(pen: &mut Pen<'_, '_>, cx: f32, cy: f32, height: f32) {
    let (top, bottom) = (cy - height / 2.0, cy + height / 2.0);
    let q = height / 4.0;
    let w = height * 0.27;
    pen.stroke(
        Path::new()
            .move_to(cx - w, top + q)
            .line_to(cx + w, bottom - q)
            .line_to(cx, bottom)
            .line_to(cx, top)
            .line_to(cx + w, top + q)
            .line_to(cx - w, bottom - q),
    );
}

fn wired_earbuds(pen: &mut Pen<'_, '_>) {
    // Two buds whose cables join into one.
    pen.ring(4.0, 4.0, 2.5);
    pen.ring(12.0, 4.0, 2.5);
    pen.stroke(
        Path::new()
            .move_to(4.0, 6.5)
            .line_to(4.0, 7.0)
            .arc_to(8.0, 11.0, 4.0, false)
            .arc_to(12.0, 7.0, 4.0, false)
            .line_to(12.0, 6.5)
            .move_to(8.0, 11.0)
            .line_to(8.0, 14.5),
    );
}

fn wireless_earbuds(pen: &mut Pen<'_, '_>) {
    // Two buds, their stems hanging from the inner side of the heads, no cable.
    for (cx, stem) in [(4.0, 4.5), (12.0, 9.5)] {
        pen.ring(cx, 4.5, 2.5);
        pen.block(stem, 6.0, 2.0, 8.5, 1.0);
    }
}

fn bone_conduction(pen: &mut Pen<'_, '_>) {
    // Two pads resting in front of the ears, joined by a band around the back of the
    // head; vibration marks next to the pads.
    pen.block(1.0, 2.0, 3.0, 5.0, 1.25);
    pen.block(12.0, 2.0, 3.0, 5.0, 1.25);
    pen.stroke(
        Path::new()
            .move_to(2.5, 7.0)
            .line_to(2.5, 8.5)
            .arc_to(13.5, 8.5, 5.5, false)
            .line_to(13.5, 7.0),
    );
    pen.stroke(Path::new().move_to(5.5, 3.0).arc_to(5.5, 6.0, 2.0, true));
    pen.stroke(Path::new().move_to(10.5, 6.0).arc_to(10.5, 3.0, 2.0, true));
}

// --- Screens and computers ------------------------------------------------------------

fn monitor(pen: &mut Pen<'_, '_>) {
    // A 16:10 screen on a stand (on the pixel column next to the middle, to stay crisp).
    pen.frame(1.5, 2.5, 13.0, 8.0, 1.25);
    pen.line(7.5, 10.5, 7.5, 13.5);
    pen.line(4.5, 13.5, 10.5, 13.5);
}

fn tv(pen: &mut Pen<'_, '_>) {
    // A wide screen on two feet.
    pen.frame(1.5, 3.5, 13.0, 7.0, 1.25);
    pen.line(4.0, 10.5, 3.0, 12.5);
    pen.line(12.0, 10.5, 13.0, 12.5);
}

fn projector(pen: &mut Pen<'_, '_>) {
    // A box with its lens on the left, vents on the right, feet, the beam above the lens.
    pen.frame(1.5, 6.5, 13.0, 6.0, 1.5);
    pen.ring(5.5, 9.5, 2.0);
    pen.dot(5.5, 9.5, 0.75);
    pen.line(10.0, 8.5, 12.0, 8.5);
    pen.line(10.0, 10.5, 12.0, 10.5);
    pen.line(3.75, 4.25, 2.75, 3.0);
    pen.line(5.5, 4.5, 5.5, 2.5);
    pen.line(7.25, 4.25, 8.25, 3.0);
}

fn laptop(pen: &mut Pen<'_, '_>) {
    // The screen above a wider base.
    pen.frame(2.5, 3.5, 11.0, 8.0, 1.25);
    pen.line(1.0, 13.5, 15.0, 13.5);
}

fn phone(pen: &mut Pen<'_, '_>) {
    pen.frame(4.5, 1.5, 7.0, 13.0, 2.0);
    pen.line(7.0, 12.5, 9.0, 12.5);
}

fn vr_headset(pen: &mut Pen<'_, '_>) {
    // Goggles with the nose cut-out.
    let (top, bottom, r) = (4.5, 11.5, 2.5);
    pen.stroke(
        Path::new()
            .move_to(4.0, top)
            .line_to(12.0, top)
            .arc_to(14.5, top + r, r, true)
            .line_to(14.5, bottom - r)
            .arc_to(12.0, bottom, r, true)
            .line_to(10.5, bottom)
            .arc_to(5.5, bottom, 2.5, false)
            .line_to(4.0, bottom)
            .arc_to(1.5, bottom - r, r, true)
            .line_to(1.5, top + r)
            .arc_to(4.0, top, r, true)
            .close(),
    );
}

// --- Microphones and other devices -------------------------------------------------------

fn desk_microphone(pen: &mut Pen<'_, '_>) {
    // A large capsule with its grille, on a stand with a flat base (USB microphones).
    pen.frame(4.5, 1.5, 7.0, 8.5, 3.5);
    pen.line(6.0, 5.5, 10.0, 5.5);
    pen.line(6.0, 7.5, 10.0, 7.5);
    pen.line(8.0, 10.0, 8.0, 12.5);
    pen.frame(4.5, 12.5, 7.0, 2.0, 1.0);
}

fn studio_microphone(pen: &mut Pen<'_, '_>) {
    // A capsule tilted towards the speaker, held by a boom arm (broadcast microphone).
    let (a, b, half) = (Point::new(4.75, 4.25), Point::new(8.75, 8.25), 2.25);
    let n = half * std::f32::consts::FRAC_1_SQRT_2;
    pen.stroke(
        Path::new()
            .move_to(a.x + n, a.y - n)
            .line_to(b.x + n, b.y - n)
            .arc_to(b.x - n, b.y + n, half, true)
            .line_to(a.x - n, a.y + n)
            .arc_to(a.x + n, a.y - n, half, true)
            .close(),
    );
    let m = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    pen.line(m.x + n, m.y - n, m.x - n, m.y + n);
    pen.stroke(Path::new().move_to(b.x + n, b.y + n).line_to(12.5, 12.0).line_to(12.5, 14.5));
    pen.line(10.5, 14.5, 14.5, 14.5);
}

fn lavalier_microphone(pen: &mut Pen<'_, '_>) {
    // A tiny capsule with its clip, the cable hanging below.
    pen.block(4.5, 1.5, 4.0, 6.0, 2.0);
    pen.frame(9.5, 2.5, 2.0, 4.0, 1.0);
    pen.stroke(
        Path::new()
            .move_to(6.5, 7.5)
            .line_to(6.5, 9.0)
            .arc_to(9.0, 11.5, 2.5, false)
            .arc_to(11.5, 14.0, 2.5, true)
            .line_to(11.5, 14.5),
    );
}

fn webcam(pen: &mut Pen<'_, '_>) {
    // A round camera with its lens, on a stand.
    pen.ring(8.0, 6.5, 5.0);
    pen.ring(8.0, 6.5, 2.0);
    pen.line(8.0, 11.5, 8.0, 14.5);
    pen.line(5.0, 14.5, 11.0, 14.5);
}

fn audio_interface(pen: &mut Pen<'_, '_>) {
    // A box with its two input sockets.
    pen.frame(1.5, 4.5, 13.0, 7.0, 1.5);
    for cx in [5.0, 11.0] {
        pen.ring(cx, 8.0, 1.75);
        pen.dot(cx, 8.0, 0.6);
    }
}

fn virtual_device(pen: &mut Pen<'_, '_>) {
    // A sound wave in a frame of corner marks.
    let (a, b, l) = (1.5, 14.5, 3.0);
    for (x, dx) in [(a, 1.0), (b, -1.0)] {
        for (y, dy) in [(a, 1.0), (b, -1.0)] {
            pen.stroke(Path::new().move_to(x, y + dy * l).line_to(x, y).line_to(x + dx * l, y));
        }
    }
    pen.stroke(
        Path::new().move_to(4.5, 8.0).arc_to(8.0, 8.0, 1.75, true).arc_to(11.5, 8.0, 1.75, false),
    );
}

fn generic_output(pen: &mut Pen<'_, '_>) {
    // The Windows speaker: box and cone, two waves.
    pen.stroke(
        Path::new()
            .move_to(1.5, 5.5)
            .line_to(4.5, 5.5)
            .line_to(8.5, 2.0)
            .line_to(8.5, 14.0)
            .line_to(4.5, 10.5)
            .line_to(1.5, 10.5)
            .close(),
    );
    let arc = |r: f32| {
        let (dx, dy) = (r * 0.64, r * 0.77);
        Path::new().move_to(8.5 + dx, 8.0 - dy).arc_to(8.5 + dx, 8.0 + dy, r, true)
    };
    pen.stroke(arc(3.0));
    pen.stroke(arc(5.5));
}

fn generic_input(pen: &mut Pen<'_, '_>) {
    // The Windows microphone: capsule, holder, stem and foot.
    pen.frame(5.5, 1.5, 5.0, 7.0, 2.5);
    pen.stroke(
        Path::new()
            .move_to(3.5, 6.0)
            .line_to(3.5, 7.0)
            .arc_to(12.5, 7.0, 4.5, false)
            .line_to(12.5, 6.0),
    );
    pen.line(8.0, 11.5, 8.0, 14.5);
    pen.line(5.5, 14.5, 10.5, 14.5);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{DeviceKind, Graphics};

    #[test]
    fn the_catalogue_is_complete_and_ids_are_unique_and_stable() {
        assert_eq!(DeviceIcon::ALL.len(), 27);
        let mut ids: Vec<&str> = DeviceIcon::ALL.iter().map(|icon| icon.id()).collect();
        for icon in DeviceIcon::ALL {
            assert_eq!(DeviceIcon::from_id(icon.id()), Some(icon));
            assert!(icon.id().bytes().all(|b| b.is_ascii_lowercase() || b == b'-'));
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 27);
        assert_eq!(DeviceIcon::from_id("nothing"), None);
    }

    #[test]
    fn every_icon_draws_inside_its_box_at_every_size() {
        let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
        for px in [16u32, 24, 36] {
            // The icon box sits in the middle of a canvas twice its size: anything drawn
            // outside the box is visible in the margin.
            let canvas = px * 2;
            let offset = (px / 2) as f32 * DESIGN_SIZE / px as f32;
            let scale = px as f32 / DESIGN_SIZE;
            for icon in DeviceIcon::ALL {
                let image = gfx
                    .rasterize(canvas, canvas, scale, 1.0, |c| {
                        c.clear(Color::TRANSPARENT);
                        paint(c, icon, Point::new(offset, offset), DESIGN_SIZE, Color::WHITE);
                    })
                    .expect("rasterize");
                let (lo, hi) = (px / 2, px / 2 + px);
                let mut inside = 0;
                for y in 0..canvas {
                    for x in 0..canvas {
                        let alpha = image.pixel(x, y)[3];
                        let in_box = (lo..hi).contains(&x) && (lo..hi).contains(&y);
                        if in_box {
                            inside += u32::from(alpha > 128);
                        } else {
                            assert_eq!(alpha, 0, "{icon:?} at {px} px draws outside at ({x}, {y})");
                        }
                    }
                }
                assert!(inside > px, "{icon:?} at {px} px draws almost nothing");
            }
        }
    }

    #[test]
    fn icons_are_all_different() {
        let gfx = Graphics::new(DeviceKind::Software).expect("graphics");
        let images: Vec<_> = DeviceIcon::ALL
            .iter()
            .map(|&icon| {
                gfx.rasterize(16, 16, 1.0, 1.0, |c| {
                    c.clear(Color::TRANSPARENT);
                    paint(c, icon, Point::new(0.0, 0.0), DESIGN_SIZE, Color::WHITE);
                })
                .expect("rasterize")
            })
            .collect();
        for (i, a) in images.iter().enumerate() {
            for (j, b) in images.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a,
                    b,
                    "{:?} and {:?} look the same",
                    DeviceIcon::ALL[i],
                    DeviceIcon::ALL[j]
                );
            }
        }
    }
}
