//! Integration test: growing GuestMem must use alloc_zeroed, not resize.
//! A counting global allocator wrapping std::alloc::System records every
//! alloc, alloc_zeroed, and realloc call so we can assert the grow path
//! never reallocs and never plain-allocs the new tail.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use heirloom_mem_flat32::GuestMem;

const K4: usize = 4 * 1024;
const M64: usize = 64 * 1024 * 1024;

// --- Counting allocator (lock-free; never allocates inside hooks) ---

const OP_ALLOC: u8 = 1;
const OP_ZEROED: u8 = 2;
const OP_REALLOC: u8 = 3;

struct Counting;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ZEROED_COUNT: AtomicUsize = AtomicUsize::new(0);
static REALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static LAST_ZEROED_SIZE: AtomicUsize = AtomicUsize::new(0);
static LARGEST_PLAIN_ALLOC: AtomicUsize = AtomicUsize::new(0);
static LARGEST_REALLOC_SIZE: AtomicUsize = AtomicUsize::new(0);
static REFUSE_ABOVE: AtomicUsize = AtomicUsize::new(usize::MAX);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if l.size() > REFUSE_ABOVE.load(Ordering::Relaxed) {
            return core::ptr::null_mut();
        }
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        LARGEST_PLAIN_ALLOC.fetch_max(l.size(), Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if l.size() > REFUSE_ABOVE.load(Ordering::Relaxed) {
            return core::ptr::null_mut();
        }
        ZEROED_COUNT.fetch_add(1, Ordering::Relaxed);
        LAST_ZEROED_SIZE.store(l.size(), Ordering::Relaxed);
        System.alloc_zeroed(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, old: Layout, new: usize) -> *mut u8 {
        if new > REFUSE_ABOVE.load(Ordering::Relaxed) {
            return core::ptr::null_mut();
        }
        REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        LARGEST_REALLOC_SIZE.fetch_max(new, Ordering::Relaxed);
        System.realloc(p, old, new)
    }
}

#[global_allocator]
static A: Counting = Counting;

fn reset_counters() {
    ALLOC_COUNT.store(0, Ordering::Relaxed);
    ZEROED_COUNT.store(0, Ordering::Relaxed);
    REALLOC_COUNT.store(0, Ordering::Relaxed);
    LAST_ZEROED_SIZE.store(0, Ordering::Relaxed);
    LARGEST_PLAIN_ALLOC.store(0, Ordering::Relaxed);
    LARGEST_REALLOC_SIZE.store(0, Ordering::Relaxed);
    REFUSE_ABOVE.store(usize::MAX, Ordering::Relaxed);
}

#[test]
fn grow_preserves_old_and_zeroes_tail_via_alloc_zeroed() {
    reset_counters();

    let mut g = GuestMem::new(K4);
    // Write a pattern across the first 4 KiB.
    for i in 0..K4 {
        g.write_u8(i as u32, (i & 0xff) as u8).unwrap();
    }

    g.grow_to(M64);

    // (a) old bytes preserved.
    for i in (0..K4).step_by(257) {
        assert_eq!(g.read_u8(i as u32).unwrap(), (i & 0xff) as u8, "old byte at {i}");
    }

    // (b) tail is zero at several offsets.
    for off in [K4, K4 + 1, M64 / 2, M64 - 1, M64 - 4096] {
        assert_eq!(g.read_u8(off as u32).unwrap(), 0, "tail byte at {off}");
    }

    // (c) the allocator saw an alloc_zeroed of exactly 64 MiB and NO
    //     realloc/alloc of >= 64 MiB during the grow.
    assert_eq!(LAST_ZEROED_SIZE.load(Ordering::Relaxed), M64,
        "expected an alloc_zeroed of 64 MiB");
    assert!(ZEROED_COUNT.load(Ordering::Relaxed) >= 1);
    assert!(LARGEST_REALLOC_SIZE.load(Ordering::Relaxed) < M64,
        "realloc seen of size {} (resize-based grow reallocs the whole buffer)",
        LARGEST_REALLOC_SIZE.load(Ordering::Relaxed));
    assert!(LARGEST_PLAIN_ALLOC.load(Ordering::Relaxed) < M64,
        "plain alloc seen of size {} (grow must use alloc_zeroed, not alloc)",
        LARGEST_PLAIN_ALLOC.load(Ordering::Relaxed));
}

#[test]
fn try_grow_to_refusal_leaves_len_unchanged() {
    reset_counters();
    // Refuse anything >= 64 MiB. Set immediately before the call, clear
    // before any assertion so a failing assert doesn't abort the process.
    REFUSE_ABOVE.store(M64 - 1, Ordering::Relaxed);

    let mut g = GuestMem::new(K4);
    let before = g.len();
    let err = g.try_grow_to(M64).unwrap_err();

    REFUSE_ABOVE.store(usize::MAX, Ordering::Relaxed);

    assert_eq!(err.requested, M64);
    assert_eq!(g.len(), before);
}

#[test]
fn grow_to_smaller_or_equal_is_noop() {
    reset_counters();
    let mut g = GuestMem::new(K4);
    let snap: Vec<u8> = (0..K4).map(|i| (i & 0xff) as u8).collect();
    g.write_bytes(0, &snap).unwrap();

    g.grow_to(K4);      // equal
    g.grow_to(K4 / 2);  // smaller

    assert_eq!(g.len(), K4);
    for i in 0..K4 {
        assert_eq!(g.read_u8(i as u32).unwrap(), snap[i], "byte at {i}");
    }
}