//! Watching the Windows audio service (NFR-05: the application survives a restart of it).
//!
//! When the audio service stops, every audio object the engine holds becomes useless; when
//! it starts again they must be created anew. Windows reports service state changes
//! through a callback on its thread pool, so this costs no thread and no polling.

use std::ffi::c_void;

use windows::core::w;
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    SubscribeServiceChangeNotifications, UnsubscribeServiceChangeNotifications,
    PSC_NOTIFICATION_REGISTRATION, SC_EVENT_STATUS_CHANGE, SC_HANDLE, SC_MANAGER_CONNECT,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
};

use crate::signals::{Signal, Sink};

/// A live subscription to the state changes of the audio service.
pub(crate) struct ServiceWatch {
    manager: SC_HANDLE,
    service: SC_HANDLE,
    subscription: PSC_NOTIFICATION_REGISTRATION,
}

impl ServiceWatch {
    /// Subscribes; `None` if Windows refuses (the engine then simply does without).
    pub(crate) fn start(sink: Sink) -> Option<Self> {
        // SAFETY: plain calls with constant arguments; the handles are closed in `drop`
        // or below on failure.
        let manager = unsafe { OpenSCManagerW(None, None, SC_MANAGER_CONNECT) }.ok()?;
        // SAFETY: as above; `manager` is a valid handle.
        let service = match unsafe { OpenServiceW(manager, w!("Audiosrv"), SERVICE_QUERY_STATUS) } {
            Ok(service) => service,
            Err(_) => {
                // SAFETY: the handle was opened above and is closed once.
                let _ = unsafe { CloseServiceHandle(manager) };
                return None;
            }
        };
        // The context lives as long as the process: Windows may still be running a
        // callback when the subscription is cancelled, so it is deliberately never freed
        // (one small allocation per engine; the application creates one engine).
        let context = Box::into_raw(Box::new(sink)) as *const c_void;
        let mut subscription = PSC_NOTIFICATION_REGISTRATION::default();
        // SAFETY: `service` is valid, the callback matches the expected signature and
        // `context` stays valid forever (see above).
        let status = unsafe {
            SubscribeServiceChangeNotifications(
                service,
                SC_EVENT_STATUS_CHANGE,
                Some(on_service_change),
                Some(context),
                &mut subscription,
            )
        };
        let watch = Self { manager, service, subscription };
        if status != 0 {
            // Dropping `watch` closes the handles; the empty subscription is not cancelled.
            return None;
        }
        Some(watch)
    }

    /// Whether the audio service is running right now.
    pub(crate) fn is_running(&self) -> bool {
        let mut status = SERVICE_STATUS::default();
        // SAFETY: `service` is a valid handle opened with SERVICE_QUERY_STATUS.
        unsafe { QueryServiceStatus(self.service, &mut status) }.is_ok()
            && status.dwCurrentState == SERVICE_RUNNING
    }
}

impl Drop for ServiceWatch {
    fn drop(&mut self) {
        if self.subscription.0 != 0 {
            // SAFETY: the subscription was made by `start` and is cancelled once.
            unsafe { UnsubscribeServiceChangeNotifications(self.subscription) };
        }
        // SAFETY: both handles were opened by `start` and are closed exactly once.
        unsafe {
            let _ = CloseServiceHandle(self.service);
            let _ = CloseServiceHandle(self.manager);
        }
    }
}

/// Called by Windows on a thread-pool thread at each state change of the service.
unsafe extern "system" fn on_service_change(_notification: u32, context: *const c_void) {
    // SAFETY: `context` is the `Sink` leaked by `start`, valid for the whole process and
    // only used through a shared reference (`Sink` is `Sync`).
    if let Some(sink) = unsafe { (context as *const Sink).as_ref() } {
        sink.send(Signal::Service);
    }
}
