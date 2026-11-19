//! The bubble above the taskbar (FR-017, FR-105): a passive popup (no
//! focus, clicks pass through) drawn by the design system, bottom centre above the
//! taskbar, hidden about two seconds after its last update.
//!
//! Unlike the flyout and the menu, the bubble moves without holding the UI thread: a
//! timer on its window steps the rise and the sink (positions follow the elapsed time,
//! so the timer's coarse period only affects smoothness, never the duration). Wheel turns
//! keep being applied while it rises, and a new content simply replaces the old one.

use std::rc::Rc;
use std::time::Instant;

use bv_ui_kit::controls::bubble::{self, BubbleContent};
use bv_ui_kit::motion::{Transition, BUBBLE_CLOSE, BUBBLE_FRAME_MS, BUBBLE_OPEN};
use bv_ui_kit::tokens::bubble::{GAP, SHOWN_MS};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    KillTimer, SetTimer, HTTRANSPARENT, WM_NCHITTEST, WM_TIMER,
};

use crate::host::Shell;
use crate::placement::{bubble_origin, hidden_origin, lerp, Edge};
use crate::popup::{Popup, BUBBLE_CLASS};
use crate::screen::Layout;

/// Timer of the bubble window stepping its motion.
const TIMER_FRAME: usize = 21;
/// Timer of the bubble window ending its showing.
const TIMER_HIDE: usize = 22;
/// Duration of one compositor frame at 60 Hz, in milliseconds: each step shows the
/// position due when the compositor next presents (as the flyout's entrance does).
const FRAME_MS: f32 = 1000.0 / 60.0;

/// Where the bubble is going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Motion {
    /// Hidden, or still in its place.
    #[default]
    Still,
    /// Rising from behind the taskbar.
    Rising { started: Instant, from: (i32, i32), to: (i32, i32) },
    /// Sinking behind the taskbar; hidden at the end.
    Sinking { started: Instant, from: (i32, i32), to: (i32, i32) },
}

/// State of the bubble while it is shown.
#[derive(Debug, Default)]
pub(crate) struct BubbleState {
    content: Option<BubbleContent>,
    motion: Motion,
    layout: Option<Layout>,
    /// Size of the window, in physical pixels.
    size: (i32, i32),
}

/// The edge the bubble rises from. Like the Windows volume indicator, it stays at the
/// bottom centre of the screen whatever the taskbar position (measured with a taskbar at
/// the top, left and right), so it always moves as with a bottom taskbar.
const EDGE: Edge = Edge::Bottom;

/// Whether the bubble rises from the bottom edge rather than appearing in place.
fn rises(animations: bool, fullscreen: bool) -> bool {
    animations && !fullscreen
}

impl Shell {
    /// Shows `content` in the bubble, or replaces what it shows (FR-106: no stacking),
    /// and keeps it about two seconds from now. Nothing is shown while the flyout is open
    /// (it shows the change itself).
    pub fn show_bubble(&self, content: BubbleContent) {
        self.present(content, true);
    }

