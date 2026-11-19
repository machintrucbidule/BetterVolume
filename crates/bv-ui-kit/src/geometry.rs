//! Geometry in device-independent pixels (DIPs: 1 DIP = 1 physical pixel at 100 % scaling).

/// A point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Point {
    /// A point from its coordinates.
    pub const fn new(x: f32, y: f32) -> Point {
        Point { x, y }
    }
}

/// A size.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Size {
    /// A size from its dimensions.
    pub const fn new(width: f32, height: f32) -> Size {
        Size { width, height }
    }

    /// The size in physical pixels at `scale` (1.0 = 100 %), rounded up so nothing is cut.
    pub fn to_pixels(self, scale: f32) -> (u32, u32) {
        let px = |v: f32| (v * scale).ceil().clamp(1.0, 16_384.0) as u32;
        (px(self.width), px(self.height))
    }
}

/// An axis-aligned rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// A rectangle from its top-left corner and size.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect { x, y, width, height }
    }

    /// The rectangle at the origin with this size.
    pub const fn from_size(size: Size) -> Rect {
        Rect { x: 0.0, y: 0.0, width: size.width, height: size.height }
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Whether the point lies inside (left and top edges included, right and bottom not).
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// The rectangle shrunk by `dx` on the left and right and `dy` on the top and bottom
    /// (never below an empty size).
    pub fn inset(&self, dx: f32, dy: f32) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            width: (self.width - 2.0 * dx).max(0.0),
            height: (self.height - 2.0 * dy).max(0.0),
        }
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: Rect) -> Rect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Rect::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_excludes_the_far_edges() {
        let r = Rect::new(10.0, 10.0, 20.0, 10.0);
        assert!(r.contains(Point::new(10.0, 10.0)));
        assert!(r.contains(Point::new(29.9, 19.9)));
        assert!(!r.contains(Point::new(30.0, 15.0)));
        assert!(!r.contains(Point::new(15.0, 20.0)));
        assert!(!r.contains(Point::new(9.9, 15.0)));
    }

    #[test]
    fn inset_never_goes_negative() {
        let r = Rect::new(0.0, 0.0, 10.0, 4.0).inset(2.0, 3.0);
        assert_eq!(r, Rect::new(2.0, 3.0, 6.0, 0.0));
    }

    #[test]
    fn pixel_sizes_round_up() {
        assert_eq!(Size::new(360.0, 100.0).to_pixels(1.0), (360, 100));
        assert_eq!(Size::new(360.0, 101.0).to_pixels(2.25), (810, 228));
        assert_eq!(Size::new(0.0, 0.0).to_pixels(1.5), (1, 1));
    }
}
