//! The Mixed Mode seam: the 68k Toolbox handlers called by value (the way PowerPC code calls them), and proof that it
//! is the SAME code path the 68k A-line bridge runs.
use guestui_abi::{RecordingGuestUi, UiCall};
use mac68k_engine::fixture::HELLO;
use mac68k_engine::mixed::{os_sig, trap_opcode, MixedError, OS_SIGS};
use mac68k_engine::Mac;

fn mac() -> Mac<RecordingGuestUi> { Mac::new(RecordingGuestUi::new()).unwrap() }

#[test]
fn every_c_callable_memory_routine_names_a_registered_os_trap() {
    let m = mac();
    for s in OS_SIGS {
        let op = trap_opcode(s.name).unwrap_or_else(|| panic!("{} is not a trap name", s.name));
        assert!(m.is_os(op), "{} (${op:04X}) has no OS-convention handler", s.name);
    }
    // negative control: a Toolbox (Pascal) trap is not in the OS table, and an OS routine is not Pascal
    let nw = trap_opcode("NewWindow").unwrap();
    assert!(m.is_pascal(nw) && !m.is_os(nw) && os_sig("NewWindow").is_none());
    assert!(!m.is_pascal(trap_opcode("NewHandle").unwrap()));
}

#[test]
fn new_handle_get_size_resize_lock_and_dispose_by_value() {
    let mut m = mac();
    let call = |m: &mut Mac<RecordingGuestUi>, name: &str, args: &[u32]| m.os_call(trap_opcode(name).unwrap(), os_sig(name).unwrap(), args).unwrap();
    let h = call(&mut m, "NewHandle", &[100]).value;
    assert_ne!(h, 0);
    assert_eq!(m.mem_error(), 0);
    assert_eq!(call(&mut m, "GetHandleSize", &[h]).value, 100);
    call(&mut m, "HLock", &[h]);
    assert_eq!(call(&mut m, "HGetState", &[h]).value, 0x80);
    call(&mut m, "SetHandleSize", &[h, 4096]);
    assert_eq!(m.mem_error(), -117, "locked handles do not resize: the 68k handler's answer, unchanged");
    call(&mut m, "HUnlock", &[h]);
    call(&mut m, "SetHandleSize", &[h, 4096]);
    assert_eq!((m.mem_error(), call(&mut m, "GetHandleSize", &[h]).value), (0, 4096));
    call(&mut m, "DisposeHandle", &[h]);
    assert_eq!(m.mm.live_handles(), 0);
    // negative control: a stale handle is the same NAMED refusal the 68k test sees
    call(&mut m, "GetHandleSize", &[h]);
    assert_eq!(m.mem_error(), -111);
    assert!(m.refused.iter().any(|r| r.starts_with("GetHandleSize:") && r.contains("memWZErr")), "{:?}", m.refused);
}

#[test]
fn the_clear_and_sys_variants_keep_their_flag_bits() {
    let mut m = mac();
    let p = m.os_call(trap_opcode("NewPtrClear").unwrap(), os_sig("NewPtrClear").unwrap(), &[16]).unwrap().value;
    m.mem.write_u32(p, 0xFFFF_FFFF).unwrap();
    m.os_call(trap_opcode("DisposePtr").unwrap(), os_sig("DisposePtr").unwrap(), &[p]).unwrap();
    // the freed block comes back cleared when asked to be, and not otherwise (the flag is read from the trap word)
    let q = m.os_call(trap_opcode("NewPtrSysClear").unwrap(), os_sig("NewPtrSysClear").unwrap(), &[16]).unwrap().value;
    assert_eq!(m.mem.read_u32(q).unwrap(), 0);
    let mut dirty = mac();
    let p = dirty.os_call(trap_opcode("NewPtr").unwrap(), os_sig("NewPtr").unwrap(), &[16]).unwrap().value;
    dirty.mem.write_u32(p, 0xFFFF_FFFF).unwrap();
    dirty.os_call(trap_opcode("DisposePtr").unwrap(), os_sig("DisposePtr").unwrap(), &[p]).unwrap();
    let q = dirty.os_call(trap_opcode("NewPtrSys").unwrap(), os_sig("NewPtrSys").unwrap(), &[16]).unwrap().value;
    assert_eq!(q, p, "the heap reuses the block");
    assert_eq!(dirty.mem.read_u32(q).unwrap(), 0xFFFF_FFFF, "NewPtr (no Clear) leaves the old bytes");
}

#[test]
fn block_move_by_value_and_its_refusal() {
    let mut m = mac();
    m.mem.write_bytes(0x4000, b"ABCDEFGH").unwrap();
    m.os_call(trap_opcode("BlockMove").unwrap(), os_sig("BlockMove").unwrap(), &[0x4000, 0x4100, 8]).unwrap();
    assert_eq!(m.mem.bytes(m.mem.span(0x4100, 8).unwrap()).unwrap(), b"ABCDEFGH");
    m.os_call(trap_opcode("BlockMove").unwrap(), os_sig("BlockMove").unwrap(), &[0x4000, 0xFFFF_FF00, 0x1000]).unwrap();
    assert_eq!(m.mem_error(), -111, "memWZErr: the 68k handler's answer for a copy that leaves guest RAM");
    assert!(m.refused.iter().any(|r| r.starts_with("BlockMove:")), "{:?}", m.refused);
}

