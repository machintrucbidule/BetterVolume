//! The context menu (FR-100): layout, pointer and keyboard behaviour, painting.
//!
//! Behaviour of the Windows 11 menus: the entry under the pointer is highlighted; a press
//! then a release on the same enabled entry chooses it; Up/Down move the highlight (it
//! wraps and skips separators and disabled entries), Home/End jump to the ends,
//! Enter/Space choose, Escape dismisses.

use crate::access::{AccessAction, AccessNode, AccessRole};
use crate::backend::{Canvas, Graphics};
use crate::icons::Icon;
use crate::tokens::menu::{
    ICON_GAP, ICON_LEFT, ICON_SIZE, ITEM_GAP, ITEM_HEIGHT, ITEM_INSET_X, ITEM_RADIUS, LABEL_RIGHT,
    MIN_WIDTH, PADDING_Y, SEPARATOR_HEIGHT,
};
use crate::tokens::typography::BODY;
use crate::{Point, Rect, Size, Theme};

/// One line of the menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    /// A choice.
    Item {
        /// Icon shown before the label.
        icon: Icon,
        /// Label (already translated).
        label: String,
        /// Whether it can be chosen.
        enabled: bool,
    },
    /// A separator line.
    Separator,
}

impl MenuEntry {
    fn selectable(&self) -> bool {
        matches!(self, MenuEntry::Item { enabled: true, .. })
    }
}

/// A key the menu reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Enter or Space.
    Activate,
    /// Escape (also the Menu and Alt keys: they close a menu in Windows).
    Dismiss,
}

/// What the menu asks its host to do after an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOutcome {
    /// Nothing visible changed.
    None,
    /// Paint again.
    Redraw,
    /// The entry at this index was chosen: close the menu and act.
    Chosen(usize),
    /// The menu was dismissed without a choice.
    Dismissed,
}

/// A laid-out menu and its interaction state.
#[derive(Clone, Debug)]
pub struct Menu {
    entries: Vec<MenuEntry>,
    rows: Vec<Rect>,
    size: Size,
    hot: Option<usize>,
    pressed: Option<usize>,
}

