//! The COM objects this library exposes: a class `Factory` and the `Tap` tree-automation
//! provider. The XAML Diagnostics host creates the `Tap` and calls `SetSite` with an
//! object that yields `IXamlDiagnostics`; from there the `Tap` reads its instruction,
//! follows the tree for a short while (`crate::tree`), reports the outcome and leaves.
//!
//! A connection is short: it subscribes to the tree, which first streams the whole
//! existing tree (the volume icon is normally found then), waits a few seconds more only
//! when the icon is not there yet (taskbar still starting), then unsubscribes. Staying
//! subscribed would keep every taskbar element referenced by the diagnostics and keep the
//! XAML composition diagnostics running, which published tools found unsafe with several
//! UI threads.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use windows::core::{
    implement, w, IUnknown, IUnknownImpl, Interface, Ref, BOOL, BSTR, GUID, HRESULT, PCWSTR,
};
use windows::Win32::Foundation::{CLASS_E_NOAGGREGATION, E_FAIL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::Ole::IObjectWithSite;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, RegisterWindowMessageW};

use crate::com;
use crate::tree::{Command, Instruction, Worker};
use crate::xaml::{
    IVisualTreeService, IVisualTreeServiceCallback, IVisualTreeServiceCallback2,
    IVisualTreeServiceCallback2_Impl, IVisualTreeServiceCallback_Impl, IXamlDiagnostics,
    InstanceHandle, ParentChildRelation, VisualElement,
};

/// How long a connection keeps following the tree when the volume icon was not in the
/// tree streamed at subscription (the taskbar may still be building it).
const GRACE: Duration = Duration::from_secs(10);

/// Outcomes posted to the owner window (`WPARAM`); the `LPARAM` is the taskbar process id.
/// Mirrored by `bv-shell` (`taskbar.rs`).
pub mod status {
    /// The volume icon was collapsed.
    pub const HIDDEN: usize = 1;
    /// The volume icon was made visible again.
    pub const RESTORED: usize = 2;
    /// No volume icon was found (unknown taskbar layout): nothing was changed.
    pub const NOT_FOUND: usize = 3;
    /// The tree could not be followed: nothing was changed.
    pub const FAILED: usize = 4;
}

/// Moves non-`Send` COM pointers onto the subscription thread.
struct Sent<T>(T);
// SAFETY: the bundled interface pointers are agile diagnostics objects (the diagnostics
// itself calls them from its own threads); they are handed to exactly one spawned thread.
unsafe impl<T> Send for Sent<T> {}

/// Class factory for [`crate::CLSID_BV_TASKBAR_HELPER`].
#[implement(IClassFactory)]
pub struct Factory;

