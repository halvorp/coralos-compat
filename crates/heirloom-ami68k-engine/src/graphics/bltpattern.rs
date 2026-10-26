//! graphics.library `BltPattern(rp, mask, xMin, yMin, xMax, yMax, byteCnt)`: the pen or the AreaPtrn, in the RastPort's draw mode, through a
//! 1-bit mask (a NULL mask = the whole rectangle). AROS rom/graphics/bltpattern.c: COMPLEMENT with JAM2 inverts the whole area (a mask limits it
//! to its set bits); without an AreaPtrn it is a `BltTemplate` of the mask (JAM2 becomes JAM1: the paper outside the mask is never painted, INVERSVID
//! flips the template); with one, the pattern's bit (16 wide, anchored at the RastPort origin; INVERSVID inverts it) selects the pen inside the
//! mask. The plane route records a solid pen through a mask as the mask's runs, each one a named fill: nothing is guessed from pixels.
use super::layout::*;
use super::record::{self, Prim};
use super::{draw, rastport, target};
use crate::{machine::Amiga, Refusal};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::{paint::{self, Brush, Clip, COMPLEMENT, JAM1, JAM2}, Rect};

/// BltPattern(a1 = rp, a0 = mask, d0..d3 = xMin yMin xMax yMax, d4 = byteCnt).
pub fn blt_pattern<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, mask, byte_cnt) = (cpu.areg(1), cpu.areg(0), cpu.dreg(4) as u16 as i32);
    let r = (sx(cpu.dreg(0)), sx(cpu.dreg(1)), sx(cpu.dreg(2)), sx(cpu.dreg(3)));
    paint_mask(a, rp, mask, r, byte_cnt)?;
    Ok(format!("[graphics] BltPattern {rp:#x} ({},{})-({},{}) mask {mask:#x} bytes/row {byte_cnt}", r.0, r.1, r.2, r.3))
}

/// The runs of set bits of the mask, row by row, as rectangles (RastPort coordinates).
fn runs(rows: &[Vec<u8>], (x0, y0, x1, _): (i32, i32, i32, i32)) -> Vec<Rect> {
    let mut out = Vec::new();
    for (dy, row) in rows.iter().enumerate() {
        let set = |x: i32| row.get((x >> 3) as usize).is_some_and(|b| b & (0x80 >> (x & 7)) != 0);
        let mut x = 0;
        while x0 + x <= x1 {
            if !set(x) { x += 1; continue; }
            let s = x;
            while x0 + x <= x1 && set(x) { x += 1; }
            out.push(Rect::new(x0 + s, y0 + dy as i32, x0 + x, y0 + dy as i32 + 1));
        }
    }
    out
}

/// Paint through the mask at guest address `mask` (0 = none) over the inclusive rectangle `r`.
pub fn paint_mask<U: GuestUi>(a: &mut Amiga<U>, rp: u32, mask: u32, r: (i32, i32, i32, i32), byte_cnt: i32) -> Result<(), Refusal> {
    let st = rastport::read(a, rp)?;
    let (x0, y0, x1, y1) = r;
    let area = Rect::from_inclusive(x0, y0, x1, y1);
    if area.is_empty() { return Ok(()); }
    let rows: Vec<Vec<u8>> = if mask == 0 { Vec::new() } else {
        (0..y1 - y0 + 1).map(|dy| (0..((x1 - x0) >> 3) + 1).map(|c| a.mem.read_u8(mask + (dy * byte_cnt + c) as u32).map_err(Refusal::mem("BltPattern mask"))).collect()).collect::<Result<_, _>>()?
    };
    let bit = |x: i32, y: i32| mask == 0 || rows.get((y - y0) as usize).is_some_and(|row| row.get(((x - x0) >> 3) as usize).is_some_and(|b| b & (0x80 >> ((x - x0) & 7)) != 0));
    let mode = st.mode;
    let pat = draw::pattern(a, &st)?;
    // The recorded form: a solid pen through a mask, in a plain state.
    let prim = (record::plain(&st) && pat.is_none() && st.mode & 4 == 0).then(|| Prim::Fills { rects: if mask == 0 { alloc::vec![area] } else { runs(&rows, r) }, pen: st.fg });
    target::paint_as(a, rp, "BltPattern", prim, |bm, clip, (ox, oy)| {
        let at = |x: i32, y: i32| (x + ox, y + oy); // bitmap -> RastPort coordinates
        let tr = area.translate(-ox, -oy);
        let keep = |x: i32, y: i32| { let (rx, ry) = at(x, y); bit(rx, ry) };
        if mode & (COMPLEMENT | JAM2) == (COMPLEMENT | JAM2) {
            let b = Brush { fg: st.fg, bg: st.bg, mode: JAM1 | COMPLEMENT, mask: st.mask };
            return paint::stencil(bm, tr, &b, clip, keep);
        }
        match &pat {
            None => {
                let b = Brush { fg: st.fg, bg: st.bg, mode: if mode & 3 == JAM2 { (mode & !3) | JAM1 } else { mode } & 7, mask: st.mask };
                paint::stencil(bm, tr, &b, clip, keep)
            }
            Some(p) => {
                let b = Brush { fg: st.fg, bg: st.bg, mode: mode & 7, mask: st.mask };
                let mut hit = Rect::default();
                for run in if mask == 0 { alloc::vec![area] } else { runs(&rows, r) } {
                    hit = hit.union(&paint::stencil(bm, run.translate(-ox, -oy), &b, clip, |x, y| { let (rx, ry) = at(x, y); p.bit(rx, ry) }));
                }
                hit
            }
        }
    })?;
    let _ = Clip::rect;
    Ok(())
}
