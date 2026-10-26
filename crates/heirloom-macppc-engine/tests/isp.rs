//! InputSprocket over the host's keyboard and mouse: the two devices, their elements, element lists and mouse events polled from the low-memory Mouse/MBState.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3300;
const BUF: u32 = 0x3400;
const EV: u32 = 0x3500;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind_from("InputSprocketLib", name), Binding::Unbound(_)), "{name} must be bound");
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    let key = heirloom_macppc_engine::glue::binding_key("InputSprocketLib", name);
    assert_eq!(m.glue_call(&key, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn u32_at(m: &M, at: u32) -> u32 { m.mac.mem.read_u32(at).unwrap() }

#[test]
fn startup_version_devices_elements_info_and_configuration() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "ISpStartup", &[]), 0);
    assert_eq!(call(&mut m, "ISpGetVersion", &[OUT]) as u32, OUT);
    assert_eq!(u32_at(&m, OUT), 0x0170_8000, "NumVersion 1.7 final, through the hidden pointer");
    // devices by class: the keyboard, the mouse, and 0 = both; the count is the number of matching devices
    for (class, n) in [(u32::from_be_bytes(*b"keyd"), 1), (u32::from_be_bytes(*b"mous"), 1), (0u32, 2), (u32::from_be_bytes(*b"joys"), 0)] {
        assert_eq!(call(&mut m, "ISpDevices_ExtractByClass", &[class, 4, OUT, BUF]), 0);
        assert_eq!(u32_at(&m, OUT), n, "class {class:#x}");
    }
    assert_eq!(call(&mut m, "ISpDevices_ActivateClass", &[u32::from_be_bytes(*b"mous")]), 0);
    // the mouse's element list: X, Y, button; info: label, kind, name
    call(&mut m, "ISpDevices_ExtractByClass", &[u32::from_be_bytes(*b"mous"), 1, OUT, BUF]);
    let mouse = u32_at(&m, BUF);
    assert_eq!(call(&mut m, "ISpDevice_GetElementList", &[mouse, OUT]), 0);
    let list = u32_at(&m, OUT);
    assert_eq!(call(&mut m, "ISpElementList_Extract", &[list, 8, OUT, BUF]), 0);
    assert_eq!(u32_at(&m, OUT), 3);
    let (x, button) = (u32_at(&m, BUF), u32_at(&m, BUF + 8));
    assert_eq!(call(&mut m, "ISpElement_GetInfo", &[x, EV]), 0);
    assert_eq!((m.mac.mem.bytes(m.mac.mem.span(EV, 4).unwrap()).unwrap(), m.mac.mem.bytes(m.mac.mem.span(EV + 4, 4).unwrap()).unwrap()), (&b"curx"[..], &b"delt"[..]));
    assert_eq!(m.get_bytes(EV + 9, m.mac.mem.read_u8(EV + 8).unwrap() as u32, "t").unwrap(), b"Mouse X Delta");
    assert_eq!(call(&mut m, "ISpElement_GetConfigurationInfo", &[button, 4, EV]), 0);
    assert_eq!(u32_at(&m, EV), 1);
    // NEGATIVE controls: an unknown element / device / list is paramErr; a too-small configuration buffer too
    assert_eq!(call(&mut m, "ISpElement_GetInfo", &[0x1234, EV]), -50);
    assert_eq!(call(&mut m, "ISpDevice_GetElementList", &[0x1234, OUT]), -50);
    assert_eq!(call(&mut m, "ISpElementList_Dispose", &[0x1234]), -50);
    assert_eq!(call(&mut m, "ISpElement_GetConfigurationInfo", &[x, 4, EV]), -50);
    assert_eq!(call(&mut m, "ISpElementList_Dispose", &[list]), 0);
    assert_eq!(call(&mut m, "ISpShutdown", &[]), 0);
    assert!(matches!(m.bind_from("InputSprocketLib", "ISpElement_NewVirtualFromNeeds"), Binding::Unbound(_)), "virtual elements stop by name");
}

#[test]
fn mouse_events_come_from_the_polled_low_memory_state() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "ISpStartup", &[]);
    call(&mut m, "ISpDevices_ExtractByClass", &[u32::from_be_bytes(*b"mous"), 1, OUT, BUF]);
    let mouse = u32_at(&m, BUF);
    call(&mut m, "ISpDevice_GetElementList", &[mouse, OUT]);
    let list = u32_at(&m, OUT);
    let next = |m: &mut M| -> Option<(u32, u32)> {
        m.mac.mem.write_u8(BUF + 0x40, 9).unwrap();
        assert_eq!(call(m, "ISpElementList_GetNextEvent", &[list, 20, EV, BUF + 0x40]), 0);
        (m.mac.mem.read_u8(BUF + 0x40).unwrap() == 1).then(|| (u32_at(m, EV + 8), u32_at(m, EV + 16)))
    };
    let (x, y, button) = (0x4000_2030u32, 0x4000_2040u32, 0x4000_2050u32);
    assert_eq!(next(&mut m), None, "the first poll only records the state");
    // the mouse moves right 5, down 3 and the button goes down
    m.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::MOUSE, 13).unwrap();
    m.mac.mem.write_u16(heirloom_mac68k_engine::lowmem::MOUSE + 2, 5).unwrap();
    m.mac.mem.write_u8(heirloom_mac68k_engine::lowmem::MB_STATE, 0).unwrap();
    assert_eq!(next(&mut m), Some((x, 5)));
    assert_eq!(next(&mut m), Some((y, 13)));
    assert_eq!(next(&mut m), Some((button, 1)));
    assert_eq!(next(&mut m), None);
    m.mac.mem.write_u8(heirloom_mac68k_engine::lowmem::MB_STATE, 0x80).unwrap();
    assert_eq!(next(&mut m), Some((button, 0)), "released");
    // NEGATIVE controls: a buffer smaller than an event is paramErr; the simple state follows the button
    assert_eq!(call(&mut m, "ISpElementList_GetNextEvent", &[list, 8, EV, BUF + 0x40]), -50);
    assert_eq!(call(&mut m, "ISpElement_GetSimpleState", &[button, OUT]), 0);
    assert_eq!(u32_at(&m, OUT), 0);
    m.mac.mem.write_u8(heirloom_mac68k_engine::lowmem::MB_STATE, 0).unwrap();
    call(&mut m, "ISpElement_GetSimpleState", &[button, OUT]);
    assert_eq!(u32_at(&m, OUT), 1);
}
