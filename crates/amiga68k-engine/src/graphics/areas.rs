//! graphics.library area fills: `InitArea`, `AreaMove`, `AreaDraw`, `AreaEllipse`, `AreaEnd` over the program's own `struct AreaInfo` and `struct
//! TmpRas` (graphics/rastport.h, gfx.h; AROS rom/graphics/{initarea,areamove,areadraw,areaellipse,areaend}.c). The vectors really live in the
//! buffer the program gave `InitArea` (two WORDs and one flag BYTE per vector, 5 bytes), so a program that counts them or reads them back
//! sees what it would on AmigaOS. `AreaEnd` closes the last polygon, computes each shape's mask with [`super::areafill`], writes it into the
//! TmpRas buffer (its `Size` is checked first: a TmpRas too small answers -1, as areaend.c does), and paints through it with `BltPattern`
//! ([`super::bltpattern`]); the vector table is then empty again and the cursor where it was.
//! `struct AreaInfo` = VctrTbl 0, VctrPtr 4, FlagTbl 8, FlagPtr 12 (pointers), Count 16, MaxCount 18, FirstX 20, FirstY 22 (WORDs): 24 bytes.
//! NOT done: the AREAOUTLINE outline of an ELLIPSE (needs DrawEllipse; a named refusal) and a vector table with no room to close its polygon.
use super::layout::*;
use super::{areafill, bltpattern, rastport};
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const AI_VCTR_TBL: u32 = 0;
const AI_VCTR_PTR: u32 = 4;
const AI_FLAG_TBL: u32 = 8;
const AI_FLAG_PTR: u32 = 12;
const AI_COUNT: u32 = 16;
const AI_MAX_COUNT: u32 = 18;
const AI_FIRST_X: u32 = 20;
const AI_FIRST_Y: u32 = 22;
/// graphics_intern.h AREAINFOFLAG_*.
const MOVE: u8 = 0;
const DRAW: u8 = 1;
const CLOSEDRAW: u8 = 2;
const ELLIPSE: u8 = 3;
/// rastport.h `AREAOUTLINE` in `rp->Flags`.
const AREAOUTLINE: u16 = 0x08;
const FULL: u32 = u32::MAX;

fn m() -> impl Fn(retro_guest_mem::MemError) -> Refusal { Refusal::mem("AreaInfo") }

struct Ai { at: u32, vptr: u32, fptr: u32, count: i16, max: i16 }

fn load<U: GuestUi>(a: &Amiga<U>, rp: u32) -> Result<Ai, Refusal> {
    let at = a.mem.read_u32(rp + RP_AREA_INFO).map_err(m())?;
    if at == 0 { return Err(Refusal(format!("Area call on RastPort {rp:#x}: it has no AreaInfo (InitArea and set rp->AreaInfo first)"))); }
    Ok(Ai { at, vptr: a.mem.read_u32(at + AI_VCTR_PTR).map_err(m())?, fptr: a.mem.read_u32(at + AI_FLAG_PTR).map_err(m())?,
        count: a.mem.read_i16(at + AI_COUNT).map_err(m())?, max: a.mem.read_i16(at + AI_MAX_COUNT).map_err(m())? })
}

