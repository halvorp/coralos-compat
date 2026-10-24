//! Menu Manager Carbon accessors by C name over the engine's menu path: titles, ids, enabled state, item properties, command-key lookup and the menu bar's visibility.
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use mac68k_engine::menu::{encode_block, Item, Parsed};

type M = MacPpc<RecordingGuestUi>;
const BUF: u32 = 0x3200;
const OUT: u32 = 0x3300;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    for (i, a) in args.iter().enumerate() { if i < 8 { regs[3 + i] = *a; } else { m.mac.mem.write_u32(0x6000 + 24 + 4 * i as u32, *a).unwrap(); } }
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
fn item(t: &str, key: u8) -> Item { Item { text: t.as_bytes().to_vec(), icon: 0, key, mark: 0, style: 0 } }

fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let file = Parsed { id: 200, flags: 0xFFFF_FFFF, title: b"File".to_vec(), items: vec![item("New", b'n'), item("Open", b'o'), item("Quit", b'q')] };
    let fork = retro_resfork::writer::ForkBuilder::new().add(b"MENU", 200, None, &encode_block(&file, 0, 0, 0)).build();
    m.mac.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(fork).unwrap());
    let h = call(&mut m, "GetMenu", &[200]) as u32;
    assert_ne!(h, 0);
    call(&mut m, "InsertMenu", &[h, 0]);
    (m, h)
}

#[test]
fn titles_ids_enabled_state_and_command_keys() {
    let (mut m, h) = machine();
    assert_eq!(call(&mut m, "GetMenuID", &[h]), 200);
    assert_eq!(call(&mut m, "CountMenuItems", &[h]), 3, "the Carbon name of CountMItems");
    assert_eq!(call(&mut m, "GetMenuTitle", &[h, OUT]), 0);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 5).unwrap()).unwrap(), b"\x04File");
    pstr(&mut m, BUF, "Datei");
    assert_eq!(call(&mut m, "SetMenuTitle", &[h, BUF]), 0);
    call(&mut m, "GetMenuTitle", &[h, OUT]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 6).unwrap()).unwrap(), b"\x05Datei");
    // Enable/Disable under their Carbon names; item 0 is the whole menu
    assert_eq!(call(&mut m, "IsMenuItemEnabled", &[h, 2]), 1);
    call(&mut m, "DisableMenuItem", &[h, 2]);
    assert_eq!(call(&mut m, "IsMenuItemEnabled", &[h, 2]), 0);
    call(&mut m, "EnableMenuItem", &[h, 2]);
    assert_eq!(call(&mut m, "IsMenuItemEnabled", &[h, 2]), 1);
    assert_eq!(call(&mut m, "IsMenuItemEnabled", &[h, 9]), 0, "NEGATIVE control: no such item");
    // MenuKey: case-insensitive, an enabled item; 0 when nothing matches or the item is disabled
    assert_eq!(call(&mut m, "MenuKey", &[u32::from(b'o')]) as u32, (200 << 16) | 2);
    assert_eq!(call(&mut m, "MenuKey", &[u32::from(b'Q')]) as u32, (200 << 16) | 3);
    call(&mut m, "DisableMenuItem", &[h, 2]);
    assert_eq!(call(&mut m, "MenuKey", &[u32::from(b'o')]), 0, "a disabled item is not a hit");
    assert_eq!(call(&mut m, "MenuKey", &[u32::from(b'z')]), 0);
    // the bar is always shown; HideMenuBar and PopUpMenuSelect stop by name
    assert_eq!((call(&mut m, "IsMenuBarVisible", &[]), call(&mut m, "ShowMenuBar", &[])), (1, 0));
    assert!(matches!(m.bind("HideMenuBar"), Binding::Unbound(_)) && matches!(m.bind("PopUpMenuSelect"), Binding::Unbound(_)));
    // NEGATIVE control: a pointer that is no menu is refused by name
    m.bind("GetMenuID");
    let mut regs = [0u32; 32];
    regs[3] = 0x1234;
    assert!(matches!(m.glue_call("GetMenuID", &mut regs), GlueOutcome::Refused(w) if w.contains("not a handle")));
}

#[test]
fn item_properties_round_trip_and_bad_items_are_param_err() {
    let (mut m, h) = machine();
    let (creator, tag) = (u32::from_be_bytes(*b"Blzz"), u32::from_be_bytes(*b"cmd "));
    assert_eq!(call(&mut m, "GetMenuItemProperty", &[h, 1, creator, tag, 4, OUT, BUF]), -5604);
    m.mac.mem.write_u32(BUF, 0x1234_5678).unwrap();
    assert_eq!(call(&mut m, "SetMenuItemProperty", &[h, 1, creator, tag, 4, BUF]), 0);
    m.mac.mem.write_u32(BUF, 0).unwrap();
    assert_eq!(call(&mut m, "GetMenuItemProperty", &[h, 1, creator, tag, 4, OUT, BUF]), 0);
    assert_eq!((m.mac.mem.read_u32(OUT).unwrap(), m.mac.mem.read_u32(BUF).unwrap()), (4, 0x1234_5678));
    assert_eq!(call(&mut m, "GetMenuItemProperty", &[h, 2, creator, tag, 4, OUT, BUF]), -5604, "item 2 has none");
    assert_eq!(call(&mut m, "GetMenuItemProperty", &[h, 1, creator, tag, 2, OUT, BUF]), -5603, "buffer too small");
    assert_eq!(call(&mut m, "SetMenuItemProperty", &[h, 9, creator, tag, 4, BUF]), -50, "no item 9");
}
