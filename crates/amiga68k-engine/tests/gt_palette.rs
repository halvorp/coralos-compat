//! GadTools PALETTE_KIND (rom workbench/libs/gadtools/paletteclass.c) as the `Swatches` role: the colours are the pens offset + i (or
//! GTPA_ColorTable[i]) read from the screen palette, GTPA_Color is a PEN held as an index, a pick is `SetValue(Choice(index))` and ends
//! with GADGETUP Code = the PEN.
mod common;
use common::gtscript::*;
use guestui_abi::*;

const LVO_CREATE_CONTEXT: i32 = -114;
const LVO_CREATE_GADGET: i32 = -30;
const LVO_SET_ATTRS: i32 = -42;
const LVO_GET_ATTRS: i32 = -174;
const LVO_OPEN_WINDOW_TAGS: i32 = -606;
const GTPA_DEPTH: u32 = GT + 16;
const GTPA_COLOR: u32 = GT + 17;
const GTPA_COLOR_OFFSET: u32 = GT + 18;
const GTPA_NUM_COLORS: u32 = GT + 70;
const GTPA_COLOR_TABLE: u32 = GT + 82;
const GADGETUP: u32 = 0x40;
const HEAP_START: u32 = 0x20_0000;

struct Pal { m: M, pal: u32, win: u32 }

fn open(creation: &[(u32, u32)], table: Option<&[u8]>) -> Pal {
    let mut m = M::new();
    let glist = DATA - 8;
    let ng = new_gadget(&mut m, Some("Colours"), 7);
    let mut tags = creation.to_vec();
    if let Some(t) = table { let p = { let at = m.s(""); m.a.mem.write_bytes(at, t).unwrap(); at }; tags.push((GTPA_COLOR_TABLE, p)); }
    let t = m.tags(&tags);
    let wtags = m.tags(&[(WA + 3, 200), (WA + 4, 120), (WA + 7, GADGETUP), (WA + 9, HEAP_START)]);
    m.run(&[gt(LVO_CREATE_CONTEXT, &[a(0, glist)]), gt(LVO_CREATE_GADGET, &[d(0, 8), a_res(0, 0), a(1, ng), a(2, t)]), intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, wtags)])]);
    let (pal, win) = (m.result(1), m.result(2));
    assert!(pal != 0 && win != 0, "{:?}", m.a.refused);
    Pal { m, pal, win }
}

impl Pal {
    fn control(&self) -> (UiObjectRef, ControlModel) {
        self.m.a.ui.controls.iter().find(|(_, (_, c))| c.token.0 == self.pal as u64).map(|(&id, (_, c))| (UiObjectRef { id, generation: 1 }, c.clone())).expect("the palette's control")
    }
    fn swatches(&self) -> (Vec<u32>, u32) { match self.control().1.kind { ControlKind::Swatches { colors, selected } => (colors, selected), k => panic!("{k:?}") } }
    fn pen_colour(&mut self, pen: usize) -> u32 { self.m.a.gfx.palette().rgb[pen] & 0xFF_FFFF }
    fn pick(&mut self, i: u32) -> Result<amiga68k_engine::Delivered, amiga68k_engine::Refusal> {
        let (ctl, _) = self.control();
        let native = *self.m.a.windows.get(&self.win).unwrap();
        self.m.a.deliver(GuestUiEvent::intent(native, ctl, CallbackToken(self.pal as u64), Intent::SetValue(ControlValue::Choice(i))))
    }
    fn messages(&self) -> Vec<(u32, u32)> {
        let port = self.m.r32(self.win + 86);
        let (mut out, mut n) = (vec![], self.m.r32(port + 20));
        while self.m.r32(n) != 0 { out.push((self.m.r32(n + 20), self.m.r16(n + 24) as u32)); n = self.m.r32(n); }
        out
    }
    fn get_color(&mut self) -> u32 {
        let out = DATA - 64;
        let t = self.m.tags(&[(GTPA_COLOR, out)]);
        self.m.run(&[gt(LVO_GET_ATTRS, &[a(0, self.pal), a(1, self.win), a(2, 0), a(3, t)])]);
        self.m.r32(out)
    }
}

#[test]
fn a_palette_is_a_row_of_swatches_from_the_screen_palette_and_a_pick_returns_the_pen() {
    // 4 colours (depth 2) starting at pen 4; GTPA_Color is the PEN 6 = index 2.
    let mut p = open(&[(GTPA_DEPTH, 2), (GTPA_COLOR_OFFSET, 4), (GTPA_COLOR, 6)], None);
    let want: Vec<u32> = (4..8).map(|pen| p.pen_colour(pen)).collect();
    assert_eq!(p.swatches(), (want.clone(), 2), "pens 4..8 from the screen palette; the selection is the index of pen 6");
    assert_eq!(p.get_color(), 6, "GT_GetGadgetAttrs(GTPA_Color) answers the PEN (GetPalettePen)");
    p.pick(3).unwrap();
    assert_eq!(p.messages(), [(GADGETUP, 7)], "GADGETUP with Code = the pen of the picked colour");
    assert_eq!(p.get_color(), 7, "the guest state follows (GTPA_Color reads the new pen)");
    // NEGATIVE CONTROLS: a colour the palette does not have is a named refusal and changes nothing.
    let e = p.pick(4).unwrap_err();
    assert!(e.0.contains("colour 4 outside its 4 colours"), "{e:?}");
    assert_eq!((p.messages().len(), p.get_color()), (1, 7));
}

