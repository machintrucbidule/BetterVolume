//! Screen-reader exposure of the popups (FR-024): a Windows UI Automation provider
//! for a design-system accessibility tree ([`bv_ui_kit::access`]).
//!
//! A popup answers `WM_GETOBJECT` with [`on_get_object`], handing over the
//! [`AccessSource`] that describes its content. The providers given to Windows hold only
//! the window handle and an element identifier: every query re-reads the source's current
//! tree, so they never show stale content, and an element that disappeared answers
//! "element not available". Requests from assistive tools (invoke, toggle, set value...)
//! are passed to the source as [`AccessAction`]s.
//!
//! Threading: the providers ask for COM threading, so Windows delivers every call on the
//! UI thread (single-threaded apartment), from its message loop. The registry of sources
//! and providers is per thread; a call reaching another thread finds nothing and answers
//! "element not available" rather than touching state.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bv_ui_kit::access::{AccessAction, AccessNode, AccessRange, AccessRole};
use bv_ui_kit::{Point, Rect};
use windows::core::{implement, Error, IUnknown, Interface, Result, BOOL, HRESULT};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::*;

/// What a popup tells screen readers about its content.
pub(crate) trait AccessSource {
    /// The current tree of the surface (bounds in DIPs relative to the window's client
    /// area). `None`: not available right now (busy, hidden).
    fn tree(&self) -> Option<AccessNode>;
    /// Physical pixels per DIP of the surface.
    fn scale(&self) -> f32;
    /// Performs a request of an assistive tool; `false` if refused or the id is unknown.
    fn act(&self, id: u64, action: AccessAction) -> bool;
    /// The element that has the keyboard focus, if any.
    fn focused(&self) -> Option<u64>;
}

/// A state of an element that changed, for [`property_changed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccessChange {
    /// The value of a slider.
    Value,
    /// The on/off state of a toggle button.
    Toggled,
    /// The selection state of a tab or a list item.
    Selected,
    /// The shown/hidden state of an expander.
    Expanded,
}

/// The framework name reported to assistive tools.
const FRAMEWORK: &str = "BetterVolume";

thread_local! {
    /// The source of each window that answered `WM_GETOBJECT` (raw handle → source).
    static SOURCES: RefCell<HashMap<isize, Rc<dyn AccessSource>>> = RefCell::new(HashMap::new());
    /// The root provider of each window, created once (one COM identity per element).
    static ROOTS: RefCell<HashMap<isize, IRawElementProviderSimple>> =
        RefCell::new(HashMap::new());
    /// The providers of the other elements, by window and element identifier.
    static ITEMS: RefCell<HashMap<(isize, u64), IRawElementProviderSimple>> =
        RefCell::new(HashMap::new());
}

/// Answers `WM_GETOBJECT` for a popup: `Some(result)` when the request was for its UI
/// Automation root, `None` otherwise (pass the message to the default procedure).
pub(crate) fn on_get_object(
    window: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
    source: Rc<dyn AccessSource>,
) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    let key = window.0 as isize;
    SOURCES.with(|s| s.borrow_mut().insert(key, source));
    let root = root_provider(key);
    // SAFETY: our own window; `root` is a valid provider; the parameters are those of the
    // message being answered.
    Some(unsafe { UiaReturnRawElementProvider(window, wparam, lparam, &root) })
}

/// The keyboard focus moved to element `id` (`None`: to the surface itself). Raised only
/// when an assistive tool listens.
pub(crate) fn focus_changed(window: HWND, id: Option<u64>) {
    if !listening() {
        return;
    }
    let key = window.0 as isize;
    let Some(tree) = source_of(key).and_then(|s| s.tree()) else { return };
    let provider = match id {
        Some(id) if id != tree.id && tree.find(id).is_some() => item_provider(key, id),
        _ => root_provider(key),
    };
    // SAFETY: a valid provider of this thread.
    let _ = unsafe { UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId) };
}

/// A state of element `id` changed. Raised only when an assistive tool listens.
pub(crate) fn property_changed(window: HWND, id: u64, change: AccessChange) {
    if !listening() {
        return;
    }
    let key = window.0 as isize;
    let Some(tree) = source_of(key).and_then(|s| s.tree()) else { return };
    let Some(node) = tree.find(id) else { return };
    let provider = if id == tree.id { root_provider(key) } else { item_provider(key, id) };
    let (property, value) = match change {
        AccessChange::Value => match node.range {
            Some(range) => (UIA_RangeValueValuePropertyId, VARIANT::from(range.value)),
            None => return,
        },
        AccessChange::Toggled => {
            (UIA_ToggleToggleStatePropertyId, VARIANT::from(toggle_state(node.toggled).0))
        }
        AccessChange::Selected => {
            (UIA_SelectionItemIsSelectedPropertyId, VARIANT::from(node.selected == Some(true)))
        }
        AccessChange::Expanded => (
            UIA_ExpandCollapseExpandCollapseStatePropertyId,
            VARIANT::from(expand_state(node.expanded).0),
        ),
    };
    let old = VARIANT::default();
    // SAFETY: a valid provider of this thread; both values outlive the call.
    let _ = unsafe { UiaRaiseAutomationPropertyChangedEvent(&provider, property, &old, &value) };
    if change == AccessChange::Selected && node.selected == Some(true) {
        // SAFETY: as above.
        let _ =
            unsafe { UiaRaiseAutomationEvent(&provider, UIA_SelectionItem_ElementSelectedEventId) };
    }
}

