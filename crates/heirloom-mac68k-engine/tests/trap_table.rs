//! _GetTrapAddress / _NGetTrapAddress ($A346 OS, $A746 Toolbox): the address of a stub that runs the same bridged trap, and
//! the `_Unimplemented` stub for a trap nobody registered (what a program probes for before using a System 7 call).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::Mac;

fn words(w: &[u16]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }

#[test]
fn a_probed_trap_answers_with_its_own_stub_or_with_the_unimplemented_stub_and_the_stub_really_runs_the_trap() {
    // d0 = number; A746/A346 = GetToolTrapAddress/GetOSTrapAddress; move.l a0,$300x
    let code = words(&[
        0x303C, 0x0113, 0xA746, 0x23C8, 0x0000, 0x3000,       // NewWindow (tool $113)            -> $3000
        0x303C, 0x001E, 0xA346, 0x23C8, 0x0000, 0x3004,       // NewPtr (OS $1E)                  -> $3004
        0x303C, 0x03FF, 0xA746, 0x23C8, 0x0000, 0x3008,       // tool $3FF: nobody registered it  -> $3008
        0x303C, 0x009F, 0xA746, 0x23C8, 0x0000, 0x300C,       // _Unimplemented itself            -> $300C
        0x303C, 0xA11E, 0xA346, 0x23C8, 0x0000, 0x3014,       // the WHOLE trap word of NewPtr ($A11E) is masked to its 8-bit number
        0x303C, 0xA913, 0xA746, 0x23C8, 0x0000, 0x3018,       // NewWindow given as the whole word $A913: the same stub as number $113
        0x7010, 0x4EB9, 0x0000, 0x5078, 0x23C8, 0x0000, 0x3010, // NewPtr(16) through the stub     -> a0 -> $3010
        0xA9F4,                                               // ExitToShell
    ]);
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let r = m.run_program(&code, 0, 50_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:#?}", m.log);
    let (win, ptr, absent, unimpl) = (l(&m, 0x3000), l(&m, 0x3004), l(&m, 0x3008), l(&m, 0x300C));
    assert_eq!((win, ptr), (0x4000 + 4 * 0x113, 0x5000 + 4 * 0x1E), "each registered trap answers with its own stub");
    assert_eq!(m.mem.read_u16(win).unwrap(), 0xA913, "the stub is the trap word ...");
    assert_eq!(m.mem.read_u16(win + 2).unwrap(), 0x4E75, "... then RTS");
    assert_eq!(absent, unimpl, "an unregistered trap answers with the _Unimplemented stub, which is what programs compare with");
    assert_eq!(l(&m, 0x3018), win, "a whole trap word and the bare number name the same trap");
    assert_eq!(l(&m, 0x3014), ptr, "a whole OS trap word and the bare number name the same trap (the ROM masks to 8 bits)");
    assert_ne!(absent, win, "NEGATIVE CONTROL: a registered trap does not look absent");
    assert_ne!(l(&m, 0x3010), 0, "calling the NewPtr stub ran the real handler: a pointer came back");
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}
