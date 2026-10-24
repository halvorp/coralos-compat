//! GadTools / Intuition handlers called directly, with the arguments the test pokes into guest RAM: a tiny generated
//! "call script" program (movea/move #imm into registers, jsr LVO(a6), store D0) so every error path and every
//! guest-visible struct can be checked without a hand-assembled program for each.
mod common;
use common::gtscript::*;
use amiga68k_engine::Step;
use guestui_abi::*;

const LVO_CREATE_MENUS: i32 = -48;
const LVO_FREE_MENUS: i32 = -54;
const LVO_CREATE_CONTEXT: i32 = -114;
const LVO_CREATE_GADGET: i32 = -30;
const LVO_SET_ATTRS: i32 = -42;
const LVO_OPEN_WINDOW_TAGS: i32 = -606;
const LVO_SET_MENU_STRIP: i32 = -264;

#[test]
fn created_menu_structs_are_what_gadtools_makes() {
    let mut m = M::new();
    let rows = [
        (1, Some("Project"), None, 0, 0, 0xAA),
        (2, Some("Open"), Some("O"), 0, 0, 1),
        (2, Some("Dim"), None, 0x10, 0, 2),                 // NM_ITEMDISABLED
        (2, Some("-"), None, 0, 0, 0),                      // NM_BARLABEL
        (2, Some("Verbose"), None, 0x0109, 0, 3),           // CHECKIT | MENUTOGGLE | CHECKED
        (2, Some("Cmd"), Some("Shift-X"), 0x4, 0, 4),       // NM_COMMANDSTRING
        (2, Some("More"), Some("M"), 0, 0, 5),
        (3, Some("Deeper"), None, 0, 0, 6),
        (1, Some("Off"), None, 1, 0, 0xBB),                 // NM_MENUDISABLED
        (0, None, None, 0, 0, 0),
    ];
    let nm = m.newmenu(&rows[..rows.len() - 1]);
    let err = DATA - 8;
    let tags = m.tags(&[(GT + 62, 1), (GT + 63, err)]);
    let win = m.tags(&[(WA + 3, 300), (WA + 4, 100), (WA + 7, 0x100)]);
    m.run(&[
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, win)]),
        gt(LVO_CREATE_MENUS, &[a(0, nm), a(1, tags)]),
        intu(LVO_SET_MENU_STRIP, &[a_res(0, 0), a_res(1, 1)]),
    ]);
    let menu = m.result(1);
    assert_ne!(menu, 0);
    assert_eq!(m.r32(err), 0, "GTMN_SecondaryError: success");
    assert_eq!(m.a.menu_allocs.len(), 1);
    // Menu: Flags MENUENABLED, name, userdata after the struct (GTMENU_USERDATA).
    assert_eq!(m.r16(menu + 12), 1);
    assert_eq!(m.a.mem.read_cstr(m.r32(menu + 14)).unwrap(), "Project");
    assert_eq!(m.r32(menu + 30), 0xAA);
    let item = |n: u32| -> u32 { let mut p = m.r32(menu + 18); for _ in 0..n { p = m.r32(p); } p };
    let flags = |n| m.r16(item(n) + 12);
    // ITEMTEXT 2 | COMMSEQ 4 | ITEMENABLED 0x10 | HIGHCOMP 0x40
    assert_eq!(flags(0), 0x56);
    assert_eq!(m.a.mem.read_u8(item(0) + 26).unwrap(), b'O');
    assert_eq!(m.r32(item(0) + 34), 1, "GTMENUITEM_USERDATA");
    assert_eq!(m.a.mem.read_cstr(m.r32(m.r32(item(0) + 18) + 12)).unwrap(), "Open", "ItemFill -> IntuiText -> IText");
    assert_eq!(m.r16(item(0) + 32), 0, "NextSelect starts clear");
    assert_eq!(flags(1), 0x02 | 0x40, "NM_ITEMDISABLED: ITEMENABLED clear");
    assert_eq!(flags(2), 0x40, "bar label: no ITEMTEXT, not enabled");
    assert_ne!(m.r32(item(2) + 18), 0, "bar label has an Image fill");
    assert_eq!(flags(3), 0x02 | 0x40 | 0x10 | 0x01 | 0x08 | 0x100, "CHECKIT | MENUTOGGLE | CHECKED kept");
    // NM_COMMANDSTRING: no COMMSEQ, the key text is the second IntuiText.
    assert_eq!(flags(4) & 0x4, 0);
    let next_text = m.r32(m.r32(item(4) + 18) + 16);
    assert_eq!(m.a.mem.read_cstr(m.r32(next_text + 12)).unwrap(), "Shift-X");
    // A parent of sub items loses COMMSEQ and shows the sub-menu mark.
    assert_eq!(flags(5) & 0x4, 0);
    let mark = m.r32(m.r32(m.r32(item(5) + 18) + 16) + 12);
    assert_eq!(m.a.mem.read_u8(mark).unwrap(), 0xBB);
    let sub = m.r32(item(5) + 28);
    assert_eq!(m.r32(sub + 34), 6);
    // Second menu is NM_MENUDISABLED.
    let off = m.r32(menu);
    assert_eq!((m.r16(off + 12) & 1, m.r32(off + 30)), (0, 0xBB));
    // It decodes through SetMenuStrip's path: the native side got the same model a hand-built strip would give.
    let model = m.a.ui.calls.iter().find_map(|c| if let UiCall::SetMenuStrip(_, mm) = c { Some(mm.clone()) } else { None }).expect("SetMenuStrip");
    assert_eq!(model.menus.len(), 2);
    assert!(model.menus[0].enabled && !model.menus[1].enabled);
    let items = &model.menus[0].items;
    assert_eq!(items.iter().map(|i| (i.label.as_str(), i.enabled, i.shortcut, i.check)).collect::<Vec<_>>(), [
        ("Open", true, Some('O'), None), ("Dim", false, None, None), ("-", false, None, None),
        ("Verbose", true, None, Some(true)), ("Cmd", true, None, None), ("More", true, None, None)]);
    assert_eq!(items[5].sub_items[0].label, "Deeper");
}