/// The structure of the tree changed (elements added or removed): assistive tools re-read
/// the children of the surface. Providers of elements that are gone are disconnected.
pub(crate) fn structure_changed(window: HWND) {
    let key = window.0 as isize;
    let tree = source_of(key).and_then(|s| s.tree());
    // Elements no longer in the tree: their providers are disconnected and forgotten.
    let gone: Vec<IRawElementProviderSimple> = ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        let stale: Vec<(isize, u64)> = items
            .keys()
            .filter(|(w, id)| *w == key && tree.as_ref().is_none_or(|t| t.find(*id).is_none()))
            .copied()
            .collect();
        stale.into_iter().filter_map(|k| items.remove(&k)).collect()
    });
    for provider in gone {
        // SAFETY: a provider created by this module on this thread.
        let _ = unsafe { UiaDisconnectProvider(&provider) };
    }
    if !listening() {
        return;
    }
    let Some(tree) = tree else { return };
    let root = root_provider(key);
    let mut runtime = runtime_id(tree.id);
    // SAFETY: a valid provider; the runtime id buffer outlives the call.
    let _ = unsafe {
        UiaRaiseStructureChangedEvent(
            &root,
            StructureChangeType_ChildrenInvalidated,
            runtime.as_mut_ptr(),
            runtime.len() as i32,
        )
    };
}

/// The window is going away: tells UI Automation, and disconnects and forgets every
/// provider of the window. Call it before destroying the window.
pub(crate) fn disconnect(window: HWND) {
    let key = window.0 as isize;
    // SAFETY: documented way to release the providers handed to UI Automation for a window.
    unsafe { UiaReturnRawElementProvider(window, WPARAM(0), LPARAM(0), None) };
    let mut providers: Vec<IRawElementProviderSimple> = ITEMS.with(|items| {
        let mut items = items.borrow_mut();
        let keys: Vec<(isize, u64)> = items.keys().filter(|(w, _)| *w == key).copied().collect();
        keys.into_iter().filter_map(|k| items.remove(&k)).collect()
    });
    providers.extend(ROOTS.with(|r| r.borrow_mut().remove(&key)));
    for provider in providers {
        // SAFETY: a provider created by this module on this thread.
        let _ = unsafe { UiaDisconnectProvider(&provider) };
    }
    let source = SOURCES.with(|s| s.borrow_mut().remove(&key));
    drop(source);
}

fn listening() -> bool {
    // SAFETY: plain query.
    unsafe { UiaClientsAreListening() }.as_bool()
}

fn source_of(window: isize) -> Option<Rc<dyn AccessSource>> {
    // Cloned out: the source is called after the registry borrow ended.
    SOURCES.with(|s| s.borrow().get(&window).cloned())
}

fn root_provider(window: isize) -> IRawElementProviderSimple {
    if let Some(root) = ROOTS.with(|r| r.borrow().get(&window).cloned()) {
        return root;
    }
    let root: IRawElementProviderSimple = Root { window }.into();
    ROOTS.with(|r| r.borrow_mut().insert(window, root.clone()));
    root
}

fn item_provider(window: isize, id: u64) -> IRawElementProviderSimple {
    if let Some(item) = ITEMS.with(|i| i.borrow().get(&(window, id)).cloned()) {
        return item;
    }
    let item: IRawElementProviderSimple = Item { window, id }.into();
    ITEMS.with(|i| i.borrow_mut().insert((window, id), item.clone()));
    item
}

/// The provider of element `id` of `tree` (the root provider for the tree's root).
fn provider(window: isize, tree: &AccessNode, id: u64) -> IRawElementProviderSimple {
    if id == tree.id {
        root_provider(window)
    } else {
        item_provider(window, id)
    }
}

fn fragment(window: isize, tree: &AccessNode, id: u64) -> Result<IRawElementProviderFragment> {
    provider(window, tree, id).cast()
}

fn hresult(code: u32) -> Error {
    Error::from_hresult(HRESULT(code as i32))
}

fn not_available() -> Error {
    hresult(UIA_E_ELEMENTNOTAVAILABLE)
}

/// "No element": a successful call returning a null interface.
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

/// The source and current tree of a window, or "element not available".
fn current(window: isize) -> Result<(Rc<dyn AccessSource>, AccessNode)> {
    let source = source_of(window).ok_or_else(not_available)?;
    let tree = source.tree().ok_or_else(not_available)?;
    Ok((source, tree))
}

