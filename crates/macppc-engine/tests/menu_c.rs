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
    assert_eq!(call(&mut m, "GetMenuTitle", &[h, OUT]) as u32, OUT, "GetMenuTitle returns its buffer");
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 5).unwrap()).unwrap(), b"\x04File");
    pstr(&mut m, BUF, "Datei");
    assert_eq!(call(&mut m, "SetMenuTitle", &[h, BUF]), 0);
    assert_eq!((call(&mut m, "SetMenuTitle", &[0, BUF]), call(&mut m, "SetMenuTitle", &[h, 0])), (-5623, -50), "spec: NULL menu is menuInvalidErr, NULL title paramErr");
    // spec: NULL menu leaves the buffer alone and returns it; NULL buffer returns NULL
    m.mac.mem.write_bytes(OUT + 0x40, b"xyz").unwrap();
    assert_eq!(call(&mut m, "GetMenuTitle", &[0, OUT + 0x40]) as u32, OUT + 0x40);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT + 0x40, 3).unwrap()).unwrap(), b"xyz");
    assert_eq!(call(&mut m, "GetMenuTitle", &[h, 0]), 0);
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
    // PopUpMenuSelect stops by name
    assert!(matches!(m.bind("PopUpMenuSelect"), Binding::Unbound(_)));
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

#[test]
fn hide_and_show_menu_bar_follow_the_spec_vectors() {
    // spec: macos9-specs/MenusLib/HideMenuBar: MBarHeight 20 visible: Hide -> 0 (saved 20), again nothing; Show -> 20, again nothing
    let (mut m, h) = machine();
    let height = |m: &M| m.mac.mem.read_u16(mac68k_engine::lowmem::MBAR_HEIGHT).unwrap();
    assert_eq!((height(&m), call(&mut m, "IsMenuBarVisible", &[])), (20, 1));
    call(&mut m, "DrawMenuBar", &[]);
    call(&mut m, "HideMenuBar", &[]);
    assert_eq!((height(&m), call(&mut m, "IsMenuBarVisible", &[])), (0, 0));
    call(&mut m, "HideMenuBar", &[]);
    assert_eq!(m.mac.menus.saved_bar_height, 20, "a second Hide does not overwrite the saved height with 0");
    assert!(m.mac.menu_strip().unwrap().menus.is_empty(), "the host's strip is empty while hidden");
    call(&mut m, "ShowMenuBar", &[]);
    assert_eq!((height(&m), call(&mut m, "IsMenuBarVisible", &[])), (20, 1));
    assert_eq!(m.mac.menu_strip().unwrap().menus.len(), 1);
    // NEGATIVE control: Show without a Hide changes nothing; a program-set non-zero MBarHeight survives Show
    call(&mut m, "ShowMenuBar", &[]);
    assert_eq!(height(&m), 20);
    call(&mut m, "HideMenuBar", &[]);
    m.mac.mem.write_u16(mac68k_engine::lowmem::MBAR_HEIGHT, 33).unwrap();
    call(&mut m, "ShowMenuBar", &[]);
    assert_eq!(height(&m), 33, "an already non-zero height is kept");
    let _ = h;
}