const LVO_CLEAR_MENU_STRIP: i32 = -54;
const LVO_LAYOUT_MENU_ITEMS: i32 = -60;
const LVO_GET_ATTRS: i32 = -174;
const LVO_MODIFY_IDCMP: i32 = -150;
const LVO_SET_WINDOW_TITLES: i32 = -276;
const LVO_CLOSE_WINDOW: i32 = -72;
const LVO_EASY_REQUEST: i32 = -588;
const HEAP_START: u32 = 0x20_0000;

fn title(l: &str) -> (u8, Option<&str>, Option<&str>, u16, u32, u32) { (1, Some(l), None, 0, 0, 0) }
fn item(l: &str) -> (u8, Option<&str>, Option<&str>, u16, u32, u32) { (2, Some(l), None, 0, 0, 0) }

fn items_in(m: &M, menu: u32) -> usize {
    let (mut n, mut p) = (0, m.r32(menu + 18));
    while p != 0 { n += 1; p = m.r32(p); }
    n
}

#[test]
fn bad_tables_fail_by_name_with_the_secondary_error_and_leak_nothing() {
    let mut m = M::new();
    let err = DATA - 8;
    let full = m.tags(&[(GT + 62, 1), (GT + 63, err)]);
    let plain = m.tags(&[(GT + 63, err)]);
    let sub_after_title = m.newmenu(&[title("A"), (3, Some("s"), None, 0, 0, 0)]);
    let item_first = m.newmenu(&[item("x")]);
    let unknown = m.newmenu(&[title("A"), (9, Some("?"), None, 0, 0, 0)]);
    let null_label = m.newmenu(&[title("A"), (2, None, None, 0, 0, 0)]);
    let mut rows = vec![title("Big")];
    rows.extend((0..70).map(|_| item("i")));
    let trimmed = m.newmenu(&rows);
    m.run(&[
        gt(LVO_CREATE_MENUS, &[a(0, sub_after_title), a(1, plain)]),   // 0: GTMENU_INVALID
        gt(LVO_CREATE_MENUS, &[a(0, item_first), a(1, full)]),         // 1: items without a title in a FULL menu
        gt(LVO_CREATE_MENUS, &[a(0, unknown), a(1, plain)]),           // 2
        gt(LVO_CREATE_MENUS, &[a(0, null_label), a(1, plain)]),        // 3
        gt(LVO_CREATE_MENUS, &[a(0, trimmed), a(1, plain)]),           // 4: GTMENU_TRIMMED, still returned
        gt(LVO_CREATE_MENUS, &[a(0, item_first), a(1, plain)]),        // 5: not full: the first item is the result
    ]);
    assert_eq!([m.result(0), m.result(1), m.result(2), m.result(3)], [0, 0, 0, 0]);
    let trimmed_menu = m.result(4);
    assert_ne!(trimmed_menu, 0);
    assert_eq!(items_in(&m, trimmed_menu), 64, "Intuition's 64-item limit: the rest is trimmed");
    assert_ne!(m.result(5), 0);
    assert!(m.a.refused.is_empty(), "GadTools errors are results, not engine refusals: {:?}", m.a.refused);
    // Only the two successes still own memory: 1 menu + 64 items, and one loose item.
    assert_eq!(m.a.menu_allocs.len(), 2);
    assert_eq!(m.a.heap.live_blocks(), 1 + 64 + 1, "the failed tables gave every block back");
}

