//! Colour value used by every design token.

/// A colour with straight (non-premultiplied) alpha; each component is in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Opacity.
    pub a: f32,
}

impl Color {
    /// Fully transparent.
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    /// Opaque black.
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    /// Opaque white.
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// An opaque colour from a `0xRRGGBB` value.
    pub const fn rgb(hex: u32) -> Color {
        Color {
            r: ((hex >> 16) & 0xFF) as f32 / 255.0,
            g: ((hex >> 8) & 0xFF) as f32 / 255.0,
            b: (hex & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// A colour from a `0xAARRGGBB` value, the notation of the Windows design resources.
    pub const fn argb(hex: u32) -> Color {
        Color { a: ((hex >> 24) & 0xFF) as f32 / 255.0, ..Color::rgb(hex) }
    }

    /// A colour from 8-bit components (opaque).
    pub const fn from_bytes(r: u8, g: u8, b: u8) -> Color {
        Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    /// The same colour with the given opacity (clamped to `0.0..=1.0`).
    pub const fn with_alpha(self, alpha: f32) -> Color {
        Color { a: alpha.clamp(0.0, 1.0), ..self }
    }

    /// Relative luminance (sRGB, WCAG definition), from 0 (black) to 1 (white).
    pub fn luminance(self) -> f32 {
        fn channel(c: f32) -> f32 {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_splits_the_components() {
        let c = Color::rgb(0xFF8000);
        assert_eq!(c.r, 1.0);
        assert!((c.g - 128.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(c.b, 0.0);
        assert_eq!(c.a, 1.0);
        // Anything above 24 bits is ignored.
        assert_eq!(Color::rgb(0xAB00_0000), Color::rgb(0));
    }

    #[test]
    fn argb_reads_the_alpha_byte() {
        let c = Color::argb(0x80FF_FFFF);
        assert!((c.a - 128.0 / 255.0).abs() < f32::EPSILON);
        assert_eq!(c.r, 1.0);
        assert_eq!(Color::argb(0xFF12_3456), Color::rgb(0x123456));
    }

    #[test]
    fn with_alpha_clamps() {
        let c = Color::rgb(0x123456);
        assert_eq!(c.with_alpha(0.5).a, 0.5);
        assert_eq!(c.with_alpha(2.0).a, 1.0);
        assert_eq!(c.with_alpha(-1.0).a, 0.0);
        assert_eq!(c.with_alpha(0.5).r, c.r);
    }

    #[test]
    fn luminance_spans_black_to_white() {
        assert_eq!(Color::BLACK.luminance(), 0.0);
        assert!((Color::WHITE.luminance() - 1.0).abs() < 1e-5);
        assert!(Color::rgb(0x60CDFF).luminance() > Color::rgb(0x005FB8).luminance());
    }
}
