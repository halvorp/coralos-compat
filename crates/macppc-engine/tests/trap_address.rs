//! "Where is trap N?" from PowerPC code: `NGetTrapAddress`, `GetOSTrapAddress`, `GetToolTrapAddress`, `GetTrapAddress` are
//! InterfaceLib C routines that answer from the SAME trap registry as the 68k `_GetTrapAddress` family, so a PowerPC program's
//! `TrapAvailable(_WaitNextEvent)` check (compare with `_Unimplemented`'s address) sees what a 68k program sees. Retro68's
//! PowerPC console runtime makes that check at startup; it was NEXT-NEEDED before these were bound.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::traptable::{trap_address, OS_STUBS, TOOL_STUBS};
use macppc_engine::symbols::{lookup, Binding, Native, TrapQuery};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;

fn machine() -> M { MacPpc::new(RecordingGuestUi::new()).unwrap() }

fn ask(m: &mut M, name: &str, args: &[u32]) -> u32 {
    m.bind(name); // what instantiating a fragment that imports `name` records
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}");
    regs[3]
}

const WAIT_NEXT_EVENT: u32 = 0x60; // _WaitNextEvent = $A860
const UNIMPLEMENTED_STUB: u32 = TOOL_STUBS + 4 * 0x9F; // _Unimplemented = $A89F
const OS_TRAP: u32 = 0;
const TOOL_TRAP: u32 = 1;

#[test]
fn the_four_imports_bind_to_the_shared_trap_registry() {
    let m = machine();
    for (name, q) in [("NGetTrapAddress", TrapQuery::ByType), ("GetOSTrapAddress", TrapQuery::Os), ("GetToolTrapAddress", TrapQuery::Tool), ("GetTrapAddress", TrapQuery::Old)] {
        assert_eq!(lookup(&m.mac, name), Binding::Native(Native::TrapAddress(q)), "{name}");
    }
}

#[test]
fn an_implemented_trap_is_not_unimplemented_and_an_absent_one_is() {
    let mut m = machine();
    let wne = ask(&mut m, "NGetTrapAddress", &[WAIT_NEXT_EVENT, TOOL_TRAP]);
    assert_eq!(wne, TOOL_STUBS + 4 * WAIT_NEXT_EVENT, "WaitNextEvent is served: its own stub");
    assert_ne!(wne, UNIMPLEMENTED_STUB);
    // a Toolbox trap nobody registered answers with the _Unimplemented stub, as a ROM without it would
    let absent = (0..0x400u32).find(|n| !m.mac.is_pascal(0xA800 | *n as u16) && !m.mac.is_os(0xA800 | *n as u16) && *n != 0x9F).unwrap();
    assert_eq!(ask(&mut m, "NGetTrapAddress", &[absent, TOOL_TRAP]), UNIMPLEMENTED_STUB, "trap {absent:#x}");
    // the stub is real guest code: the trap word, then RTS
    assert_eq!((m.mac.mem.read_u16(wne).unwrap(), m.mac.mem.read_u16(wne + 2).unwrap()), (0xA860, 0x4E75));
}

#[test]
fn the_type_argument_and_the_entry_point_pick_the_table() {
    let mut m = machine();
    let tool = ask(&mut m, "NGetTrapAddress", &[WAIT_NEXT_EVENT, TOOL_TRAP]);
    let os = ask(&mut m, "NGetTrapAddress", &[WAIT_NEXT_EVENT, OS_TRAP]);
    assert!((TOOL_STUBS..TOOL_STUBS + 0x1000).contains(&tool) && (OS_STUBS..OS_STUBS + 0x400).contains(&os), "{tool:#x} {os:#x}");
    assert_eq!(ask(&mut m, "GetToolTrapAddress", &[WAIT_NEXT_EVENT]), tool);
    assert_eq!(ask(&mut m, "GetOSTrapAddress", &[WAIT_NEXT_EVENT]), os);
    // the old entry point has no type: a number of $100 or more is a Toolbox trap
    assert_eq!(ask(&mut m, "GetTrapAddress", &[0x1B5]), ask(&mut m, "GetToolTrapAddress", &[0x1B5]));
    // NEGATIVE CONTROL: only tTyp == ToolTrap (1) is the Toolbox table; any other value is the OS table
    assert_eq!(ask(&mut m, "NGetTrapAddress", &[WAIT_NEXT_EVENT, 2]), os);
}

#[test]
fn the_powerpc_answer_is_the_68k_answer() {
    let mut m = machine();
    for (number, tool) in [(WAIT_NEXT_EVENT, true), (0x9F, true), (0x1E, false), (0x46, false)] {
        let ppc = ask(&mut m, "NGetTrapAddress", &[number, u32::from(tool)]);
        let (m68k, _) = trap_address(&mut m.mac, if tool { 0xA746 } else { 0xA346 }, number).unwrap();
        assert_eq!(ppc, m68k, "trap {number:#x} tool={tool}");
    }
}