#[test]
fn secondary_error_codes() {
    for (rows, full, want) in [
        (vec![title("A"), (3, Some("s"), None, 0, 0, 0)], false, 2u32), // sub right after a title
        (vec![item("x")], true, 2),
        (vec![title("A"), (9, Some("?"), None, 0, 0, 0)], false, 2),
        (vec![title("Big")].into_iter().chain((0..65).map(|_| item("i"))).collect(), false, 1),
        (vec![title("A"), item("ok")], true, 0),
    ] {
        let mut m = M::new();
        let err = DATA - 8;
        m.a.mem.write_u32(err, 99).unwrap();
        let nm = m.newmenu(&rows);
        let tags = m.tags(&[(GT + 62, full as u32), (GT + 63, err)]);
        m.run(&[gt(LVO_CREATE_MENUS, &[a(0, nm), a(1, tags)])]);
        assert_eq!(m.r32(err), want, "{rows:?} full={full}");
    }
}

#[test]
fn free_menus_refuses_what_it_did_not_make_and_what_a_window_still_shows() {
    let mut m = M::new();
    let nm = m.newmenu(&[title("A"), item("x")]);
    let win = m.tags(&[(WA + 3, 100), (WA + 4, 60), (WA + 7, 0x100)]);
    m.run(&[
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, win)]),                   // 0
        gt(LVO_CREATE_MENUS, &[a(0, nm), a(1, 0)]),                          // 1
        intu(LVO_SET_MENU_STRIP, &[a_res(0, 0), a_res(1, 1)]),               // 2
        gt(LVO_FREE_MENUS, &[a_res(0, 1)]),                                  // 3: refused, still shown
        gt(LVO_FREE_MENUS, &[a(0, 0x1234)]),                                 // 4: refused, not ours
        gt(LVO_FREE_MENUS, &[a(0, 0)]),                                      // 5: NULL is fine
        intu(LVO_CLEAR_MENU_STRIP, &[a_res(0, 0)]),                          // 6
        gt(LVO_FREE_MENUS, &[a_res(0, 1)]),                                  // 7: now fine
        gt(LVO_FREE_MENUS, &[a_res(0, 1)]),                                  // 8: double free is refused
    ]);
    assert_eq!(m.a.refused, [
        format!("gadtools.library/FreeMenus: FreeMenus: {:#x} is still the menu strip of an open window", m.result(1)),
        "gadtools.library/FreeMenus: FreeMenus: 0x1234 was not made by CreateMenusA".to_string(),
        format!("gadtools.library/FreeMenus: FreeMenus: {:#x} was not made by CreateMenusA", m.result(1)),
    ]);
    assert_eq!(m.a.heap.live_blocks(), 2 + 4 + 1, "only the window, its UserPort, its WindowPort and its drawing surface (canvas BitMap, Layer, RPort, BorderRPort) are left");
    assert!(m.a.menu_allocs.is_empty() && m.a.menu_strips.is_empty());
}

