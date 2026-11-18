//! Waking the UI thread from another thread (`docs/ARCHITECTURE.md` §3).
//!
//! Background sources (Core Audio callbacks today, MQTT later) never touch application
//! state. They queue their news in their own channel and call [`Waker::wake`], which posts
//! one message to the hidden window; the shell then delivers [`crate::Event::Wake`] to the
//! handler on the UI thread, which drains the sources.
//!
//! Wake-ups are coalesced: while one is pending, further calls post nothing, so a burst
//! of notifications (a volume slider dragged in another program) costs one message.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

/// What a waker posts to the hidden window.
pub(crate) const WM_WAKE: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 2;

#[derive(Debug)]
struct Inner {
    /// The hidden window, as an integer (window handles are not `Send`); 0 once it is gone.
    window: AtomicIsize,
    /// A wake-up message is on its way and not yet handled.
    pending: AtomicBool,
}

/// Wakes the UI thread. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct Waker(Arc<Inner>);

impl Waker {
    pub(crate) fn new(window: HWND) -> Self {
        Self(Arc::new(Inner {
            window: AtomicIsize::new(window.0 as isize),
            pending: AtomicBool::new(false),
        }))
    }

    /// Asks the UI thread to deliver [`crate::Event::Wake`]. Never blocks; does nothing
    /// once the application is closing.
    pub fn wake(&self) {
        if self.0.pending.swap(true, Ordering::AcqRel) {
            return; // already on its way: it will see this news too
        }
        let window = self.0.window.load(Ordering::Acquire);
        if window == 0 {
            return;
        }
        // SAFETY: posting a message that carries no pointer, so whatever window receives
        // it cannot misuse it. The handle is our hidden window; `close` clears it while
        // that window is being destroyed, but a thread that read it just before may still
        // post: to a destroyed window that only fails. Only if Windows had already reused
        // the handle value (a matter of microseconds) would another window get one
        // argument-less private message; accepted as negligible.
        let posted =
            unsafe { PostMessageW(Some(HWND(window as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0)) };
        if posted.is_err() {
            // The message queue is full or the window is gone: allow a later retry.
            self.0.pending.store(false, Ordering::Release);
        }
    }

    /// Called on the UI thread when the wake-up message arrives, *before* the handler
    /// drains its sources: news arriving from now on posts a new message.
    pub(crate) fn acknowledge(&self) {
        // A read-modify-write, not a plain store: it orders this release with the
        // `swap` in `wake`, so a source that queued news and saw "pending" is sure to
        // have its news seen by the drain that follows.
        self.0.pending.swap(false, Ordering::AcqRel);
    }

    /// The window is being destroyed: later wake-ups do nothing.
    pub(crate) fn close(&self) {
        self.0.window.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(waker: &Waker) -> bool {
        waker.0.pending.load(Ordering::Acquire)
    }

    #[test]
    fn a_failed_post_allows_a_later_retry() {
        // Not a window handle: posting fails, so `wake` releases `pending` again.
        let waker = Waker::new(HWND(std::ptr::dangling_mut()));
        waker.wake();
        assert!(!pending(&waker));
    }

    #[test]
    fn wake_ups_are_coalesced_until_acknowledged() {
        let waker = Waker::new(HWND(std::ptr::dangling_mut()));
        waker.0.pending.store(true, Ordering::Release);
        waker.wake(); // already pending: nothing is posted, it stays pending
        assert!(pending(&waker));
        waker.acknowledge();
        assert!(!pending(&waker));
    }

    #[test]
    fn a_closed_waker_posts_nothing() {
        let waker = Waker::new(HWND(std::ptr::dangling_mut()));
        waker.close();
        waker.wake();
        // Nothing was posted (the window is gone); staying "pending" keeps later calls
        // from even trying.
        assert!(pending(&waker));
    }

    #[test]
    fn waker_is_usable_from_other_threads() {
        fn assert_send_sync<T: Send + Sync + Clone + 'static>() {}
        assert_send_sync::<Waker>();
    }
}
