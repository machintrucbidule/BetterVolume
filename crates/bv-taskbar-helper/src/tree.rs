//! The work done inside the taskbar once the TAP is connected: follow the streamed XAML
//! visual tree, spot the system tray's volume icon by the glyph it draws, and collapse or
//! restore it. Every operation is fail-safe: if anything expected is missing, it leaves
//! the tree untouched (guard rail, ARCHITECTURE section 7).
//!
//! Identification does not depend on the Windows display language: each Windows 11
//! tray icon is a `SystemTray.IconView` named "SystemTrayIcon"; a font icon draws its glyph
//! in a `TextBlock` named "InnerTextBlock" inside a `SystemTray.TextIconContent`, and the
//! volume icon is the one whose glyph is one of the Windows volume glyphs (the same rule the
//! published taskbar tweak tools use). The pure part of that rule is [`is_volume_glyph`] and
//! [`Ancestry`], unit-tested; the rest reads the live tree.

use std::collections::HashMap;

use windows::core::{BSTR, PCWSTR};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::GetCurrentThreadId;

use crate::com::borrowed_wide;
use crate::xaml::{
    IVisualTreeService, InstanceHandle, ParentChildRelation, PropertyChainSource,
    PropertyChainValue, VisualElement, MUTATION_ADD, MUTATION_REMOVE,
};

/// What a freshly loaded TAP was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Collapse the Windows volume tray icon.
    Hide,
    /// Make it visible again.
    Restore,
}

/// The initialisation string of a load: `"<hide|restore> <owner process id> <owner window>"`.
/// The owner is the BetterVolume process (the TAP restores the icon when it ends) and its
/// message window (where the outcome is reported); 0 means none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction {
    /// What to do.
    pub command: Command,
    /// Process to watch while the icon is hidden (0 = none).
    pub owner_pid: u32,
    /// Window told the outcome (0 = none).
    pub owner_window: isize,
}

impl Instruction {
    /// Reads an initialisation string; anything unexpected gives `None` (nothing is done).
    pub fn parse(data: &str) -> Option<Instruction> {
        let mut parts = data.split_whitespace();
        let command = match parts.next()? {
            "hide" => Command::Hide,
            "restore" => Command::Restore,
            _ => return None,
        };
        let owner_pid = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let owner_window = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        if parts.next().is_some() {
            return None;
        }
        Some(Instruction { command, owner_pid, owner_window })
    }
}

/// The Windows volume glyphs of the tray icon (Segoe Fluent Icons / Segoe MDL2 Assets):
/// muted, the four level steps, volume disabled (no device) and the bars variant.
const VOLUME_GLYPHS: [char; 7] =
    ['\u{E74F}', '\u{E992}', '\u{E993}', '\u{E994}', '\u{E995}', '\u{EA85}', '\u{EBC5}'];

/// Whether a tray icon's glyph text is the volume icon: exactly one character, one of the
/// volume glyphs.
pub fn is_volume_glyph(text: &str) -> bool {
    let mut chars = text.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if VOLUME_GLYPHS.contains(&c))
}

/// The elements of the tray that matter, by type and name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A tray icon (`SystemTray.IconView` named "SystemTrayIcon").
    TrayIcon,
    /// The content of a font-drawn icon (`SystemTray.TextIconContent`).
    TextIcon,
    /// The glyph of such an icon (`TextBlock` named "InnerTextBlock").
    Glyph,
    /// Anything else.
    Other,
}

impl Kind {
    /// The kind of a streamed element from its full type name and its name.
    pub fn of(type_name: &str, name: &str) -> Kind {
        if type_name == "SystemTray.IconView" && name == "SystemTrayIcon" {
            Kind::TrayIcon
        } else if type_name == "SystemTray.TextIconContent" {
            Kind::TextIcon
        } else if type_name == "Windows.UI.Xaml.Controls.TextBlock" && name == "InnerTextBlock" {
            Kind::Glyph
        } else {
            Kind::Other
        }
    }
}

