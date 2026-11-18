//! COM helpers: apartment initialisation, strings returned by COM, property values.

use std::marker::PhantomData;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToStringAlloc, PropVariantToUInt32, PROPVARIANT,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use crate::AudioError;

/// Keeps COM initialised on the current thread while alive.
///
/// The UI thread is a single-threaded apartment. If the thread was already initialised
/// in another mode, COM is still usable and nothing is undone on drop.
pub(crate) struct Apartment {
    initialised: bool,
    // COM initialisation is per thread: the guard must stay on the thread that made it.
    _not_send: PhantomData<*const ()>,
}

impl Apartment {
    pub(crate) fn enter() -> Self {
        // SAFETY: plain call; balanced by `CoUninitialize` in `drop` when it succeeded.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // Success includes S_FALSE (already initialised in the same mode), which must be
        // balanced too. RPC_E_CHANGED_MODE (already initialised in another mode) must not.
        debug_assert!(result.is_ok() || result == RPC_E_CHANGED_MODE, "{result:?}");
        Self { initialised: result.is_ok(), _not_send: PhantomData }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.initialised {
            // SAFETY: balances the successful `CoInitializeEx` of `enter`, on the same
            // thread (the guard is not `Send`).
            unsafe { CoUninitialize() };
        }
    }
}

/// Takes ownership of a string allocated by COM, converts it and frees it.
pub(crate) fn take_string(value: PWSTR) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: COM returned a valid NUL-terminated string that we own.
    let text = unsafe { value.to_string() }.unwrap_or_default();
    // SAFETY: the string was allocated with the COM allocator and is freed exactly once.
    unsafe { CoTaskMemFree(Some(value.0 as *const _)) };
    text
}

/// A property value that is cleared when dropped.
struct Value(PROPVARIANT);

impl Drop for Value {
    fn drop(&mut self) {
        // SAFETY: the value was filled by `IPropertyStore::GetValue` and is cleared once.
        let _ = unsafe { PropVariantClear(&mut self.0) };
    }
}

fn value(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<Value> {
    // SAFETY: `key` points to a valid property key for the duration of the call.
    unsafe { store.GetValue(key) }.ok().map(Value)
}

/// A text property, or `None` if it is absent or not text.
pub(crate) fn string_property(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let value = value(store, key)?;
    // SAFETY: `value` holds a valid property value; the result is a COM string we own.
    let text = unsafe { PropVariantToStringAlloc(&value.0) }.ok()?;
    Some(take_string(text)).filter(|s| !s.is_empty())
}

/// A number property, or `None` if it is absent or not a number.
pub(crate) fn u32_property(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    let value = value(store, key)?;
    // SAFETY: `value` holds a valid property value.
    unsafe { PropVariantToUInt32(&value.0) }.ok()
}

/// A NUL-terminated UTF-16 copy of `text`, for passing to Windows.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A pointer to a buffer made by [`wide`]; valid while the buffer lives.
pub(crate) fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// Maps a Windows failure to an [`AudioError`].
pub(crate) fn audio_error(error: &windows::core::Error) -> AudioError {
    use windows::Win32::Foundation::{ERROR_NOT_FOUND, RPC_E_DISCONNECTED};
    use windows::Win32::Media::Audio::{
        AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_SERVICE_NOT_RUNNING,
    };
    /// "The RPC server is unavailable" (`RPC_S_SERVER_UNAVAILABLE` as an HRESULT).
    const RPC_SERVER_UNAVAILABLE: windows::core::HRESULT =
        windows::core::HRESULT(0x8007_06BA_u32 as i32);
    let code = error.code();
    if code == AUDCLNT_E_DEVICE_INVALIDATED || code == ERROR_NOT_FOUND.to_hresult() {
        AudioError::DeviceGone
    } else if code == AUDCLNT_E_SERVICE_NOT_RUNNING
        || code == RPC_E_DISCONNECTED
        || code == RPC_SERVER_UNAVAILABLE
    {
        AudioError::ServiceUnavailable
    } else {
        AudioError::Windows(code.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_strings_are_terminated() {
        assert_eq!(wide("ab"), [97, 98, 0]);
        assert_eq!(wide(""), [0]);
    }

    #[test]
    fn errors_are_classified() {
        use windows::Win32::Foundation::E_INVALIDARG;
        use windows::Win32::Media::Audio::AUDCLNT_E_DEVICE_INVALIDATED;
        let gone = windows::core::Error::from(AUDCLNT_E_DEVICE_INVALIDATED);
        assert_eq!(audio_error(&gone), AudioError::DeviceGone);
        let other = windows::core::Error::from(E_INVALIDARG);
        assert_eq!(audio_error(&other), AudioError::Windows(E_INVALIDARG.0));
    }

    #[test]
    fn apartment_can_be_entered_twice_on_one_thread() {
        // A fresh test thread: the first entry initialises COM, the second one finds it
        // already initialised (S_FALSE), and both must be balanced on drop.
        let outer = Apartment::enter();
        let inner = Apartment::enter();
        assert!(outer.initialised && inner.initialised);
        drop(inner);
        drop(outer);
        // Balanced: the next entry initialises COM again.
        assert!(Apartment::enter().initialised);
    }
}
