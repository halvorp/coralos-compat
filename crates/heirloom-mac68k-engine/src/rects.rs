//! QuickDraw's pure geometry: `Rect` and `Point` arithmetic that needs no port and draws nothing (SetRect, OffsetRect,
//! InsetRect, SectRect, UnionRect, Pt2Rect, PtInRect, EmptyRect, EqualRect, SetPt, AddPt, SubPt, EqualPt).
//! Semantics from Inside Macintosh "QuickDraw": a Rect is top, left, bottom, right; it contains the points with
//! `top <= v < bottom` and `left <= h < right`; a Point passed by value is one long, `v` in the high word. Arithmetic
//! wraps at 16 bits like the 68k word instructions did.
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg, Rect};
use crate::qd::port::write_rect;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

impl Rect {
    pub fn is_empty(&self) -> bool { self.bottom <= self.top || self.right <= self.left }
    pub fn contains(&self, v: i16, h: i16) -> bool { v >= self.top && v < self.bottom && h >= self.left && h < self.right }
    /// SectRect: the intersection, or the all-zero rect when it is empty (as the real routine stores).
    pub fn sect(&self, o: &Rect) -> Rect {
        let r = Rect { top: self.top.max(o.top), left: self.left.max(o.left), bottom: self.bottom.min(o.bottom), right: self.right.min(o.right) };
        if r.is_empty() { Rect { top: 0, left: 0, bottom: 0, right: 0 } } else { r }
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect { top: self.top.min(o.top), left: self.left.min(o.left), bottom: self.bottom.max(o.bottom), right: self.right.max(o.right) }
    }
    pub fn offset(&self, dh: i16, dv: i16) -> Rect {
        Rect { top: self.top.wrapping_add(dv), left: self.left.wrapping_add(dh), bottom: self.bottom.wrapping_add(dv), right: self.right.wrapping_add(dh) }
    }
    /// InsetRect: shrink by (dh, dv) per side; a rect that collapses becomes the all-zero rect.
    pub fn inset(&self, dh: i16, dv: i16) -> Rect {
        let r = Rect { top: self.top.wrapping_add(dv), left: self.left.wrapping_add(dh), bottom: self.bottom.wrapping_sub(dv), right: self.right.wrapping_sub(dh) };
        if r.is_empty() { Rect { top: 0, left: 0, bottom: 0, right: 0 } } else { r }
    }
}

fn pt(v: u32) -> (i16, i16) { ((v >> 16) as i16, v as i16) }
fn done(log: &str) -> Result<Done, Refusal> { Ok(Done { result: 0, log: format!("[quickdraw] {log}") }) }
fn flag(name: &str, b: bool) -> Result<Done, Refusal> { Ok(Done { result: u32::from(b), log: format!("[quickdraw] {name} -> {b}") }) }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA8A7, &[Arg::L, Arg::W, Arg::W, Arg::W, Arg::W], None, set_rect::<U>)?; // SetRect(VAR r; left, top, right, bottom)
    m.register(0xA8A8, &[Arg::L, Arg::W, Arg::W], None, offset_rect::<U>)?;
    m.register(0xA8A9, &[Arg::L, Arg::W, Arg::W], None, inset_rect::<U>)?;
    m.register(0xA8AA, &[Arg::L, Arg::L, Arg::L], Some(Arg::B), sect_rect::<U>)?;
    m.register(0xA8AB, &[Arg::L, Arg::L, Arg::L], None, union_rect::<U>)?;
    m.register(0xA8AC, &[Arg::L, Arg::L, Arg::L], None, pt2_rect::<U>)?;
    m.register(0xA8AD, &[Arg::L, Arg::L], Some(Arg::B), pt_in_rect::<U>)?;
    m.register(0xA8AE, &[Arg::L], Some(Arg::B), empty_rect::<U>)?;
    m.register(0xA8A6, &[Arg::L, Arg::L], Some(Arg::B), equal_rect::<U>)?;
    m.register(0xA880, &[Arg::L, Arg::W, Arg::W], None, set_pt::<U>)?;
    m.register(0xA87E, &[Arg::L, Arg::L], None, add_pt::<U>)?;
    m.register(0xA87F, &[Arg::L, Arg::L], None, sub_pt::<U>)?;
    m.register(0xA881, &[Arg::L, Arg::L], Some(Arg::B), equal_pt::<U>)
}