/// How far above a glyph its tray icon may be (the Windows 11 tray nests it 6 or 7 levels
/// down; a margin is kept for small layout changes).
const MAX_DEPTH: usize = 12;

/// The ancestors of an element, nearest first, as kinds and handles.
pub struct Ancestry<'a>(pub &'a [(Kind, InstanceHandle)]);

impl Ancestry<'_> {
    /// The tray icon a glyph belongs to: the nearest tray icon above it, provided a
    /// font-icon content lies between them (a glyph elsewhere is not a tray icon's).
    pub fn tray_icon(&self) -> Option<InstanceHandle> {
        let mut text_icon = false;
        for (kind, handle) in self.0.iter().take(MAX_DEPTH) {
            match kind {
                Kind::TextIcon => text_icon = true,
                Kind::TrayIcon => return text_icon.then_some(*handle),
                _ => {}
            }
        }
        None
    }
}

/// One element of the tree as far as we follow it.
#[derive(Clone, Copy)]
struct Node {
    parent: InstanceHandle,
    /// Its position among its parent's children.
    index: u32,
    kind: Kind,
}

/// A glyph whose text was not set yet when it was added: looked at again on later
/// callbacks of the same thread (its handle is only usable there).
#[derive(Clone, Copy)]
struct Pending {
    glyph: InstanceHandle,
    thread: u32,
}

/// Most glyphs kept for a later look (the tray has a dozen icons at most).
const MAX_PENDING: usize = 32;

/// The padding that takes back (or gives back) the spacing a horizontal stack keeps next
/// to the hidden icon's empty slot. A collapsed child still gets the stack's
/// spacing, which would leave a gap of a few pixels where the icon was; an empty slot
/// cannot shrink below nothing (and a negative margin on a neighbour was tried live: it
/// does not move the icons), so the stack's own padding on that side gives it back. With
/// the hidden icon last (or first) of at least two, the stack is then exactly as wide, its
/// icons exactly where they would be, as without that icon.
///
/// The stack's padding is Windows' own, set in its markup, so it cannot simply be cleared
/// afterwards: the change is recognised by its shape instead. Windows' padding is
/// symmetric; hiding reduces the side next to the empty slot by the spacing, and only a
/// symmetric padding is reduced; restoring only grows a side that is exactly the spacing
/// short of the other. So repeating either command never shifts anything, and a stack
/// Windows rebuilt (symmetric again) is left alone.
///
/// `padding` and `spacing` are the stack's values in XAML text form; `index` is the hidden
/// icon's position among the `count` children. `None` = leave the padding as it is.
pub fn gap_padding(
    padding: &str,
    spacing: &str,
    index: u32,
    count: u32,
    command: Command,
) -> Option<String> {
    const EPSILON: f64 = 0.001;
    let spacing: f64 = spacing.trim().parse().ok()?;
    if !(spacing > 0.0 && spacing < 100.0) || count < 2 || index >= count {
        return None;
    }
    let mut sides = parse_thickness(padding)?;
    // A thickness is (left, top, right, bottom): the side next to the empty slot.
    let (side, other) = if index == count - 1 {
        (2, 0)
    } else if index == 0 {
        (0, 2)
    } else {
        return None;
    };
    match command {
        Command::Hide => {
            let symmetric = (sides[side] - sides[other]).abs() < EPSILON;
            if !symmetric || sides[side] < spacing - EPSILON {
                return None;
            }
            sides[side] = (sides[side] - spacing).max(0.0);
        }
        Command::Restore => {
            let reduced = (sides[other] - sides[side] - spacing).abs() < EPSILON;
            if !reduced {
                return None;
            }
            sides[side] = sides[other];
        }
    }
    Some(sides.iter().map(|v| format_number(*v)).collect::<Vec<_>>().join(","))
}

/// A XAML thickness in text form: one value (all sides), two (horizontal, vertical) or
/// four (left, top, right, bottom), separated by commas or spaces.
fn parse_thickness(text: &str) -> Option<[f64; 4]> {
    let values: Vec<f64> = text
        .split([',', ' '])
        .filter(|p| !p.is_empty())
        .map(|p| p.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<f64>>>()?;
    match values.as_slice() {
        [all] => Some([*all; 4]),
        [h, v] => Some([*h, *v, *h, *v]),
        [l, t, r, b] => Some([*l, *t, *r, *b]),
        _ => None,
    }
}

/// A number in the shortest XAML text form ("4", "2.5").
fn format_number(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    }
}