#[test]
fn kinds_the_native_side_cannot_express_are_named_refusals_and_return_null() {
    let mut m = M::new();
    let glist = DATA - 8;
    let ng = new_gadget(&mut m, Some("x"), 1);
    let bad_slider = m.tags(&[(GT + 38, 9), (GT + 39, 1)]);
    m.run(&[
        gt(LVO_CREATE_CONTEXT, &[a(0, glist)]),                                                       // 0
        gt(LVO_CREATE_GADGET, &[d(0, 8), a_res(0, 0), a(1, ng), a(2, 0)]),                           // 1 PALETTE_KIND with no tags: 2 colours (now a Swatches control)
        gt(LVO_CREATE_GADGET, &[d(0, 10), a_res(0, 0), a(1, ng), a(2, 0)]),                          // 2 (the number GadTools leaves unused)
        gt(LVO_CREATE_GADGET, &[d(0, 99), a_res(0, 0), a(1, ng), a(2, 0)]),                          // 3 no such kind
        gt(LVO_CREATE_GADGET, &[d(0, 11), a_res(0, 0), a(1, ng), a(2, bad_slider)]),                 // 4 min > max
        gt(LVO_CREATE_GADGET, &[d(0, 2), a(0, 0), a(1, ng), a(2, 0)]),                               // 5 previous NULL
    ]);
    assert_ne!(m.result(1), 0, "PALETTE_KIND is bridged (tests/gt_palette.rs)");
    assert_eq!((2..=5).map(|i| m.result(i)).collect::<Vec<_>>(), [0; 4]);
    let r = &m.a.refused;
    assert!(r[0].contains("kind 10 not bridged") && r[1].contains("kind 99 not bridged"), "{r:?}");
    assert!(r[2].contains("GTSL_Min 9 > GTSL_Max 1"), "{r:?}");
    assert_eq!(r.len(), 3, "a NULL previous is GadTools' chained-failure rule, not a refusal");
    assert_eq!(m.a.gadgets.len(), 2, "the context gadget and the palette");
}

