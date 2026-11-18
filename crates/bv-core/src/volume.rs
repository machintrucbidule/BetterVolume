//! Volume maths (FR-015, FR-051, FR-073).
//!
//! Windows stores a volume as a scalar from 0.0 to 1.0; the application shows and accepts
//! whole percents from 0 to 100.

use crate::settings::{WHEEL_STEP_MAX, WHEEL_STEP_MIN};

/// Highest volume, in percent.
pub const MAX_PERCENT: u8 = 100;

/// The whole percent shown for a Windows volume scalar (rounded to the nearest; anything
/// out of range or not a number is brought back to 0..=100).
pub fn to_percent(scalar: f32) -> u8 {
    if scalar.is_nan() {
        return 0;
    }
    // In range after the clamp, so the conversion cannot truncate.
    (scalar.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// The Windows volume scalar for a percent (values above 100 count as 100).
pub fn to_scalar(percent: u8) -> f32 {
    f32::from(percent.min(MAX_PERCENT)) / 100.0
}

/// The volume after `notches` wheel notches (positive = up, negative = down) of `step`
/// percent each, kept within 0..=100. `step` is brought back to the allowed wheel-step
/// range first (FR-073), so a damaged setting can never freeze or jump the volume.
pub fn step(percent: u8, notches: i32, step: u8) -> u8 {
    let step = i64::from(step.clamp(WHEEL_STEP_MIN, WHEEL_STEP_MAX));
    let current = i64::from(percent.min(MAX_PERCENT));
    let target = current + i64::from(notches) * step;
    // In range after the clamp, so the conversion cannot truncate.
    target.clamp(0, i64::from(MAX_PERCENT)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_round_to_the_nearest_percent() {
        assert_eq!(to_percent(0.0), 0);
        assert_eq!(to_percent(1.0), 100);
        assert_eq!(to_percent(0.5), 50);
        assert_eq!(to_percent(0.004), 0);
        assert_eq!(to_percent(0.006), 1);
        assert_eq!(to_percent(0.994), 99);
        assert_eq!(to_percent(0.996), 100);
    }

    #[test]
    fn out_of_range_scalars_are_clamped() {
        assert_eq!(to_percent(-0.5), 0);
        assert_eq!(to_percent(1.5), 100);
        assert_eq!(to_percent(f32::NAN), 0);
        assert_eq!(to_percent(f32::INFINITY), 100);
        assert_eq!(to_percent(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn percents_convert_back_exactly() {
        for percent in 0..=100u8 {
            assert_eq!(to_percent(to_scalar(percent)), percent);
        }
        assert_eq!(to_scalar(0), 0.0);
        assert_eq!(to_scalar(100), 1.0);
        assert_eq!(to_scalar(250), 1.0);
    }

    #[test]
    fn wheel_moves_by_the_step() {
        assert_eq!(step(50, 1, 2), 52);
        assert_eq!(step(50, -1, 2), 48);
        assert_eq!(step(50, 3, 2), 56);
        assert_eq!(step(51, 1, 2), 53);
        assert_eq!(step(50, 0, 2), 50);
        assert_eq!(step(50, 1, 10), 60);
    }

    #[test]
    fn wheel_stops_at_the_ends() {
        assert_eq!(step(99, 1, 2), 100);
        assert_eq!(step(100, 1, 2), 100);
        assert_eq!(step(1, -1, 2), 0);
        assert_eq!(step(0, -1, 2), 0);
        assert_eq!(step(50, i32::MAX, 10), 100);
        assert_eq!(step(50, i32::MIN, 10), 0);
    }

    #[test]
    fn wheel_step_is_brought_back_into_range() {
        assert_eq!(step(50, 1, 0), 51);
        assert_eq!(step(50, 1, 200), 60);
        assert_eq!(step(200, -1, 2), 98);
    }
}