// ----- Pure helpers (tested) -------------------------------------------------------------

/// The UI Automation control type of a role.
fn control_type(role: AccessRole) -> UIA_CONTROLTYPE_ID {
    match role {
        AccessRole::Pane => UIA_PaneControlTypeId,
        AccessRole::List => UIA_ListControlTypeId,
        AccessRole::ListItem => UIA_ListItemControlTypeId,
        AccessRole::TabList => UIA_TabControlTypeId,
        AccessRole::Tab => UIA_TabItemControlTypeId,
        AccessRole::Button | AccessRole::ToggleButton | AccessRole::Expander => {
            UIA_ButtonControlTypeId
        }
        AccessRole::Slider => UIA_SliderControlTypeId,
        AccessRole::Group => UIA_GroupControlTypeId,
        AccessRole::Text => UIA_TextControlTypeId,
        AccessRole::Menu => UIA_MenuControlTypeId,
        AccessRole::MenuItem => UIA_MenuItemControlTypeId,
        AccessRole::Separator => UIA_SeparatorControlTypeId,
        AccessRole::ComboBox => UIA_ComboBoxControlTypeId,
        AccessRole::Hyperlink => UIA_HyperlinkControlTypeId,
        AccessRole::Edit => UIA_EditControlTypeId,
    }
}

/// Whether an element supports a control pattern.
fn supports(node: &AccessNode, pattern: UIA_PATTERN_ID) -> bool {
    match node.role {
        AccessRole::Button | AccessRole::MenuItem | AccessRole::Hyperlink => {
            pattern == UIA_InvokePatternId
        }
        AccessRole::Edit => pattern == UIA_ValuePatternId,
        AccessRole::ComboBox => {
            pattern == UIA_ExpandCollapsePatternId || pattern == UIA_SelectionPatternId
        }
        AccessRole::ListItem => {
            pattern == UIA_InvokePatternId || pattern == UIA_SelectionItemPatternId
        }
        AccessRole::Tab => pattern == UIA_SelectionItemPatternId,
        AccessRole::List | AccessRole::TabList => pattern == UIA_SelectionPatternId,
        AccessRole::ToggleButton => pattern == UIA_TogglePatternId,
        AccessRole::Expander => pattern == UIA_ExpandCollapsePatternId,
        AccessRole::Slider => pattern == UIA_RangeValuePatternId && node.range.is_some(),
        AccessRole::Pane | AccessRole::Group | AccessRole::Text | AccessRole::Menu => false,
        AccessRole::Separator => false,
    }
}

/// Where navigation from element `id` leads, as an element identifier.
// Windows constant names in patterns.
#[allow(non_upper_case_globals)]
fn navigate(tree: &AccessNode, id: u64, direction: NavigateDirection) -> Option<u64> {
    let node = tree.find(id)?;
    match direction {
        NavigateDirection_FirstChild => node.children.first().map(|c| c.id),
        NavigateDirection_LastChild => node.children.last().map(|c| c.id),
        NavigateDirection_Parent => tree.parent_of(id).map(|p| p.id),
        NavigateDirection_NextSibling | NavigateDirection_PreviousSibling => {
            let siblings = &tree.parent_of(id)?.children;
            let index = siblings.iter().position(|c| c.id == id)?;
            let other = if direction == NavigateDirection_NextSibling {
                index.checked_add(1)?
            } else {
                index.checked_sub(1)?
            };
            siblings.get(other).map(|c| c.id)
        }
        _ => None,
    }
}

/// The deepest element whose bounds contain `point` (DIPs), the root if none does.
fn hit(tree: &AccessNode, point: Point) -> u64 {
    fn deepest(node: &AccessNode, point: Point) -> Option<u64> {
        if !node.bounds.contains(point) {
            return None;
        }
        // Later children are drawn on top: test them first.
        node.children.iter().rev().find_map(|c| deepest(c, point)).or(Some(node.id))
    }
    tree.children.iter().rev().find_map(|c| deepest(c, point)).unwrap_or(tree.id)
}

/// The runtime identifier of an element: unique within the window's fragment.
fn runtime_id(id: u64) -> [i32; 3] {
    [UiaAppendRuntimeId as i32, (id & 0xFFFF_FFFF) as u32 as i32, (id >> 32) as u32 as i32]
}

/// Whether an element lies outside the visible surface (scrolled away).
fn offscreen(tree: &AccessNode, node: &AccessNode) -> bool {
    let (a, b) = (tree.bounds, node.bounds);
    b.width <= 0.0
        || b.height <= 0.0
        || b.right() <= a.x
        || b.x >= a.right()
        || b.bottom() <= a.y
        || b.y >= a.bottom()
}

fn toggle_state(toggled: Option<bool>) -> ToggleState {
    if toggled == Some(true) {
        ToggleState_On
    } else {
        ToggleState_Off
    }
}