#[test]
fn set_and_get_gadget_attrs_for_each_kind() {
    let mut m = M::new();
    let glist = DATA - 8;
    let (chk, str_, int, sld) = (new_gadget(&mut m, Some("Check"), 2), new_gadget(&mut m, None, 3), new_gadget(&mut m, None, 4), new_gadget(&mut m, None, 5));
    let t_chk = m.tags(&[(GT + 4, 1)]);
    let abc = m.s("abc");
    let t_str = m.tags(&[(GT + 45, abc), (GT + 46, 5)]);
    let t_int = m.tags(&[(GT + 47, (-42i32) as u32), (GT + 48, 6)]);
    let t_sld = m.tags(&[(GT + 38, 0), (GT + 39, 10), (GT + 40, 4)]);
    let wtags = m.tags(&[(WA + 3, 200), (WA + 4, 120), (WA + 7, 0x40), (WA + 9, HEAP_START)]);
    let hi = m.s("hello world");
    let set_chk = m.tags(&[(GT + 4, 0)]);
    let set_str = m.tags(&[(GT + 45, hi)]);
    let set_int_ok = m.tags(&[(GT + 47, 123456)]);
    let set_int_big = m.tags(&[(GT + 47, 1234567)]);
    let set_sld = m.tags(&[(GT + 40, 100)]);
    let wrong_kind = m.tags(&[(GT + 4, 1)]);
    let disabled = m.tags(&[(GT + 40, 2), (TAG_USER + 0x30000 + 14, 1)]);
    let out = DATA - 64;
    let get_sld = m.tags(&[(GT + 40, out), (GT + 38, out + 4), (GT + 39, out + 8), (GT + 4, out + 12)]);
    let get_rest = m.tags(&[(GT + 47, out + 16), (GT + 45, out + 20)]);
    let get_chk = m.tags(&[(GT + 4, out + 24)]);
    m.run(&[
        gt(LVO_CREATE_CONTEXT, &[a(0, glist)]),                                                       // 0 == HEAP_START
        gt(LVO_CREATE_GADGET, &[d(0, 2), a_res(0, 0), a(1, chk), a(2, t_chk)]),                       // 1
        gt(LVO_CREATE_GADGET, &[d(0, 12), a_res(0, 1), a(1, str_), a(2, t_str)]),                     // 2
        gt(LVO_CREATE_GADGET, &[d(0, 3), a_res(0, 2), a(1, int), a(2, t_int)]),                       // 3
        gt(LVO_CREATE_GADGET, &[d(0, 11), a_res(0, 3), a(1, sld), a(2, t_sld)]),                      // 4
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, wtags)]),                                          // 5
        gt(LVO_SET_ATTRS, &[a_res(0, 1), a_res(1, 5), a(2, 0), a(3, set_chk)]),
        gt(LVO_SET_ATTRS, &[a_res(0, 2), a_res(1, 5), a(2, 0), a(3, set_str)]),
        gt(LVO_SET_ATTRS, &[a_res(0, 3), a_res(1, 5), a(2, 0), a(3, set_int_ok)]),
        gt(LVO_SET_ATTRS, &[a_res(0, 3), a_res(1, 5), a(2, 0), a(3, set_int_big)]),                  // refused
        gt(LVO_SET_ATTRS, &[a_res(0, 4), a_res(1, 5), a(2, 0), a(3, set_sld)]),                      // clamped
        gt(LVO_SET_ATTRS, &[a_res(0, 4), a_res(1, 5), a(2, 0), a(3, wrong_kind)]),                   // refused
        gt(LVO_SET_ATTRS, &[a_res(0, 4), a_res(1, 5), a(2, 0), a(3, disabled)]),                     // level applied, GA_Disabled ghosts the control
        gt(LVO_GET_ATTRS, &[a_res(0, 4), a_res(1, 5), a(2, 0), a(3, get_sld)]),                      // 13: 3 of 4 readable
        gt(LVO_GET_ATTRS, &[a_res(0, 3), a_res(1, 5), a(2, 0), a(3, get_rest)]),                     // 14: integer: number only
        gt(LVO_GET_ATTRS, &[a_res(0, 2), a_res(1, 5), a(2, 0), a(3, get_rest)]),                     // 15: string: string only
        gt(LVO_GET_ATTRS, &[a_res(0, 1), a_res(1, 5), a(2, 0), a(3, get_chk)]),                      // 16
        gt(LVO_GET_ATTRS, &[a(0, 0), a_res(1, 5), a(2, 0), a(3, get_chk)]),                          // 17: NULL gadget
        gt(LVO_CREATE_GADGET, &[d(0, 2), a_res(0, 4), a(1, chk), a(2, t_chk)]),                       // 18
        gt(LVO_CREATE_GADGET, &[d(0, 3), a_res(0, 18), a(1, int), a(2, t_int)]),                      // 19
    ]);
    let si19 = m.r32(m.result(19) + 34);
    assert_eq!(m.a.mem.read_cstr(m.r32(si19)).unwrap(), "-42", "GTIN_Number -42 is in the buffer");
    assert_eq!(m.r32(si19 + 28) as i32, -42, "and in StringInfo.LongInt");
    assert_eq!(m.result(0), HEAP_START);
    let ctl = |m: &M, i: u32| m.a.ui.calls.iter().filter_map(|c| match c { UiCall::SetControl(_, c, v) => Some((c.id, v.clone())), _ => None }).nth(i as usize);
    // Window is object 1; controls 2.. in creation order: Check 2, string 3, integer 4, slider 5.
    assert_eq!(ctl(&m, 0), Some((2, ControlValue::Checked(false))));
    let (chk_g, str_g, int_g) = (m.result(1), m.result(2), m.result(3));
    assert_eq!(m.r16(chk_g + 12) & 0x80, 0, "SELECTED cleared by GTCB_Checked FALSE");
    assert_ne!(m.r16(m.result(18) + 12) & 0x80, 0, "a checkbox created with GTCB_Checked starts SELECTED");
    assert_eq!(ctl(&m, 1), Some((3, ControlValue::Text("hello".into()))), "cut to GTST_MaxChars 5");
    let si = m.r32(str_g + 34);
    assert_eq!(m.a.mem.read_cstr(m.r32(si)).unwrap(), "hello");
    assert_eq!((m.r16(si + 16), m.r16(si + 10)), (5, 6));
    assert_eq!(ctl(&m, 2), Some((4, ControlValue::Text("123456".into()))));
    assert_eq!(m.r32(m.r32(int_g + 34) + 28), 123456, "StringInfo.LongInt");
    assert_eq!(ctl(&m, 3), Some((5, ControlValue::Level(10))), "GTSL_Level clamped to the slider's range");
    assert_eq!(ctl(&m, 4), Some((5, ControlValue::Level(2))), "the level of a call that also sets GA_Disabled is applied");
    assert!(m.a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControlEnabled(_, c, false) if c.id == 5)), "GA_Disabled reaches the host as set_control_enabled(false) on the slider");
    assert_eq!(m.r16(m.result(4) + 12) & 0x100, 0x100, "and the gadget's GFLG_DISABLED is set (gadtools ghosting is Intuition's flag)");
    assert_eq!(m.a.ui.calls.iter().filter(|c| matches!(c, UiCall::SetControlEnabled(..))).count(), 1, "NEGATIVE CONTROL: no other control was ghosted");
    let r = &m.a.refused;
    assert!(r[0].contains("does not fit MaxChars 6"), "{r:?}");
    assert!(r[1].contains("on a kind-11 gadget"), "{r:?}");
    // GT_GetGadgetAttrsA fills what the kind has, counts it, and records the rest as refused.
    assert_eq!((m.r32(out), m.r32(out + 4), m.r32(out + 8)), (2, 0, 10), "level 2 (the last applied), min 0, max 10");
    assert_eq!(m.result(13), 3);
    assert!(r[2].contains("not readable on a kind-11 gadget"), "{r:?}");
    assert_eq!((m.result(14), m.r32(out + 16) as i32), (1, 123456));
    assert_eq!((m.result(15), m.a.mem.read_cstr(m.r32(out + 20)).unwrap()), (1, "hello".to_string()));
    assert_eq!((m.result(16), m.r32(out + 24)), (1, 0), "checkbox was unchecked by the set");
    assert_eq!(m.result(17), 0, "a NULL gadget fills nothing");
    assert_eq!(r.len(), 2 + 1 + 1 + 1, "one refusal per unreadable tag: slider/checked, integer/string, string/number: {r:?}");
}