impl Ai {
    fn store<U: GuestUi>(&self, a: &mut Amiga<U>) -> Result<(), Refusal> {
        a.mem.write_u32(self.at + AI_VCTR_PTR, self.vptr).map_err(m())?;
        a.mem.write_u32(self.at + AI_FLAG_PTR, self.fptr).map_err(m())?;
        a.mem.write_u16(self.at + AI_COUNT, self.count as u16).map_err(m())
    }
    /// Append a vector and its flag.
    fn push<U: GuestUi>(&mut self, a: &mut Amiga<U>, x: i32, y: i32, flag: u8) -> Result<(), Refusal> {
        a.mem.write_u16(self.vptr, x as u16).map_err(m())?;
        a.mem.write_u16(self.vptr + 2, y as u16).map_err(m())?;
        a.mem.write_u8(self.fptr, flag).map_err(m())?;
        self.vptr += 4;
        self.fptr += 1;
        self.count += 1;
        Ok(())
    }
    fn last_flag<U: GuestUi>(&self, a: &Amiga<U>) -> Result<u8, Refusal> { a.mem.read_u8(self.fptr - 1).map_err(m()) }
    fn set_first<U: GuestUi>(&self, a: &mut Amiga<U>, x: i32, y: i32) -> Result<(), Refusal> {
        a.mem.write_u16(self.at + AI_FIRST_X, x as u16).map_err(m())?;
        a.mem.write_u16(self.at + AI_FIRST_Y, y as u16).map_err(m())
    }
    /// `areaclosepolygon`: an open polygon (the last vector is a DRAW) is closed by a CLOSEDRAW back to its first point, or, when the last DRAW
    /// is already there, that vector becomes the CLOSEDRAW. The caller made sure there is room for one more vector.
    fn close<U: GuestUi>(&mut self, a: &mut Amiga<U>) -> Result<(), Refusal> {
        if self.last_flag(a)? != DRAW { return Ok(()); }
        let (fx, fy) = (a.mem.read_i16(self.at + AI_FIRST_X).map_err(m())?, a.mem.read_i16(self.at + AI_FIRST_Y).map_err(m())?);
        let (lx, ly) = (a.mem.read_i16(self.vptr - 4).map_err(m())?, a.mem.read_i16(self.vptr - 2).map_err(m())?);
        if (lx, ly) != (fx, fy) { self.push(a, fx as i32, fy as i32, CLOSEDRAW) } else { a.mem.write_u8(self.fptr - 1, CLOSEDRAW).map_err(m()) }
    }
}

/// InitArea(a0 = AreaInfo, a1 = buffer, d0 = maxvectors): the buffer holds `maxvectors` vectors of 5 bytes.
pub fn init_area<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ai, buf, max) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as u16);
    let flags = buf + 4 * max as u32;
    for (o, v) in [(AI_VCTR_TBL, buf), (AI_VCTR_PTR, buf), (AI_FLAG_TBL, flags), (AI_FLAG_PTR, flags)] { a.mem.write_u32(ai + o, v).map_err(m())?; }
    a.mem.write_u16(ai + AI_COUNT, 0).map_err(m())?;
    a.mem.write_u16(ai + AI_MAX_COUNT, max).map_err(m())?;
    Ok(format!("[graphics] InitArea {ai:#x} buffer {buf:#x} {max} vectors"))
}

fn result(cpu: &mut Cpu, ok: bool) { cpu.set_d0(if ok { 0 } else { FULL }); }

/// AreaMove(a1 = rp, d0 = x, d1 = y) -> 0, or -1 when the vector table is full.
pub fn area_move<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x, y) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let mut ai = load(a, rp)?;
    let ok = (|| -> Result<bool, Refusal> {
        if ai.count + 1 > ai.max { return Ok(false); }
        if ai.count == 0 { ai.set_first(a, x, y)?; ai.push(a, x, y, MOVE)?; return Ok(true); }
        if ai.last_flag(a)? == MOVE {
            // a MOVE right after a MOVE replaces it
            ai.set_first(a, x, y)?;
            a.mem.write_u16(ai.vptr - 4, x as u16).map_err(m())?;
            a.mem.write_u16(ai.vptr - 2, y as u16).map_err(m())?;
            return Ok(true);
        }
        ai.close(a)?;
        if ai.count + 1 > ai.max { return Ok(false); }
        ai.set_first(a, x, y)?;
        ai.push(a, x, y, MOVE)?;
        Ok(true)
    })()?;
    ai.store(a)?;
    result(cpu, ok);
    Ok(format!("[graphics] AreaMove {rp:#x} {x},{y} -> {}", if ok { 0 } else { -1 }))
}