fn expand_state(expanded: Option<bool>) -> ExpandCollapseState {
    if expanded == Some(true) {
        ExpandCollapseState_Expanded
    } else {
        ExpandCollapseState_Collapsed
    }
}

/// The value an assistive tool may set: inside the range, or refused.
fn checked_value(range: AccessRange, value: f64) -> Option<f64> {
    (value.is_finite() && value >= range.min && value <= range.max).then_some(value)
}

/// A property of an element, `None` when the element does not define it.
#[allow(non_upper_case_globals)]
fn property(
    tree: &AccessNode,
    node: &AccessNode,
    focused: Option<u64>,
    id: UIA_PROPERTY_ID,
) -> Option<VARIANT> {
    Some(match id {
        UIA_ControlTypePropertyId => VARIANT::from(control_type(node.role).0),
        UIA_NamePropertyId => VARIANT::from(node.name.as_str()),
        UIA_HelpTextPropertyId => VARIANT::from(node.help.as_deref()?),
        UIA_IsEnabledPropertyId => VARIANT::from(node.enabled),
        UIA_IsKeyboardFocusablePropertyId => VARIANT::from(node.focusable && node.enabled),
        UIA_HasKeyboardFocusPropertyId => VARIANT::from(focused == Some(node.id)),
        UIA_AutomationIdPropertyId => VARIANT::from(format!("bv-{}", node.id).as_str()),
        UIA_FrameworkIdPropertyId => VARIANT::from(FRAMEWORK),
        UIA_IsControlElementPropertyId => VARIANT::from(true),
        UIA_IsContentElementPropertyId => VARIANT::from(node.role != AccessRole::Separator),
        UIA_IsOffscreenPropertyId if node.id != tree.id => VARIANT::from(offscreen(tree, node)),
        _ => return None,
    })
}

// ----- Providers -------------------------------------------------------------------------

/// Options of every provider: in-process server side, calls delivered by COM on the UI
/// thread.
fn options() -> ProviderOptions {
    ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0)
}

fn bounds_on_screen(window: isize, scale: f32, bounds: Rect) -> UiaRect {
    let mut origin = POINT::default();
    // SAFETY: plain query; a stale handle only makes it fail (origin stays 0, 0).
    let _ = unsafe { ClientToScreen(HWND(window as *mut core::ffi::c_void), &mut origin) };
    let scale = f64::from(scale);
    UiaRect {
        left: f64::from(origin.x) + f64::from(bounds.x) * scale,
        top: f64::from(origin.y) + f64::from(bounds.y) * scale,
        width: f64::from(bounds.width) * scale,
        height: f64::from(bounds.height) * scale,
    }
}

fn property_value(window: isize, id: Option<u64>, property_id: UIA_PROPERTY_ID) -> Result<VARIANT> {
    let (source, tree) = current(window)?;
    let node = match id {
        Some(id) => tree.find(id).ok_or_else(not_available)?,
        None => &tree,
    };
    Ok(property(&tree, node, source.focused(), property_id).unwrap_or_default())
}

#[allow(non_upper_case_globals)]
fn navigate_from(
    window: isize,
    id: Option<u64>,
    direction: NavigateDirection,
) -> Result<IRawElementProviderFragment> {
    let (_, tree) = current(window)?;
    let from = id.unwrap_or(tree.id);
    if tree.find(from).is_none() {
        return Err(not_available());
    }
    // The root's parent and siblings belong to the window it is hosted in.
    if from == tree.id
        && !matches!(direction, NavigateDirection_FirstChild | NavigateDirection_LastChild)
    {
        return none();
    }
    match navigate(&tree, from, direction) {
        Some(target) => fragment(window, &tree, target),
        None => none(),
    }
}

fn runtime_id_array(id: u64) -> Result<*mut SAFEARRAY> {
    let parts = runtime_id(id);
    // SAFETY: plain allocation of a one-dimensional array of 32-bit integers, handed to
    // the caller (UI Automation frees it).
    let array = unsafe { SafeArrayCreateVector(VT_I4, 0, parts.len() as u32) };
    if array.is_null() {
        return Err(Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY));
    }
    for (index, part) in parts.iter().enumerate() {
        let index = index as i32;
        // SAFETY: `array` holds `parts.len()` integers; the value is copied.
        unsafe { SafeArrayPutElement(array, &index, (part as *const i32).cast()) }?;
    }
    Ok(array)
}

/// The surface itself: the fragment root, hosted in the popup window.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct Root {
    window: isize,
}

impl IRawElementProviderSimple_Impl for Root_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        none()
    }

    fn GetPropertyValue(&self, property_id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        property_value(self.window, None, property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // SAFETY: plain call on our own window.
        unsafe { UiaHostProviderFromHwnd(HWND(self.window as *mut core::ffi::c_void)) }
    }
}