#[test]
fn modify_idcmp_creates_and_removes_the_user_port() {
    let mut m = M::new();
    let none = m.tags(&[(WA + 3, 100), (WA + 4, 60)]);
    m.run(&[
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, none)]),         // 0: no IDCMP, no port
        intu(LVO_MODIFY_IDCMP, &[a_res(0, 0), d(0, 0x200)]),        // 1: a port appears
        intu(LVO_MODIFY_IDCMP, &[a(0, 0x1234), d(0, 0x200)]),       // 2: not a window: refused
        intu(LVO_MODIFY_IDCMP, &[a_res(0, 0), d(0, 0)]),            // 3: and goes
        intu(LVO_CLOSE_WINDOW, &[a_res(0, 0)]),
    ]);
    assert_eq!((m.result(1), m.result(3)), (1, 1), "ModifyIDCMP returns TRUE");
    assert_eq!(m.a.refused, ["intuition.library/ModifyIDCMP: ModifyIDCMP: 0x1234 is not an open window"]);
    assert_eq!(m.a.heap.live_blocks(), 0, "no port left behind");
    // The same through the machine, stepwise, to see the port and signal in between.
    let mut m = M::new();
    let none = m.tags(&[(WA + 3, 100), (WA + 4, 60)]);
    m.a.start_hunk(&script(&[intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, none)]), intu(LVO_MODIFY_IDCMP, &[a_res(0, 0), d(0, 0x200)])])).unwrap();
    assert_eq!(m.a.step(1_000_000).0, Step::Exited);
    let win = m.result(0);
    assert_eq!(m.r32(win + 82), 0x200);
    let port = m.r32(win + 86);
    assert_ne!(port, 0);
    assert!(m.r32(win + 86) != 0 && m.a.mem.read_u8(port + 15).unwrap() >= 16, "a user signal bit");
}

