//! A zero-size request must never reach the GLOBAL allocator. `GlobalAlloc::alloc`'s contract makes a zero-size layout the caller's
//! bug, and the CoralOS heap answers it with NULL, which the runtime turns into `memory allocation of 0 bytes failed`. This binary
//! installs an allocator that records every zero-size `alloc`/`alloc_zeroed`/`realloc` (with a backtrace) and then loads, starts and
//! runs the engine over the PEF fixture and, when the local corpus has it, the real Glider PRO 1.1.2 PowerPC application
//! (`GLIDER_PRO_PPC`; a missing file is skipped only with `RETRO_ALLOW_MISSING_REFS`).
use guestui_abi::RecordingGuestUi;
use macppc_engine::source::fragment_bytes;
use macppc_engine::MacPpc;
use retro_pef::Pef;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, PpcHost};

struct Watch;
/// The largest single request since the last reset: a guest-sized length must never become a host-sized allocation.
static LARGEST: AtomicUsize = AtomicUsize::new(0);
static ZERO: Mutex<Vec<String>> = Mutex::new(Vec::new());
thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }

fn note(what: &str) {
    if BUSY.with(|b| b.replace(true)) { return; }
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    ZERO.lock().unwrap().push(format!("{what}\n{bt}"));
    BUSY.with(|b| b.set(false));
}
unsafe impl GlobalAlloc for Watch {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { LARGEST.fetch_max(l.size(), Ordering::Relaxed); if l.size() == 0 { note("alloc(0)"); } System.alloc(l) }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { LARGEST.fetch_max(l.size(), Ordering::Relaxed); if l.size() == 0 { note("alloc_zeroed(0)"); } System.alloc_zeroed(l) }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { System.dealloc(p, l) }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { LARGEST.fetch_max(n, Ordering::Relaxed); if n == 0 { note("realloc(.., 0)"); } System.realloc(p, l, n) }
}
#[global_allocator]
static A: Watch = Watch;

fn assert_no_zero(what: &str) {
    let z = ZERO.lock().unwrap();
    assert!(z.is_empty(), "{what}: {} zero-size allocation(s) reached the global allocator; first:\n{}", z.len(), z[0]);
}

#[test]
fn the_fixture_application_never_asks_the_allocator_for_zero_bytes() {
    let (name, bytes) = fragment_bytes(&macppc_engine::fixture::macbinary_app(&macppc_engine::fixture::startup_app()), None).unwrap();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate(&name, &Pef::parse(&bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let _ = m.run_loop(1_000_000);
    assert_no_zero("fixture");
}

#[test]
fn glider_pro_for_powerpc_never_asks_the_allocator_for_zero_bytes() {
    let path = std::env::var("GLIDER_PRO_PPC").unwrap_or_else(|_| "/data/build/corpus-bin/mac/glider-pro/fx/Glider PRO\u{2122}.bin".into());
    let file = match std::fs::read(&path) {
        Ok(f) => f,
        Err(e) if std::env::var("RETRO_ALLOW_MISSING_REFS").is_ok() => { eprintln!("SKIPPED: {path}: {e}"); return }
        Err(e) => panic!("{path}: {e}; set GLIDER_PRO_PPC or RETRO_ALLOW_MISSING_REFS=1"),
    };
    let (name, bytes) = fragment_bytes(&file, None).unwrap();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.attach_resources(&file).unwrap();
    let frag = m.instantiate(&name, &Pef::parse(&bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    let _ = m.run_loop(50_000_000);
    assert_no_zero("Glider PRO");
}

#[test]
fn the_detector_sees_a_zero_size_request_negative_control() {
    // Without this, "no zero-size allocation" above could be an allocator that never looks.
    let before = ZERO.lock().unwrap().len();
    let l = Layout::from_size_align(0, 1).unwrap();
    // SAFETY: only to prove the watcher fires; System's malloc(0) answers a unique pointer and it is freed with the same layout.
    unsafe { let p = A.alloc(l); A.dealloc(p, l); }
    let after = ZERO.lock().unwrap().len();
    assert_eq!(after, before + 1, "alloc(0) must be recorded");
    ZERO.lock().unwrap().truncate(before);
}

#[test]
fn a_guest_chosen_length_is_refused_by_name_before_any_host_allocation_is_sized_from_it() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert!(!matches!(m.bind_from("CarbonLib", "CFStringCreateWithCharacters"), Binding::Unbound(_)));
    let mut regs = [0u32; 32];
    regs[4] = 0x2000; // UniChars at a valid address ...
    regs[5] = 0xFFFF_FFF0; // ... and a length no guest RAM holds
    LARGEST.store(0, Ordering::Relaxed);
    let out = m.glue_call("CFStringCreateWithCharacters", &mut regs);
    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(matches!(out, GlueOutcome::Refused(ref why) if why.contains("leave guest RAM")), "{out:?}");
    assert!(largest < 1 << 20, "a {largest}-byte host allocation was sized from the guest's length");
}