/// AreaDraw(a1 = rp, d0 = x, d1 = y) -> 0, or -1 when full.
pub fn area_draw<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x, y) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    let mut ai = load(a, rp)?;
    let ok = ai.count + 1 <= ai.max;
    if ok { ai.push(a, x, y, DRAW)?; ai.store(a)?; }
    result(cpu, ok);
    Ok(format!("[graphics] AreaDraw {rp:#x} {x},{y} -> {}", if ok { 0 } else { -1 }))
}

/// AreaEllipse(a1 = rp, d0/d1 = centre, d2/d3 = radii a, b) -> 0, or -1 when there is no room for two vectors.
pub fn area_ellipse<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, cx, cy, ra, rb) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)), sx(cpu.dreg(2)), sx(cpu.dreg(3)));
    let mut ai = load(a, rp)?;
    let ok = (|| -> Result<bool, Refusal> {
        if ai.count + 2 > ai.max { return Ok(false); }
        if ai.count != 0 {
            ai.close(a)?;
            if ai.count + 2 > ai.max { return Ok(false); }
            if ai.last_flag(a)? == MOVE { ai.vptr -= 4; ai.fptr -= 1; ai.count -= 1; } // a MOVE just before it is dropped
            if ai.count + 2 > ai.max { return Ok(false); }
        }
        ai.push(a, cx, cy, ELLIPSE)?;
        ai.push(a, ra, rb, ELLIPSE)?;
        Ok(true)
    })()?;
    ai.store(a)?;
    result(cpu, ok);
    Ok(format!("[graphics] AreaEllipse {rp:#x} ({cx},{cy}) {ra}x{rb} -> {}", if ok { 0 } else { -1 }))
}

/// One filled shape of the vector table: a polygon's vertices (the last one the first again) or an ellipse.
enum Shape { Polygon(Vec<(i32, i32)>), Ellipse(i32, i32, i32, i32) }