/// One entry of an element's property-value chain.
struct ChainEntry<'a> {
    name: &'a str,
    index: u32,
    value: &'a str,
    /// A value hidden by a stronger one.
    overridden: bool,
}

/// Live state of a connected TAP, behind the TAP's lock.
pub struct Worker {
    service: IVisualTreeService,
    command: Command,
    nodes: HashMap<InstanceHandle, Node>,
    pending: Vec<Pending>,
    /// Tray icons already handled (a tree streamed again reports them again).
    done: Vec<InstanceHandle>,
}

impl Worker {
    pub fn new(service: IVisualTreeService, command: Command) -> Self {
        Worker { service, command, nodes: HashMap::new(), pending: Vec::new(), done: Vec::new() }
    }

    /// Handle one streamed tree change. Returns whether the volume icon was handled by
    /// this call.
    ///
    /// # Safety
    /// `element` fields point to strings the host owns for the duration of the call; the
    /// call runs on the UI thread the element belongs to (the diagnostics guarantee it).
    pub unsafe fn on_change(
        &mut self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation: i32,
    ) -> bool {
        if mutation == MUTATION_REMOVE {
            self.nodes.remove(&element.handle);
            self.pending.retain(|p| p.glyph != element.handle);
            self.done.retain(|h| *h != element.handle);
            return false;
        }
        if mutation != MUTATION_ADD {
            return false;
        }
        // SAFETY: both point to caller-owned NUL-terminated strings valid for the call.
        let (type_name, name) =
            unsafe { (borrowed_wide(element.type_name), borrowed_wide(element.name)) };
        let kind = Kind::of(&type_name, &name);
        self.nodes.insert(
            element.handle,
            Node { parent: relation.parent, index: relation.child_index, kind },
        );

        // SAFETY: this callback runs on the thread of the elements it reports.
        let thread = unsafe { GetCurrentThreadId() };
        let mut handled = false;
        if kind == Kind::Glyph {
            // SAFETY: a live handle, on its own thread.
            handled = unsafe { self.look_at(element.handle, thread) };
        }
        // Glyphs whose text arrived since: the tree keeps streaming while the tray builds.
        if !self.pending.is_empty() {
            let waiting: Vec<Pending> = self.pending.clone();
            for p in waiting.into_iter().filter(|p| p.thread == thread) {
                // SAFETY: handles of this thread, still in the tree (removals drop them).
                handled |= unsafe { self.look_at(p.glyph, thread) };
            }
        }
        handled
    }

    /// Looks at a glyph: when it is the volume icon's, handles its tray icon. A glyph
    /// without text yet is kept for a later look.
    ///
    /// # Safety
    /// `glyph` must be a live handle of the current thread.
    unsafe fn look_at(&mut self, glyph: InstanceHandle, thread: u32) -> bool {
        // SAFETY: per the contract.
        let text = unsafe { self.effective(glyph, "Text") }.unwrap_or_default();
        self.pending.retain(|p| p.glyph != glyph);
        if text.is_empty() {
            if self.pending.len() < MAX_PENDING {
                self.pending.push(Pending { glyph, thread });
            }
            return false;
        }
        if !is_volume_glyph(&text) {
            return false;
        }
        let Some(icon) = self.tray_icon_of(glyph) else { return false };
        if self.done.contains(&icon) {
            return false;
        }
        // SAFETY: the tray icon is an ancestor of a live element of this thread.
        let ok = unsafe { self.apply(icon) };
        crate::com::trace(&format!("volume icon {icon}: {:?} -> {ok}", self.command));
        if ok {
            self.done.push(icon);
        }
        ok
    }

