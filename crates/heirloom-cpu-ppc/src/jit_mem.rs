//! Tier 2's executable memory: the [`CodeMem`] provider the CALLER injects (heirloom-cpu-ppc makes no syscall and holds no
//! authority), and the arena that fills it under the PUBLISH model of the PowerPC ruling's phase 2: pages are written while
//! RW (never executable) and then published RX (never writable again until the whole arena is reset). Nothing is ever
//! patched in place: chaining goes through a RW link table (jit.rs), so code needs no write after its page is published.
//!
//! [`HostCodeMem`] is the Linux x86-64 provider (raw mmap / mprotect / munmap syscalls, no dependency) for the host tests,
//! the bench and the WC3 harness. On CoralOS the provider will be macppc.engine's (syscall 47's publish form, delegated by
//! CapKind::ExecRegion: increment 2 of /build/patches/h86/ppc-fast-tier-plan.md, 2026-11-02); until a caller passes one,
//! Tier::Jit runs tier 1 and `JitStats::refused` names it.

/// Executable memory as a caller provides it. `reserve` once; then any page-aligned range may be published (RW -> RX) and
/// the whole reservation reset to RW (every published page withdrawn, all code dropped).
pub trait CodeMem {
    /// Reserve `len` bytes (a multiple of 4096), readable and writable, NOT executable. None = refused.
    fn reserve(&mut self, len: usize) -> Option<*mut u8>;
    /// Make `[ptr, ptr + len)` (page aligned, inside the reservation) read + execute, no write. False = refused.
    fn publish(&mut self, ptr: *mut u8, len: usize) -> bool;
    /// Make the whole reservation read + write, not executable, again (an arena reset: no code of it runs afterwards).
    fn reset(&mut self, ptr: *mut u8, len: usize) -> bool;
}

pub(crate) const PAGE: usize = 4096;

/// The code arena: `[base, base + len)` of a provider, published up to `published`, written from `cur`.
pub(crate) struct Arena {
    mem: alloc::boxed::Box<dyn CodeMem>,
    pub(crate) base: *mut u8,
    pub(crate) len: usize,
    /// Everything below this is published (RX); `cur` >= it. Page aligned.
    published: usize,
    cur: usize,
}

impl Arena {
    pub(crate) fn new(mut mem: alloc::boxed::Box<dyn CodeMem>, len: usize) -> Option<Arena> {
        let base = mem.reserve(len)?;
        Some(Arena { mem, base, len, published: 0, cur: 0 })
    }
    /// Copy `code` to the unpublished tail; None when it does not fit (the caller resets the arena).
    pub(crate) fn put(&mut self, code: &[u8]) -> Option<*const u8> {
        if self.cur + code.len() > self.len { return None; }
        // SAFETY: [base + cur, base + cur + code.len()) is inside the reservation (checked) and at or above `published`, so
        // it is RW (reserved RW, or reset to RW) and no published code lives there.
        let p = unsafe { self.base.add(self.cur) };
        unsafe { core::ptr::copy_nonoverlapping(code.as_ptr(), p, code.len()) };
        self.cur += code.len();
        Some(p)
    }
    /// True when there is written code not yet published.
    pub(crate) fn pending(&self) -> bool { self.cur > self.published }
    /// Publish everything written (rounded up to whole pages: the rest of the last page is never written again).
    pub(crate) fn publish(&mut self) -> bool {
        if !self.pending() { return true; }
        let end = (self.cur + PAGE - 1) & !(PAGE - 1);
        let end = end.min(self.len);
        // SAFETY: published <= end <= len: the range is inside the reservation.
        let ok = self.mem.publish(unsafe { self.base.add(self.published) }, end - self.published);
        self.published = end;
        self.cur = end;
        ok
    }
    /// Withdraw every published page (back to RW) and start writing at the base again.
    pub(crate) fn reset(&mut self) -> bool {
        self.published = 0;
        self.cur = 0;
        self.mem.reset(self.base, self.len)
    }
    pub(crate) fn used(&self) -> usize { self.cur }
    /// Bytes written and not yet published.
    pub(crate) fn unpublished(&self) -> usize { self.cur - self.published }
}

/// Linux x86-64 executable memory for the host (tests, bench, the WC3 harness): an anonymous private mapping, RW until
/// published, then RX.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[derive(Default)]
pub struct HostCodeMem { map: Option<(usize, usize)> }

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl HostCodeMem {
    pub fn new() -> HostCodeMem { HostCodeMem { map: None } }
    fn sys(nr: usize, a: usize, b: usize, c: usize, d: usize, e: isize, f: usize) -> isize {
        let ret: isize;
        // SAFETY: a raw Linux syscall (mmap 9 / mprotect 10 / munmap 11) with arguments this file builds; the kernel validates
        // them and touches no memory of ours beyond the mapping it is asked about. rcx and r11 are clobbered by `syscall`.
        unsafe {
            core::arch::asm!("syscall", inlateout("rax") nr as isize => ret, in("rdi") a, in("rsi") b, in("rdx") c,
                in("r10") d, in("r8") e, in("r9") f, lateout("rcx") _, lateout("r11") _, options(nostack));
        }
        ret
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl CodeMem for HostCodeMem {
    fn reserve(&mut self, len: usize) -> Option<*mut u8> {
        const PROT_RW: usize = 3;
        const MAP_PRIVATE_ANON: usize = 0x22;
        let r = Self::sys(9, 0, len, PROT_RW, MAP_PRIVATE_ANON, -1, 0);
        if r < 0 { return None; }
        self.map = Some((r as usize, len));
        Some(r as *mut u8)
    }
    fn publish(&mut self, ptr: *mut u8, len: usize) -> bool { Self::sys(10, ptr as usize, len, 5, 0, 0, 0) == 0 }
    fn reset(&mut self, ptr: *mut u8, len: usize) -> bool { Self::sys(10, ptr as usize, len, 3, 0, 0, 0) == 0 }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Drop for HostCodeMem {
    fn drop(&mut self) {
        if let Some((p, l)) = self.map.take() { Self::sys(11, p, l, 0, 0, 0, 0); }
    }
}
