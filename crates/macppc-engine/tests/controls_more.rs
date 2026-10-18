//! A pop-up menu control through the PowerPC glue: the SAME Control Manager table the 68k test (`mac68k-engine/tests/controls_more.rs`)
//! drives, called the way a PPC CPU calls the Toolbox (r3.. = the Pascal arguments). The guest-visible result must be the 68k run's:
//! the personality is the Toolbox, not the processor.
use guestui_abi::{ChoiceStyle, ControlKind, ControlValue, Intent, RecordingGuestUi};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use mac68k_engine::menu::{encode_block, Item, Parsed};
use mac68k_engine::pascal::Rect;
use mac68k_engine::toolbox::WindowSpec;
use mac68k_engine::translate::Obj;

type M = MacPpc<RecordingGuestUi>;
const RECT: u32 = 0x3100;
const TITLE: u32 = 0x3140;
const MENU_ID: i16 = 200;

fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let menu = Parsed { id: MENU_ID, flags: 0xFFFF_FFFF, title: b"Size".to_vec(), items: ["Small", "Medium", "Large"].iter().map(|t| Item { text: t.as_bytes().to_vec(), icon: 0, key: 0, mark: 0, style: 0 }).collect() };
    let fork = retro_resfork::writer::ForkBuilder::new().add(b"MENU", MENU_ID, None, &encode_block(&menu, 0, 0, 0)).build();
    m.mac.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(fork).unwrap());
    mac68k_engine::quickdraw::fill_globals(&mut m.mac, 0x3F00).unwrap();
    m.mac.qd_globals = 0x3F00;
    let spec = WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "W".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 };
    let win = m.mac.make_window(spec).unwrap();
    for (i, v) in [10i16, 20, 34, 140].into_iter().enumerate() { m.mac.mem.write_u16(RECT + 2 * i as u32, v as u16).unwrap(); }
    m.mac.mem.write_bytes(TITLE, &[5, b'S', b'i', b'z', b'e', b':']).unwrap();
    m.mac.ev.queue.clear();
    (m, win)
}

/// One Toolbox call by name with PowerPC register arguments; returns r3.
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}

#[test]
fn a_popup_choice_through_the_powerpc_glue_is_in_button_with_the_value_set_like_on_the_68k() {
    let (mut m, win) = machine();
    let ctl = call(&mut m, "NewControl", &[win, RECT, TITLE, 1, 1, MENU_ID as u16 as u32, 40, 1008, 0]);
    assert_ne!(ctl, 0, "{:?}", m.mac.refused);
    let shown = m.mac.tr.projected(Obj::Control(ctl)).expect("projected").clone();
    assert_eq!(shown.model.kind, ControlKind::Choice { style: ChoiceStyle::Popup, labels: vec!["Small".into(), "Medium".into(), "Large".into()], selected: 0 });
    assert_eq!((call(&mut m, "GetControlMinimum", &[ctl]), call(&mut m, "GetControlMaximum", &[ctl])), (1, 3), "the CDEF published min 1 and max = the item count");
    m.mac.ui.push_intent(m.mac.windows[&win].native.unwrap(), shown.native, Intent::SetValue(ControlValue::Choice(2))).unwrap();
    m.mac.pump_ui();
    // the mouseDown is queued; the program answers it with TrackControl at the point (local: the middle of the pop-up)
    let part = call(&mut m, "TrackControl", &[ctl, (22 << 16) | 80, 0]);
    assert_eq!(part, 10, "inButton, as on the 68k");
    assert_eq!(call(&mut m, "GetControlValue", &[ctl]), 3, "contrlValue is the chosen item before TrackControl returns");
    // NEGATIVE control: an item the menu does not have is refused by name and nothing is queued for the program
    m.mac.ev.queue.clear();
    m.mac.ui.push_intent(m.mac.windows[&win].native.unwrap(), m.mac.tr.projected(Obj::Control(ctl)).unwrap().native, Intent::SetValue(ControlValue::Choice(3))).unwrap();
    m.mac.pump_ui();
    assert!(m.mac.refused.iter().any(|r| r.contains("item 3 outside the pop-up's 3 items")), "{:?}", m.mac.refused);
    assert!(m.mac.ev.queue.is_empty());
}