impl IRawElementProviderFragment_Impl for Root_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_from(self.window, None, direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // A root hosted in a window takes its identity from the window.
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        // Given by the host window.
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        root_provider(self.window).cast()
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let (source, tree) = current(self.window)?;
        let mut point = POINT { x: x as i32, y: y as i32 };
        // SAFETY: plain conversion on our own window.
        let _ = unsafe { ScreenToClient(HWND(self.window as *mut core::ffi::c_void), &mut point) };
        let scale = source.scale().max(0.1);
        let dip = Point::new(point.x as f32 / scale, point.y as f32 / scale);
        fragment(self.window, &tree, hit(&tree, dip))
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let (source, tree) = current(self.window)?;
        match source.focused() {
            Some(id) if id != tree.id && tree.find(id).is_some() => {
                fragment(self.window, &tree, id)
            }
            _ => none(),
        }
    }
}

/// Any element below the root.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider,
    IToggleProvider,
    ISelectionItemProvider,
    ISelectionProvider,
    IExpandCollapseProvider,
    IRangeValueProvider,
    IValueProvider
)]
struct Item {
    window: isize,
    id: u64,
}

impl Item_Impl {
    /// The element's current description.
    fn node(&self) -> Result<(Rc<dyn AccessSource>, AccessNode, AccessNode)> {
        let (source, tree) = current(self.window)?;
        let node = tree.find(self.id).cloned().ok_or_else(not_available)?;
        Ok((source, tree, node))
    }

    /// Passes a request to the source, if the element can take it.
    fn act(&self, action: AccessAction) -> Result<()> {
        let (source, _, node) = self.node()?;
        if !node.enabled {
            return Err(hresult(UIA_E_ELEMENTNOTENABLED));
        }
        if source.act(self.id, action) {
            Ok(())
        } else {
            Err(hresult(UIA_E_INVALIDOPERATION))
        }
    }

    fn range(&self) -> Result<AccessRange> {
        self.node()?.2.range.ok_or_else(|| hresult(UIA_E_INVALIDOPERATION))
    }
}

impl IRawElementProviderSimple_Impl for Item_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(options())
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        let (_, _, node) = self.node()?;
        if !supports(&node, pattern) {
            return none();
        }
        item_provider(self.window, self.id).cast()
    }

    fn GetPropertyValue(&self, property_id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        property_value(self.window, Some(self.id), property_id)
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for Item_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        navigate_from(self.window, Some(self.id), direction)
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        self.node()?;
        runtime_id_array(self.id)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let (source, tree, node) = self.node()?;
        if offscreen(&tree, &node) {
            return Ok(UiaRect::default());
        }
        Ok(bounds_on_screen(self.window, source.scale(), node.bounds))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        self.act(AccessAction::Focus)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        root_provider(self.window).cast()
    }
}

impl IInvokeProvider_Impl for Item_Impl {
    fn Invoke(&self) -> Result<()> {
        self.act(AccessAction::Invoke)
    }
}

impl IToggleProvider_Impl for Item_Impl {
    fn Toggle(&self) -> Result<()> {
        self.act(AccessAction::Toggle)
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        Ok(toggle_state(self.node()?.2.toggled))
    }
}

impl ISelectionItemProvider_Impl for Item_Impl {
    fn Select(&self) -> Result<()> {
        self.act(AccessAction::Select)
    }

    fn AddToSelection(&self) -> Result<()> {
        // A single choice: adding is selecting when nothing else may stay selected.
        self.act(AccessAction::Select)
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        // One choice is always selected: it cannot be removed.
        Err(hresult(UIA_E_INVALIDOPERATION))
    }

    fn IsSelected(&self) -> Result<BOOL> {
        Ok(BOOL::from(self.node()?.2.selected == Some(true)))
    }

    fn SelectionContainer(&self) -> Result<IRawElementProviderSimple> {
        let (_, tree, _) = self.node()?;
        match tree.parent_of(self.id) {
            Some(parent) => Ok(provider(self.window, &tree, parent.id)),
            None => none(),
        }
    }
}

impl ISelectionProvider_Impl for Item_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let (_, tree, node) = self.node()?;
        let selected: Vec<IRawElementProviderSimple> = node
            .children
            .iter()
            .filter(|c| c.selected == Some(true))
            .map(|c| provider(self.window, &tree, c.id))
            .collect();
        // SAFETY: plain allocation of an array of interface pointers, handed to the caller.
        let array = unsafe { SafeArrayCreateVector(VT_UNKNOWN, 0, selected.len() as u32) };
        if array.is_null() {
            return Err(Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY));
        }
        for (index, provider) in selected.iter().enumerate() {
            let index = index as i32;
            let unknown: IUnknown = provider.cast()?;
            // SAFETY: `array` holds `selected.len()` interface pointers; the array takes
            // its own reference on the interface it stores.
            unsafe { SafeArrayPutElement(array, &index, unknown.as_raw()) }?;
        }
        Ok(array)
    }

    fn CanSelectMultiple(&self) -> Result<BOOL> {
        Ok(BOOL::from(false))
    }

    fn IsSelectionRequired(&self) -> Result<BOOL> {
        Ok(BOOL::from(true))
    }
}