/// AreaEnd(a1 = rp) -> 0, or -1 when the TmpRas is too small or the table is damaged.
pub fn area_end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(1);
    let mut ai = load(a, rp)?;
    let tmp = a.mem.read_u32(rp + RP_TMP_RAS).map_err(Refusal::mem("RastPort.TmpRas"))?;
    if ai.count == 0 || tmp == 0 { cpu.set_d0(0); return Ok(format!("[graphics] AreaEnd {rp:#x}: nothing to fill or no TmpRas")); }
    if ai.count >= ai.max && ai.last_flag(a)? == DRAW {
        let (fx, fy) = (a.mem.read_i16(ai.at + AI_FIRST_X).map_err(m())?, a.mem.read_i16(ai.at + AI_FIRST_Y).map_err(m())?);
        if (a.mem.read_i16(ai.vptr - 4).map_err(m())?, a.mem.read_i16(ai.vptr - 2).map_err(m())?) != (fx, fy) {
            return Err(Refusal(format!("AreaEnd {rp:#x}: the vector table is full ({} vectors) and has no room to close its polygon", ai.count)));
        }
    }
    ai.close(a)?;
    let (vtbl, ftbl) = (a.mem.read_u32(ai.at + AI_VCTR_TBL).map_err(m())?, a.mem.read_u32(ai.at + AI_FLAG_TBL).map_err(m())?);
    let n = ai.count as u32;
    let mut vecs: Vec<(i32, i32, u8)> = Vec::new();
    for i in 0..n { vecs.push((a.mem.read_i16(vtbl + 4 * i).map_err(m())? as i32, a.mem.read_i16(vtbl + 4 * i + 2).map_err(m())? as i32, a.mem.read_u8(ftbl + i).map_err(m())?)); }
    let st = rastport::read(a, rp)?;
    let (tr_ptr, tr_size) = (a.mem.read_u32(tmp).map_err(Refusal::mem("TmpRas"))?, a.mem.read_u32(tmp + 4).map_err(Refusal::mem("TmpRas"))?);
    let mut shapes: Vec<Shape> = Vec::new();
    let (mut poly, mut i, mut idx_first) = (Vec::<(i32, i32)>::new(), 0usize, 0usize);
    while i < vecs.len() {
        let (x, y, f) = vecs[i];
        match f {
            MOVE => { rastport::move_to(a, rp, x, y)?; poly = alloc::vec![(x, y)]; }
            DRAW => poly.push((x, y)),
            CLOSEDRAW => {
                poly.push((x, y));
                // "first_idx + 2 <= last_idx": at least three vertices before the closing one are not needed; a line (3 vectors) is not filled
                if poly.len() >= 4 { shapes.push(Shape::Polygon(core::mem::take(&mut poly))); }
                poly.clear();
                idx_first = i + 1;
            }
            ELLIPSE => { shapes.push(Shape::Ellipse(x, y, vecs[i + 1].0, vecs[i + 1].1)); i += 1; idx_first = i + 1; }
            other => { rastport::move_to(a, rp, st.cp.0, st.cp.1)?; return Err(Refusal(format!("AreaEnd {rp:#x}: vector {i} has the unknown flag {other}"))); }
        }
        i += 1;
    }
    let _ = idx_first;
    for sh in &shapes {
        let mask = match sh { Shape::Polygon(v) => areafill::polygon(v), Shape::Ellipse(cx, cy, ra, rb) => areafill::ellipse(*cx, *cy, *ra, *rb) };
        if (tr_size as i64) < mask.bytes_per_row as i64 * (mask.max_y - mask.min_y + 1) as i64 { cpu.set_d0(FULL); return Ok(format!("[graphics] AreaEnd {rp:#x}: TmpRas of {tr_size} bytes is too small -> -1")); }
        a.mem.write_bytes(tr_ptr, &mask.bits).map_err(Refusal::mem("TmpRas buffer"))?;
        bltpattern::paint_mask(a, rp, tr_ptr, (mask.min_x, mask.min_y, mask.max_x, mask.max_y), mask.bytes_per_row)?;
        if st.flags & AREAOUTLINE != 0 { outline(a, rp, sh, &st)?; }
    }
    ai.vptr = a.mem.read_u32(ai.at + AI_VCTR_TBL).map_err(m())?;
    ai.fptr = ftbl;
    ai.count = 0;
    ai.store(a)?;
    rastport::move_to(a, rp, st.cp.0, st.cp.1)?;
    a.mem.write_u16(rp + RP_FLAGS, st.flags).map_err(Refusal::mem("RastPort.Flags"))?;
    cpu.set_d0(0);
    Ok(format!("[graphics] AreaEnd {rp:#x}: {} shape(s) filled", shapes.len()))
}

/// AREAOUTLINE: the shape's outline in the outline pen (PolyDraw for a polygon), the pen and flags put back.
fn outline<U: GuestUi>(a: &mut Amiga<U>, rp: u32, sh: &Shape, st: &rastport::Rp) -> Result<(), Refusal> {
    let Shape::Polygon(v) = sh else { return Err(Refusal(format!("AreaEnd {rp:#x}: AREAOUTLINE of an ellipse needs DrawEllipse, which is not implemented"))) };
    a.mem.write_u8(rp + RP_FG_PEN, st.aol).map_err(Refusal::mem("RastPort"))?;
    // PolyDraw: Move to the first vertex, Draw to the others
    let r = (|| { rastport::move_to(a, rp, v[0].0, v[0].1)?; for &(x, y) in &v[1..] { super::draw::draw_to(a, rp, x, y)?; } Ok(()) })();
    r?;
    a.mem.write_u8(rp + RP_FG_PEN, st.fg).map_err(Refusal::mem("RastPort"))
}