impl Menu {
    /// Lays the entries out. `label_width` gives the width of a label in DIPs at the given
    /// text scale (the backend measures it with the real font).
    pub fn new(
        entries: Vec<MenuEntry>,
        text_scale: f32,
        label_width: impl Fn(&str) -> f32,
    ) -> Menu {
        let item_height = (ITEM_HEIGHT - BODY.line_height + BODY.line_height * text_scale).round();
        let label_x = ITEM_INSET_X + ICON_LEFT + ICON_SIZE + ICON_GAP;
        let widest = entries
            .iter()
            .filter_map(|e| match e {
                MenuEntry::Item { label, .. } => Some(label_width(label)),
                MenuEntry::Separator => None,
            })
            .fold(0.0_f32, f32::max);
        let width = (label_x + widest + LABEL_RIGHT + ITEM_INSET_X).ceil().max(MIN_WIDTH);
        let mut rows = Vec::with_capacity(entries.len());
        let mut y = PADDING_Y;
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                y += ITEM_GAP;
            }
            let height = match entry {
                MenuEntry::Item { .. } => item_height,
                MenuEntry::Separator => SEPARATOR_HEIGHT,
            };
            rows.push(Rect::new(ITEM_INSET_X, y, width - 2.0 * ITEM_INSET_X, height));
            y += height;
        }
        let size = Size::new(width, y + PADDING_Y);
        Menu { entries, rows, size, hot: None, pressed: None }
    }

    /// Lays the entries out, measuring the labels with the real font.
    pub fn measured(entries: Vec<MenuEntry>, graphics: &Graphics, text_scale: f32) -> Menu {
        Menu::new(entries, text_scale, |label| {
            graphics.measure_text(label, BODY, text_scale).map_or(0.0, |s| s.width)
        })
    }

    /// Size of the menu.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The highlighted entry, if any.
    pub fn highlighted(&self) -> Option<usize> {
        self.hot
    }

    /// The selectable entry at a point.
    pub fn hit(&self, p: Point) -> Option<usize> {
        // The highlight spans the whole width: the inset margins count as the entry.
        let p = Point::new(p.x.clamp(ITEM_INSET_X, self.size.width - ITEM_INSET_X - 0.01), p.y);
        self.rows.iter().position(|r| r.contains(p)).filter(|&i| self.entries[i].selectable())
    }

    /// The pointer moved to `p` (inside or outside the menu).
    pub fn pointer_move(&mut self, p: Point) -> MenuOutcome {
        let hot = self.hit(p);
        // Leaving every entry keeps nothing highlighted (Windows behaviour).
        self.set_hot(hot)
    }

    /// The pointer left the window.
    pub fn pointer_leave(&mut self) -> MenuOutcome {
        self.pressed = None;
        self.set_hot(None)
    }

    /// A button was pressed at `p`.
    pub fn press(&mut self, p: Point) -> MenuOutcome {
        self.pressed = self.hit(p);
        self.hot = self.pressed.or(self.hot);
        MenuOutcome::Redraw
    }

    /// The button was released at `p`: chooses the entry if it is the one pressed.
    pub fn release(&mut self, p: Point) -> MenuOutcome {
        let pressed = self.pressed.take();
        match (pressed, self.hit(p)) {
            (Some(a), Some(b)) if a == b => MenuOutcome::Chosen(a),
            // A release without a press on an entry (the right click that opened the menu
            // ends here) chooses the entry under the pointer, as Windows menus do.
            (None, Some(b)) => MenuOutcome::Chosen(b),
            _ => MenuOutcome::Redraw,
        }
    }

    /// A key was pressed.
    pub fn key(&mut self, key: MenuKey) -> MenuOutcome {
        let selectable: Vec<usize> =
            (0..self.entries.len()).filter(|&i| self.entries[i].selectable()).collect();
        let (Some(&first), Some(&last)) = (selectable.first(), selectable.last()) else {
            return if key == MenuKey::Dismiss {
                MenuOutcome::Dismissed
            } else {
                MenuOutcome::None
            };
        };
        let position = self.hot.and_then(|h| selectable.iter().position(|&i| i == h));
        let next = match key {
            MenuKey::Dismiss => return MenuOutcome::Dismissed,
            MenuKey::Activate => {
                return match self.hot {
                    Some(i) if self.entries[i].selectable() => MenuOutcome::Chosen(i),
                    _ => MenuOutcome::None,
                };
            }
            MenuKey::Home => first,
            MenuKey::End => last,
            MenuKey::Down => match position {
                Some(p) => selectable[(p + 1) % selectable.len()],
                None => first,
            },
            MenuKey::Up => match position {
                Some(p) => selectable[(p + selectable.len() - 1) % selectable.len()],
                None => last,
            },
        };
        self.set_hot(Some(next))
    }

    fn set_hot(&mut self, hot: Option<usize>) -> MenuOutcome {
        if self.hot == hot {
            return MenuOutcome::None;
        }
        self.hot = hot;
        MenuOutcome::Redraw
    }

    /// The menu described for screen readers (FR-024): one entry per line.
    pub fn access_tree(&self, name: &str) -> AccessNode {
        let mut root = AccessNode::new(0, AccessRole::Menu, name, Rect::from_size(self.size));
        for (index, (entry, row)) in self.entries.iter().zip(&self.rows).enumerate() {
            let node = match entry {
                MenuEntry::Item { label, enabled, .. } => {
                    let mut node =
                        AccessNode::new(Menu::access_id(index), AccessRole::MenuItem, label, *row);
                    node.enabled = *enabled;
                    node.focusable = *enabled;
                    node
                }
                MenuEntry::Separator => {
                    AccessNode::new(Menu::access_id(index), AccessRole::Separator, "", *row)
                }
            };
            root.children.push(node);
        }
        root.id = 1;
        root
    }

    /// The screen-reader identifier of the line at `index`.
    pub fn access_id(index: usize) -> u64 {
        index as u64 + 2
    }

    /// The screen-reader identifier of the highlighted entry.
    pub fn focused_access_id(&self) -> Option<u64> {
        self.hot.map(Menu::access_id)
    }

    /// A request from an assistive tool.
    pub fn access_action(&mut self, id: u64, action: AccessAction) -> MenuOutcome {
        let Some(index) = id.checked_sub(2).map(|i| i as usize) else {
            return MenuOutcome::None;
        };
        if !self.entries.get(index).is_some_and(MenuEntry::selectable) {
            return MenuOutcome::None;
        }
        match action {
            AccessAction::Invoke | AccessAction::Select => MenuOutcome::Chosen(index),
            AccessAction::Focus => self.set_hot(Some(index)),
            _ => MenuOutcome::None,
        }
    }

    /// Paints the menu (background included).
    pub fn paint(&self, canvas: &mut Canvas<'_>, theme: &Theme) {
        super::paint_surface(canvas, theme, self.size);
        self.paint_items(canvas, theme);
    }

    /// Paints the entries only, over a background already painted.
    pub fn paint_items(&self, canvas: &mut Canvas<'_>, theme: &Theme) {
        for (index, (entry, row)) in self.entries.iter().zip(&self.rows).enumerate() {
            match entry {
                MenuEntry::Separator => {
                    let y = row.y + (row.height / 2.0).floor() + 0.5;
                    // The line spans the whole menu, past the entry insets.
                    canvas.line(
                        Point::new(0.0, y),
                        Point::new(self.size.width, y),
                        1.0,
                        theme.divider,
                    );
                }
                MenuEntry::Item { icon, label, enabled } => {
                    let hot = self.hot == Some(index) && *enabled;
                    if hot {
                        let fill = if self.pressed == Some(index) {
                            theme.subtle_pressed
                        } else {
                            theme.subtle_hover
                        };
                        canvas.fill_rounded(*row, ITEM_RADIUS, fill);
                    }
                    let color = match (enabled, hot) {
                        (false, _) => theme.text_disabled,
                        (true, true) => theme.subtle_hover_text,
                        (true, false) => theme.text_primary,
                    };
                    let icon_box = Rect::new(row.x + ICON_LEFT, row.y, ICON_SIZE, row.height);
                    canvas.glyph(icon.glyph(), ICON_SIZE, icon_box, color);
                    let label_x = icon_box.right() + ICON_GAP;
                    let label_box = Rect::new(label_x, row.y, row.right() - label_x, row.height);
                    canvas.text(label, BODY, label_box, crate::backend::TextAlign::Leading, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str) -> MenuEntry {
        MenuEntry::Item { icon: Icon::Settings, label: label.into(), enabled: true }
    }

    /// Sound settings · Mixer · — · Settings (disabled) · — · Quit
    fn sample() -> Menu {
        Menu::new(
            vec![
                item("Sound settings"),
                item("Volume mixer"),
                MenuEntry::Separator,
                MenuEntry::Item { icon: Icon::Settings, label: "Settings".into(), enabled: false },
                MenuEntry::Separator,
                item("Quit"),
            ],
            1.0,
            |label| label.len() as f32 * 7.0,
        )
    }

    fn centre(menu: &Menu, index: usize) -> Point {
        let r = menu.rows[index];
        Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    #[test]
    fn layout_stacks_entries_with_padding() {
        let menu = sample();
        assert_eq!(menu.rows[0].y, PADDING_Y);
        assert_eq!(menu.rows[0].height, ITEM_HEIGHT);
        assert_eq!(menu.rows[2].height, SEPARATOR_HEIGHT);
        assert_eq!(menu.rows[1].y, menu.rows[0].bottom() + ITEM_GAP);
        assert_eq!(menu.size().height, menu.rows[5].bottom() + PADDING_Y);
        assert!(menu.size().width >= MIN_WIDTH);
    }

    #[test]
    fn width_follows_the_longest_label() {
        let wide = Menu::new(vec![item(&"x".repeat(80))], 1.0, |l| l.len() as f32 * 7.0);
        assert!(wide.size().width > 560.0);
    }

    #[test]
    fn text_scaling_grows_the_entries() {
        let menu = Menu::new(vec![item("a")], 1.5, |_| 10.0);
        assert_eq!(menu.rows[0].height, ITEM_HEIGHT + 10.0);
    }

    #[test]
    fn pointer_highlights_only_enabled_entries() {
        let mut menu = sample();
        assert_eq!(menu.pointer_move(centre(&menu, 1)), MenuOutcome::Redraw);
        assert_eq!(menu.highlighted(), Some(1));
        assert_eq!(menu.pointer_move(centre(&menu, 1)), MenuOutcome::None);
        menu.pointer_move(centre(&menu, 3));
        assert_eq!(menu.highlighted(), None, "disabled entry");
        menu.pointer_move(centre(&menu, 2));
        assert_eq!(menu.highlighted(), None, "separator");
        // The inset margin left of the highlight still counts as the entry.
        menu.pointer_move(Point::new(1.0, centre(&menu, 0).y));
        assert_eq!(menu.highlighted(), Some(0));
    }

    #[test]
    fn press_and_release_on_the_same_entry_chooses_it() {
        let mut menu = sample();
        menu.press(centre(&menu, 0));
        assert_eq!(menu.release(centre(&menu, 0)), MenuOutcome::Chosen(0));
        menu.press(centre(&menu, 0));
        assert_eq!(menu.release(centre(&menu, 1)), MenuOutcome::Redraw, "moved away");
        menu.press(centre(&menu, 3));
        assert_eq!(menu.release(centre(&menu, 3)), MenuOutcome::Redraw, "disabled");
        // The release of the right click that opened the menu, over an entry.
        assert_eq!(menu.release(centre(&menu, 5)), MenuOutcome::Chosen(5));
    }

    #[test]
    fn keyboard_skips_separators_and_disabled_entries_and_wraps() {
        let mut menu = sample();
        menu.key(MenuKey::Down);
        assert_eq!(menu.highlighted(), Some(0));
        menu.key(MenuKey::Down);
        assert_eq!(menu.highlighted(), Some(1));
        menu.key(MenuKey::Down);
        assert_eq!(menu.highlighted(), Some(5));
        menu.key(MenuKey::Down);
        assert_eq!(menu.highlighted(), Some(0), "wraps");
        menu.key(MenuKey::Up);
        assert_eq!(menu.highlighted(), Some(5), "wraps back");
        menu.key(MenuKey::Home);
        assert_eq!(menu.highlighted(), Some(0));
        menu.key(MenuKey::End);
        assert_eq!(menu.highlighted(), Some(5));
        assert_eq!(menu.key(MenuKey::Activate), MenuOutcome::Chosen(5));
        assert_eq!(menu.key(MenuKey::Dismiss), MenuOutcome::Dismissed);
    }

    #[test]
    fn up_without_highlight_starts_at_the_end_and_enter_does_nothing() {
        let mut menu = sample();
        assert_eq!(menu.key(MenuKey::Activate), MenuOutcome::None);
        menu.key(MenuKey::Up);
        assert_eq!(menu.highlighted(), Some(5));
    }

    #[test]
    fn a_menu_without_choices_can_only_be_dismissed() {
        let mut menu = Menu::new(vec![MenuEntry::Separator], 1.0, |_| 0.0);
        assert_eq!(menu.key(MenuKey::Down), MenuOutcome::None);
        assert_eq!(menu.key(MenuKey::Dismiss), MenuOutcome::Dismissed);
    }

    #[test]
    fn screen_readers_see_every_line_and_can_choose() {
        let mut menu = sample();
        let tree = menu.access_tree("Menu");
        assert_eq!(tree.children.len(), 6);
        assert_eq!(tree.children[2].role, AccessRole::Separator);
        assert!(!tree.children[3].enabled);
        let quit = tree.children[5].id;
        assert_eq!(menu.access_action(quit, AccessAction::Invoke), MenuOutcome::Chosen(5));
        let disabled = tree.children[3].id;
        assert_eq!(menu.access_action(disabled, AccessAction::Invoke), MenuOutcome::None);
        menu.access_action(quit, AccessAction::Focus);
        assert_eq!(menu.focused_access_id(), Some(quit));
    }

    #[test]
    fn leaving_clears_the_highlight() {
        let mut menu = sample();
        menu.pointer_move(centre(&menu, 0));
        assert_eq!(menu.pointer_leave(), MenuOutcome::Redraw);
        assert_eq!(menu.highlighted(), None);
    }
}
