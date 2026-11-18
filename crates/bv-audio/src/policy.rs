//! Default-device switching (FR-041, FR-072) — the only undocumented Windows interface the
//! application uses.
//!
//! Windows has no public API to change the default audio device. Its own sound settings
//! use the "policy config" COM object, which every comparable tool also relies on; its
//! layout has been stable since Windows 7. Everything about it is confined to this file:
//! if a future Windows drops or changes it, creating the object or the call fails and the
//! switch reports [`AudioError::SwitchUnavailable`] / [`AudioError::SwitchRefused`];
//! nothing else in the application is affected.

use windows::core::GUID;
use windows::Win32::Media::Audio::{eCommunications, eConsole, eMultimedia, ERole};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use crate::com::{pcwstr, wide};
use crate::{AudioError, SwitchScope};
use raw::IPolicyConfig;

/// The policy config object (`CPolicyConfigClient`).
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

mod raw {
    // Method names follow the Windows declaration, hence not snake case.
    #![allow(non_snake_case)]

    use std::ffi::c_void;

    use windows::core::{interface, IUnknown, IUnknown_Vtbl, HRESULT, PCWSTR};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::ERole;

    /// The policy config interface (`IPolicyConfig`, Windows 7 and later). Only
    /// `SetDefaultEndpoint` is used; the methods before it are declared to keep the method
    /// table in its real order and must never be called.
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        pub fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut c_void) -> HRESULT;
        pub fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            default: i32,
            format: *mut *mut c_void,
        ) -> HRESULT;
        pub fn ResetDeviceFormat(&self, device: PCWSTR) -> HRESULT;
        pub fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint: *mut c_void,
            mix: *mut c_void,
        ) -> HRESULT;
        pub fn GetProcessingPeriod(
            &self,
            device: PCWSTR,
            default: i32,
            default_period: *mut i64,
            minimum_period: *mut i64,
        ) -> HRESULT;
        pub fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> HRESULT;
        pub fn GetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        pub fn SetShareMode(&self, device: PCWSTR, mode: *mut c_void) -> HRESULT;
        pub fn GetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const PROPERTYKEY,
            value: *mut c_void,
        ) -> HRESULT;
        pub fn SetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const PROPERTYKEY,
            value: *mut c_void,
        ) -> HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
        pub fn SetEndpointVisibility(&self, device: PCWSTR, visible: i32) -> HRESULT;
    }
}

/// The Windows roles a switch with `scope` changes.
pub(crate) fn roles(scope: SwitchScope) -> &'static [ERole] {
    match scope {
        SwitchScope::MediaAndCalls => &[eConsole, eMultimedia, eCommunications],
        SwitchScope::MediaOnly => &[eConsole, eMultimedia],
        SwitchScope::CallsOnly => &[eCommunications],
    }
}

/// Makes `device_id` the default device for every role of `scope`.
///
/// Stops at the first refused role; the roles already changed stay changed, and the
/// engine re-reads the defaults from the notifications Windows sends, so what the
/// application shows always matches Windows.
pub(crate) fn set_default(device_id: &str, scope: SwitchScope) -> Result<(), AudioError> {
    // SAFETY: creating an in-process COM object; COM is initialised on this thread.
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
            .map_err(|_| AudioError::SwitchUnavailable)?;
    let id = wide(device_id);
    for &role in roles(scope) {
        // SAFETY: `id` is a NUL-terminated buffer that outlives the call; the method
        // slot matches the real interface (see the declaration above).
        unsafe { policy.SetDefaultEndpoint(pcwstr(&id), role) }
            .ok()
            .map_err(|_| AudioError::SwitchRefused)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_map_to_windows_roles() {
        assert_eq!(roles(SwitchScope::MediaAndCalls), [eConsole, eMultimedia, eCommunications]);
        assert_eq!(roles(SwitchScope::MediaOnly), [eConsole, eMultimedia]);
        assert_eq!(roles(SwitchScope::CallsOnly), [eCommunications]);
    }

    #[test]
    fn the_method_table_has_the_expected_size() {
        // IUnknown (3 entries) + the 12 methods above, in order: SetDefaultEndpoint must
        // be entry 13 (counting from 0).
        let entries = size_of::<raw::IPolicyConfig_Vtbl>() / size_of::<usize>();
        assert_eq!(entries, 15);
        assert_eq!(
            std::mem::offset_of!(raw::IPolicyConfig_Vtbl, SetDefaultEndpoint) / size_of::<usize>(),
            13
        );
    }
}
