//! The tray context menu (FR-100): a design-system menu in a popup window, run
//! like a Windows menu: [`Shell::popup_menu`] returns once the user chose or dismissed.

use std::rc::Rc;

use bv_ui_kit::controls::menu::{Menu, MenuEntry, MenuKey, MenuOutcome};
use bv_ui_kit::icons::Icon;
use bv_ui_kit::motion::MENU_OPEN;
use bv_ui_kit::tokens::menu::TASKBAR_GAP;
use bv_ui_kit::{Point, Theme};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_APPS, VK_DOWN, VK_END, VK_ESCAPE, VK_F10,
    VK_HOME, VK_MENU, VK_RETURN, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, SetForegroundWindow, TrackPopupMenu, TranslateMessage, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTALIGN, TPM_RIGHTBUTTON, TPM_TOPALIGN, TRACK_POPUP_MENU_FLAGS, WA_INACTIVE,
    WM_ACTIVATE, WM_CANCELMODE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
};

use crate::host::Shell;
use crate::placement::{icon_menu_origin, menu_origin, touches_taskbar, Edge, ScreenRect};
use crate::popup::{Popup, Unfocused};

/// One line of a context menu, with the value returned when it is chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuLine<T> {
    /// A choice.
    Item {
        /// Returned by [`Shell::popup_menu`] when chosen.
        value: T,
        /// Icon before the label.
        icon: Icon,
        /// Label (already translated).
        label: String,
        /// Whether it can be chosen.
        enabled: bool,
    },
    /// A separator line.
    Separator,
}

/// Screen-reader access to the menu: reads the menu being shown through the message
/// window.
pub(crate) struct MenuAccess {
    pub(crate) owner: HWND,
}

impl MenuAccess {
    fn with_host<R>(&self, f: impl FnOnce(&crate::host::Host<'_>) -> Option<R>) -> Option<R> {
        // SAFETY: UI Automation calls the providers on the UI thread (COM threading), where
        // the message window's user data is the host or 0.
        let host = unsafe { crate::host::host_of(self.owner) }?;
        f(host)
    }
}

impl crate::uia::AccessSource for MenuAccess {
    fn tree(&self) -> Option<bv_ui_kit::access::AccessNode> {
        self.with_host(|host| {
            let session = host.shell.menu_session.try_borrow().ok()?;
            let name = host.shell.app_name.try_borrow().ok()?;
            session.as_ref().map(|s| s.menu.access_tree(&name))
        })
    }

    fn scale(&self) -> f32 {
        self.with_host(|host| host.shell.menu_popup.try_borrow().ok()?.as_ref().map(|p| p.scale()))
            .unwrap_or(1.0)
    }

    fn act(&self, id: u64, action: bv_ui_kit::access::AccessAction) -> bool {
        self.with_host(|host| {
            let shell = &host.shell;
            let popup = shell.menu_popup.try_borrow().ok()?.clone()?;
            let outcome = {
                let mut session = shell.menu_session.try_borrow_mut().ok()?;
                session.as_mut()?.menu.access_action(id, action)
            };
            match outcome {
                MenuOutcome::None => return Some(false),
                MenuOutcome::Redraw => shell.repaint_menu(&popup),
                MenuOutcome::Chosen(_) | MenuOutcome::Dismissed => {
                    if let Some(session) = shell.menu_session.borrow_mut().as_mut() {
                        session.outcome = Some(outcome);
                    }
                    // Wake the menu's message loop so it sees the outcome.
                    // SAFETY: posting a message without pointers to our own window.
                    let _ =
                        unsafe { PostMessageW(Some(shell.window), WM_NULL, WPARAM(0), LPARAM(0)) };
                }
            }
            Some(true)
        })
        .unwrap_or(false)
    }

