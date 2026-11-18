//! Vector paths, for icons drawn by the design system itself.

use crate::{Point, Size};

/// One step of a path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    /// Starts a new figure at a point.
    MoveTo(Point),
    /// Straight line to a point.
    LineTo(Point),
    /// Elliptic arc to a point.
    ArcTo {
        /// End point.
        to: Point,
        /// Radii of the ellipse.
        radii: Size,
        /// Clockwise (on screen, y down) rather than counter-clockwise.
        clockwise: bool,
        /// The longer of the two possible arcs.
        large: bool,
    },
    /// Closes the current figure back to its start.
    Close,
}

/// A path made of figures, in DIPs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// The steps, in order.
    pub segments: Vec<Segment>,
}

impl Path {
    /// An empty path.
    pub fn new() -> Path {
        Path::default()
    }

    /// Starts a figure.
    pub fn move_to(mut self, x: f32, y: f32) -> Path {
        self.segments.push(Segment::MoveTo(Point::new(x, y)));
        self
    }

    /// Adds a line.
    pub fn line_to(mut self, x: f32, y: f32) -> Path {
        self.segments.push(Segment::LineTo(Point::new(x, y)));
        self
    }

    /// Adds a circular arc of radius `r`.
    pub fn arc_to(mut self, x: f32, y: f32, r: f32, clockwise: bool) -> Path {
        self.segments.push(Segment::ArcTo {
            to: Point::new(x, y),
            radii: Size::new(r, r),
            clockwise,
            large: false,
        });
        self
    }

    /// Adds an elliptic arc.
    pub fn ellipse_to(mut self, x: f32, y: f32, radii: Size, clockwise: bool) -> Path {
        self.segments.push(Segment::ArcTo { to: Point::new(x, y), radii, clockwise, large: false });
        self
    }

    /// Closes the figure.
    pub fn close(mut self) -> Path {
        self.segments.push(Segment::Close);
        self
    }
}
