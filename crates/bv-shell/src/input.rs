//! Small pieces of input logic, independent of Windows calls so they can be tested.

/// One notch of a classic mouse wheel (`WHEEL_DELTA`).
pub(crate) const WHEEL_DELTA: i32 = 120;

/// Turns raw wheel deltas into whole notches. Precision wheels and touchpads send
/// fractions of a notch: they add up until a whole notch is reached, in either direction.
#[derive(Debug, Default)]
pub(crate) struct WheelAccumulator {
    rest: i32,
}

impl WheelAccumulator {
    /// Adds a delta (positive = away from the user = volume up) and returns the whole
    /// notches it completes.
    pub fn add(&mut self, delta: i32) -> i32 {
        // A change of direction forgets the unfinished part, like Windows controls do.
        if (delta > 0) != (self.rest > 0) && self.rest != 0 {
            self.rest = 0;
        }
        self.rest += delta;
        let notches = self.rest / WHEEL_DELTA;
        self.rest -= notches * WHEEL_DELTA;
        notches
    }

    /// Forgets the unfinished part (the pointer left the icon).
    pub fn reset(&mut self) {
        self.rest = 0;
    }
}

/// Milliseconds between two message times (`GetMessageTime`), which wrap around.
pub(crate) fn elapsed_ms(from: u32, to: u32) -> u32 {
    to.wrapping_sub(from)
}

/// Windows reports a keyboard activation of a tray icon twice for one press of Enter.
/// Ignores a second one within this delay.
pub(crate) const KEY_SELECT_REPEAT_MS: u32 = 250;

/// A click on the tray icon while the flyout is open first takes the focus away from
/// the flyout (which closes it), then reports the click. A click arriving within this
/// delay after such a closing means "close", not "open again" (FR-013).
pub(crate) const REOPEN_GUARD_MS: u32 = 500;

/// How much earlier than the closing a click may be timed and still be the one that caused
/// it (it waited in the queue while the flyout was busy with its entrance).
pub(crate) const QUEUED_CLICK_MS: u32 = 1000;

/// Decides what a primary action on the tray icon does to the flyout.
#[derive(Debug, Default)]
pub(crate) struct FlyoutToggle {
    /// Message time at which the flyout last closed because it lost the focus.
    closed_by_focus_loss: Option<u32>,
    /// Message time of the last keyboard activation.
    last_key_select: Option<u32>,
}

/// What to do with the flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToggleAction {
    Open,
    Close,
    Nothing,
}

impl FlyoutToggle {
    /// The flyout just closed because it lost the focus, at message time `time`.
    pub fn closed_by_focus_loss(&mut self, time: u32) {
        self.closed_by_focus_loss = Some(time);
    }

    /// The flyout opened (by any route): an older closing stamp no longer applies.
    pub fn opened(&mut self) {
        self.closed_by_focus_loss = None;
    }

    /// A primary action (click or keyboard) at message time `time`; `open` tells whether
    /// the flyout is currently shown.
    pub fn primary(&mut self, time: u32, open: bool, keyboard: bool) -> ToggleAction {
        if keyboard {
            let repeat = self
                .last_key_select
                .is_some_and(|last| elapsed_ms(last, time) < KEY_SELECT_REPEAT_MS);
            self.last_key_select = Some(time);
            if repeat {
                return ToggleAction::Nothing;
            }
        }
        if open {
            return ToggleAction::Close;
        }
        // Signed: a click queued while the flyout was busy (its entrance) carries a time
        // earlier than the closing it caused, which counts as "just closed" too; bounded
        // on both sides, so a stamp left for weeks (or a wrapped counter) never counts.
        let just_closed = self.closed_by_focus_loss.take().is_some_and(|closed| {
            let gap = time.wrapping_sub(closed) as i32;
            (-(QUEUED_CLICK_MS as i32)..REOPEN_GUARD_MS as i32).contains(&gap)
        });
        if just_closed {
            ToggleAction::Nothing
        } else {
            ToggleAction::Open
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_notches_come_out_at_once() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.add(120), 1);
        assert_eq!(wheel.add(-240), -2);
        assert_eq!(wheel.add(360), 3);
    }

    #[test]
    fn fractions_add_up() {
        let mut wheel = WheelAccumulator::default();
        assert_eq!(wheel.add(40), 0);
        assert_eq!(wheel.add(40), 0);
        assert_eq!(wheel.add(50), 1);
        assert_eq!(wheel.add(110), 1, "10 left over plus 110");
        assert_eq!(wheel.add(-30), 0, "direction change forgets the rest");
        assert_eq!(wheel.add(-90), -1);
        wheel.add(100);
        wheel.reset();
        assert_eq!(wheel.add(20), 0);
    }

    #[test]
    fn message_times_wrap_around() {
        assert_eq!(elapsed_ms(u32::MAX - 9, 10), 20);
        assert_eq!(elapsed_ms(100, 350), 250);
    }

    #[test]
    fn click_toggles() {
        let mut toggle = FlyoutToggle::default();
        assert_eq!(toggle.primary(1000, false, false), ToggleAction::Open);
        assert_eq!(toggle.primary(3000, true, false), ToggleAction::Close);
    }

    #[test]
    fn click_that_took_the_focus_away_does_not_reopen() {
        let mut toggle = FlyoutToggle::default();
        toggle.closed_by_focus_loss(10_000);
        assert_eq!(toggle.primary(10_050, false, false), ToggleAction::Nothing);
        // Only once: the next click opens.
        assert_eq!(toggle.primary(10_100, false, false), ToggleAction::Open);
        // Long after a focus loss, a click opens.
        toggle.closed_by_focus_loss(20_000);
        assert_eq!(toggle.primary(21_000, false, false), ToggleAction::Open);
    }

    #[test]
    fn click_queued_during_the_entrance_closes_without_reopening() {
        let mut toggle = FlyoutToggle::default();
        // The click happened at 10 000 but was handled after the focus loss it caused,
        // recorded at 10 180.
        toggle.closed_by_focus_loss(10_180);
        assert_eq!(toggle.primary(10_000, false, false), ToggleAction::Nothing);
        // Across the wrap of the millisecond counter too.
        toggle.closed_by_focus_loss(5);
        assert_eq!(toggle.primary(u32::MAX - 100, false, false), ToggleAction::Nothing);
        // A stamp weeks old (the counter wrapped half-way) never swallows a click.
        toggle.closed_by_focus_loss(0);
        assert_eq!(toggle.primary(0x8000_0010, false, false), ToggleAction::Open);
        // An opening by another route clears the stamp.
        toggle.closed_by_focus_loss(20_000);
        toggle.opened();
        assert_eq!(toggle.primary(20_100, false, false), ToggleAction::Open);
    }

    #[test]
    fn double_keyboard_report_counts_once() {
        let mut toggle = FlyoutToggle::default();
        assert_eq!(toggle.primary(5000, false, true), ToggleAction::Open);
        assert_eq!(toggle.primary(5010, true, true), ToggleAction::Nothing);
        assert_eq!(toggle.primary(6000, true, true), ToggleAction::Close);
        // A mouse click right after a key press is not a repeat.
        assert_eq!(toggle.primary(6010, false, false), ToggleAction::Open);
    }
}