    /// Paints `content` and shows it. `renew`: a new content, kept two more seconds (a
    /// sinking bubble comes back up); otherwise the same content painted again (theme
    /// change), leaving its motion and remaining time alone.
    fn present(&self, content: BubbleContent, renew: bool) {
        if self.quitting.get() || self.flyout_visible() {
            return;
        }
        let Some(popup) = self.bubble_window() else { return };
        let Some(graphics) = self.graphics() else { return };
        let theme = popup.apply_theme(bubble::theme(self.theme()));
        // A shown bubble whose surface was dropped (graphics objects rebuilt) starts over.
        if popup.is_visible() && !popup.has_surface() {
            self.hide_bubble();
        }
        let visible = popup.is_visible();
        // A shown bubble stays on its monitor; a new showing follows the taskbar.
        let layout = match self.bubble_state.borrow().layout {
            Some(layout) if visible => layout,
            _ => crate::screen::taskbar_layout(),
        };
        let scale = layout.scale;
        let size_dip = bubble::size(&content, &graphics, theme.text_scale);
        let size =
            ((size_dip.width * scale).round() as i32, (size_dip.height * scale).round() as i32);
        // Truncated: 14 DIP at 225 % is 31 px, as measured on the Windows indicator.
        let gap = (GAP * scale) as i32;
        let target = bubble_origin(size, &layout.area, gap);
        let paint =
            |c: &mut bv_ui_kit::backend::Canvas<'_>| bubble::paint(c, &theme, &content, size_dip);
        let motion = self.bubble_state.borrow().motion;
        if visible {
            // Painted without window messages, then moved and sized in one step.
            if let Err(error) = popup.resize_render(&graphics, size, theme.text_scale, paint) {
                crate::log::error(&format!("bubble not drawn: {error}"));
                self.hide_bubble();
                self.reset_graphics();
                return;
            }
            let motion = match motion {
                Motion::Still => {
                    popup.set_frame(&graphics, target, size);
                    Motion::Still
                }
                Motion::Rising { started, from, .. } => {
                    popup.set_frame(&graphics, popup.origin(), size);
                    Motion::Rising { started, from, to: target }
                }
                // A new content comes back up from where it is.
                Motion::Sinking { .. } if renew => {
                    popup.set_frame(&graphics, popup.origin(), size);
                    Motion::Rising { started: Instant::now(), from: popup.origin(), to: target }
                }
                sinking @ Motion::Sinking { .. } => {
                    popup.set_frame(&graphics, popup.origin(), size);
                    sinking
                }
            };
            self.bubble_state.borrow_mut().motion = motion;
        } else {
            let Some(graphics) =
                self.paint_popup(&popup, "bubble", size, scale, theme.text_scale, &paint)
            else {
                return;
            };
            let motion = if rises(theme.animations, crate::screen::fullscreen_in_front()) {
                let start = hidden_origin(target, size, &layout.monitor, EDGE);
                popup.show_passive(&graphics, layout, start, true);
                Motion::Rising { started: Instant::now(), from: start, to: target }
            } else {
                popup.show_passive(&graphics, layout, target, false);
                Motion::Still
            };
            self.bubble_state.borrow_mut().motion = motion;
        }
        {
            let mut state = self.bubble_state.borrow_mut();
            state.content = Some(content);
            state.layout = Some(layout);
            state.size = size;
        }
        // SAFETY: timers of our own window (setting one again restarts it).
        unsafe {
            if matches!(self.bubble_state.borrow().motion, Motion::Rising { .. }) {
                SetTimer(Some(popup.window()), TIMER_FRAME, BUBBLE_FRAME_MS, None);
            }
            // A fresh showing always gets its time (even when painted again after a
            // graphics reset).
            if renew || !visible {
                SetTimer(Some(popup.window()), TIMER_HIDE, SHOWN_MS, None);
            }
        }
    }

    /// Hides the bubble at once (the flyout opens, the application ends).
    pub fn hide_bubble(&self) {
        let Some(popup) = self.bubble_popup.borrow().clone() else { return };
        // SAFETY: timers of our own window.
        unsafe {
            let _ = KillTimer(Some(popup.window()), TIMER_FRAME);
            let _ = KillTimer(Some(popup.window()), TIMER_HIDE);
        }
        {
            let mut state = self.bubble_state.borrow_mut();
            state.content = None;
            state.motion = Motion::Still;
        }
        popup.hide_now();
    }

    /// The Windows appearance changed: a shown bubble is painted again in the new theme.
    pub(crate) fn restyle_bubble(&self) {
        let shown = self.bubble_popup.borrow().as_ref().is_some_and(|p| p.is_visible());
        let content = self.bubble_state.borrow().content.clone();
        if let (true, Some(content)) = (shown, content) {
            self.present(content, false);
        }
    }

    pub(crate) fn is_bubble_window(&self, window: HWND) -> bool {
        self.bubble_popup.borrow().as_ref().is_some_and(|p| p.window() == window)
    }

