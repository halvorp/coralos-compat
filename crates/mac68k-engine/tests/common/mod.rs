//! A tiny program builder + rig for QuickDraw behaviour tests: flat 68k programs that push Pascal frames and call traps.
#![allow(dead_code)]
use guestui_abi::RecordingGuestUi;
use mac68k_engine::pascal::Rect;
use mac68k_engine::toolbox::WindowSpec;
use mac68k_engine::Mac;

pub mod ctl;

pub const GLOBALS: u32 = 0x3F00; // qd.thePort; the globals sit below it
pub const WIN_TOP: i16 = 100;
pub const WIN_LEFT: i16 = 50;
pub const WIN_W: i32 = 200;
pub const WIN_H: i32 = 120;
pub const SCRATCH: u32 = 0x3000; // results land here

#[derive(Default)]
pub struct Prog(pub Vec<u8>);
impl Prog {
    pub fn new() -> Prog { Prog::default() }
    fn raw(&mut self, w: &[u16]) -> &mut Self { for x in w { self.0.extend_from_slice(&x.to_be_bytes()); } self }
    /// move.l #v,-(sp)
    pub fn l(&mut self, v: u32) -> &mut Self { self.raw(&[0x2F3C, (v >> 16) as u16, v as u16]) }
    /// move.w #v,-(sp)
    pub fn w(&mut self, v: i16) -> &mut Self { self.raw(&[0x3F3C, v as u16]) }
    /// a Pascal BYTE/Boolean/CHAR argument: the byte rides in the high half of its 16-bit slot
    pub fn b(&mut self, v: u8) -> &mut Self { self.w((u16::from(v) << 8) as i16) }
    /// a Pascal CHAR / Style argument: a 16-bit slot with the value in the LOW byte (QuickDraw/Text.a: MOVE.B 5(SP))
    pub fn ch(&mut self, v: u8) -> &mut Self { self.w(i16::from(v)) }
    pub fn clr_w(&mut self) -> &mut Self { self.raw(&[0x4267]) }
    pub fn clr_l(&mut self) -> &mut Self { self.raw(&[0x42A7]) }
    pub fn trap(&mut self, op: u16) -> &mut Self { self.raw(&[op]) }
    /// move.w (sp)+,addr.l
    pub fn pop_w(&mut self, addr: u32) -> &mut Self { self.raw(&[0x33DF, (addr >> 16) as u16, addr as u16]) }
    /// move.l (sp)+,addr.l
    pub fn pop_l(&mut self, addr: u32) -> &mut Self { self.raw(&[0x23DF, (addr >> 16) as u16, addr as u16]) }
    /// push the word at `at` (move.w at.l,-(sp))
    pub fn mem_w(&mut self, at: u32) -> &mut Self { self.raw(&[0x3F39, (at >> 16) as u16, at as u16]) }
    /// Assemble scripted Toolbox calls (by name, Pascal frames from the machine's own signatures) into this program.
    pub fn script(&mut self, m: &Mac<RecordingGuestUi>, calls: &[mac68k_engine::fixture::ScriptCall]) -> &mut Self {
        use mac68k_engine::fixture::ScriptArg::{Lit, Mem};
        use mac68k_engine::pascal::Arg;
        for c in calls {
            let op = mac68k_engine::mixed::trap_opcode(c.name).unwrap_or_else(|| panic!("{} is not a trap", c.name));
            let (layout, result) = m.pascal_signature(op).unwrap_or_else(|| panic!("{} is not implemented", c.name));
            match result { Some(Arg::L) => { self.clr_l(); } Some(_) => { self.clr_w(); } None => {} }
            for (k, a) in layout.iter().zip(&c.args) {
                match (k, a) {
                    (Arg::L, Lit(v)) => { self.l(*v); } (Arg::W, Lit(v)) => { self.w(*v as i16); } (Arg::B, Lit(v)) => { self.b(*v as u8); }
                    (Arg::L, Mem(a)) => { self.rgn_arg(*a); } (_, Mem(a)) => { self.mem_w(*a); }
                }
            }
            assert_eq!(layout.len(), c.args.len(), "{}: argument count", c.name);
            self.trap(op);
            match (result, c.save) { (Some(Arg::L), s) => { self.pop_l(s.unwrap_or(0x3F80)); } (Some(_), s) => { self.pop_w(s.unwrap_or(0x3F80)); } (None, _) => {} }
        }
        self
    }
    pub fn exit(&mut self) -> &mut Self { self.trap(0xA9F4) }
    // ---- named calls (trap words verified by Mac::new through qd::reg) ----
    pub fn set_port(&mut self, p: u32) -> &mut Self { self.l(p).trap(0xA873) }
    pub fn rect_op(&mut self, trap: u16, r: u32) -> &mut Self { self.l(r).trap(trap) }
    pub fn paint_rect(&mut self, r: u32) -> &mut Self { self.rect_op(0xA8A2, r) }
    pub fn frame_rect(&mut self, r: u32) -> &mut Self { self.rect_op(0xA8A1, r) }
    pub fn erase_rect(&mut self, r: u32) -> &mut Self { self.rect_op(0xA8A3, r) }
    pub fn invert_rect(&mut self, r: u32) -> &mut Self { self.rect_op(0xA8A4, r) }
    pub fn fill_rect(&mut self, r: u32, pat: u32) -> &mut Self { self.l(r).l(pat).trap(0xA8A5) }
    pub fn pen_size(&mut self, w: i16, h: i16) -> &mut Self { self.w(w).w(h).trap(0xA89B) }
    pub fn pen_mode(&mut self, m: i16) -> &mut Self { self.w(m).trap(0xA89C) }
    pub fn pen_pat(&mut self, p: u32) -> &mut Self { self.l(p).trap(0xA89D) }
    pub fn move_to(&mut self, h: i16, v: i16) -> &mut Self { self.w(h).w(v).trap(0xA893) }
    pub fn line_to(&mut self, h: i16, v: i16) -> &mut Self { self.w(h).w(v).trap(0xA891) }
    pub fn pen_normal(&mut self) -> &mut Self { self.trap(0xA89E) }
    pub fn draw_string(&mut self, s: u32) -> &mut Self { self.l(s).trap(0xA884) }
    pub fn new_rgn(&mut self, store: u32) -> &mut Self { self.clr_l().trap(0xA8D8).pop_l(store) }
    pub fn rect_rgn(&mut self, rgn_at: u32, r: u32) -> &mut Self { self.rgn_arg(rgn_at).l(r).trap(0xA8DF) }
    /// push the region handle stored at `at` (move.l at.l,-(sp))
    pub fn rgn_arg(&mut self, at: u32) -> &mut Self { self.raw(&[0x2F39, (at >> 16) as u16, at as u16]) }
}