    /// Collapses the tray icon and gives back the spacing its empty slot keeps in the stack
    /// of icons (see [`gap_padding`]); or undoes both. Only the icon's visibility is
    /// required; the spacing is adjusted only when the layout is the expected one.
    /// Repeating a command changes nothing more.
    ///
    /// # Safety
    /// `icon` must be a live handle of the current thread.
    unsafe fn apply(&self, icon: InstanceHandle) -> bool {
        // SAFETY: per the contract (every handle below is the icon or an ancestor).
        unsafe {
            let collapsed = self.effective(icon, "Visibility").as_deref() == Some(COLLAPSED);
            let ok = match self.command {
                Command::Hide => {
                    collapsed || self.set_property(icon, "Visibility", VISIBILITY_TYPE, "Collapsed")
                }
                // Clearing our value lets Windows' own apply (Visible); a value of Windows'
                // own (none was seen) is replaced by Visible.
                Command::Restore => {
                    !collapsed
                        || self.clear_property(icon, "Visibility")
                        || self.set_property(icon, "Visibility", VISIBILITY_TYPE, "Visible")
                }
            };
            if !ok {
                return false;
            }
            let stack = self
                .nodes
                .get(&icon)
                .and_then(|i| self.nodes.get(&i.parent).map(|c| (c.parent, c.index)));
            if let Some((stack, index)) = stack {
                if let Some(padding) = self.gap_padding_of(stack, index) {
                    let set = self.set_property(stack, "Padding", THICKNESS_TYPE, &padding);
                    crate::com::trace(&format!("stack padding {padding} -> {set}"));
                }
            }
            true
        }
    }

    /// The padding to give the stack of icons for the command, if the stack is the
    /// expected horizontal one (see [`gap_padding`]).
    ///
    /// # Safety
    /// `stack` must be a live handle of the current thread.
    unsafe fn gap_padding_of(&self, stack: InstanceHandle, index: u32) -> Option<String> {
        // SAFETY: per the contract.
        unsafe {
            if self.effective(stack, "Orientation").as_deref() != Some(HORIZONTAL) {
                return None;
            }
            let spacing = self.effective(stack, "Spacing")?;
            let padding = self.effective(stack, "Padding")?;
            let count = self.nodes.values().filter(|n| n.parent == stack).count() as u32;
            gap_padding(&padding, &spacing, index, count, self.command)
        }
    }

    /// The tray icon above a glyph, from the parents seen so far.
    fn tray_icon_of(&self, glyph: InstanceHandle) -> Option<InstanceHandle> {
        let mut chain = Vec::with_capacity(MAX_DEPTH);
        let mut at = self.nodes.get(&glyph)?.parent;
        while chain.len() < MAX_DEPTH {
            let Some(node) = self.nodes.get(&at) else { break };
            chain.push((node.kind, at));
            at = node.parent;
        }
        Ancestry(&chain).tray_icon()
    }

