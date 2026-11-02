//! KeyTranslate through a program-supplied 'KCHR': tables, modifier index, key-up, dead keys with a completer and a no-match character (Executor's C_KeyTranslate algorithm);
//! and through the installed (British) layout that smKCHRCache names.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const K: u32 = 0x4000;
const STATE: u32 = 0x3F00;

fn kchr(m: &mut M) {
    let mem = &mut m.mac.mem;
    mem.write_bytes(K, &[0u8; 2 + 256 + 2 + 2 * 128 + 64]).unwrap();
    mem.write_u16(K, 1).unwrap(); // version
    mem.write_u8(K + 2 + 2, 1).unwrap(); // modifier index 2 (shift) -> table 1
    mem.write_u16(K + 258, 2).unwrap(); // two tables
    mem.write_u8(K + 260, b'a').unwrap(); // table 0, virtual key 0
    mem.write_u8(K + 260 + 2, b'b').unwrap(); // vk 2
    mem.write_u8(K + 260 + 128, b'A').unwrap(); // table 1, vk 0
    // dead keys: one record for (table 0, vk 1): completer a -> 0x87, no-match '`'
    let d = K + 260 + 256;
    mem.write_u16(d, 1).unwrap();
    mem.write_bytes(d + 2, &[0, 1, 0, 1, b'a', 0x87, 0, b'`']).unwrap();
}
fn kt(m: &mut M, code: u32, state: u32) -> Result<u32, String> { kt_with(m, K, code, state) }
/// KeyTranslate(kchr, code, state) through the glue.
fn kt_with(m: &mut M, kchr: u32, code: u32, state: u32) -> Result<u32, String> {
    m.bind("KeyTranslate");
    let mut regs = [0u32; 32];
    regs[3] = kchr; regs[4] = code; regs[5] = state;
    match m.glue_call("KeyTranslate", &mut regs) { GlueOutcome::Return => Ok(regs[3]), GlueOutcome::Refused(w) => Err(w), o => panic!("{o:?}") }
}
#[test]
fn layout_lookup_modifiers_and_dead_keys_through_a_programs_own_kchr() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    kchr(&mut m);
    m.mac.mem.write_u32(STATE, 0).unwrap();
    assert_eq!(kt(&mut m, 0, STATE), Ok(u32::from(b'a')));
    assert_eq!(kt(&mut m, 0x0200, STATE), Ok(u32::from(b'A')), "shift selects table 1");
    // the dead key: no character, state becomes non-zero; the next key 'a' completes to 0x87 and the state resets
    assert_eq!(kt(&mut m, 1, STATE), Ok(0));
    assert_ne!(m.mac.mem.read_u32(STATE).unwrap(), 0);
    assert_eq!(kt(&mut m, 0, STATE), Ok(0x87));
    assert_eq!(m.mac.mem.read_u32(STATE).unwrap(), 0);
    // a key with no completer after a dead key: its own character with the no-match character in bits 16..23
    kt(&mut m, 1, STATE).unwrap();
    assert_eq!(kt(&mut m, 2, STATE), Ok(u32::from(b'b') | u32::from(b'`') << 16));
    // key-up of the dead key yields its no-match character; NEGATIVE control: the key-DOWN did not
    assert_eq!(kt(&mut m, 0x81, STATE), Ok(u32::from(b'`')));
}

/// The installed layout (British, the CoralOS default region): smKCHRCache names its data, and KeyTranslate through it - or through NULL, "the current
/// layout" - answers. Storm (Warcraft III) asks GetScriptManagerVariable(smKCHRCache) and KeyTranslates every key through it.
#[test]
fn the_installed_british_layout_is_smkchrcache_and_keytranslate_reads_it() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.bind("GetScriptManagerVariable");
    let mut regs = [0u32; 32];
    regs[3] = 38; // smKCHRCache
    assert!(matches!(m.glue_call("GetScriptManagerVariable", &mut regs), GlueOutcome::Return));
    let cache = regs[3];
    assert_ne!(cache, 0, "a layout is installed");
    assert_eq!(kt_with(&mut m, cache, 0x23, 0), Ok(u32::from(b'p')), "the P key");
    assert_eq!(kt_with(&mut m, cache, 0x0200 | 0x23, 0), Ok(u32::from(b'P')), "Shift-P");
    assert_eq!(kt_with(&mut m, cache, 0x0200 | 0x14, 0), Ok(0xA3), "British: Shift-3 is the pound sign (Mac Roman 0xA3)");
    assert_eq!(kt_with(&mut m, cache, 0x24, 0), Ok(0x0D), "Return");
    assert_eq!(kt_with(&mut m, 0, 0x33, 0), Ok(0x08), "NULL is the installed layout: Delete");
    // the same layout every time (one copy for the life of the machine)
    m.bind("GetScriptManagerVariable");
    let mut regs = [0u32; 32];
    regs[3] = 38;
    m.glue_call("GetScriptManagerVariable", &mut regs);
    assert_eq!(regs[3], cache);
    // NEGATIVE CONTROL: a program's own layout still wins over the installed one
    kchr(&mut m);
    assert_eq!(kt_with(&mut m, K, 0x0200, 0), Ok(u32::from(b'A')));
    assert_eq!(kt_with(&mut m, K, 0x23, 0), Ok(0), "the test layout has no P key");
}