    fn focused(&self) -> Option<u64> {
        self.with_host(|host| {
            let session = host.shell.menu_session.try_borrow().ok()?;
            session.as_ref()?.menu.focused_access_id()
        })
    }
}

/// The menu being shown.
pub(crate) struct Session {
    menu: Menu,
    theme: Theme,
    outcome: Option<MenuOutcome>,
    tracking_leave: bool,
}

impl Shell {
    /// Shows a context menu opened at `(x, y)` (screen pixels: the pointer or the icon)
    /// and waits for the user's choice. Returns the value of the chosen entry, or `None`
    /// if the menu was dismissed (Escape, click elsewhere) or could not be shown.
    ///
    /// Events arriving while the menu is open are not lost: they are delivered to the
    /// handler, in order, as soon as it returns.
    pub fn popup_menu<T: Copy>(&self, lines: &[MenuLine<T>], x: i32, y: i32) -> Option<T> {
        if self.menu_session.borrow().is_some() {
            return None; // one menu at a time
        }
        let index = match self.run_menu(lines, x, y) {
            Ok(index) => index?,
            // The design-system menu cannot be shown (graphics unavailable): the plain
            // Windows menu, so that every entry, Quit included, stays reachable.
            Err(()) => native_menu(self.window, lines, x, y)?,
        };
        match lines.get(index) {
            Some(MenuLine::Item { value, enabled: true, .. }) => Some(*value),
            _ => None,
        }
    }

