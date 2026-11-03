//! `WC3_HEAP=lla`: the harness process allocates through linked_list_allocator 0.10 - first fit over an address-sorted hole list,
//! the allocator coralos's coral-lazy-heap is built on (a CoralOS process's heap; with nothing in front of it, every alloc and free
//! walks the list) - instead of glibc's malloc, so a host run shows what the engine's allocations cost on such a heap. Off (the default) everything goes to the system allocator. Either way the
//! heap's calls and the TSC cycles spent in them are counted ([`counts`]) for the harness's per-window lines. What the HARNESS
//! does (the mock worker, which on CoralOS is another process) runs [`outside`]: on the system allocator and not counted, so the
//! linked-list heap holds the engine's allocations alone, as macppc.engine's heap does.
use linked_list_allocator::LockedHeap;
use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed};

pub struct Switch;
static ON: AtomicBool = AtomicBool::new(false);
static HEAP: LockedHeap = LockedHeap::empty();
static BASE: AtomicUsize = AtomicUsize::new(0);
static END: AtomicUsize = AtomicUsize::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
static CYCLES: AtomicU64 = AtomicU64::new(0);
thread_local! { static HARNESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// Run `f` as the harness: its allocations go to the system allocator and are not counted.
pub fn outside<R>(f: impl FnOnce() -> R) -> R {
    let was = HARNESS.with(|h| h.replace(true));
    let r = f();
    HARNESS.with(|h| h.set(was));
    r
}

fn engine() -> bool { !HARNESS.with(|h| h.get()) }

/// `WC3_HEAP_SITES=<n>`: every n-th engine allocation from [`sample`] on records the engine frames of its call stack.
static EVERY: AtomicU64 = AtomicU64::new(0);
static SITES: std::sync::Mutex<Option<std::collections::BTreeMap<String, u64>>> = std::sync::Mutex::new(None);

/// Start recording every `every`-th engine allocation's call site (0 = off).
pub fn sample(every: u64) {
    if EVERY.swap(every, Relaxed) != every { outside(|| eprintln!("[WC3-HEAP] recording every {every}-th engine allocation's call site")); }
}

fn record() {
    outside(|| {
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        // the first frames that are the engine's (or its libraries'), not the allocator's or std's
        let frames: Vec<String> = bt.lines().map(str::trim).filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
            .filter_map(|l| l.split_once(": ").map(|(_, f)| f.to_string()))
            .filter(|f| f.contains("heirloom") && !f.contains("wc3_heap"))
            .map(|f| f.split("::<").next().unwrap_or(&f).replace("heirloom_macppc_engine::machine::MacPpc<heirloom_ui_abi::recording::RecordingGuestUi>", "M"))
            .take(4).collect();
        let mut g = SITES.lock().unwrap();
        *g.get_or_insert_with(Default::default).entry(frames.join(" < ")).or_insert(0) += 1;
    });
}

/// The recorded call sites, most frequent first.
pub fn sites() -> Vec<(u64, String)> {
    let g = outside(|| SITES.lock().unwrap());
    let mut v: Vec<(u64, String)> = g.iter().flatten().map(|(k, n)| (*n, k.clone())).collect();
    v.sort_unstable_by(|a, b| b.cmp(a));
    v
}

fn tsc() -> u64 {
    // SAFETY: RDTSC reads a counter and touches no memory.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Route every later allocation to a linked-list heap of `bytes` (reserved from the system, touched only as it is used).
pub fn enable(bytes: usize) {
    let layout = Layout::from_size_align(bytes, 4096).unwrap();
    // SAFETY: a non-zero layout; the block is handed to the heap for the rest of the process and never freed.
    let p = unsafe { System.alloc(layout) };
    assert!(!p.is_null(), "no {bytes} bytes for the linked-list heap");
    // SAFETY: `p .. p + bytes` is a fresh block this heap owns alone.
    unsafe { HEAP.lock().init(p, bytes) };
    BASE.store(p as usize, Relaxed);
    END.store(p as usize + bytes, Relaxed);
    ON.store(true, Relaxed);
}

/// (heap calls, TSC cycles spent in them) so far.
pub fn counts() -> (u64, u64) { (CALLS.load(Relaxed), CYCLES.load(Relaxed)) }

fn ours(p: *mut u8) -> bool { (BASE.load(Relaxed)..END.load(Relaxed)).contains(&(p as usize)) }

// SAFETY: every block is served by exactly one allocator and returned to the one that served it (by address range); the counters
// are side effects only.
unsafe impl GlobalAlloc for Switch {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if !engine() {
            // SAFETY: the caller's contract for `alloc` is passed through.
            return unsafe { System.alloc(l) };
        }
        let t = tsc();
        let p = if ON.load(Relaxed) {
            HEAP.lock().allocate_first_fit(l).map_or(std::ptr::null_mut(), NonNull::as_ptr)
        } else {
            // SAFETY: the caller's contract for `alloc` is passed through.
            unsafe { System.alloc(l) }
        };
        let n = CALLS.fetch_add(1, Relaxed);
        CYCLES.fetch_add(tsc().wrapping_sub(t), Relaxed);
        let every = EVERY.load(Relaxed);
        if every != 0 && n % every == 0 { record(); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let counted = engine();
        let t = tsc();
        if ours(p) {
            // SAFETY: `p` came from this heap with layout `l` (its address is in the heap's block).
            unsafe { HEAP.lock().deallocate(NonNull::new_unchecked(p), l) }
        } else {
            // SAFETY: `p` came from System with layout `l`.
            unsafe { System.dealloc(p, l) }
        }
        if counted {
            CALLS.fetch_add(1, Relaxed);
            CYCLES.fetch_add(tsc().wrapping_sub(t), Relaxed);
        }
    }
}
