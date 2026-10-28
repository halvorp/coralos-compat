//! Machine profiles (profile.rs): the CPU model is part of the machine, chosen by NAME from the bundle's data, and it decides the
//! exception stack frame. A 68000 pushes SR + PC (6 bytes); a 68020 pushes a format word as well (8 bytes), so a TRAP handler that
//! cleans up by size (`addq.w #2,sp ; rts`, which is what MacWrite 4.5 installs for TRAP #4) only balances the stack on the former.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::Mac;

/// move.l sp,d5 ; trap #4 ; move.l sp,d6 ; move.l d5,$3000 ; move.l d6,$3004 ; _ExitToShell
const PROG: &[u8] = &[0x2A, 0x0F, 0x4E, 0x44, 0x2C, 0x0F, 0x23, 0xC5, 0x00, 0x00, 0x30, 0x00, 0x23, 0xC6, 0x00, 0x00, 0x30, 0x04, 0xA9, 0xF4];
const HANDLER: u32 = 0x4000;

/// TRAP #4's handler, the way MacWrite 4.5 writes it: addq.w #2,sp ; rts.
fn run(profile: Option<&str>) -> (u32, u32) {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    if let Some(p) = profile { m.set_machine(p).unwrap(); }
    m.mem.write_bytes(HANDLER, &[0x54, 0x4F, 0x4E, 0x75]).unwrap();
    m.mem.write_u32(36 * 4, HANDLER).unwrap(); // vector 36 = TRAP #4
    m.start(PROG, 0).unwrap();
    let r = m.run_loop(10_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:#?}", r.stats, m.log);
    (m.mem.read_u32(0x3000).unwrap(), m.mem.read_u32(0x3004).unwrap())
}

#[test]
fn the_68000_profile_pushes_a_six_byte_frame_and_the_default_68020_an_eight_byte_one() {
    let (before, after) = run(Some("mac-plus"));
    assert_eq!(before, after, "68000: the handler's addq.w #2,sp ; rts leaves the stack exactly as it was");
    // NEGATIVE CONTROL: the default machine (68020) and an explicit mac-ii: the format word stays on the stack, two bytes short.
    for p in [None, Some("mac-ii")] {
        let (before, after) = run(p);
        assert_eq!(before - after, 2, "68020: the 8-byte frame leaves 2 bytes behind ({p:?})");
    }
    let (before, after) = run(Some("mac-se"));
    assert_eq!(before, after, "mac-se is a 68000 too");
}

#[test]
fn an_unknown_machine_is_a_named_refusal_and_changes_nothing() {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    let e = m.set_machine("mac-zzz").unwrap_err();
    assert!(e.0.contains("unknown machine profile \"mac-zzz\"") && e.0.contains("mac-plus"), "{}", e.0);
    assert_eq!(m.profile.name, "mac-ii", "still the default");
    m.set_machine("MAC-PLUS").unwrap();
    assert_eq!(m.profile.name, "mac-plus", "names are ASCII case-insensitive");
}

#[test]
fn the_profile_is_what_gestalt_and_sysenvirons_answer() {
    // 'mach' / 'cput' / 'proc': mac-plus is gestaltMacPlus (4) with a 68000 (cput 0, SysEnvirons env68000 = 1); mac-ii is machine 6, 68020.
    let ask = |p: &str, sel: &[u8; 4]| {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        m.set_machine(p).unwrap();
        let sel = u32::from_be_bytes(*sel);
        let mut prog = vec![0x20, 0x3C, (sel >> 24) as u8, (sel >> 16) as u8, (sel >> 8) as u8, sel as u8, 0xA1, 0xAD, 0x23, 0xC8, 0x00, 0x00, 0x30, 0x00, 0xA9, 0xF4];
        m.start(&std::mem::take(&mut prog), 0).unwrap();
        m.run_loop(10_000).unwrap();
        m.mem.read_u32(0x3000).unwrap()
    };
    assert_eq!((ask("mac-plus", b"mach"), ask("mac-plus", b"cput"), ask("mac-plus", b"proc")), (4, 0, 1));
    assert_eq!((ask("mac-ii", b"mach"), ask("mac-ii", b"cput"), ask("mac-ii", b"proc")), (6, 2, 3));
}

// ---- the real application ------------------------------------------------------------------------------------------------------

fn launch_macwrite_45(profile: &str) -> Option<Mac<RecordingGuestUi>> {
    let path = "/data/build/corpus-bin/mac/macwrite/u45/MacWrite.rsrc";
    let file = std::fs::read(path).ok().or_else(|| { eprintln!("SKIPPED: corpus binary {path} not present (docs/CORPUS.md)"); None })?;
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.stop_on_unimplemented = true;
    m.set_machine(profile).unwrap();
    m.launch(&file).unwrap();
    Some(m)
}

/// Step a fresh MacWrite 4.5 until its TRAP #4 handler (the vector the application installed) is executed. The instruction ring
/// is process-wide, so this test serialises on `RING` against the other guests of this file.
fn reaches_its_trap_4_handler(profile: &str) -> Option<bool> {
    let _g = RING.lock().unwrap_or_else(|e| e.into_inner());
    let mut m = launch_macwrite_45(profile)?;
    for _ in 0..400 {
        let (step, _) = m.step(500);
        let h = m.mem.read_u32(36 * 4).unwrap_or(0);
        if h != 0 && heirloom_mac68k_engine::cpu::recent_pcs().contains(&h) { return Some(true); }
        if step != heirloom_mac68k_engine::Step::BudgetSpent { break; }
    }
    Some(false)
}
static RING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn finished_macwrite_45(profile: &str) -> Option<Mac<RecordingGuestUi>> {
    let mut m = launch_macwrite_45(profile)?;
    m.run_loop(5_000_000).unwrap();
    Some(m)
}

#[test]
fn macwrite_4_5_passes_its_trap_4_path_on_a_mac_plus_and_stumbles_on_a_68020() {
    let Some(entered) = reaches_its_trap_4_handler("mac-plus") else { return };
    assert!(entered, "the application's TRAP #4 handler ran: the test exercises the frame, not a program that never traps");
    let Some(plus) = finished_macwrite_45("mac-plus") else { return };
    assert!(plus.unimplemented_at.is_empty(), "no stray stop after the handler: {:?}", plus.refused.last());
    assert!(plus.ui.calls.iter().any(|c| format!("{c:?}").contains("OpenWindow") && format!("{c:?}").contains("Untitled")), "it opened its document window");
    assert!(plus.log.iter().any(|l| l.contains("GetNewWindow 301 \"Untitled\"")), "{:?}", plus.log.last());
    // NEGATIVE CONTROL: the same binary on the default 68020 returns through a frame two bytes longer and runs off into low memory.
    assert_eq!(reaches_its_trap_4_handler("mac-ii"), Some(true), "it runs the same handler");
    let Some(ii) = finished_macwrite_45("mac-ii") else { return };
    // where the garbage after the bad return leads depends on the memory layout (the frame buffer's size moved it): a stray A-line word in
    // low memory, or a spin in zeroed RAM until the budget ends. Either way the program is lost, which is what is pinned.
    if let Some(&(op, pc)) = ii.unimplemented_at.last() { assert!(op == 0xAA80 && pc < 0x1000, "stray A-line word {op:#06x} at pc {pc:#x}"); }
    assert!(!ii.ui.calls.iter().any(|c| format!("{c:?}").contains("OpenWindow")), "no window on the 68020");
}


#[test]
fn the_display_is_the_machines_a_mac_ii_has_640_by_480_eight_bit_and_the_plus_and_se_512_by_342_one_bit() {
    // Glider PRO copies 640 x 460 pixels into the frame buffer with its own code (22,112 stores past the end of a 512 x 342 buffer);
    // that is the Mac II's 13" RGB display. The 1-bit machines keep the compact Mac's screen.
    let read = |m: &Mac<RecordingGuestUi>| ([0, 2, 4, 6].map(|o| m.mem.read_u16(0x0834 + o).unwrap()), m.mem.read_u16(0x0106).unwrap());
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(read(&m), ([0, 0, 480, 640], 80), "the default machine is a Mac II: CrsrPin and ScreenRow");
    let base = m.mem.read_u32(0x824).unwrap(); // ScrnBase
    m.mem.write_u8(base + 640 * 480 - 1, 1).expect("the frame buffer holds the whole 640 x 480 screen: its last pixel is writable");
    for (name, bounds, row) in [("mac-ii", [0, 0, 480, 640], 80), ("mac-plus", [0, 0, 342, 512], 64), ("mac-se", [0, 0, 342, 512], 64)] {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        m.set_machine(name).unwrap();
        assert_eq!(read(&m), (bounds, row), "{name}");
    }
}
