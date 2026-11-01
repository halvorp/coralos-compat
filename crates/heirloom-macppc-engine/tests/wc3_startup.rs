//! Warcraft III's start-up calls that are not GL (the 2026-11-01 game call map, /build/patches/h84/wc3-game-calls/GAPS.md section 1),
//! each served instead of stopping the game: the Power Manager's UpdateSystemActivity and the Desktop Manager's PBDTGetPath.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(name, &mut regs);
    assert!(matches!(o, GlueOutcome::Return), "{name}: {o:?}");
    regs[3]
}

#[test]
fn update_system_activity_is_accepted_for_the_defined_activities_only() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "UpdateSystemActivity", &[1]), 0, "UsrActivity: no guest idle-sleep timer to reset");
    assert_eq!(call(&mut m, "UpdateSystemActivity", &[9]) as i32, -50, "NEGATIVE CONTROL: an undefined activity type");
}

#[test]
fn pbdtgetpath_reports_no_desktop_database_by_error_never_a_fake_one() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    const PB: u32 = 0x0003_8000;
    m.mac.mem.zero(PB, 128).unwrap();
    m.mac.mem.write_u16(PB + 22, (-1i16) as u16).unwrap(); // ioVRefNum: the default volume
    assert_eq!(call(&mut m, "PBDTGetPath", &[PB]) as i32, -50, "the volume exists and has no Desktop Database (vMAttrib bHasDesktopMgr clear)");
    m.mac.mem.write_u16(PB + 22, (-77i16) as u16).unwrap();
    assert_eq!(call(&mut m, "PBDTGetPath", &[PB]) as i32, -35, "NEGATIVE CONTROL: no such volume is nsvErr");
}

#[test]
fn register_component_resource_file_registers_none_and_says_so() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "RegisterComponentResourceFile", &[3, 1]), 0, "no component is run here: the honest count is 0");
    assert!(m.mac.log.iter().any(|l| l.contains("RegisterComponentResourceFile")));
}

#[test]
fn open_a_default_component_finds_none_and_answers_nil() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    const OUT: u32 = 0x0003_9000;
    m.mac.mem.write_u32(OUT, 0xDEAD_BEEF).unwrap();
    assert_eq!(call(&mut m, "OpenADefaultComponent", &[u32::from_be_bytes(*b"mhlr"), 0, OUT]) as i32, -3000, "invalidComponentID");
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0, "NIL instance");
}