fn set_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    let r = Rect { left: a.get(1) as i16, top: a.get(2) as i16, right: a.get(3) as i16, bottom: a.get(4) as i16 };
    write_rect(&mut m.mem, a.get(0), r)?;
    done(&format!("SetRect {:#x} = ({},{},{},{})", a.get(0), r.top, r.left, r.bottom, r.right))
}
fn offset_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = read_rect(&m.mem, c.args.get(0))?.offset(c.args.get(1) as i16, c.args.get(2) as i16);
    write_rect(&mut m.mem, c.args.get(0), r)?;
    done("OffsetRect")
}
fn inset_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = read_rect(&m.mem, c.args.get(0))?.inset(c.args.get(1) as i16, c.args.get(2) as i16);
    write_rect(&mut m.mem, c.args.get(0), r)?;
    done("InsetRect")
}
fn sect_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = read_rect(&m.mem, c.args.get(0))?.sect(&read_rect(&m.mem, c.args.get(1))?);
    write_rect(&mut m.mem, c.args.get(2), r)?;
    flag("SectRect", !r.is_empty())
}
fn union_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = read_rect(&m.mem, c.args.get(0))?.union(&read_rect(&m.mem, c.args.get(1))?);
    write_rect(&mut m.mem, c.args.get(2), r)?;
    done("UnionRect")
}
fn pt2_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let ((v1, h1), (v2, h2)) = (pt(c.args.get(0)), pt(c.args.get(1)));
    write_rect(&mut m.mem, c.args.get(2), Rect { top: v1.min(v2), left: h1.min(h2), bottom: v1.max(v2), right: h1.max(h2) })?;
    done("Pt2Rect")
}
fn pt_in_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (v, h) = pt(c.args.get(0));
    flag("PtInRect", read_rect(&m.mem, c.args.get(1))?.contains(v, h))
}
fn empty_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { flag("EmptyRect", read_rect(&m.mem, c.args.get(0))?.is_empty()) }
fn equal_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    flag("EqualRect", read_rect(&m.mem, c.args.get(0))? == read_rect(&m.mem, c.args.get(1))?)
}
fn set_pt<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let e = Refusal::mem("Point");
    m.mem.write_u16(c.args.get(0), c.args.get(2) as u16).map_err(&e)?; // v first
    m.mem.write_u16(c.args.get(0) + 2, c.args.get(1) as u16).map_err(&e)?;
    done("SetPt")
}
fn add_sub<U: GuestUi>(m: &mut Mac<U>, c: &Call, sign: i16) -> Result<Done, Refusal> {
    let (sv, sh) = pt(c.args.get(0));
    let at = c.args.get(1);
    let e = Refusal::mem("Point");
    let (dv, dh) = (m.mem.read_i16(at).map_err(&e)?, m.mem.read_i16(at + 2).map_err(&e)?);
    m.mem.write_u16(at, dv.wrapping_add(sign.wrapping_mul(sv)) as u16).map_err(&e)?;
    m.mem.write_u16(at + 2, dh.wrapping_add(sign.wrapping_mul(sh)) as u16).map_err(&e)?;
    done(if sign == 1 { "AddPt" } else { "SubPt" })
}
fn add_pt<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { add_sub(m, c, 1) }
fn sub_pt<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { add_sub(m, c, -1) }
fn equal_pt<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { flag("EqualPt", c.args.get(0) == c.args.get(1)) }

#[cfg(test)]
mod tests {
    use super::*;
    const R: Rect = Rect { top: 10, left: 20, bottom: 110, right: 220 };

    #[test]
    fn rect_algebra() {
        assert_eq!(R.offset(1, 2), Rect { top: 12, left: 21, bottom: 112, right: 221 });
        assert_eq!(R.inset(5, 7), Rect { top: 17, left: 25, bottom: 103, right: 215 });
        assert!(R.contains(10, 20) && !R.contains(110, 100) && !R.contains(50, 220), "half-open: bottom and right are outside");
        assert_eq!(R.sect(&Rect { top: 50, left: 100, bottom: 500, right: 500 }), Rect { top: 50, left: 100, bottom: 110, right: 220 });
        assert_eq!(R.union(&Rect { top: 0, left: 300, bottom: 5, right: 310 }), Rect { top: 0, left: 20, bottom: 110, right: 310 });
        // NEGATIVE: disjoint rects intersect to the all-zero (empty) rect; an inset past the middle collapses
        let none = R.sect(&Rect { top: 200, left: 0, bottom: 300, right: 10 });
        assert_eq!(none, Rect { top: 0, left: 0, bottom: 0, right: 0 });
        assert!(none.is_empty());
        assert_eq!(R.inset(200, 0), Rect { top: 0, left: 0, bottom: 0, right: 0 });
        assert!(Rect { top: 5, left: 5, bottom: 5, right: 9 }.is_empty(), "zero height is empty");
        assert_eq!(Rect { top: i16::MAX, left: 0, bottom: 0, right: 0 }.offset(0, 1).top, i16::MIN, "16-bit wrap, no panic");
    }
}
