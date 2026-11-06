use alloc::{format, vec::Vec};
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::{
    enter::{osxx86_leave, SavedCtx},
    Host,
};

/// The host and guest-frame state reachable by the native libSystem entry points.
pub struct Session {
    /// The host, lifetime-erased for the duration of the run.
    pub host: *mut dyn Host,
    /// Where the engine's registers were saved on entry.
    pub save: *mut SavedCtx,
    /// `[start, end)` address ranges a guest pointer may point into.
    pub readable: Vec<(usize, usize)>,
    /// Set when the guest called `dyld_stub_binder`.
    pub stub_binder_hit: bool,
}

impl Session {
    /// A session over `host`; `save` receives the registers `osxx86_enter` saves and `readable` lists the guest-pointer ranges `write` accepts.
    pub fn new(host: &mut dyn Host, save: *mut SavedCtx, readable: Vec<(usize, usize)>) -> Session {
        let host = host as *mut dyn Host;
        // SAFETY: `run` keeps the host borrow and this session alive until `end` clears ACTIVE.
        let host = unsafe { core::mem::transmute::<*mut dyn Host, *mut (dyn Host + 'static)>(host) };
        Session { host, save, readable, stub_binder_hit: false }
    }
}

static ACTIVE: AtomicPtr<Session> = AtomicPtr::new(core::ptr::null_mut());

/// Makes `s` the active session; false when another guest is already running in this process.
pub fn begin(s: &mut Session) -> bool {
    ACTIVE
        .compare_exchange(core::ptr::null_mut(), s as *mut Session, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Clears the active session.
pub fn end() {
    ACTIVE.store(core::ptr::null_mut(), Ordering::SeqCst);
}

/// The native function a Mach-O symbol name binds to, if this table serves it.
pub fn resolve(symbol: &str) -> Option<usize> {
    match symbol {
        "_write" => Some(osx_write as *const () as usize),
        "_exit" | "__exit" => Some(osx_exit as *const () as usize),
        "dyld_stub_binder" => Some(osx_stub_binder as *const () as usize),
        _ => None,
    }
}

/// A native entry was reached with no guest running, which only a forged call can do: fault the engine process, the fault domain.
fn no_guest_is_running() -> ! {
    // SAFETY: `ud2` raises an invalid-opcode fault and never returns.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}

/// libSystem `write(2)` for descriptors 1 and 2: the buffer must lie wholly inside the image or the guest stack, else -1.
pub extern "C" fn osx_write(fd: i32, buf: *const u8, n: usize) -> isize {
    if n == 0 {
        return 0;
    }
    if fd != 1 && fd != 2 {
        return -1;
    }
    let session = ACTIVE.load(Ordering::SeqCst);
    if session.is_null() {
        return -1;
    }
    let buf_start = buf as usize;
    let Some(buf_end) = buf_start.checked_add(n) else {
        return -1;
    };
    // SAFETY: ACTIVE is cleared only after the guest has left and its session is no longer used.
    let readable = unsafe { &(*session).readable };
    let mut contained = false;
    for &(start, end) in readable {
        if buf_start >= start && buf_end <= end {
            contained = true;
            break;
        }
    }
    if !contained {
        return -1;
    }
    // SAFETY: the complete buffer was range-checked against one active readable reservation.
    let slice = unsafe { core::slice::from_raw_parts(buf, n) };
    // SAFETY: the session is active; its transmuted host pointer remains borrowed by `run`.
    let host = unsafe { &mut *(*session).host };
    host.marker(&format!("[OSXX86 WRITE fd={} n={}]", fd, n));
    host.write(fd, slice)
}

/// libSystem `exit` / `_exit`: abandons the guest and returns `code` from the engine's enter call.
pub extern "C" fn osx_exit(code: i32) -> ! {
    let session = ACTIVE.load(Ordering::SeqCst);
    if session.is_null() {
        no_guest_is_running();
    }
    // SAFETY: the session is active and `save` belongs to the frame that called osxx86_enter.
    unsafe { osxx86_leave((*session).save, code) }
}

/// `dyld_stub_binder`: reached only when an import was left for lazy binding, which the loader never does; refuses by name.
pub extern "C" fn osx_stub_binder() -> ! {
    let session = ACTIVE.load(Ordering::SeqCst);
    if session.is_null() {
        no_guest_is_running();
    }
    // SAFETY: the active session exclusively owns this flag while guest code is running.
    unsafe {
        (*session).stub_binder_hit = true;
    }
    // SAFETY: the session is active and `save` belongs to the frame that called osxx86_enter.
    unsafe { osxx86_leave((*session).save, -1) }
}