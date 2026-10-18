//! Shared by the gfx_* integration tests: a generated "call script" program (load registers, `jsr lvo(a6)`, store D0) so
//! each graphics call can be made with exact arguments and its result read back, plus helpers to build guest structs.
#![allow(dead_code)]
use amiga68k_engine::machine::{GRAPHICS_BASE, INTUITION_BASE};
use amiga68k_engine::{Amiga, Step};
use guestui_abi::*;
use retro_hunk::writer::{be16, be32, build, Hunk};

pub const AT: u32 = 0x10_0000;
pub const RESULTS: u32 = AT;
pub const DATA: u32 = AT + 0x1000;
pub const TAG_USER: u32 = 0x8000_0000;
pub const WA: u32 = TAG_USER + 99;

#[derive(Clone, Copy)]
pub enum R { D(u32), A(u32) }
#[derive(Clone, Copy)]
pub enum V { Imm(u32), Res(u32) }
impl From<u32> for V { fn from(v: u32) -> V { V::Imm(v) } }
pub type Call = (u32, i32, Vec<(R, V)>);

pub fn script(calls: &[Call]) -> Vec<u8> {
    let mut c = vec![];
    for (i, (base, lvo, regs)) in calls.iter().enumerate() {
        for (r, v) in regs {
            let (n, is_a) = match r { R::D(n) => (*n as u16, false), R::A(n) => (*n as u16, true) };
            let (op, val) = match v { V::Imm(x) => (if is_a { 0x207C } else { 0x203C }, *x), V::Res(i) => (if is_a { 0x2079 } else { 0x2039 }, RESULTS + 4 * i) };
            be16(&mut c, op | n << 9);
            be32(&mut c, val);
        }
        be16(&mut c, 0x2C7C); be32(&mut c, *base);
        be16(&mut c, 0x4EAE); be16(&mut c, *lvo as i16 as u16);
        be16(&mut c, 0x23C0); be32(&mut c, RESULTS + 4 * i as u32);
    }
    be16(&mut c, 0x4E75);
    build(&[Hunk::code(c, vec![])], None)
}

pub fn gfx(lvo: i32, regs: &[(R, V)]) -> Call { (GRAPHICS_BASE, lvo, regs.to_vec()) }
pub fn intu(lvo: i32, regs: &[(R, V)]) -> Call { (INTUITION_BASE, lvo, regs.to_vec()) }
pub fn a(n: u32, v: u32) -> (R, V) { (R::A(n), V::Imm(v)) }
pub fn d(n: u32, v: u32) -> (R, V) { (R::D(n), V::Imm(v)) }
pub fn di(n: u32, v: i32) -> (R, V) { (R::D(n), V::Imm(v as u32)) }
pub fn a_res(n: u32, i: u32) -> (R, V) { (R::A(n), V::Res(i)) }
pub fn d_res(n: u32, i: u32) -> (R, V) { (R::D(n), V::Res(i)) }

