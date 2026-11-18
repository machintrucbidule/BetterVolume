//! Declarations transcribed from the public Windows SDK header `xamlOM.h` (XAML
//! Diagnostics). Only the methods the helper uses are reached; the ones before them are
//! declared so each vtable keeps its real order (as in `bv-audio`'s policy interface).
//!
//! The structs that cross the callback boundary (`VisualElement`, `ParentChildRelation`,
//! `SourceInfo`) and the property-chain results (`PropertyChainSource`,
//! `PropertyChainValue`) are `#[repr(C)]` and use raw `*mut u16` for their `BSTR` fields,
//! so receiving them never takes ownership of strings the host still owns.

use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT, PCWSTR};

/// An opaque handle to a live XAML object inside the taskbar (`InstanceHandle`).
pub type InstanceHandle = u64;

/// `VisualMutationType`: an element was added to, or removed from, the tree.
pub const MUTATION_ADD: i32 = 0;
/// Declared for completeness of the enum; the helper reacts to additions only.
#[allow(dead_code)]
pub const MUTATION_REMOVE: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceInfo {
    pub file_name: *mut u16,
    pub line_number: u32,
    pub column_number: u32,
    pub char_position: u32,
    pub hash: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParentChildRelation {
    pub parent: InstanceHandle,
    pub child: InstanceHandle,
    pub child_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualElement {
    pub handle: InstanceHandle,
    pub src_info: SourceInfo,
    pub type_name: *mut u16,
    pub name: *mut u16,
    pub num_children: u32,
}

#[repr(C)]
pub struct PropertyChainSource {
    pub handle: InstanceHandle,
    pub target_type: *mut u16,
    pub name: *mut u16,
    pub source: i32,
    pub src_info: SourceInfo,
}

#[repr(C)]
pub struct PropertyChainValue {
    pub index: u32,
    pub type_name: *mut u16,
    pub declaring_type: *mut u16,
    pub value_type: *mut u16,
    pub item_type: *mut u16,
    pub value: *mut u16,
    pub overridden: i32,
    pub metadata_bits: i64,
    pub property_name: *mut u16,
    pub property_chain_index: u32,
}

#[interface("18C9E2B6-3F43-4116-9F2B-FF935D7770D2")]
pub unsafe trait IXamlDiagnostics: IUnknown {
    pub fn GetDispatcher(&self, dispatcher: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn GetUiLayer(&self, layer: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn GetApplication(&self, application: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn GetIInspectableFromHandle(
        &self,
        handle: InstanceHandle,
        instance: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
    pub fn GetHandleFromIInspectable(
        &self,
        instance: *mut core::ffi::c_void,
        handle: *mut InstanceHandle,
    ) -> HRESULT;
    pub fn HitTest(
        &self,
        rect: windows::Win32::Foundation::RECT,
        count: *mut u32,
        handles: *mut *mut InstanceHandle,
    ) -> HRESULT;
    pub fn RegisterInstance(
        &self,
        instance: *mut core::ffi::c_void,
        handle: *mut InstanceHandle,
    ) -> HRESULT;
    /// The last argument of `InitializeXamlDiagnosticsEx`, read once by each TAP instance.
    pub fn GetInitializationData(&self, data: *mut *mut u16) -> HRESULT;
}

#[interface("A593B11A-D17F-48BB-8F66-83910731C8A5")]
pub unsafe trait IVisualTreeService: IUnknown {
    pub fn AdviseVisualTreeChange(&self, callback: *mut core::ffi::c_void) -> HRESULT;
    pub fn UnadviseVisualTreeChange(&self, callback: *mut core::ffi::c_void) -> HRESULT;
    pub fn GetEnums(&self, count: *mut u32, enums: *mut *mut core::ffi::c_void) -> HRESULT;
    pub fn CreateInstance(
        &self,
        type_name: PCWSTR,
        value: PCWSTR,
        handle: *mut InstanceHandle,
    ) -> HRESULT;
    pub fn GetPropertyValuesChain(
        &self,
        handle: InstanceHandle,
        source_count: *mut u32,
        sources: *mut *mut PropertyChainSource,
        property_count: *mut u32,
        values: *mut *mut PropertyChainValue,
    ) -> HRESULT;
    pub fn SetProperty(
        &self,
        handle: InstanceHandle,
        value: InstanceHandle,
        property_index: u32,
    ) -> HRESULT;
    pub fn ClearProperty(&self, handle: InstanceHandle, property_index: u32) -> HRESULT;
    pub fn GetCollectionCount(&self, handle: InstanceHandle, count: *mut u32) -> HRESULT;
    pub fn GetCollectionElements(
        &self,
        handle: InstanceHandle,
        start: u32,
        count: *mut u32,
        elements: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
    pub fn AddChild(&self, parent: InstanceHandle, child: InstanceHandle, index: u32) -> HRESULT;
    pub fn RemoveChild(&self, parent: InstanceHandle, index: u32) -> HRESULT;
    pub fn ClearChildren(&self, parent: InstanceHandle) -> HRESULT;
}

#[interface("130F5136-EC43-4F61-89C7-9801A36D2E95")]
pub unsafe trait IVisualTreeService2: IVisualTreeService {
    pub fn GetPropertyIndex(
        &self,
        object: InstanceHandle,
        property_name: PCWSTR,
        index: *mut u32,
    ) -> HRESULT;
    pub fn GetProperty(
        &self,
        object: InstanceHandle,
        property_index: u32,
        value: *mut InstanceHandle,
    ) -> HRESULT;
    pub fn ReplaceResource(
        &self,
        dictionary: InstanceHandle,
        key: InstanceHandle,
        new_value: InstanceHandle,
    ) -> HRESULT;
    pub fn RenderTargetBitmap(
        &self,
        handle: InstanceHandle,
        options: i32,
        max_width: u32,
        max_height: u32,
        bitmap: *mut *mut core::ffi::c_void,
    ) -> HRESULT;
}

#[interface("AA7A8931-80E4-4FEC-8F3B-553F87B4966E")]
pub unsafe trait IVisualTreeServiceCallback: IUnknown {
    pub fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation_type: i32,
    ) -> HRESULT;
}

#[interface("BAD9EB88-AE77-4397-B948-5FA2DB0A19EA")]
pub unsafe trait IVisualTreeServiceCallback2: IVisualTreeServiceCallback {
    pub fn OnElementStateChanged(
        &self,
        element: InstanceHandle,
        element_state: i32,
        context: PCWSTR,
    ) -> HRESULT;
}