#[test]
fn a_color_table_names_the_pens_and_num_colors_beats_depth() {
    let mut p = open(&[(GTPA_DEPTH, 5), (GTPA_NUM_COLORS, 3), (GTPA_COLOR, 5)], Some(&[9, 1, 5]));
    let want: Vec<u32> = [9, 1, 5].iter().map(|&pen| p.pen_colour(pen)).collect();
    assert_eq!(p.swatches(), (want, 2), "GTPA_NumColors 3 wins over GTPA_Depth 5; pen 5 is table index 2");
    p.pick(1).unwrap();
    assert_eq!(p.messages(), [(GADGETUP, 1)], "Code is table[1]");
}

#[test]
fn set_attrs_moves_the_selection_by_pen_and_a_depth_change_resets_a_selection_that_no_longer_fits() {
    let mut p = open(&[(GTPA_DEPTH, 3), (GTPA_COLOR, 6)], None);
    assert_eq!((p.swatches().0.len(), p.swatches().1), (8, 6));
    let t = p.m.tags(&[(GTPA_COLOR, 2)]);
    p.m.run(&[gt(LVO_SET_ATTRS, &[a(0, p.pal), a(1, p.win), a(2, 0), a(3, t)])]);
    assert_eq!(p.swatches().1, 2, "GTPA_Color 2 (a pen) selects index 2");
    assert!(p.m.a.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(_, _, ControlValue::Choice(2)))), "the host was told");
    let t = p.m.tags(&[(GTPA_COLOR, 6), (GTPA_DEPTH, 2)]);
    p.m.run(&[gt(LVO_SET_ATTRS, &[a(0, p.pal), a(1, p.win), a(2, 0), a(3, t)])]);
    assert_eq!((p.swatches().0.len(), p.swatches().1), (4, 0), "palette_set: after a depth change `if (pd_Color > NumColors - 1) pd_Color = 0`");
}

#[test]
fn a_palette_deeper_than_the_palette_is_a_named_refusal() {
    let mut m = M::new();
    let ng = new_gadget(&mut m, None, 1);
    let t = m.tags(&[(GTPA_DEPTH, 9)]); // 512 colours: pens past the screen's 256
    let t2 = m.tags(&[(GTPA_DEPTH, 40)]);
    m.run(&[gt(LVO_CREATE_CONTEXT, &[a(0, DATA - 8)]), gt(LVO_CREATE_GADGET, &[d(0, 8), a_res(0, 0), a(1, ng), a(2, t)]), gt(LVO_CREATE_GADGET, &[d(0, 8), a_res(0, 0), a(1, ng), a(2, t2)])]);
    assert_eq!((m.result(1), m.result(2)), (0, 0));
    assert!(m.a.refused.iter().any(|r| r.contains("outside the screen's 256-entry palette")), "{:?}", m.a.refused);
    assert!(m.a.refused.iter().any(|r| r.contains("GTPA_Depth 40 is beyond 16 bits")), "{:?}", m.a.refused);
}

#[test]
fn a_gadtools_gadget_added_with_addglist_gets_its_control_and_loses_it_with_removeglist() {
    const ADD_GLIST: i32 = -438;
    const REMOVE_GLIST: i32 = -444;
    let mut m = M::new();
    let ng = new_gadget(&mut m, Some("Colours"), 7);
    let wtags = m.tags(&[(WA + 3, 200), (WA + 4, 120), (WA + 7, GADGETUP)]); // no WA_Gadgets
    m.run(&[gt(LVO_CREATE_CONTEXT, &[a(0, DATA - 8)]), gt(LVO_CREATE_GADGET, &[d(0, 8), a_res(0, 0), a(1, ng), a(2, 0)]), intu(LVO_OPEN_WINDOW_TAGS, &[a(0, 0), a(1, wtags)])]);
    let (pal, win) = (m.result(1), m.result(2));
    assert!(m.a.ui.controls.is_empty(), "not in the window yet");
    m.run(&[intu(ADD_GLIST, &[a(0, win), a(1, pal), d(0, 0xFFFF), d(1, 1), a(2, 0)])]);
    assert_eq!(m.a.ui.controls.len(), 1, "AddGList projects a GadTools gadget like WA_Gadgets does");
    m.run(&[intu(REMOVE_GLIST, &[a(0, win), a(1, pal), d(0, 1)])]);
    assert!(m.a.ui.controls.is_empty(), "RemoveGList retires it");
}
