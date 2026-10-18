//! The "call script" harness: a tiny generated m68k program (movea/move #imm into registers, jsr LVO(a6), store D0) that calls
//! library functions with arguments the test pokes into guest RAM, so every error path and guest-visible struct can be checked
//! without a hand-assembled program for each. Shared by gt_calls.rs and gt_kinds_more.rs.
#![allow(dead_code)]
use amiga68k_engine::machine::{GADTOOLS_BASE, INTUITION_BASE};
use amiga68k_engine::{Amiga, Step};
use guestui_abi::*;
use retro_hunk::writer::{be16, be32, build, Hunk};

pub const AT: u32 = 0x10_0000; // scratch guest RAM (between the program and the stack): results, then argument data
pub const RESULTS: u32 = AT;
pub const DATA: u32 = AT + 0x1000;
pub const TAG_USER: u32 = 0x8000_0000;
pub const GT: u32 = TAG_USER + 0x80000;
pub const WA: u32 = TAG_USER + 99;

#[derive(Clone, Copy)]
pub enum R { D(u32), A(u32) }
/// A register argument: an immediate, or the D0 an earlier call (by index) returned.
#[derive(Clone, Copy)]
pub enum V { Imm(u32), Res(u32) }
impl From<u32> for V { fn from(v: u32) -> V { V::Imm(v) } }
pub type Call = (u32, i32, Vec<(R, V)>);

/// The program: for each call load the registers, `movea.l #base,a6; jsr lvo(a6)`, store D0 at RESULTS + 4*i.
pub fn script(calls: &[Call]) -> Vec<u8> {
    let mut c = vec![];
    for (i, (base, lvo, regs)) in calls.iter().enumerate() {
        for (r, v) in regs {
            let (n, is_a) = match r { R::D(n) => (*n as u16, false), R::A(n) => (*n as u16, true) };
            // move.l #imm,Dn / movea.l #imm,An  |  move.l abs.l,Dn / movea.l abs.l,An
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

pub fn call(base: u32, lvo: i32, regs: &[(R, V)]) -> Call { (base, lvo, regs.to_vec()) }
pub fn gt(lvo: i32, regs: &[(R, V)]) -> Call { call(GADTOOLS_BASE, lvo, regs) }
pub fn intu(lvo: i32, regs: &[(R, V)]) -> Call { call(INTUITION_BASE, lvo, regs) }
pub fn a(n: u32, v: u32) -> (R, V) { (R::A(n), V::Imm(v)) }
pub fn d(n: u32, v: u32) -> (R, V) { (R::D(n), V::Imm(v)) }
pub fn a_res(n: u32, i: u32) -> (R, V) { (R::A(n), V::Res(i)) }

pub struct M { pub a: Amiga<RecordingGuestUi>, pub next: u32 }
impl M {
    pub fn new() -> M { M { a: Amiga::new(RecordingGuestUi::new()).unwrap(), next: DATA } }
    /// Put a C string in scratch RAM, return its address.
    pub fn s(&mut self, s: &str) -> u32 {
        let at = self.next;
        self.a.mem.write_bytes(at, s.as_bytes()).unwrap();
        self.next += (s.len() as u32 + 4) & !1;
        at
    }
    pub fn tags(&mut self, t: &[(u32, u32)]) -> u32 {
        let at = (self.next + 3) & !3;
        for (i, (k, v)) in t.iter().chain(&[(0, 0)]).enumerate() { self.a.mem.write_u32(at + 8 * i as u32, *k).unwrap(); self.a.mem.write_u32(at + 8 * i as u32 + 4, *v).unwrap(); }
        self.next = at + 8 * (t.len() as u32 + 1);
        at
    }
    /// A NewMenu table: (type, label, key, flags, exclude, userdata); label u32::MAX = NM_BARLABEL.
    pub fn newmenu(&mut self, rows: &[(u8, Option<&str>, Option<&str>, u16, u32, u32)]) -> u32 {
        let at = (self.next + 3) & !3;
        self.next = at + 20 * (rows.len() as u32 + 1);
        for (i, &(ty, label, key, flags, excl, user)) in rows.iter().enumerate() {
            let (label, key) = (label.map(|l| if l == "-" { u32::MAX } else { self.s(l) }).unwrap_or(0), key.map(|k| self.s(k)).unwrap_or(0));
            let e = at + 20 * i as u32;
            self.a.mem.write_u8(e, ty).unwrap(); self.a.mem.write_u32(e + 2, label).unwrap(); self.a.mem.write_u32(e + 6, key).unwrap();
            self.a.mem.write_u16(e + 10, flags).unwrap(); self.a.mem.write_u32(e + 12, excl).unwrap(); self.a.mem.write_u32(e + 16, user).unwrap();
        }
        at
    }
    pub fn run(&mut self, calls: &[Call]) {
        self.a.start_hunk(&script(calls)).unwrap();
        let (s, _) = self.a.step(1_000_000);
        assert_eq!(s, Step::Exited, "{:?}", self.a.refused);
    }
    pub fn result(&self, i: u32) -> u32 { self.a.mem.read_u32(RESULTS + 4 * i).unwrap() }
    pub fn r16(&self, at: u32) -> u16 { self.a.mem.read_u16(at).unwrap() }
    pub fn r32(&self, at: u32) -> u32 { self.a.mem.read_u32(at).unwrap() }
}


/// A NewGadget at `m.next` (30 bytes zeroed; 10,10 60x14; text `label`, id `id`).
pub fn new_gadget(m: &mut M, label: Option<&str>, id: u16) -> u32 {
    let at = (m.next + 3) & !3;
    m.next = at + 32;
    for i in 0..30 { m.a.mem.write_u8(at + i, 0).unwrap(); }
    for (o, v) in [(0, 10u16), (2, 10), (4, 60), (6, 14)] { m.a.mem.write_u16(at + o, v).unwrap(); }
    let l = label.map(|l| m.s(l)).unwrap_or(0);
    m.a.mem.write_u32(at + 8, l).unwrap();
    m.a.mem.write_u16(at + 16, id).unwrap();
    at
}

