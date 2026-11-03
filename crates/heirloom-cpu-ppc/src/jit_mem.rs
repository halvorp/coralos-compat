//! Tier 2's executable memory: the [`CodeMem`] provider the CALLER injects (heirloom-cpu-ppc makes no syscall and holds no
//! authority), and the arena that fills it under the PUBLISH model of the PowerPC ruling's phase 2: pages are written while
//! RW (never executable) and then published RX (never writable again until its segment is reused or the arena reset).
//! Nothing is ever patched in place: chaining goes through a RW link table (jit.rs), so code needs no write after its page
//! is published.
//!
//! [`HostCodeMem`] is the Linux x86-64 provider (raw mmap / mprotect / munmap syscalls, no dependency) for the host tests,
//! the bench and the WC3 harness. On CoralOS the provider will be macppc.engine's (syscall 47's publish form, delegated by
//! CapKind::ExecRegion: increment 2 of /build/patches/h86/ppc-fast-tier-plan.md, 2026-11-02); until a caller passes one,
//! Tier::Jit runs tier 1 and `JitStats::refused` names it.

/// Executable memory as a caller provides it. `reserve` once; then any page-aligned range may be published (RW -> RX) and
/// any page-aligned range withdrawn back to RW (one segment of the arena when it is reused, or the whole reservation).
pub trait CodeMem {
    /// Reserve `len` bytes (a multiple of 4096), readable and writable, NOT executable. None = refused.
    fn reserve(&mut self, len: usize) -> Option<*mut u8>;
    /// Make `[ptr, ptr + len)` (page aligned, inside the reservation) read + execute, no write. False = refused.
    fn publish(&mut self, ptr: *mut u8, len: usize) -> bool;
    /// Make `[ptr, ptr + len)` (page aligned, inside the reservation) read + write, not executable, again: no code of it runs
    /// afterwards (a segment being reused, or the whole arena).
    fn reset(&mut self, ptr: *mut u8, len: usize) -> bool;
}

pub(crate) const PAGE: usize = 4096;
/// The arena's first page: the trampoline and the shared stubs (never evicted; only a whole reset rewrites it).
pub(crate) const STUB_BYTES: usize = PAGE;
/// A reused segment is withdrawn (back to RW) this much at a time, just ahead of where code is written, not whole: on CoralOS a
/// withdraw is one TLB shootdown round per page, and a 4 MiB segment withdrawn at once was a 50-ms frame (1,024 pages) under
/// the host's model of that cost. Its blocks are all unreachable from the moment it is chosen, so the pages still RX until
/// their turn hold code nothing can run.
pub(crate) const WITHDRAW_CHUNK: usize = 64 << 10;

/// The code arena: `[base, base + len)` of a provider. Its first page holds the stubs; the rest is `nseg` equal SEGMENTS used
/// in turn (FIFO): code is written into the current segment from `cur`, published up to `published`, and when the current
/// segment is full the NEXT one is reused - the caller first drops every block that lives in it (`next_segment` says
/// which), then its pages go back to RW. So a full arena costs one segment's blocks, never all of them.
pub(crate) struct Arena {
    mem: alloc::boxed::Box<dyn CodeMem>,
    pub(crate) base: *mut u8,
    pub(crate) len: usize,
    seg_len: usize,
    nseg: usize,
    /// The segment written now; None while the stubs are written.
    seg: Option<usize>,
    /// Segments that have held published code since the last whole reset (bit per segment).
    used: u64,
    /// Everything of the current region below this is published (RX); `cur` >= it. Page aligned.
    published: usize,
    cur: usize,
    /// The end of the region written now (the stub page, or the current segment).
    limit: usize,
    /// Below this the current region is RW (written or writable); from it to `limit`, pages of a reused segment not yet
    /// withdrawn (still RX, unreachable).
    rw_end: usize,
    /// A withdraw the provider refused (the JIT stops: pages another CPU may still execute must not be written).
    pub(crate) withdraw_failed: bool,
    /// What the provider was asked (every call is a syscall on CoralOS, and every page a TLB shootdown there).
    pub(crate) io: CodeMemStats,
    /// Opt-in clock for `io.time` (the JIT's translation clock); None = no clock reads.
    pub(crate) timer: Option<fn() -> u64>,
    /// A segment has been reused since the last whole reset: every page written now costs older code.
    pub(crate) wrapped: bool,
}