    /// Forgets the tree (the connection ends).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.pending.clear();
    }

    /// The property-value chain of an element, visited entry by entry; the arrays are
    /// freed afterwards.
    ///
    /// # Safety
    /// `handle` must come from the connected tree, on its thread.
    unsafe fn with_chain(&self, handle: InstanceHandle, visit: &mut dyn FnMut(&ChainEntry<'_>)) {
        let mut source_count = 0u32;
        let mut sources: *mut PropertyChainSource = core::ptr::null_mut();
        let mut value_count = 0u32;
        let mut values: *mut PropertyChainValue = core::ptr::null_mut();
        // SAFETY: out-pointers are valid; the callee allocates the arrays.
        let hr = unsafe {
            self.service.GetPropertyValuesChain(
                handle,
                &mut source_count,
                &mut sources,
                &mut value_count,
                &mut values,
            )
        };
        if hr.is_err() {
            return;
        }
        // SAFETY: the callee reports value_count valid entries (none if the array is null).
        unsafe {
            if !values.is_null() {
                for i in 0..value_count as usize {
                    let v = &*values.add(i);
                    let name = borrowed_wide(v.property_name);
                    let value = borrowed_wide(v.value);
                    visit(&ChainEntry {
                        name: &name,
                        index: v.index,
                        value: &value,
                        overridden: v.overridden != 0,
                    });
                }
            }
            free_value_chain(sources, source_count, values, value_count);
        }
    }

    /// The value a named property of an element has (its entry that no other overrides),
    /// in text form, or `None`.
    ///
    /// # Safety
    /// `handle` must come from the connected tree, on its thread.
    unsafe fn effective(&self, handle: InstanceHandle, property: &str) -> Option<String> {
        let mut found = None;
        // SAFETY: per the contract.
        unsafe {
            self.with_chain(handle, &mut |e| {
                if found.is_none() && e.name == property && !e.overridden {
                    found = Some(e.value.to_owned());
                }
            });
        }
        found
    }

    /// Removes the local value of a named property. Returns whether it was removed.
    ///
    /// # Safety
    /// `handle` must come from the connected tree, on its thread.
    unsafe fn clear_property(&self, handle: InstanceHandle, property: &str) -> bool {
        // SAFETY: per the contract.
        let Some(index) = (unsafe { self.property_index(handle, property) }) else {
            return false;
        };
        // SAFETY: a live handle and an index from its own chain.
        unsafe { self.service.ClearProperty(handle, index) }.is_ok()
    }

    /// The index of a named property, from the element's own chain (`GetPropertyIndex`
    /// refuses the names on the taskbar).
    ///
    /// # Safety
    /// `handle` must come from the connected tree, on its thread.
    unsafe fn property_index(&self, handle: InstanceHandle, property: &str) -> Option<u32> {
        let mut index = None;
        // SAFETY: per the contract.
        unsafe {
            self.with_chain(handle, &mut |e| {
                if index.is_none() && e.name == property {
                    index = Some(e.index);
                }
            });
        }
        index
    }

    /// Set a property of an element to a value of `type_name` made from `value` (the
    /// XAML text form). Returns whether it was applied; any failure leaves the element
    /// untouched (fail-safe).
    ///
    /// # Safety
    /// `element` must be a live handle from the connected tree, on its thread.
    unsafe fn set_property(
        &self,
        element: InstanceHandle,
        property: &str,
        type_name: &str,
        value: &str,
    ) -> bool {
        // SAFETY: per the contract.
        let Some(index) = (unsafe { self.property_index(element, property) }) else {
            return false;
        };
        let type_name = wide(type_name);
        let value = wide(value);
        let mut value_handle: InstanceHandle = 0;
        // SAFETY: valid wide strings and out-pointer.
        let hr = unsafe {
            self.service.CreateInstance(
                PCWSTR(type_name.as_ptr()),
                PCWSTR(value.as_ptr()),
                &mut value_handle,
            )
        };
        if hr.is_err() || value_handle == 0 {
            return false;
        }
        // SAFETY: both handles are live; the index came from this element's property chain.
        unsafe { self.service.SetProperty(element, value_handle, index) }.is_ok()
    }
}

/// Full type names of the values the helper sets (the taskbar hosts system XAML).
const VISIBILITY_TYPE: &str = "Windows.UI.Xaml.Visibility";
const THICKNESS_TYPE: &str = "Windows.UI.Xaml.Thickness";
/// `Visibility.Collapsed` and `Orientation.Horizontal` as the chain shows them.
const COLLAPSED: &str = "1";
const HORIZONTAL: &str = "1";