    /// Runs the design-system menu: `Ok(choice)`, or `Err(())` if it cannot be shown.
    fn run_menu<T>(&self, lines: &[MenuLine<T>], x: i32, y: i32) -> Result<Option<usize>, ()> {
        self.refresh_appearance();
        let popup = self.menu_window().ok_or(())?;
        let theme = self.theme();
        let entries = lines
            .iter()
            .map(|line| match line {
                MenuLine::Item { icon, label, enabled, .. } => {
                    MenuEntry::Item { icon: *icon, label: label.clone(), enabled: *enabled }
                }
                MenuLine::Separator => MenuEntry::Separator,
            })
            .collect();
        // Measured with graphics objects released right after: a rebuild after a drawing
        // failure (below) must not find the old ones still alive.
        let menu = {
            let graphics = self.graphics().ok_or(())?;
            Menu::measured(entries, &graphics, theme.text_scale)
        };

        let layout = crate::screen::layout_at(x, y);
        let size = menu.size().to_pixels(layout.scale);
        let size = (size.0 as i32, size.1 as i32);
        let theme = popup.apply_theme(theme);
        let graphics = self
            .paint_popup(&popup, "menu", size, layout.scale, theme.text_scale, &|c| {
                menu.paint(c, &theme)
            })
            .ok_or(())?;
        *self.menu_session.borrow_mut() =
            Some(Session { menu, theme, outcome: None, tracking_leave: false });

        let gap = (TASKBAR_GAP * layout.scale).round() as i32;
        // Opened on the tray icon (pointer or keyboard): centred on the icon, like the
        // menus of the Windows tray icons; anywhere else, at the point.
        let icon = self
            .icon_rect()
            .map(|r| ScreenRect::new(r.left, r.top, r.right, r.bottom))
            // A little tolerance: the keyboard anchor may sit on the icon's edge.
            .filter(|r| x >= r.left - 2 && x <= r.right + 2 && y >= r.top - 2 && y <= r.bottom + 2);
        let origin = match icon {
            Some(icon) => icon_menu_origin(&icon, size, &layout.area, layout.edge, gap),
            None => menu_origin((x, y), size, &layout.area, layout.edge, gap),
        };
        // A menu opened higher than the taskbar (from the hidden-icons panel) appears in
        // place: rising from the taskbar would make it cross half the screen.
        let rises = touches_taskbar(origin, size, &layout.area, layout.edge, gap)
            && !crate::screen::hidden_icons_open();
        let motion = (theme.animations && rises).then_some(MENU_OPEN);
        let focused = popup.show(&graphics, layout, origin, motion, Unfocused::Hide);
        if !focused {
            // Without the focus the menu would never learn of a click elsewhere and could
            // not be dismissed: give up at once (a new right click opens it normally).
            crate::log::info("menu dismissed: Windows refused it the focus");
            if let Some(session) = self.menu_session.borrow_mut().as_mut() {
                session.outcome = Some(MenuOutcome::Dismissed);
            }
        }

        // A local message loop until the menu has an outcome (what Windows menus do).
        let mut message = MSG::default();
        loop {
            if self.menu_session.borrow().as_ref().is_none_or(|s| s.outcome.is_some()) {
                break;
            }
            // SAFETY: `message` is a valid structure for the calls to fill and read.
            let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if status == 0 {
                // The application is quitting: hand the quit request back to the main
                // loop and drop the menu.
                // SAFETY: plain call.
                unsafe { PostQuitMessage(message.wParam.0 as i32) };
                break;
            }
            if status == -1 {
                break;
            }
            // SAFETY: as above.
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let session = self.menu_session.borrow_mut().take();
        popup.hide(None, None);
        Ok(match session.and_then(|s| s.outcome) {
            Some(MenuOutcome::Chosen(index)) => Some(index),
            _ => None,
        })
    }

    /// The menu window, created on first use.
    fn menu_window(&self) -> Option<Rc<Popup>> {
        // A window destroyed behind our back is replaced rather than reused.
        let existing = self.menu_popup.borrow().clone();
        match existing {
            Some(popup) if popup.is_alive() => return Some(popup),
            Some(_) => drop(self.menu_popup.borrow_mut().take()),
            None => {}
        }
        match Popup::create(self.window, crate::popup::MENU_CLASS) {
            Ok(popup) => {
                let popup = Rc::new(popup);
                *self.menu_popup.borrow_mut() = Some(popup.clone());
                Some(popup)
            }
            Err(error) => {
                crate::log::error(&format!(
                    "menu window not created: {}",
                    crate::ShellError::from(error)
                ));
                None
            }
        }
    }

    pub(crate) fn is_menu_window(&self, window: HWND) -> bool {
        self.menu_popup.borrow().as_ref().is_some_and(|p| p.window() == window)
    }

    /// A message for the menu window; `None` lets the default procedure handle it.
    pub(crate) fn menu_message(
        &self,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Option<LRESULT> {
        let popup = self.menu_popup.borrow().clone()?;
        let point = || {
            let scale = popup.scale().max(0.1);
            let x = f32::from((lparam.0 & 0xFFFF) as u16 as i16);
            let y = f32::from(((lparam.0 >> 16) & 0xFFFF) as u16 as i16);
            Point::new(x / scale, y / scale)
        };
        let outcome = {
            let mut session = self.menu_session.borrow_mut();
            let session = session.as_mut()?;
            match message {
                WM_MOUSEMOVE => {
                    if !session.tracking_leave {
                        let mut track = TRACKMOUSEEVENT {
                            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                            dwFlags: TME_LEAVE,
                            hwndTrack: popup.window(),
                            dwHoverTime: 0,
                        };
                        // SAFETY: valid structure for our own window.
                        session.tracking_leave = unsafe { TrackMouseEvent(&mut track) }.is_ok();
                    }
                    session.menu.pointer_move(point())
                }
                WM_MOUSELEAVE => {
                    session.tracking_leave = false;
                    session.menu.pointer_leave()
                }
                WM_LBUTTONDOWN | WM_RBUTTONDOWN => session.menu.press(point()),
                WM_LBUTTONUP | WM_RBUTTONUP => session.menu.release(point()),
                WM_KEYDOWN | WM_SYSKEYDOWN => match menu_key(wparam) {
                    Some(key) => session.menu.key(key),
                    None => MenuOutcome::None,
                },
                WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => MenuOutcome::Dismissed,
                WM_CANCELMODE => MenuOutcome::Dismissed,
                _ => return None,
            }
        };
        match outcome {
            MenuOutcome::None => {}
            MenuOutcome::Redraw => {
                self.repaint_menu(&popup);
                if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                    // The highlight moved by keyboard: screen readers follow it.
                    let focused = self
                        .menu_session
                        .borrow()
                        .as_ref()
                        .and_then(|s| s.menu.focused_access_id());
                    crate::uia::focus_changed(popup.window(), focused);
                }
            }
            MenuOutcome::Chosen(_) | MenuOutcome::Dismissed => {
                if let Some(session) = self.menu_session.borrow_mut().as_mut() {
                    session.outcome = Some(outcome);
                }
            }
        }
        Some(LRESULT(0))
    }

    fn repaint_menu(&self, popup: &Popup) {
        let Some(graphics) = self.graphics() else { return };
        let session = self.menu_session.borrow();
        let Some(session) = session.as_ref() else { return };
        let painted = popup.repaint(&graphics, session.theme.text_scale, |c| {
            session.menu.paint(c, &session.theme)
        });
        if let Err(error) = painted {
            crate::log::error(&format!("menu not redrawn: {error}"));
        }
    }
}

/// Flags of the plain menu: it opens away from the taskbar, wherever the taskbar is.
fn native_alignment(edge: Edge) -> TRACK_POPUP_MENU_FLAGS {
    let side = match edge {
        Edge::Bottom => TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
        Edge::Top => TPM_RIGHTALIGN | TPM_TOPALIGN,
        Edge::Left => TPM_LEFTALIGN | TPM_BOTTOMALIGN,
        Edge::Right => TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
    };
    TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | side
}

/// The plain Windows menu with the same lines: the fallback when graphics are unavailable.
/// Returns the index of the chosen line.
fn native_menu<T>(owner: HWND, lines: &[MenuLine<T>], x: i32, y: i32) -> Option<usize> {
    // SAFETY: no arguments; returns a new menu, destroyed below.
    let menu = unsafe { CreatePopupMenu() }.ok()?;
    for (index, line) in lines.iter().enumerate() {
        let added = match line {
            MenuLine::Item { label, enabled, .. } => {
                let text: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let flags = if *enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                // SAFETY: valid menu; the text outlives the call (the menu copies it).
                unsafe { AppendMenuW(menu, flags, index + 1, PCWSTR(text.as_ptr())) }
            }
            // SAFETY: valid menu.
            MenuLine::Separator => unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) },
        };
        if added.is_err() {
            // SAFETY: the menu was created above and is destroyed once.
            let _ = unsafe { DestroyMenu(menu) };
            return None;
        }
    }
    // SAFETY: our own window; a background window's menu closes on an outside click only
    // when that window is in the foreground (documented TrackPopupMenu requirement).
    let chosen = unsafe {
        let _ = SetForegroundWindow(owner);
        let chosen = TrackPopupMenu(
            menu,
            native_alignment(crate::screen::layout_at(x, y).edge),
            x,
            y,
            None,
            owner,
            None,
        );
        let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        chosen
    };
    usize::try_from(chosen.0).ok()?.checked_sub(1)
}