impl windows::Win32::System::Com::IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if punkouter.as_ref().is_some() {
            return CLASS_E_NOAGGREGATION.ok();
        }
        let tap: IUnknown = Tap::new().into();
        // SAFETY: riid/ppvobject come from the COM runtime for a CreateInstance call.
        unsafe { tap.query(riid, ppvobject) }.ok()
    }

    fn LockServer(&self, _flock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

/// What the callback (taskbar UI threads) and the subscription thread share.
struct Shared {
    worker: Mutex<Option<Worker>>,
    /// Set when the volume icon was handled; wakes the subscription thread.
    found: Mutex<bool>,
    wake: Condvar,
}

// SAFETY: the worker's COM pointers are agile diagnostics objects, and every access goes
// through the mutex; the elements' handles are only used on their own UI thread (the
// worker checks the thread). This lets the COM object be `Send + Sync`, which
// `#[implement]` requires.
unsafe impl Send for Shared {}
// SAFETY: see the `Send` impl above.
unsafe impl Sync for Shared {}

/// The shared state of a TAP.
struct SharedState(Arc<Shared>);

/// The tree-automation provider loaded into the taskbar.
#[implement(IObjectWithSite, IVisualTreeServiceCallback, IVisualTreeServiceCallback2)]
pub struct Tap {
    shared: SharedState,
}

impl Tap {
    fn new() -> Self {
        com::add_object();
        Tap {
            shared: SharedState(Arc::new(Shared {
                worker: Mutex::new(None),
                found: Mutex::new(false),
                wake: Condvar::new(),
            })),
        }
    }

    /// Connect to the visual-tree service behind `site`, read the instruction and follow
    /// the tree for a while. Any missing piece leaves the taskbar untouched (guard rail).
    fn connect(
        &self,
        site: &IUnknown,
        callback: IVisualTreeServiceCallback,
    ) -> windows::core::Result<()> {
        let diagnostics: IXamlDiagnostics = site.cast()?;

        let mut data: *mut u16 = core::ptr::null_mut();
        // SAFETY: valid out-pointer; we adopt the returned BSTR.
        unsafe { diagnostics.GetInitializationData(&mut data) }.ok()?;
        let raw = if data.is_null() {
            String::new()
        } else {
            // SAFETY: GetInitializationData hands us a BSTR to own.
            unsafe { BSTR::from_raw(data) }.to_string()
        };
        let Some(instruction) = Instruction::parse(&raw) else {
            return Ok(());
        };
        com::trace(&format!("connect: {instruction:?}"));

        let service: IVisualTreeService = diagnostics.cast()?;
        let worker = Worker::new(service.clone(), instruction.command);
        *self.shared.0.worker.lock().map_err(|_| windows::core::Error::from(E_FAIL))? =
            Some(worker);

        // Subscribe on a thread of its own: the diagnostics then delivers the callbacks on
        // the taskbar's XAML UI threads. Subscribing on the thread the host used for
        // `SetSite` can hang or deliver nothing (TranslucentTB's finding).
        let bundle = Sent((service, callback, self.shared.0.clone()));
        let spawned =
            std::thread::Builder::new().name("bv-taskbar-helper".into()).spawn(move || {
                // Capture the wrapper whole (not its inner fields) so the closure stays `Send`.
                let bundle = bundle;
                let Sent((service, callback, shared)) = bundle;
                follow(&shared, &service, &callback, instruction);
            });
        if spawned.is_err() {
            report(instruction, status::FAILED);
        }
        Ok(())
    }
}

/// The subscription thread: subscribe (the existing tree streams during the call), wait
/// a little if the icon was not there, unsubscribe, report, and watch the owner.
fn follow(
    shared: &Shared,
    service: &IVisualTreeService,
    callback: &IVisualTreeServiceCallback,
    instruction: Instruction,
) {
    // SAFETY: a valid callback pointer; the service AddRefs it while subscribed.
    let hr = unsafe { service.AdviseVisualTreeChange(callback.as_raw()) };
    com::trace(&format!("advise -> 0x{:08X}", hr.0 as u32));
    let found = if hr.is_ok() {
        let guard = shared.found.lock().ok();
        let found = guard
            .and_then(|g| shared.wake.wait_timeout_while(g, GRACE, |found| !*found).ok())
            .is_some_and(|(g, _)| *g);
        // SAFETY: the same callback pointer as subscribed.
        let _ = unsafe { service.UnadviseVisualTreeChange(callback.as_raw()) };
        found
    } else {
        false
    };
    if let Ok(mut worker) = shared.worker.lock() {
        if let Some(worker) = worker.as_mut() {
            worker.clear();
        }
        *worker = None;
    }
    let outcome = match (hr.is_ok(), found, instruction.command) {
        (false, _, _) => status::FAILED,
        (true, false, _) => status::NOT_FOUND,
        (true, true, Command::Hide) => status::HIDDEN,
        (true, true, Command::Restore) => status::RESTORED,
    };
    com::trace(&format!("outcome {outcome}"));
    // While the icon is hidden, its owner is watched: if BetterVolume ends in any way
    // (quit, crash, ended by the user), the Windows icon comes back (guard rail).
    match instruction.command {
        Command::Hide if found => crate::watch::watch(instruction.owner_pid),
        Command::Hide => {}
        Command::Restore => crate::watch::cancel(),
    }
    report(instruction, outcome);
}

/// Tells the owner window the outcome of this connection (registered message
/// "BetterVolume.TaskbarHelper").
fn report(instruction: Instruction, outcome: usize) {
    if instruction.owner_window == 0 {
        return;
    }
    // SAFETY: a constant name; posting to a window that may be gone only fails.
    unsafe {
        let message = RegisterWindowMessageW(w!("BetterVolume.TaskbarHelper"));
        if message != 0 {
            let window = HWND(instruction.owner_window as *mut core::ffi::c_void);
            let _ = PostMessageW(
                Some(window),
                message,
                WPARAM(outcome),
                LPARAM(GetCurrentProcessId() as isize),
            );
        }
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        com::release_object();
    }
}

impl windows::Win32::System::Ole::IObjectWithSite_Impl for Tap_Impl {
    fn SetSite(&self, punksite: Ref<IUnknown>) -> windows::core::Result<()> {
        com::trace(&format!("SetSite(some={})", punksite.as_ref().is_some()));
        if let Some(site) = punksite.as_ref() {
            // A failure to connect is swallowed into a clean HRESULT: the guard rail is to
            // leave the taskbar as it is, never to disturb the host.
            let callback: IVisualTreeServiceCallback = self.to_interface();
            let _ = self.connect(site, callback);
        } else if let Ok(mut guard) = self.shared.0.worker.lock() {
            // The host is detaching us; drop the connected worker.
            *guard = None;
        }
        Ok(())
    }

    fn GetSite(
        &self,
        _riid: *const GUID,
        ppvsite: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if !ppvsite.is_null() {
            // SAFETY: a valid out-pointer from the caller.
            unsafe { *ppvsite = core::ptr::null_mut() };
        }
        E_FAIL.ok()
    }
}

impl IVisualTreeServiceCallback_Impl for Tap_Impl {
    unsafe fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation_type: i32,
    ) -> HRESULT {
        let shared = &self.shared.0;
        let handled = match shared.worker.lock() {
            Ok(mut guard) => match guard.as_mut() {
                // SAFETY: the struct fields point to host-owned strings valid for the
                // call, which runs on the elements' own UI thread.
                Some(worker) => unsafe { worker.on_change(relation, element, mutation_type) },
                None => false,
            },
            Err(_) => false,
        };
        if handled {
            if let Ok(mut found) = shared.found.lock() {
                *found = true;
                shared.wake.notify_all();
            }
        }
        // Always S_OK: an error from this callback makes XAML stop sending events.
        crate::hr_ok()
    }
}

impl IVisualTreeServiceCallback2_Impl for Tap_Impl {
    unsafe fn OnElementStateChanged(
        &self,
        _element: InstanceHandle,
        _element_state: i32,
        _context: PCWSTR,
    ) -> HRESULT {
        // Not used; the helper acts only on tree additions. Always S_OK.
        crate::hr_ok()
    }
}