/// Free the two arrays `GetPropertyValuesChain` allocated and every `BSTR` inside them.
///
/// # Safety
/// The pointers and counts must be exactly what `GetPropertyValuesChain` returned.
unsafe fn free_value_chain(
    sources: *mut crate::xaml::PropertyChainSource,
    source_count: u32,
    values: *mut PropertyChainValue,
    value_count: u32,
) {
    // SAFETY: adopt each BSTR so its Drop frees it; then free the arrays themselves.
    unsafe {
        if !values.is_null() {
            for i in 0..value_count as usize {
                let v = &mut *values.add(i);
                for p in [
                    &mut v.type_name,
                    &mut v.declaring_type,
                    &mut v.value_type,
                    &mut v.item_type,
                    &mut v.value,
                    &mut v.property_name,
                ] {
                    adopt_bstr(*p);
                    *p = core::ptr::null_mut();
                }
            }
            CoTaskMemFree(Some(values as *const core::ffi::c_void));
        }
        if !sources.is_null() {
            for i in 0..source_count as usize {
                let s = &mut *sources.add(i);
                for p in [
                    &mut s.target_type,
                    &mut s.name,
                    &mut s.src_info.file_name,
                    &mut s.src_info.hash,
                ] {
                    adopt_bstr(*p);
                    *p = core::ptr::null_mut();
                }
            }
            CoTaskMemFree(Some(sources as *const core::ffi::c_void));
        }
    }
}

/// Free a `BSTR` by raw pointer (adopting it so its Drop calls `SysFreeString`).
///
/// # Safety
/// `p` must be null or a `BSTR` the caller owns and no longer uses.
unsafe fn adopt_bstr(p: *mut u16) {
    if !p.is_null() {
        // SAFETY: the caller transfers ownership of a real BSTR.
        drop(unsafe { BSTR::from_raw(p) });
    }
}

