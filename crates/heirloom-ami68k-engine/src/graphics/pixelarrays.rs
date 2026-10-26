//! graphics.library byte-per-pixel transfers: ReadPixelLine8 / WritePixelLine8 / ReadPixelArray8 / WritePixelArray8.
//!
//! ReadPixelLine8(a0 = rp, d0 = x, d1 = y, d2 = width, a2 = array, a1 = temp rp) -> pixels read; WritePixelLine8 the same
//! in the other direction. The Array8 forms take a rectangle (d0, d1)-(d2, d3) inclusive and a row-major array of
//! (xstop-xstart+1) bytes per row. A pixel outside the bitmap or the layer clip reads as 0 (AROS reads 0 outside) and is
//! skipped on write; the temp RastPort is not needed (the engine converts planar to chunky itself) and is not touched.
//! Writing uses the pen number directly (no draw mode, no mask), as the autodoc says.
use alloc::format;
use alloc::string::String;
use super::target;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::Rect;

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("ReadPixelLine8", |a, c| line(a, c, false)).named("WritePixelLine8", |a, c| line(a, c, true))
        .named("ReadPixelArray8", |a, c| array(a, c, false)).named("WritePixelArray8", |a, c| array(a, c, true))
}

fn transfer<U: GuestUi>(a: &mut Amiga<U>, rp: u32, (x0, y0): (i32, i32), (w, h): (i32, i32), buf: u32, write: bool, what: &str) -> Result<u32, Refusal> {
    if w <= 0 || h <= 0 { return Ok(0); }
    let n = (w * h) as u32;
    let mut data = alloc::vec![0u8; n as usize];
    if write { for (i, d) in data.iter_mut().enumerate() { *d = a.mem.read_u8(buf + i as u32).map_err(Refusal::mem("pixel array"))?; } }
    let mut out = alloc::vec![0u8; n as usize];
    target::paint(a, rp, what, |bm, clip, (ox, oy)| {
        let mut touched = Rect::default();
        for j in 0..h {
            for i in 0..w {
                let (px, py) = (x0 + i - ox, y0 + j - oy);
                let k = (j * w + i) as usize;
                if !clip.allows(px, py) || bm.get(px, py).is_none() { continue; }
                if write { bm.set(px, py, data[k]); touched = touched.union(&Rect::new(px, py, px + 1, py + 1)); }
                else { out[k] = bm.get(px, py).unwrap_or(0); }
            }
        }
        touched
    })?;
    if !write { for (i, d) in out.iter().enumerate() { a.mem.write_u8(buf + i as u32, *d).map_err(Refusal::mem("pixel array"))?; } }
    Ok(n)
}

fn line<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, write: bool) -> Result<String, Refusal> {
    let (rp, x, y, w, buf) = (cpu.areg(0), cpu.dreg(0) as i16 as i32, cpu.dreg(1) as i16 as i32, cpu.dreg(2) as i16 as i32, cpu.areg(2)); // width is a WORD (AROS readpixelline8.c): the upper half of D2 is the caller's garbage
    let n = transfer(a, rp, (x, y), (w, 1), buf, write, if write { "WritePixelLine8" } else { "ReadPixelLine8" })?;
    cpu.set_d0(n);
    Ok(format!("[graphics] {}PixelLine8 {rp:#x} {x},{y} width {w} -> {n}", if write { "Write" } else { "Read" }))
}

fn array<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, write: bool) -> Result<String, Refusal> {
    let (rp, x0, y0, x1, y1, buf) = (cpu.areg(0), cpu.dreg(0) as i16 as i32, cpu.dreg(1) as i16 as i32, cpu.dreg(2) as i16 as i32, cpu.dreg(3) as i16 as i32, cpu.areg(2));
    let n = transfer(a, rp, (x0, y0), (x1 - x0 + 1, y1 - y0 + 1), buf, write, if write { "WritePixelArray8" } else { "ReadPixelArray8" })?;
    cpu.set_d0(n);
    Ok(format!("[graphics] {}PixelArray8 {rp:#x} ({x0},{y0})-({x1},{y1}) -> {n}", if write { "Write" } else { "Read" }))
}