pub struct G { pub a: Amiga<RecordingGuestUi>, pub next: u32 }
impl G {
    pub fn new() -> G { G { a: Amiga::new(RecordingGuestUi::new()).unwrap(), next: DATA } }
    pub fn bytes(&mut self, b: &[u8]) -> u32 {
        let at = (self.next + 3) & !3;
        self.a.mem.write_bytes(at, b).unwrap();
        self.next = at + b.len() as u32 + 4;
        at
    }
    pub fn s(&mut self, s: &str) -> u32 { let mut v = s.as_bytes().to_vec(); v.push(0); self.bytes(&v) }
    pub fn zeroed(&mut self, n: u32) -> u32 { self.bytes(&vec![0; n as usize]) }
    pub fn tags(&mut self, t: &[(u32, u32)]) -> u32 {
        let mut v = vec![];
        for (k, x) in t.iter().chain(&[(0, 0)]) { v.extend(k.to_be_bytes()); v.extend(x.to_be_bytes()); }
        self.bytes(&v)
    }
    pub fn words(&mut self, w: &[u16]) -> u32 { self.bytes(&w.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<_>>()) }
    /// Run a call script to its end (the program exits; windows stay open).
    pub fn run(&mut self, calls: &[Call]) {
        self.a.start_hunk(&script(calls)).unwrap();
        let (s, _) = self.a.step(10_000_000);
        assert_eq!(s, Step::Exited, "{:?}", self.a.refused);
    }
    pub fn result(&self, i: u32) -> u32 { self.a.mem.read_u32(RESULTS + 4 * i).unwrap() }
    pub fn r8(&self, at: u32) -> u8 { self.a.mem.read_u8(at).unwrap() }
    pub fn r16(&self, at: u32) -> u16 { self.a.mem.read_u16(at).unwrap() }
    pub fn r32(&self, at: u32) -> u32 { self.a.mem.read_u32(at).unwrap() }
    /// A private chunky bitmap of w x h x depth and a RastPort drawing on it (via AllocBitMap + InitRastPort).
    pub fn bitmap_rp(&mut self, w: u32, h: u32, depth: u32) -> (u32, u32) {
        let rp = self.zeroed(100);
        self.run(&[gfx(ALLOC_BITMAP, &[d(0, w), d(1, h), d(2, depth), d(3, 1), a(0, 0)]), gfx(INIT_RASTPORT, &[a(1, rp)])]);
        let bm = self.result(0);
        assert_ne!(bm, 0);
        self.a.mem.write_u32(rp + 4, bm).unwrap();
        (bm, rp)
    }
    /// Select topaz 8 (the fixed 8x8 face, resident at 0x8000) on `rp`, as a program that asks for "topaz.font" by name does:
    /// the DEFAULT font is the Gute Form face (retro-theme), so tests of topaz mechanics say so explicitly.
    pub fn set_topaz(&mut self, rp: u32) { self.run(&[gfx(-66, &[a(1, rp), a(0, 0x8000)])]); }
    /// The host-side pixel of a guest bitmap.
    pub fn px(&self, bm: u32, x: i32, y: i32) -> Option<u8> { self.a.gfx.bitmaps[&bm].bm.get(x, y) }
    pub fn rows(&self, bm: u32, x0: i32, y0: i32, w: i32, h: i32) -> Vec<Vec<u8>> {
        (y0..y0 + h).map(|y| (x0..x0 + w).map(|x| self.px(bm, x, y).unwrap_or(255)).collect()).collect()
    }
}

// LVOs from src/lvo_names.rs (graphics.library, intuition.library), looked up by name.
pub const ALLOC_BITMAP: i32 = -918;
pub const FREE_BITMAP: i32 = -924;
pub const INIT_RASTPORT: i32 = -198;
pub const SET_APEN: i32 = -342;
pub const SET_BPEN: i32 = -348;
pub const SET_DRMD: i32 = -354;
pub const SET_ABPEN_DRMD: i32 = -894;
pub const RECT_FILL: i32 = -306;
pub const MOVE: i32 = -240;
pub const DRAW: i32 = -246;
pub const TEXT: i32 = -60;
pub const TEXT_LENGTH: i32 = -54;
pub const READ_PIXEL: i32 = -318;
pub const WRITE_PIXEL: i32 = -324;
pub const ERASE_RECT: i32 = -810;
pub const SCROLL_RASTER: i32 = -396;
pub const BLT_BITMAP: i32 = -30;
pub const BLT_BITMAP_RASTPORT: i32 = -606;
pub const CLIP_BLIT: i32 = -552;
pub const BLT_TEMPLATE: i32 = -36;
pub const BLT_MASK_BITMAP_RASTPORT: i32 = -636;
pub const SET_RAST: i32 = -234;
pub const CLEAR_EOL: i32 = -42;
pub const OPEN_WINDOW: i32 = -204;
pub const CLOSE_WINDOW: i32 = -72;
pub const SET_RGB32: i32 = -852;