#[test]
fn wrong_arity_wrong_convention_and_unimplemented_are_named_errors_not_calls() {
    let mut m = mac();
    let nh = trap_opcode("NewHandle").unwrap();
    assert!(matches!(m.os_call(nh, os_sig("NewHandle").unwrap(), &[1, 2]), Err(MixedError::ArgCount { want: 1, got: 2, .. })));
    assert!(matches!(m.os_call(trap_opcode("NewWindow").unwrap(), os_sig("NewHandle").unwrap(), &[1]), Err(MixedError::WrongConvention(_))));
    assert!(matches!(m.pascal_call(nh, &[1]), Err(MixedError::WrongConvention(_))));
    assert!(matches!(m.pascal_call(trap_opcode("NewWindow").unwrap(), &[0]), Err(MixedError::ArgCount { want: 8, got: 1, .. })));
    let sysbeep = trap_opcode("SysBeep").unwrap();
    assert!(matches!(m.pascal_call(sysbeep, &[1]), Err(MixedError::Unimplemented(n)) if n == "SysBeep"));
    assert!(m.log.is_empty() && m.refused.is_empty(), "structural errors never reach a handler");
}

/// The proof of reuse: the program HELLO runs NewWindow through the 68k A-line bridge; the same call made by value
/// goes through the same handler and produces the same log line and the same GuestUi window.
#[test]
fn new_window_by_value_is_the_same_handler_as_the_68k_bridge() {
    let mut bridge = mac();
    bridge.run_program(HELLO, 0, 10_000).unwrap();
    let want_log = bridge.log.iter().find(|l| l.contains("NewWindow")).unwrap().clone();
    let want_model = bridge.ui.calls.iter().find_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).unwrap();

    let mut m = mac();
    m.mem.write_bytes(0x4000, &[0, 40, 0, 30, 0, 240, 1, 94]).unwrap(); // Rect 40,30,240,350
    m.mem.write_bytes(0x4010, b"\x09Hello Mac").unwrap();
    let nw = trap_opcode("NewWindow").unwrap();
    // wStorage, boundsRect, title, visible, procID, behind, goAwayFlag, refCon
    let r = m.pascal_call(nw, &[0, 0x4000, 0x4010, 0xFFFF_FF01, 0xFFFF_0000, 0xFFFF_FFFF, 0xFFFF_FF01, 0]).unwrap();
    assert!(!r.halted && !r.blocked && r.value != 0);
    let got_log = m.log.iter().find(|l| l.contains("NewWindow")).unwrap();
    let cut = |s: &str| s.split(" -> ").next().unwrap().to_string();
    assert_eq!(cut(got_log), cut(&want_log), "same handler, same log line");
    let got_model = m.ui.calls.iter().find_map(|c| if let UiCall::OpenWindow(w) = c { Some(w.clone()) } else { None }).unwrap();
    assert_eq!((got_model.title.clone(), got_model.rect, got_model.role, got_model.closable, got_model.resizable),
               (want_model.title.clone(), want_model.rect, want_model.role, want_model.closable, want_model.resizable));
    assert!(m.windows.contains_key(&r.value), "the WindowPtr that came back is the window the handler opened");
    // Booleans and shorts are truncated to their slot: the junk above bit 7 / bit 15 of a PowerPC register is ignored
    // (it was 0xFFFF_FF01 above and the window is visible, closable) and a result is what the handler returned.
    // Negative control: a refusal is the same named refusal, and the call returns 0 like the 68k does.
    let empty = pascal_new_window_with_empty_rect(&mut m);
    assert_eq!(empty, 0);
    assert!(m.refused.iter().any(|r| r.starts_with("NewWindow:") && r.contains("empty boundsRect")), "{:?}", m.refused);
}

fn pascal_new_window_with_empty_rect(m: &mut Mac<RecordingGuestUi>) -> u32 {
    m.mem.write_bytes(0x4100, &[0; 8]).unwrap();
    m.pascal_call(trap_opcode("NewWindow").unwrap(), &[0, 0x4100, 0x4010, 1, 0, 0xFFFF_FFFF, 1, 0]).unwrap().value
}

#[test]
fn get_resource_and_exit_to_shell_by_value() {
    let mut m = mac();
    // no resource file is open: GetResource answers NIL and sets ResErr, as on the 68k
    let get = trap_opcode("GetResource").unwrap();
    let r = m.pascal_call(get, &[u32::from_be_bytes(*b"WIND"), 128]).unwrap();
    assert_eq!(r.value, 0);
    let res_error = m.pascal_call(trap_opcode("ResError").unwrap(), &[]).unwrap();
    assert_eq!(res_error.value as u16 as i16, -192, "resNotFound");
    // ExitToShell asks to halt the CALLER's program, not the process
    let exit = m.pascal_call(trap_opcode("ExitToShell").unwrap(), &[]).unwrap();
    assert!(exit.halted);
    assert!(!m.pascal_call(trap_opcode("InitWindows").unwrap(), &[]).unwrap().halted, "negative control: other calls do not halt");
}

#[test]
fn a_blocking_handler_called_without_68k_registers_is_refused_and_leaves_no_wait_behind() {
    let mut m = mac();
    let wne = trap_opcode("WaitNextEvent").unwrap();
    // sleep > 0 and nothing pending would block the (nonexistent) 68k caller: refused by name, no pending wait remains
    let e = m.pascal_call(wne, &[0xFFFF, 0x6000, 30, 0]).unwrap_err();
    assert!(matches!(e, MixedError::NeedsCpuRegisters(ref n) if n == "WaitNextEvent"), "{e}");
    assert!(m.ev.waiting.is_none(), "a later 68k step must not deliver an event into low memory");
    // negative control: sleep 0 polls and returns the null event without touching registers
    let r = m.pascal_call(wne, &[0xFFFF, 0x6000, 0, 0]).unwrap();
    assert!(!r.blocked && r.value == 0);
}