/// A NUL-terminated UTF-16 buffer for a `PCWSTR` argument.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_volume_glyph_is_recognised() {
        for glyph in VOLUME_GLYPHS {
            assert!(is_volume_glyph(&glyph.to_string()), "{:X}", glyph as u32);
        }
    }

    #[test]
    fn other_tray_glyphs_and_texts_are_left_alone() {
        // Network (Wi-Fi, Ethernet), battery, microphone, location, and plain text.
        for text in ["\u{E701}", "\u{E839}", "\u{EAA5}", "\u{E3C1}", "\u{F8C0}", "\u{E81D}"] {
            assert!(!is_volume_glyph(text), "{:X}", text.chars().next().map_or(0, |c| c as u32));
        }
        assert!(!is_volume_glyph(""));
        assert!(!is_volume_glyph("Volume"));
        // Two glyphs are not one icon.
        assert!(!is_volume_glyph("\u{E995}\u{E995}"));
    }

    #[test]
    fn instructions_parse() {
        assert_eq!(
            Instruction::parse("hide 1234 5678"),
            Some(Instruction { command: Command::Hide, owner_pid: 1234, owner_window: 5678 })
        );
        assert_eq!(
            Instruction::parse(" restore "),
            Some(Instruction { command: Command::Restore, owner_pid: 0, owner_window: 0 })
        );
        for bad in ["", "whatever", "hide x", "hide 1 2 3", "hide -1 0", "Hide 1 2"] {
            assert_eq!(Instruction::parse(bad), None, "{bad:?}");
        }
    }

    fn hide(padding: &str, spacing: &str, index: u32, count: u32) -> Option<String> {
        gap_padding(padding, spacing, index, count, Command::Hide)
    }

    fn restore(padding: &str, spacing: &str, index: u32, count: u32) -> Option<String> {
        gap_padding(padding, spacing, index, count, Command::Restore)
    }

    #[test]
    fn the_gap_of_a_hidden_last_icon_comes_off_the_right_padding() {
        // The Windows 11 quick-settings button: network then volume, padding 4,0,4,0,
        // spacing 4 (read on the live taskbar).
        assert_eq!(hide("4,0,4,0", "4", 1, 2).as_deref(), Some("4,0,0,0"));
        assert_eq!(hide("6,0,6,0", "4", 2, 3).as_deref(), Some("6,0,2,0"));
        assert_eq!(hide("4.5,0,4.5,0", "2.5", 1, 2).as_deref(), Some("4.5,0,2,0"));
    }

    #[test]
    fn the_gap_of_a_hidden_first_icon_comes_off_the_left_padding() {
        assert_eq!(hide("4,0,4,0", "4", 0, 2).as_deref(), Some("0,0,4,0"));
        // One- and two-value thickness forms.
        assert_eq!(hide("4", "4", 0, 3).as_deref(), Some("0,4,4,4"));
        assert_eq!(hide("6 2", "4", 0, 2).as_deref(), Some("2,2,6,2"));
    }

    #[test]
    fn restoring_gives_back_exactly_what_hiding_took() {
        for (padding, spacing, index, count) in
            [("4,0,4,0", "4", 1, 2), ("4,0,4,0", "4", 0, 2), ("5.5,1,5.5,1", "2.5", 2, 3)]
        {
            let hidden = hide(padding, spacing, index, count).expect("hidden");
            let restored = restore(&hidden, spacing, index, count).expect("restored");
            assert_eq!(parse_thickness(&restored), parse_thickness(padding), "{padding}");
        }
    }

    #[test]
    fn repeating_a_command_changes_nothing_more() {
        // Hiding again: the padding is no longer symmetric.
        assert_eq!(hide("4,0,0,0", "4", 1, 2), None);
        // Restoring a padding that was never reduced (or a stack Windows rebuilt).
        assert_eq!(restore("4,0,4,0", "4", 1, 2), None);
        // A padding reduced by something else is not "restored".
        assert_eq!(restore("4,0,2,0", "4", 1, 2), None);
    }

    #[test]
    fn other_layouts_are_left_alone() {
        // A middle icon, a lone icon, too little padding, asymmetric padding, no spacing,
        // an index out of range, unreadable values.
        assert_eq!(hide("4,0,4,0", "4", 1, 3), None);
        assert_eq!(hide("4,0,4,0", "4", 0, 1), None);
        assert_eq!(hide("2,0,2,0", "4", 1, 2), None);
        assert_eq!(hide("8,0,4,0", "4", 1, 2), None);
        assert_eq!(hide("4,0,4,0", "0", 1, 2), None);
        assert_eq!(hide("4,0,4,0", "4", 5, 2), None);
        for bad in ["", "a,b,c,d", "1,2,3", "NaN"] {
            assert_eq!(hide(bad, "4", 1, 2), None, "{bad:?}");
        }
        assert_eq!(hide("4,0,4,0", "x", 1, 2), None);
    }

    #[test]
    fn kinds_follow_type_and_name() {
        assert_eq!(Kind::of("SystemTray.IconView", "SystemTrayIcon"), Kind::TrayIcon);
        assert_eq!(Kind::of("SystemTray.IconView", "Other"), Kind::Other);
        assert_eq!(Kind::of("SystemTray.TextIconContent", ""), Kind::TextIcon);
        assert_eq!(Kind::of("Windows.UI.Xaml.Controls.TextBlock", "InnerTextBlock"), Kind::Glyph);
        assert_eq!(Kind::of("Windows.UI.Xaml.Controls.TextBlock", "Label"), Kind::Other);
    }

    #[test]
    fn a_glyph_belongs_to_the_nearest_tray_icon_through_its_font_content() {
        // InnerTextBlock < Base < ContainerGrid < TextIconContent < ContentGrid <
        // ContainerGrid < IconView (the Windows 11 tray layout).
        let chain = [
            (Kind::Other, 1),
            (Kind::Other, 2),
            (Kind::TextIcon, 3),
            (Kind::Other, 4),
            (Kind::Other, 5),
            (Kind::TrayIcon, 6),
            (Kind::TrayIcon, 7),
        ];
        assert_eq!(Ancestry(&chain).tray_icon(), Some(6));
    }

    #[test]
    fn a_glyph_outside_a_font_icon_content_is_not_an_icon() {
        let chain = [(Kind::Other, 1), (Kind::TrayIcon, 2), (Kind::TextIcon, 3)];
        assert_eq!(Ancestry(&chain).tray_icon(), None);
        assert_eq!(Ancestry(&[]).tray_icon(), None);
    }

    #[test]
    fn a_tray_icon_too_far_above_is_not_its_own() {
        let mut chain = vec![(Kind::TextIcon, 0)];
        chain.extend((1..MAX_DEPTH as u64).map(|h| (Kind::Other, h)));
        chain.push((Kind::TrayIcon, 99));
        assert_eq!(Ancestry(&chain).tray_icon(), None);
    }
}