/// The provider calls an arena made: on CoralOS each call is one EXEC_CODE_PROTECT syscall and each page that changes
/// state one TLB shootdown round (the h86 Z83 finding: a JIT that published a page at a time crawled at 1/40 of tier 1).
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeMemStats {
    /// publish calls, and the pages they covered.
    pub(crate) publishes: u64,
    pub(crate) publish_pages: u64,
    /// reset calls (a segment reused, or the whole arena), and the pages they covered.
    pub(crate) withdraws: u64,
    pub(crate) withdraw_pages: u64,
    /// Calls the provider refused.
    pub(crate) failures: u64,
    /// Time inside the provider, in the units of the JIT's clock (0 without one).
    pub(crate) time: u64,
}

impl Arena {
    /// `len` bytes: the stub page and `nseg` (1..=64) segments of whole pages.
    /// `len` bytes - or, while the provider refuses that, half as many, down to `min` (None below it). `len()` says what was
    /// reserved.
    pub(crate) fn new(mut mem: alloc::boxed::Box<dyn CodeMem>, len: usize, min: usize, nseg: usize) -> Option<Arena> {
        let nseg = nseg.clamp(1, 64);
        let mut len = len;
        let base = loop {
            if len < min || (len.saturating_sub(STUB_BYTES) / nseg) & !(PAGE - 1) == 0 { return None; }
            if let Some(b) = mem.reserve(len) { break b; }
            len = (len / 2) & !(PAGE - 1);
        };
        let seg_len = ((len - STUB_BYTES) / nseg) & !(PAGE - 1);
        Some(Arena { mem, base, len, seg_len, nseg, seg: None, used: 0, published: 0, cur: 0, limit: STUB_BYTES, rw_end: STUB_BYTES, withdraw_failed: false, io: CodeMemStats::default(), timer: None, wrapped: false })
    }
    fn seg_start(&self, s: usize) -> usize { STUB_BYTES + s * self.seg_len }
    /// One provider call, counted and timed: `publish` (RX) or not (back to RW).
    fn call(&mut self, publish: bool, ptr: *mut u8, len: usize) -> bool {
        let t0 = self.timer.map(|c| c());
        let ok = if publish { self.mem.publish(ptr, len) } else { self.mem.reset(ptr, len) };
        if let (Some(c), Some(t0)) = (self.timer, t0) { self.io.time += c().wrapping_sub(t0); }
        let pages = (len / PAGE) as u64;
        if publish { self.io.publishes += 1; self.io.publish_pages += pages; } else { self.io.withdraws += 1; self.io.withdraw_pages += pages; }
        self.io.failures += !ok as u64;
        ok
    }
    /// Forget the written, unpublished code (it was never reachable): the space is written again.
    pub(crate) fn discard(&mut self) { self.cur = self.published; }
    /// Copy `code` to the unpublished tail of the current region; None when it does not fit there.
    /// Bytes of one segment: no block's code may be longer.
    pub(crate) fn segment_bytes(&self) -> usize { self.seg_len }
    /// None when it does not fit (or a withdraw was refused: `withdraw_failed`).
    pub(crate) fn put(&mut self, code: &[u8]) -> Option<*const u8> {
        if self.cur + code.len() > self.limit { return None; }
        // a reused segment: withdraw the next chunk(s) first
        while self.cur + code.len() > self.rw_end {
            let n = WITHDRAW_CHUNK.min(self.limit - self.rw_end);
            // SAFETY: rw_end < limit <= len: the chunk is inside the reservation.
            if !self.call(false, unsafe { self.base.add(self.rw_end) }, n) { self.withdraw_failed = true; return None; }
            self.rw_end += n;
        }
        // SAFETY: [base + cur, base + cur + code.len()) is inside the reservation (limit <= len) and at or above `published`
        // in a region that was reserved RW or reset to RW, so it is RW and no published code lives there.
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
        let end = ((self.cur + PAGE - 1) & !(PAGE - 1)).min(self.limit);
        // SAFETY: published <= end <= limit <= len: the range is inside the reservation.
        let ok = self.call(true, unsafe { self.base.add(self.published) }, end - self.published);
        self.published = end;
        self.cur = end;
        if let Some(s) = self.seg { self.used |= 1 << s; }
        ok
    }
    /// Start writing blocks: the first segment (after the stubs are published).
    pub(crate) fn begin(&mut self) { self.enter(0); self.rw_end = self.limit; }
    fn enter(&mut self, s: usize) {
        self.seg = Some(s);
        self.cur = self.seg_start(s);
        self.published = self.cur;
        self.limit = self.cur + self.seg_len;
    }
    /// The segment after the current one, whose blocks the caller must drop BEFORE `reuse` makes its pages writable.
    pub(crate) fn next_segment(&self) -> usize { self.seg.map_or(0, |s| (s + 1) % self.nseg) }
    /// The address range of segment `s`.
    pub(crate) fn segment(&self, s: usize) -> (u64, u64) {
        let a = self.base as u64 + self.seg_start(s) as u64;
        (a, a + self.seg_len as u64)
    }
    /// Which segment `p` (an address in the arena) is in; None for the stub page.
    pub(crate) fn segment_of(&self, p: u64) -> Option<usize> {
        let off = p.checked_sub(self.base as u64)? as usize;
        (off >= STUB_BYTES && off < STUB_BYTES + self.nseg * self.seg_len).then(|| (off - STUB_BYTES) / self.seg_len)
    }
    /// Write into the next segment from its start (its blocks already dropped by the caller); if it ever held published code
    /// its pages are withdrawn to RW a chunk at a time as `put` reaches them. The current segment's written code must be
    /// published first.
    pub(crate) fn reuse(&mut self) {
        let s = self.next_segment();
        self.wrapped = true;
        let held_code = self.used & (1 << s) != 0;
        self.used &= !(1 << s);
        self.enter(s);
        self.rw_end = if held_code { self.cur } else { self.limit };
    }
    /// Withdraw every published page (back to RW) and start writing the stubs again.
    pub(crate) fn reset(&mut self) -> bool {
        self.seg = None;
        self.wrapped = false;
        self.used = 0;
        self.published = 0;
        self.cur = 0;
        self.limit = STUB_BYTES;
        self.rw_end = STUB_BYTES;
        self.call(false, self.base, self.len)
    }
    /// Bytes of code space written in the current segment (the arena's other segments hold older code).
    pub(crate) fn used(&self) -> usize { self.cur - self.seg.map_or(0, |s| self.seg_start(s)) }
    /// Bytes written and not yet published.
    pub(crate) fn unpublished(&self) -> usize { self.cur - self.published }
}

/// Linux x86-64 executable memory for the host (tests, bench, the WC3 harness): an anonymous private mapping, RW until
/// published, then RX.
#[cfg(all(feature = "host-codemem", target_os = "linux", target_arch = "x86_64"))]
#[derive(Default)]
pub struct HostCodeMem { map: Option<(usize, usize)> }

#[cfg(all(feature = "host-codemem", target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(feature = "host-codemem", target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(feature = "host-codemem", target_os = "linux", target_arch = "x86_64"))]
impl Drop for HostCodeMem {
    fn drop(&mut self) {
        if let Some((p, l)) = self.map.take() { Self::sys(11, p, l, 0, 0, 0, 0); }
    }
}