#[test]
fn set_window_titles_reaches_the_native_window_and_minus_one_keeps_the_title() {
    let mut m = M::new();
    let w = m.tags(&[(WA + 3, 100), (WA + 4, 60), (WA + 11, 0)]);
    let (first, second) = (m.s("Renamed"), m.s("Screen"));
    m.run(&[
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, w)]),
        intu(LVO_SET_WINDOW_TITLES, &[a_res(0, 0), a(1, first), a(2, second)]),
        intu(LVO_SET_WINDOW_TITLES, &[a_res(0, 0), a(1, u32::MAX), a(2, second)]),   // unchanged
        intu(LVO_SET_WINDOW_TITLES, &[a_res(0, 0), a(1, 0), a(2, u32::MAX)]),         // NULL clears
        intu(LVO_SET_WINDOW_TITLES, &[a(0, 0x1234), a(1, first), a(2, 0)]),           // not a window
    ]);
    let titles: Vec<_> = m.a.ui.calls.iter().filter_map(|c| if let UiCall::SetWindowTitle(_, t) = c { Some(t.as_str()) } else { None }).collect();
    assert_eq!(titles, ["Renamed", ""], "-1 made no native call");
    assert_eq!(m.r32(m.result(0) + 32), 0, "Window.Title follows");
    assert_eq!(m.a.refused.len(), 1, "{:?}", m.a.refused);
}

#[test]
fn window_manager_calls_are_accepted_and_a_null_easystruct_requests_nothing() {
    let mut m = M::new();
    let w = m.tags(&[(WA + 3, 100), (WA + 4, 60)]);
    m.run(&[
        intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, w)]),
        intu(-450, &[a_res(0, 0)]),                 // ActivateWindow
        intu(-312, &[a_res(0, 0)]),                 // WindowToFront
        intu(-306, &[a_res(0, 0)]),                 // WindowToBack
        intu(-222, &[a(0, 0), a_res(1, 0), a(2, 0)]),           // RefreshGadgets
        intu(-432, &[a(0, 0), a_res(1, 0), a(2, 0), d(0, 1)]),  // RefreshGList
        intu(-450, &[a(0, 0x1234)]),                // not a window: refused
        intu(LVO_EASY_REQUEST, &[a_res(0, 0), a(1, 0), a(2, 0), a(3, 0)]),
    ]);
    assert_eq!(m.a.refused, ["intuition.library/ActivateWindow: ActivateWindow: 0x1234 is not an open window"], "EasyRequestArgs is bridged (requester.rs)");
    assert!(m.a.ui.calls.iter().all(|c| !matches!(c, UiCall::Request(..))), "a NULL EasyStruct asks the host nothing (the original answers 0)");
    assert_eq!(m.a.log.iter().filter(|l| l.contains("native window manager's")).count(), 3);
}

#[test]
fn layout_menu_items_makes_the_box_at_least_as_wide_as_its_title() {
    let mut m = M::new();
    let nm = m.newmenu(&[title("A rather long menu title"), item("x"), item("y")]);
    m.run(&[gt(LVO_CREATE_MENUS, &[a(0, nm), a(1, 0)])]);
    let menu = m.result(0);
    let items = m.r32(menu + 18);
    let with_menu = m.tags(&[(GT + 60, menu)]);
    let plain = m.tags(&[]);
    m.run(&[gt(LVO_LAYOUT_MENU_ITEMS, &[a(0, items), a(1, 0), a(2, plain)])]);
    assert_eq!(m.result(0), 1);
    assert_eq!(m.r16(items + 8), 8 + 2 + 2, "alone: one 8 px glyph plus the 2+2 px item padding");
    m.run(&[gt(LVO_LAYOUT_MENU_ITEMS, &[a(0, items), a(1, 0), a(2, with_menu)])]);
    assert_eq!(m.r16(items + 8), 24 * 8, "with GTMN_Menu: as wide as the 24-character title");
    assert_eq!(m.r16(m.r32(items) + 8), 24 * 8, "every item the same width");
    assert!(m.a.refused.is_empty(), "{:?}", m.a.refused);
}