pub fn set_rect(m: &mut Mac<RecordingGuestUi>, at: u32, top: i16, left: i16, bottom: i16, right: i16) {
    for (i, v) in [top, left, bottom, right].into_iter().enumerate() { m.mem.write_u16(at + 2 * i as u32, v as u16).unwrap(); }
}
pub fn put(m: &mut Mac<RecordingGuestUi>, at: u32, b: &[u8]) { m.mem.write_bytes(at, b).unwrap(); }
pub fn pstr(s: &str) -> Vec<u8> { let mut v = vec![s.len() as u8]; v.extend_from_slice(s.as_bytes()); v }

pub struct Rig { pub m: Mac<RecordingGuestUi>, pub win: u32 }
impl Rig {
    pub fn new() -> Rig { Rig::with(true) }
    /// The CANVAS route (the safe mode a host without plane replay gets): the window is published as pixels, not a display list.
    pub fn canvas() -> Rig { Rig::canvas_with(true) }
    pub fn canvas_with(visible: bool) -> Rig { let mut r = Rig::with(visible); r.m.qd.plane.unsupported = true; r }
    pub fn with(visible: bool) -> Rig {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        mac68k_engine::quickdraw::fill_globals(&mut m, GLOBALS).unwrap();
        m.qd_globals = GLOBALS;
        let spec = WindowSpec { storage: 0, rect: Rect { top: WIN_TOP, left: WIN_LEFT, bottom: WIN_TOP + WIN_H as i16, right: WIN_LEFT + WIN_W as i16 }, title: "QD".into(), visible, proc_id: 0, behind: 0xFFFF_FFFF, go_away: true, ref_con: 0 };
        let win = m.make_window(spec).unwrap();
        m.ev.queue.clear();
        Rig { m, win }
    }
    /// Run `p` to its ExitToShell; refusals fail the test unless `allow_refusals`.
    pub fn run(&mut self, p: &Prog) -> &mut Self {
        self.m.start(&p.0, 0).unwrap();
        let r = self.m.run_loop(5_000_000).unwrap();
        assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}\n{:#?}", r.stats, self.m.log);
        self
    }
    pub fn no_refusals(&self) { assert!(self.m.refused.is_empty(), "{:?}", self.m.refused); }
    pub fn black(&self, x: i32, y: i32) -> bool { self.m.qd.canvases[&self.win].get(x, y) == Some(1) }
    pub fn count_black(&self) -> usize { self.m.qd.canvases[&self.win].pixels().iter().filter(|&&p| p == 1).count() }
    pub fn count_in(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> usize { (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| self.black(x, y)).count() }
    /// ASCII picture of a canvas region, for failure messages.
    pub fn pic(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> String { (y0..y1).map(|y| (x0..x1).map(|x| if self.black(x, y) { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("\n") }
}
