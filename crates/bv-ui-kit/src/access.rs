//! Accessibility description of a surface (FR-024): what a screen reader must know about
//! each control, without any Windows type. Controls describe themselves as a tree of
//! [`AccessNode`]s; the shell exposes that tree through Windows UI Automation and turns
//! the requests of assistive tools into [`AccessAction`]s for the control.

use crate::Rect;

/// What kind of element a node is (maps to a UI Automation control type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessRole {
    /// The surface itself (flyout panel, menu window).
    Pane,
    /// A group of choices of which one is current (device tiles).
    List,
    /// One choice of a [`AccessRole::List`] (a device tile): selectable, invokable.
    ListItem,
    /// A row of tabs.
    TabList,
    /// One tab: selectable.
    Tab,
    /// A push button: invokable.
    Button,
    /// A button with an on/off state (mute): toggleable.
    ToggleButton,
    /// A button that shows or hides a section: expandable.
    Expander,
    /// A slider: has a range value.
    Slider,
    /// A group of related controls (an application row, a section).
    Group,
    /// Static text.
    Text,
    /// A menu.
    Menu,
    /// A menu entry: invokable.
    MenuItem,
    /// A separator line.
    Separator,
    /// A drop-down choice: expandable; its children are the choices (list items), the
    /// selected one marked.
    ComboBox,
    /// A link: invokable.
    Hyperlink,
    /// A text box: has a text value.
    Edit,
}

/// The value of a slider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccessRange {
    /// Current value.
    pub value: f64,
    /// Smallest value.
    pub min: f64,
    /// Largest value.
    pub max: f64,
    /// Change of one small step (arrow key).
    pub small_change: f64,
    /// Change of one large step (Page Up / Page Down).
    pub large_change: f64,
}

/// One element of the accessibility tree.
#[derive(Clone, Debug, PartialEq)]
pub struct AccessNode {
    /// Identifier, unique in its tree and stable while the element exists (content
    /// updates keep it), so that assistive tools can follow the element.
    pub id: u64,
    /// Kind of element.
    pub role: AccessRole,
    /// Name read by screen readers (already translated).
    pub name: String,
    /// Additional description (tooltip text), if any.
    pub help: Option<String>,
    /// Position and size in DIPs, relative to the surface's top-left corner.
    pub bounds: Rect,
    /// Whether the element can be used.
    pub enabled: bool,
    /// Whether the element can take the keyboard focus.
    pub focusable: bool,
    /// Selection state, for [`AccessRole::ListItem`] and [`AccessRole::Tab`].
    pub selected: Option<bool>,
    /// On/off state, for [`AccessRole::ToggleButton`].
    pub toggled: Option<bool>,
    /// Shown/hidden state, for [`AccessRole::Expander`].
    pub expanded: Option<bool>,
    /// Value, for [`AccessRole::Slider`].
    pub range: Option<AccessRange>,
    /// Text value, for [`AccessRole::Edit`].
    pub value: Option<String>,
    /// Children, in reading order.
    pub children: Vec<AccessNode>,
}

impl AccessNode {
    /// A node with no state and no children.
    pub fn new(id: u64, role: AccessRole, name: impl Into<String>, bounds: Rect) -> AccessNode {
        AccessNode {
            id,
            role,
            name: name.into(),
            help: None,
            bounds,
            enabled: true,
            focusable: false,
            selected: None,
            toggled: None,
            expanded: None,
            range: None,
            value: None,
            children: Vec::new(),
        }
    }

    /// The node with this identifier in this subtree.
    pub fn find(&self, id: u64) -> Option<&AccessNode> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// The parent of the node with this identifier in this subtree.
    pub fn parent_of(&self, id: u64) -> Option<&AccessNode> {
        if self.children.iter().any(|c| c.id == id) {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.parent_of(id))
    }
}

/// A request from an assistive tool on one element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccessAction {
    /// Activate (button, menu entry, tile).
    Invoke,
    /// Switch the on/off state.
    Toggle,
    /// Make this choice the selected one (tab, tile).
    Select,
    /// Show the section.
    Expand,
    /// Hide the section.
    Collapse,
    /// Set the slider to this value.
    SetValue(f64),
    /// Give the element the keyboard focus.
    Focus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_are_found_with_their_parent() {
        let mut root =
            AccessNode::new(1, AccessRole::Pane, "panel", Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut list = AccessNode::new(2, AccessRole::List, "", Rect::new(0.0, 0.0, 10.0, 5.0));
        list.children.push(AccessNode::new(3, AccessRole::ListItem, "a", Rect::default()));
        root.children.push(list);
        assert_eq!(root.find(3).map(|n| n.name.as_str()), Some("a"));
        assert_eq!(root.parent_of(3).map(|n| n.id), Some(2));
        assert_eq!(root.parent_of(1), None);
        assert!(root.find(9).is_none());
    }
}