    /// A message for the bubble window; `None` lets the default procedure handle it.
    pub(crate) fn bubble_message(
        &self,
        message: u32,
        wparam: WPARAM,
        _: LPARAM,
    ) -> Option<LRESULT> {
        match message {
            // Clicks and hovering go to what lies below.
            WM_NCHITTEST => Some(LRESULT(HTTRANSPARENT as isize)),
            WM_TIMER if wparam.0 == TIMER_FRAME => {
                self.bubble_step();
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == TIMER_HIDE => {
                self.bubble_timeout();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    /// One step of the rise or the sink.
    fn bubble_step(&self) {
        let Some(popup) = self.bubble_popup.borrow().clone() else { return };
        let (motion, size) = {
            let state = self.bubble_state.borrow();
            (state.motion, state.size)
        };
        let Some(graphics) = self.graphics() else {
            self.hide_bubble();
            return;
        };
        let step = |started: Instant, from, to, transition: Transition| {
            let elapsed = started.elapsed().as_secs_f32() * 1000.0 + FRAME_MS;
            popup.set_frame(&graphics, lerp(from, to, transition.progress(elapsed)), size);
            elapsed >= transition.duration_ms as f32
        };
        let done = match motion {
            Motion::Still => true,
            Motion::Rising { started, from, to } => {
                let done = step(started, from, to, BUBBLE_OPEN);
                if done {
                    // In place: on top of everything.
                    popup.raise(&graphics);
                }
                done
            }
            Motion::Sinking { started, from, to } => {
                let done = step(started, from, to, BUBBLE_CLOSE);
                if done {
                    self.bubble_state.borrow_mut().content = None;
                    popup.hide_now();
                }
                done
            }
        };
        if done {
            self.bubble_state.borrow_mut().motion = Motion::Still;
            // SAFETY: a timer of our own window.
            let _ = unsafe { KillTimer(Some(popup.window()), TIMER_FRAME) };
        }
    }

    /// The bubble was shown long enough: it sinks behind the taskbar (or vanishes).
    fn bubble_timeout(&self) {
        let Some(popup) = self.bubble_popup.borrow().clone() else { return };
        // SAFETY: a timer of our own window.
        let _ = unsafe { KillTimer(Some(popup.window()), TIMER_HIDE) };
        let (layout, size) = {
            let state = self.bubble_state.borrow();
            (state.layout, state.size)
        };
        let graphics = self.graphics();
        let (Some(layout), Some(graphics)) = (layout, graphics) else {
            self.hide_bubble();
            return;
        };
        if !rises(self.theme().animations, crate::screen::fullscreen_in_front()) {
            self.hide_bubble();
            return;
        }
        let from = popup.origin();
        let to = hidden_origin(from, size, &layout.monitor, EDGE);
        popup.lower(&graphics);
        self.bubble_state.borrow_mut().motion =
            Motion::Sinking { started: Instant::now(), from, to };
        // SAFETY: a timer of our own window.
        unsafe { SetTimer(Some(popup.window()), TIMER_FRAME, BUBBLE_FRAME_MS, None) };
    }

    /// The bubble window, created on first use.
    fn bubble_window(&self) -> Option<Rc<Popup>> {
        let existing = self.bubble_popup.borrow().clone();
        match existing {
            Some(popup) if popup.is_alive() => return Some(popup),
            Some(_) => drop(self.bubble_popup.borrow_mut().take()),
            None => {}
        }
        match Popup::create_passive_framed(self.window, BUBBLE_CLASS) {
            Ok(popup) => {
                let popup = Rc::new(popup);
                *self.bubble_popup.borrow_mut() = Some(popup.clone());
                Some(popup)
            }
            Err(error) => {
                crate::log::error(&format!(
                    "bubble window not created: {}",
                    crate::ShellError::from(error)
                ));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bubble_rises_from_the_bottom_with_animations_on() {
        assert_eq!(EDGE, Edge::Bottom);
        assert!(rises(true, false));
        assert!(!rises(false, false));
        // A full-screen program hides the taskbar layer: the bubble appears in place.
        assert!(!rises(true, true));
    }
}