impl IExpandCollapseProvider_Impl for Item_Impl {
    fn Expand(&self) -> Result<()> {
        self.act(AccessAction::Expand)
    }

    fn Collapse(&self) -> Result<()> {
        self.act(AccessAction::Collapse)
    }

    fn ExpandCollapseState(&self) -> Result<ExpandCollapseState> {
        Ok(expand_state(self.node()?.2.expanded))
    }
}

impl IValueProvider_Impl for Item_Impl {
    fn SetValue(&self, _value: &windows::core::PCWSTR) -> Result<()> {
        // Text is typed through the keyboard; setting it from outside is not offered.
        Err(hresult(UIA_E_INVALIDOPERATION))
    }

    fn Value(&self) -> Result<windows::core::BSTR> {
        Ok(windows::core::BSTR::from(self.node()?.2.value.unwrap_or_default().as_str()))
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(BOOL::from(false))
    }
}

impl IRangeValueProvider_Impl for Item_Impl {
    fn SetValue(&self, value: f64) -> Result<()> {
        let range = self.range()?;
        let value = checked_value(range, value)
            .ok_or_else(|| Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG))?;
        self.act(AccessAction::SetValue(value))
    }

    fn Value(&self) -> Result<f64> {
        Ok(self.range()?.value)
    }

    fn IsReadOnly(&self) -> Result<BOOL> {
        Ok(BOOL::from(!self.node()?.2.enabled))
    }

    fn Maximum(&self) -> Result<f64> {
        Ok(self.range()?.max)
    }

    fn Minimum(&self) -> Result<f64> {
        Ok(self.range()?.min)
    }

    fn LargeChange(&self) -> Result<f64> {
        Ok(self.range()?.large_change)
    }

    fn SmallChange(&self) -> Result<f64> {
        Ok(self.range()?.small_change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(x, y, w, h)
    }

    /// pane(1) > [list(2) > [item(3), item(4)], slider(5)]
    fn sample() -> AccessNode {
        let mut root = AccessNode::new(1, AccessRole::Pane, "Panel", rect(0.0, 0.0, 100.0, 100.0));
        let mut list = AccessNode::new(2, AccessRole::List, "Outputs", rect(0.0, 0.0, 100.0, 40.0));
        let mut a =
            AccessNode::new(3, AccessRole::ListItem, "Speakers", rect(0.0, 0.0, 50.0, 40.0));
        a.selected = Some(true);
        a.focusable = true;
        let mut b =
            AccessNode::new(4, AccessRole::ListItem, "Headphones", rect(50.0, 0.0, 50.0, 40.0));
        b.selected = Some(false);
        b.focusable = true;
        list.children = vec![a, b];
        let mut slider =
            AccessNode::new(5, AccessRole::Slider, "Volume", rect(0.0, 60.0, 100.0, 20.0));
        slider.focusable = true;
        slider.range = Some(AccessRange {
            value: 40.0,
            min: 0.0,
            max: 100.0,
            small_change: 1.0,
            large_change: 10.0,
        });
        root.children = vec![list, slider];
        root
    }

    #[test]
    fn roles_map_to_control_types() {
        assert_eq!(control_type(AccessRole::ListItem), UIA_ListItemControlTypeId);
        assert_eq!(control_type(AccessRole::ToggleButton), UIA_ButtonControlTypeId);
        assert_eq!(control_type(AccessRole::Expander), UIA_ButtonControlTypeId);
        assert_eq!(control_type(AccessRole::Tab), UIA_TabItemControlTypeId);
        assert_eq!(control_type(AccessRole::TabList), UIA_TabControlTypeId);
        assert_eq!(control_type(AccessRole::Slider), UIA_SliderControlTypeId);
        assert_eq!(control_type(AccessRole::Menu), UIA_MenuControlTypeId);
        assert_eq!(control_type(AccessRole::Separator), UIA_SeparatorControlTypeId);
    }

    #[test]
    fn patterns_follow_the_role() {
        let tree = sample();
        let item = tree.find(3).unwrap();
        assert!(supports(item, UIA_InvokePatternId));
        assert!(supports(item, UIA_SelectionItemPatternId));
        assert!(!supports(item, UIA_TogglePatternId));
        assert!(supports(tree.find(2).unwrap(), UIA_SelectionPatternId));
        assert!(supports(tree.find(5).unwrap(), UIA_RangeValuePatternId));
        assert!(!supports(&tree, UIA_InvokePatternId));
        let mut toggle = AccessNode::new(9, AccessRole::ToggleButton, "Mute", Rect::default());
        assert!(supports(&toggle, UIA_TogglePatternId));
        toggle.role = AccessRole::Expander;
        assert!(supports(&toggle, UIA_ExpandCollapsePatternId));
        assert!(!supports(&toggle, UIA_InvokePatternId));
        let mut slider = tree.find(5).unwrap().clone();
        slider.range = None;
        assert!(!supports(&slider, UIA_RangeValuePatternId), "a slider without a value");
    }

    #[test]
    fn navigation_walks_children_and_siblings() {
        let tree = sample();
        assert_eq!(navigate(&tree, 1, NavigateDirection_FirstChild), Some(2));
        assert_eq!(navigate(&tree, 1, NavigateDirection_LastChild), Some(5));
        assert_eq!(navigate(&tree, 3, NavigateDirection_NextSibling), Some(4));
        assert_eq!(navigate(&tree, 4, NavigateDirection_NextSibling), None);
        assert_eq!(navigate(&tree, 3, NavigateDirection_PreviousSibling), None);
        assert_eq!(navigate(&tree, 4, NavigateDirection_PreviousSibling), Some(3));
        assert_eq!(navigate(&tree, 4, NavigateDirection_Parent), Some(2));
        assert_eq!(navigate(&tree, 2, NavigateDirection_Parent), Some(1));
        assert_eq!(navigate(&tree, 1, NavigateDirection_Parent), None);
        assert_eq!(navigate(&tree, 5, NavigateDirection_FirstChild), None);
        assert_eq!(navigate(&tree, 42, NavigateDirection_FirstChild), None, "unknown id");
    }

    #[test]
    fn hit_testing_finds_the_deepest_element() {
        let tree = sample();
        assert_eq!(hit(&tree, Point::new(10.0, 10.0)), 3);
        assert_eq!(hit(&tree, Point::new(60.0, 10.0)), 4);
        assert_eq!(hit(&tree, Point::new(10.0, 70.0)), 5);
        assert_eq!(hit(&tree, Point::new(10.0, 50.0)), 1, "between elements: the surface");
    }

    #[test]
    fn runtime_ids_carry_the_whole_identifier() {
        let id = 0x1234_5678_9ABC_DEF0_u64;
        let parts = runtime_id(id);
        assert_eq!(parts[0], UiaAppendRuntimeId as i32);
        assert_eq!(parts[1] as u32, 0x9ABC_DEF0);
        assert_eq!(parts[2] as u32, 0x1234_5678);
        assert_ne!(runtime_id(1), runtime_id(2));
    }

    #[test]
    fn elements_outside_the_surface_are_offscreen() {
        let tree = sample();
        assert!(!offscreen(&tree, tree.find(5).unwrap()));
        let below = AccessNode::new(7, AccessRole::Slider, "", rect(0.0, 120.0, 100.0, 20.0));
        assert!(offscreen(&tree, &below));
        let empty = AccessNode::new(8, AccessRole::Text, "", rect(10.0, 10.0, 0.0, 10.0));
        assert!(offscreen(&tree, &empty));
    }

    #[test]
    fn values_outside_the_range_are_refused() {
        let range = sample().find(5).unwrap().range.unwrap();
        assert_eq!(checked_value(range, 50.0), Some(50.0));
        assert_eq!(checked_value(range, 0.0), Some(0.0));
        assert_eq!(checked_value(range, 101.0), None);
        assert_eq!(checked_value(range, -1.0), None);
        assert_eq!(checked_value(range, f64::NAN), None);
    }

    #[test]
    fn properties_describe_the_element() {
        let tree = sample();
        let item = tree.find(3).unwrap();
        let get = |id| property(&tree, item, Some(3), id);
        assert_eq!(
            get(UIA_ControlTypePropertyId),
            Some(VARIANT::from(UIA_ListItemControlTypeId.0))
        );
        assert_eq!(get(UIA_NamePropertyId), Some(VARIANT::from("Speakers")));
        assert_eq!(get(UIA_HasKeyboardFocusPropertyId), Some(VARIANT::from(true)));
        assert_eq!(get(UIA_IsKeyboardFocusablePropertyId), Some(VARIANT::from(true)));
        assert_eq!(get(UIA_AutomationIdPropertyId), Some(VARIANT::from("bv-3")));
        assert_eq!(get(UIA_FrameworkIdPropertyId), Some(VARIANT::from(FRAMEWORK)));
        assert_eq!(get(UIA_HelpTextPropertyId), None, "no tooltip");
        assert_eq!(get(UIA_IsOffscreenPropertyId), Some(VARIANT::from(false)));
        let separator = AccessNode::new(9, AccessRole::Separator, "", Rect::default());
        assert_eq!(
            property(&tree, &separator, None, UIA_IsContentElementPropertyId),
            Some(VARIANT::from(false))
        );
    }

    /// A real window on its own thread, read and driven by the UI Automation client of
    /// this process from the test thread, as a screen reader would.
    mod live {
        use std::sync::mpsc;
        use std::sync::{Arc, Mutex};

        use windows::core::w;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
        };
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
            PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, MSG, WM_APP,
            WM_GETOBJECT, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
        };

        use super::*;

        type Log = Arc<Mutex<Vec<(u64, AccessAction)>>>;

        struct Fake {
            log: Log,
        }

        impl AccessSource for Fake {
            fn tree(&self) -> Option<AccessNode> {
                Some(sample())
            }
            fn scale(&self) -> f32 {
                1.0
            }
            fn act(&self, id: u64, action: AccessAction) -> bool {
                self.log.lock().map(|mut l| l.push((id, action))).is_ok()
            }
            fn focused(&self) -> Option<u64> {
                Some(3)
            }
        }

        thread_local! {
            static FAKE: RefCell<Option<Rc<dyn AccessSource>>> = const { RefCell::new(None) };
        }

        const WM_STOP: u32 = WM_APP + 77;

        extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if message == WM_GETOBJECT {
                if let Some(source) = FAKE.with(|f| f.borrow().clone()) {
                    if let Some(result) = on_get_object(window, w, l, source) {
                        return result;
                    }
                }
            }
            if message == WM_STOP {
                disconnect(window);
                // SAFETY: our own window, on its thread.
                unsafe {
                    let _ = DestroyWindow(window);
                    PostQuitMessage(0);
                }
                return LRESULT(0);
            }
            // SAFETY: forwarding the unmodified message.
            unsafe { DefWindowProcW(window, message, w, l) }
        }

        /// Runs the window thread; sends back its handle.
        fn window_thread(log: Log, ready: mpsc::Sender<isize>) {
            // SAFETY: balanced below.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            FAKE.with(|f| *f.borrow_mut() = Some(Rc::new(Fake { log })));
            // SAFETY: plain calls creating a hidden window of a class registered here.
            let window = unsafe {
                let instance = GetModuleHandleW(None).expect("module");
                let class = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: instance.into(),
                    lpszClassName: w!("BetterVolume.UiaTest"),
                    ..Default::default()
                };
                RegisterClassW(&class);
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    w!("BetterVolume.UiaTest"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    100,
                    100,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )
                .expect("window")
            };
            let _ = ready.send(window.0 as isize);
            let mut message = MSG::default();
            // SAFETY: a plain message loop on this thread.
            unsafe {
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            FAKE.with(|f| f.borrow_mut().take());
            // SAFETY: balances the initialisation above.
            unsafe { CoUninitialize() };
        }

        #[test]
        fn a_screen_reader_reads_and_drives_the_tree() {
            let log: Log = Arc::new(Mutex::new(Vec::new()));
            let (ready, handle) = mpsc::channel();
            let thread_log = log.clone();
            let thread = std::thread::spawn(move || window_thread(thread_log, ready));
            let window = HWND(handle.recv().expect("window handle") as *mut core::ffi::c_void);

            // SAFETY: COM initialised for this thread, balanced at the end; every call is
            // a plain client call on objects of this thread.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let outcome = (|| -> Result<()> {
                    let uia: IUIAutomation =
                        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                    let root = uia.ElementFromHandle(window)?;
                    assert_eq!(root.CurrentName()?.to_string(), "Panel");
                    let all = uia.CreateTrueCondition()?;
                    let children = root.FindAll(TreeScope_Children, &all)?;
                    assert_eq!(children.Length()?, 2);
                    let list = children.GetElement(0)?;
                    assert_eq!(list.CurrentControlType()?, UIA_ListControlTypeId);
                    let items = list.FindAll(TreeScope_Children, &all)?;
                    assert_eq!(items.Length()?, 2);
                    let second = items.GetElement(1)?;
                    assert_eq!(second.CurrentName()?.to_string(), "Headphones");
                    assert_eq!(second.CurrentControlType()?, UIA_ListItemControlTypeId);
                    let invoke: IUIAutomationInvokePattern =
                        second.GetCurrentPatternAs(UIA_InvokePatternId)?;
                    invoke.Invoke()?;
                    let slider = children.GetElement(1)?;
                    let range: IUIAutomationRangeValuePattern =
                        slider.GetCurrentPatternAs(UIA_RangeValuePatternId)?;
                    assert_eq!(range.CurrentValue()?, 40.0);
                    assert_eq!(range.CurrentMaximum()?, 100.0);
                    range.SetValue(55.0)?;
                    assert!(range.SetValue(500.0).is_err(), "out of range");
                    let focused = items.GetElement(0)?;
                    assert!(focused.CurrentHasKeyboardFocus()?.as_bool());
                    Ok(())
                })();
                let _ = PostMessageW(Some(window), WM_STOP, WPARAM(0), LPARAM(0));
                CoUninitialize();
                outcome.expect("UI Automation client calls");
            }
            thread.join().expect("window thread");
            // UI Automation itself moves the focus to an element before acting on it.
            let log: Vec<_> = log
                .lock()
                .expect("log")
                .iter()
                .filter(|(_, action)| *action != AccessAction::Focus)
                .copied()
                .collect();
            assert_eq!(log, vec![(4, AccessAction::Invoke), (5, AccessAction::SetValue(55.0))]);
        }
    }
}
