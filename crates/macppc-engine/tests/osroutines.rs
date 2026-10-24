//! OS routines with pointer outputs or packed registers, called as a PowerPC program calls them (r3.. = the C arguments):
//! `FlushEvents`, `Delay`, `PurgeSpace`, `MaxMem`, the Handle utilities and `Gestalt` (`macppc_engine::osroutines`).
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use mac68k_engine::events::MacEvent;

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3300;
const BUF: u32 = 0x3400;

fn call_raw(m: &mut M, name: &str, args: &[u32]) -> (GlueOutcome, u32) {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(name, &mut regs);
    (o, regs[3])
}
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 { let (o, r) = call_raw(m, name, args); assert_eq!(o, GlueOutcome::Return, "{name}: {:?}", m.mac.refused); r }
fn ev(what: u16) -> MacEvent { MacEvent { what, message: 0, when: 0, v: 0, h: 0, modifiers: 0 } }

#[test]
fn flush_events_packs_event_mask_and_stop_mask_into_the_register_it_was_given() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // mouseDown = 1, keyDown = 3: masks are 1 << what
    m.mac.ev.queue.extend([ev(1), ev(3), ev(3)]);
    call(&mut m, "FlushEvents", &[1 << 3, 0]);
    assert_eq!(m.mac.ev.queue.iter().map(|e| e.what).collect::<Vec<_>>(), [1], "both keyDowns flushed, the mouseDown kept");
    // Negative control: a stopMask hit first protects what follows it (proves the second argument reaches the high half of D0)
    m.mac.ev.queue.extend([ev(3)]);
    call(&mut m, "FlushEvents", &[1 << 3, 1 << 1]);
    assert_eq!(m.mac.ev.queue.iter().map(|e| e.what).collect::<Vec<_>>(), [1, 3], "stopped at the mouseDown: nothing flushed");
}

#[test]
fn delay_purgespace_and_maxmem_store_their_results_through_the_callers_pointers() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let t0 = m.mac.ev.ticks;
    call(&mut m, "Delay", &[30, OUT]);
    assert_eq!(m.mac.ev.ticks, t0 + 30);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), t0 + 30, "finalTicks");
    call(&mut m, "PurgeSpace", &[OUT, OUT + 4]);
    let (total, contig) = (m.mac.mem.read_u32(OUT).unwrap(), m.mac.mem.read_u32(OUT + 4).unwrap());
    assert!(total > 0 && contig > 0 && contig <= total, "{total} {contig}");
    m.mac.mem.write_u32(OUT + 8, 0xFFFF_FFFF).unwrap();
    let max = call(&mut m, "MaxMem", &[OUT + 8]);
    assert!(max > 0);
    assert_eq!(m.mac.mem.read_u32(OUT + 8).unwrap(), 0, "the grow amount: the heap cannot grow");
    // a result pointer outside RAM is refused by name, not ignored
    let (o, _) = call_raw(&mut m, "PurgeSpace", &[0xF000_0000, OUT]);
    assert!(matches!(o, GlueOutcome::Refused(ref w) if w.contains("PurgeSpace") && w.contains("outside guest RAM")), "{o:?}");
}

#[test]
fn the_handle_utilities_copy_append_and_report_errors() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_bytes(BUF, b"abcdef").unwrap();
    // PtrToHand(srcPtr, &dstHandle, size)
    assert_eq!(call(&mut m, "PtrToHand", &[BUF, OUT, 6]), 0);
    let h = m.mac.mem.read_u32(OUT).unwrap();
    assert_eq!(m.mac.handle_bytes(h).unwrap(), b"abcdef");
    // PtrAndHand(ptr, handle, size) appends
    assert_eq!(call(&mut m, "PtrAndHand", &[BUF, h, 2]), 0);
    assert_eq!(m.mac.handle_bytes(h).unwrap(), b"abcdefab");
    // HandToHand(&handle) replaces the handle with a COPY
    assert_eq!(call(&mut m, "HandToHand", &[OUT]), 0);
    let copy = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(copy, h);
    assert_eq!(m.mac.handle_bytes(copy).unwrap(), b"abcdefab");
    assert_eq!(call(&mut m, "HandAndHand", &[copy, h]), 0);
    assert_eq!(m.mac.handle_bytes(h).unwrap().len(), 16);
    // Negative control: a bad source handle is memWZErr (-111) and writes nothing
    m.mac.mem.write_u32(OUT, 0x1234).unwrap();
    assert_eq!(call(&mut m, "HandToHand", &[OUT]) as i32, -111);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0x1234);
}

#[test]
fn gestalt_answers_the_powerpc_architecture_and_still_refuses_what_it_cannot_answer() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(OUT, 0).unwrap();
    assert_eq!(call(&mut m, "Gestalt", &[u32::from_be_bytes(*b"sysa"), OUT]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 2, "gestaltPowerPC");
    // the shared answers still apply
    assert_eq!(call(&mut m, "Gestalt", &[u32::from_be_bytes(*b"vers"), OUT]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 1);
    // Negative control: an unknown selector is gestaltUndefSelectorErr and the response is untouched
    m.mac.mem.write_u32(OUT, 0xCAFE).unwrap();
    assert_eq!(call(&mut m, "Gestalt", &[u32::from_be_bytes(*b"zzzz"), OUT]) as i32, -5551);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xCAFE);
}

#[test]
fn the_powerpc_gestalt_profile_answers_only_what_is_true_and_names_what_it_will_not() {
    use macppc_engine::gestalt_ppc::{answer, unanswered, UNANSWERED};
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let sel = |s: &[u8; 4]| u32::from_be_bytes(*s);
    // true of this engine: PowerPC, a G3-class (750) processor, CFM present, no AltiVec
    for (s, v) in [(b"sysa", 2u32), (b"cput", 0x108), (b"cfrg", 1), (b"altv", 0)] {
        m.mac.mem.write_u32(OUT, 0xFFFF).unwrap();
        assert_eq!(call(&mut m, "Gestalt", &[sel(s), OUT]), 0);
        assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), v, "{}", String::from_utf8_lossy(s));
    }
    // not true yet: unanswered BY NAME, the response untouched, the reason in the log
    for (s, why) in UNANSWERED {
        m.mac.mem.write_u32(OUT, 0xCAFE).unwrap();
        assert_eq!(call(&mut m, "Gestalt", &[sel(s), OUT]) as i32, -5551, "{}", String::from_utf8_lossy(*s));
        assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xCAFE);
        let name = String::from_utf8_lossy(*s).to_string();
        assert!(m.mac.log.iter().any(|l| l.contains(&format!("Gestalt '{name}'")) && l.contains(why)), "{name}: {:?}", m.mac.log.last());
    }
    for s in ["cbon", "appr", "nav ", "thds", "mpsv", "sysv"] { assert!(unanswered(*s.as_bytes().first_chunk::<4>().unwrap()).is_some(), "{s}"); }
    // NEGATIVE CONTROL: nothing is both answered and unanswered
    for (s, _) in UNANSWERED { assert!(answer(**s).is_none(), "{}", String::from_utf8_lossy(*s)); }
}
