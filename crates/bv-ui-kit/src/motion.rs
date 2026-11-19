//! Motion tokens: durations and easing curves of the Windows 11 transitions.

/// A CSS-style cubic Bézier easing curve from (0, 0) to (1, 1) with two control points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    /// First control point, horizontal (time) coordinate, within `0.0..=1.0`.
    pub x1: f32,
    /// First control point, vertical (progress) coordinate.
    pub y1: f32,
    /// Second control point, horizontal coordinate, within `0.0..=1.0`.
    pub x2: f32,
    /// Second control point, vertical coordinate.
    pub y2: f32,
}

impl CubicBezier {
    /// Windows "decelerate" (fast start, gentle stop): entrances.
    pub const DECELERATE: CubicBezier = CubicBezier { x1: 0.0, y1: 0.0, x2: 0.0, y2: 1.0 };
    /// Windows "accelerate" (gentle start, fast end): exits.
    pub const ACCELERATE: CubicBezier = CubicBezier { x1: 1.0, y1: 0.0, x2: 1.0, y2: 1.0 };
    /// Constant speed.
    pub const LINEAR: CubicBezier = CubicBezier { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };

    /// Progress (usually 0 to 1) at time `t` (0 to 1, clamped).
    pub fn ease(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        // Solve x(s) = t for the curve parameter s by bisection (x is monotonic because
        // both control abscissas lie within 0..=1), then evaluate y(s).
        let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
        let mut s = t;
        for _ in 0..32 {
            let x = bezier(self.x1, self.x2, s);
            if (x - t).abs() < 1e-6 {
                break;
            }
            if x < t {
                lo = s;
            } else {
                hi = s;
            }
            s = 0.5 * (lo + hi);
        }
        bezier(self.y1, self.y2, s)
    }
}

/// One coordinate of a cubic Bézier from 0 to 1 with control values `a` and `b`.
fn bezier(a: f32, b: f32, s: f32) -> f32 {
    let u = 1.0 - s;
    3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
}

/// A transition: how long it lasts, how it eases, and how far it moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// Duration in milliseconds.
    pub duration_ms: u32,
    /// Easing curve.
    pub easing: CubicBezier,
}

impl Transition {
    /// Progress (0 to 1) after `elapsed_ms`; 1 once the duration has passed. A transition
    /// of zero duration is complete at once.
    pub fn progress(&self, elapsed_ms: f32) -> f32 {
        if self.duration_ms == 0 {
            return 1.0;
        }
        self.easing.ease(elapsed_ms / self.duration_ms as f32)
    }
}

/// Flyout entrance: rises from behind the taskbar edge (system flyout motion).
pub const FLYOUT_OPEN: Transition =
    Transition { duration_ms: 250, easing: CubicBezier::DECELERATE };
/// Flyout exit: sinks back behind the taskbar edge.
pub const FLYOUT_CLOSE: Transition =
    Transition { duration_ms: 167, easing: CubicBezier::ACCELERATE };
/// Change of the flyout height while it is open.
pub const FLYOUT_RESIZE: Transition =
    Transition { duration_ms: 250, easing: CubicBezier::DECELERATE };
/// Period of the timer driving a flyout height change, in milliseconds (the compositor
/// shows about one frame every 16 ms).
pub const RESIZE_FRAME_MS: u32 = 10;
/// Time the pointer must rest on a control before its tooltip shows, in milliseconds.
pub const TOOLTIP_DELAY_MS: u32 = 500;
/// Context menu entrance.
pub const MENU_OPEN: Transition = Transition { duration_ms: 200, easing: CubicBezier::DECELERATE };
/// How long an application row that vanished from the flyout is still shown, in
/// milliseconds: after an output switch, an application's sound reaches the new output a
/// moment later, and its row must not disappear and come back.
pub const ROW_REMOVAL_HOLD_MS: u32 = 1000;
/// Bubble entrance (FR-105): rises from the bottom edge of the screen (from behind a bottom
/// taskbar), like the Windows volume indicator.
pub const BUBBLE_OPEN: Transition =
    Transition { duration_ms: 250, easing: CubicBezier::DECELERATE };
/// Bubble exit: sinks back to the bottom edge of the screen.
pub const BUBBLE_CLOSE: Transition =
    Transition { duration_ms: 167, easing: CubicBezier::ACCELERATE };
/// Period of the timer driving the bubble's motion, in milliseconds. The bubble moves
/// without holding the UI thread: wheel turns keep being applied while it rises.
pub const BUBBLE_FRAME_MS: u32 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_start_at_zero_and_end_at_one() {
        for curve in [CubicBezier::DECELERATE, CubicBezier::ACCELERATE, CubicBezier::LINEAR] {
            assert_eq!(curve.ease(0.0), 0.0);
            assert_eq!(curve.ease(1.0), 1.0);
            assert_eq!(curve.ease(-3.0), 0.0);
            assert_eq!(curve.ease(7.0), 1.0);
        }
    }

    #[test]
    fn curves_are_monotonic() {
        for curve in [CubicBezier::DECELERATE, CubicBezier::ACCELERATE] {
            let mut previous = 0.0;
            for i in 1..=100 {
                let y = curve.ease(i as f32 / 100.0);
                assert!(y >= previous - 1e-5, "{curve:?} at {i}");
                previous = y;
            }
        }
    }

    #[test]
    fn decelerate_is_fast_first_and_accelerate_slow_first() {
        assert!(CubicBezier::DECELERATE.ease(0.25) > 0.6);
        assert!(CubicBezier::ACCELERATE.ease(0.25) < 0.1);
        assert!((CubicBezier::LINEAR.ease(0.3) - 0.3).abs() < 1e-4);
    }

    #[test]
    fn progress_follows_the_duration() {
        let t = Transition { duration_ms: 200, easing: CubicBezier::LINEAR };
        assert!((t.progress(100.0) - 0.5).abs() < 1e-4);
        assert_eq!(t.progress(500.0), 1.0);
        let instant = Transition { duration_ms: 0, easing: CubicBezier::LINEAR };
        assert_eq!(instant.progress(0.0), 1.0);
    }
}
