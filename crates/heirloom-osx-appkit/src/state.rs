//! The adapter's process-wide state: the host the classes publish through, the application singleton and the open windows.
//! One guest runs per process on one thread, so the state sits in plain cells; it is reset by [`begin`].
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use heirloom_objc::runtime;
use heirloom_ui_abi::{GuestUi, UiRefusal};

/// What the classes need from their host: the UI seam (None when the host has no UI) and the marker log.
pub trait AppHost {
    /// The seam windows, controls and actions go through.
    fn ui(&mut self) -> Option<&mut dyn GuestUi>;
    /// A marker line (`[OSXX86-... OK ...]`).
    fn marker(&mut self, line: &str);
}

pub(crate) struct Registry {
    host: Option<*mut dyn AppHost>,
    /// The NSApplication singleton (0 before `sharedApplication`).
    pub app: usize,
    /// The data cell `_NSApp` binds to.
    pub nsapp: usize,
    /// Open windows in the order they were ordered front.
    pub windows: Vec<usize>,
    /// Windows that already produced `[OSXX86-WINDOW OK]`.
    pub reported: Vec<usize>,
    pub stop: bool,
}

struct Cell(UnsafeCell<Registry>);
// SAFETY: one guest runs per process on one thread; `begin` and `end` bracket its life.
unsafe impl Sync for Cell {}

static REG: Cell = Cell(UnsafeCell::new(Registry { host: None, app: 0, nsapp: 0, windows: Vec::new(), reported: Vec::new(), stop: false }));

pub(crate) fn reg() -> &'static mut Registry {
    // SAFETY: single guest thread, no overlapping borrows across message sends (callers copy what they need out first).
    unsafe { &mut *REG.0.get() }
}

/// Starts a guest's UI session over `host`.
///
/// # Safety
/// `host` must outlive the guest: call [`end`] before it is dropped.
pub unsafe fn begin(host: &mut dyn AppHost) {
    let r = reg();
    r.app = 0;
    r.nsapp = 0;
    r.windows.clear();
    r.reported.clear();
    r.stop = false;
    // SAFETY: the lifetime is erased for the guest's duration, which the caller bounds.
    r.host = Some(unsafe { core::mem::transmute::<*mut dyn AppHost, *mut (dyn AppHost + 'static)>(host as *mut dyn AppHost) });
}

/// Ends the session; the classes refuse by name afterwards.
pub fn end() {
    let r = reg();
    r.host = None;
    r.windows.clear();
    r.app = 0;
}

fn host_ptr() -> *mut dyn AppHost {
    match reg().host {
        Some(h) => h,
        None => runtime::fatal("OSXX86-UI-UNAVAILABLE the app used the UI with no host session"),
    }
}

/// Runs `f` on the UI seam; ends the guest by name when the host has none.
pub(crate) fn ui<R>(f: impl FnOnce(&mut dyn GuestUi) -> R) -> R {
    let h = host_ptr();
    // SAFETY: the host outlives the session (see `begin`) and nothing else borrows it while a class method runs.
    match unsafe { (*h).ui() } {
        Some(u) => f(u),
        None => runtime::fatal("OSXX86-UI-UNAVAILABLE the host offers no UI seam"),
    }
}

pub(crate) fn marker(line: &str) {
    let h = host_ptr();
    // SAFETY: as in `ui`.
    unsafe { (*h).marker(line) }
}

/// Ends the guest by name for a refusal from the UI seam.
pub(crate) fn refused(what: &str, e: UiRefusal) -> ! {
    let msg: String = format!("OSXX86-UI-REFUSED {what}: {e}");
    runtime::fatal(&msg)
}