/// The menu key for a virtual key, if the menu reacts to it.
fn menu_key(wparam: WPARAM) -> Option<MenuKey> {
    let key = wparam.0 as u16;
    Some(match key {
        k if k == VK_UP.0 => MenuKey::Up,
        k if k == VK_DOWN.0 => MenuKey::Down,
        k if k == VK_HOME.0 => MenuKey::Home,
        k if k == VK_END.0 => MenuKey::End,
        k if k == VK_RETURN.0 || k == VK_SPACE.0 => MenuKey::Activate,
        // Escape, Alt, F10 and the Menu key close a menu in Windows.
        k if k == VK_ESCAPE.0 || k == VK_MENU.0 || k == VK_F10.0 || k == VK_APPS.0 => {
            MenuKey::Dismiss
        }
        _ => return None,
    })
}

impl Shell {
    /// A close request (Alt+F4, another program) reached a popup: the flyout is hidden,
    /// the menu dismissed; neither window is destroyed.
    pub(crate) fn close_popup(&self, window: HWND) {
        if self.is_flyout_window(window) {
            self.hide_flyout();
        } else if self.is_menu_window(window) {
            if let Some(session) = self.menu_session.borrow_mut().as_mut() {
                session.outcome = Some(MenuOutcome::Dismissed);
            }
            // A request sent (not posted) does not wake the menu's message loop: post an
            // empty message so it sees the outcome at once.
            // SAFETY: posting a message without pointers to our own window.
            let _ = unsafe { PostMessageW(Some(self.window), WM_NULL, WPARAM(0), LPARAM(0)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_keys_are_mapped() {
        assert_eq!(menu_key(WPARAM(VK_DOWN.0 as usize)), Some(MenuKey::Down));
        assert_eq!(menu_key(WPARAM(VK_SPACE.0 as usize)), Some(MenuKey::Activate));
        assert_eq!(menu_key(WPARAM(VK_RETURN.0 as usize)), Some(MenuKey::Activate));
        assert_eq!(menu_key(WPARAM(VK_ESCAPE.0 as usize)), Some(MenuKey::Dismiss));
        assert_eq!(menu_key(WPARAM(VK_MENU.0 as usize)), Some(MenuKey::Dismiss));
        assert_eq!(menu_key(WPARAM(0x41)), None, "letters do nothing");
    }
}
