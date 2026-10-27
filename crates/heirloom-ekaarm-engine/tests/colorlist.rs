//! The environment's colour list, read from the ROM's `eikpriv` resource file (synthetic in `support`), and the descriptor formatter.
mod support;
use heirloom_cpu_arm::Memory;
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{loader, Process};
use heirloom_ui_abi::RecordingGuestUi;

const STATIC: &str = "_ZN7CCoeEnv6StaticEv";
const COLOR: &str = "_ZNK10CColorList5ColorE13TLogicalColor";
const COLOR_ARRAY: &str = "_ZNK10CColorList10ColorArrayE4TUid";
const CONTAINS: &str = "_ZNK10CColorList18ContainsColorArrayE4TUid";
const ARRAY_COUNT: &str = "_ZNK11CColorArray5CountEv";
const FORMAT: &str = "_ZN6TDes166FormatE11TRefByValueIK7TDesC16Ez";
const ALLOC: &str = "_ZN4User6AllocZEi";

fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub"));
    for (i, a) in args.iter().enumerate() { p.cpu.regs[i] = *a; }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) { (NativeOutcome::Return, r) => r, (o, _) => panic!("{name}: {o:?}") }
}

fn env() -> (Process, u32) {
    let mut p = loader::bare(&[STATIC, COLOR, COLOR_ARRAY, CONTAINS, ARRAY_COUNT, FORMAT, ALLOC, "_ZN4User4FreeEPv"]).unwrap();
    p.set_drive(support::drive_with_rom());
    p.set_ui(RecordingGuestUi::new());
    let e = ok(&mut p, STATIC, &[]);
    (p, e)
}

#[test]
fn the_environment_holds_the_colour_list_the_rom_resource_names() {
    let (mut p, env) = env();
    let list = p.mem.read_u32(env + 0xE4).unwrap();
    assert_ne!(list, 0, "CEikonEnv::iColorList is set");
    assert_eq!(ok(&mut p, COLOR, &[list, 3]), 0xff03_0303, "logical colour 3 is the resource's colour for 3");
    assert_eq!(ok(&mut p, COLOR, &[list, 58]), 0xff3a_3a3a);
    match call(&mut p, COLOR, &[list, 59]).0 { NativeOutcome::Refused(m) => assert!(m.contains("NEXT-NEEDED egul") && m.contains("59"), "{m}"), o => panic!("{o:?}") }
    assert_eq!(ok(&mut p, CONTAINS, &[list, 0x1000_57c2]), 1, "the extras array is filed under KEikCustomColorsArrayValue");
    assert_eq!(ok(&mut p, CONTAINS, &[list, 1]), 0);
    let array = ok(&mut p, COLOR_ARRAY, &[list, 0x1000_57c2]);
    assert_eq!(ok(&mut p, ARRAY_COUNT, &[array]), 0);
    assert!(matches!(call(&mut p, COLOR_ARRAY, &[list, 1]).0, NativeOutcome::Refused(_)));
}

#[test]
fn a_run_without_the_rom_resource_refuses_by_name_and_makes_no_colours() {
    let mut p = loader::bare(&[STATIC, "_ZN4User6AllocZEi", "_ZN4User4FreeEPv"]).unwrap();
    p.set_drive(heirloom_ekaarm_engine::drive::Drive::new());
    p.set_ui(RecordingGuestUi::new());
    match call(&mut p, STATIC, &[]).0 { NativeOutcome::Refused(m) => assert!(m.contains("0x19bd6000"), "{m}"), o => panic!("{o:?}") }
}

/// A `TPtrC16` over `text` (word 0 = length | type 1 << 28, word 1 = the data) and a `TBuf16<max>` (type 3).
fn ptrc(p: &mut Process, text: &str) -> u32 {
    let units: Vec<u16> = text.encode_utf16().collect();
    let data = ok(p, ALLOC, &[units.len() as u32 * 2 + 2]);
    for (i, u) in units.iter().enumerate() { p.mem.write_u16(data + i as u32 * 2, *u).unwrap(); }
    let d = ok(p, ALLOC, &[8]);
    p.mem.write_u32(d, units.len() as u32 | 1 << 28).unwrap();
    p.mem.write_u32(d + 4, data).unwrap();
    d
}

#[test]
fn format_expands_the_symbian_directives_over_register_and_stack_arguments() {
    let (mut p, _) = env();
    let fmt = ptrc(&mut p, "%d\t%c: <%S>\t%LD / %LD kB\t%02x|%3d|%-3d|%%");
    let name = ptrc(&mut p, "Flash");
    let buf = ok(&mut p, ALLOC, &[8 + 200]);
    p.mem.write_u32(buf, 3 << 28).unwrap();
    p.mem.write_u32(buf + 4, 100).unwrap();
    // args: 2 (r2), 'C' (r3), then the stack: &name; %LD pair 141 (aligned to the next even slot after &name at slot 4 -> slot 6,7); pair 0 (8,9); 10; 7; 7
    let sp = ok(&mut p, ALLOC, &[64]);
    for (i, v) in [name, 0, 141, 0, 0, 0, 10, 7, 7].iter().enumerate() { p.mem.write_u32(sp + i as u32 * 4, *v).unwrap(); }
    p.cpu.regs[13] = sp;
    p.cpu.regs[2] = 2;
    p.cpu.regs[3] = 'C' as u32;
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(FORMAT)).unwrap();
    p.cpu.regs[0] = buf;
    p.cpu.regs[1] = fmt;
    assert!(matches!(natives::call(&mut p, idx), NativeOutcome::Return));
    let len = p.mem.read_u32(buf).unwrap() & 0x0fff_ffff;
    let text: Vec<u16> = (0..len).map(|i| p.mem.read_u16(buf + 8 + i * 2).unwrap()).collect();
    assert_eq!(String::from_utf16(&text).unwrap(), "2\tC: <Flash>\t141 / 0 kB\t0a|  7|7  |%");
}
