//! graphics.library Flood(a1 = RastPort, d2 = mode, d0 = x, d1 = y) -> TRUE/FALSE (AROS rom/graphics/flood.c semantics).
//!
//! Mode 0 (outline): fill outward from (x, y) over every 4-connected pixel that is NOT the outline pen (rp_AOlPen); mode 1
//! (colour): fill every 4-connected pixel that has the colour found at (x, y). The fill uses the foreground pen (draw mode as
//! set), stays inside the layer clip and the bitmap, and needs a TmpRas on the RastPort (without one Flood answers FALSE, as on
//! AmigaOS). The scratch raster itself is not used: the fill is computed on the engine's own bitmap.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::{layout::*, rastport, target};
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;
use retro_raster::{paint, Rect};

pub fn flood<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, mode, x, y) = (cpu.areg(1), cpu.dreg(2), cpu.dreg(0) as i16 as i32, cpu.dreg(1) as i16 as i32);
    if a.mem.read_u32(rp + RP_TMP_RAS).map_err(Refusal::mem("RastPort"))? == 0 { cpu.set_d0(0); return Ok(format!("[graphics] Flood {rp:#x}: no TmpRas -> FALSE")); }
    if mode > 1 { return Err(Refusal(format!("Flood: mode {mode} is neither 0 (outline) nor 1 (colour)"))); }
    let st = rastport::read(a, rp)?;
    let mut b = st.brush();
    b.mode = 0; // JAM1: the fill is the foreground colour
    let aol = st.aol;
    let r = target::paint(a, rp, "Flood", |bm, clip, (ox, oy)| {
        let (sx, sy) = (x - ox, y - oy);
        let bounds = bm.rect();
        if !clip.allows(sx, sy) || bm.get(sx, sy).is_none() { return Rect::default(); }
        let seed = bm.get(sx, sy).unwrap_or(0);
        let want = |v: u8| if mode == 0 { v != aol } else { v == seed };
        let mut touched = Rect::default();
        let mut seen = alloc::collections::BTreeSet::new();
        let mut stack: Vec<(i32, i32)> = alloc::vec![(sx, sy)];
        while let Some((px, py)) = stack.pop() {
            if px < bounds.x0 || py < bounds.y0 || px >= bounds.x1 || py >= bounds.y1 || !clip.allows(px, py) || !seen.insert((px, py)) { continue; }
            if !bm.get(px, py).map_or(false, want) { continue; }
            touched = touched.union(&paint::plot(bm, px, py, &b, clip));
            stack.extend([(px + 1, py), (px - 1, py), (px, py + 1), (px, py - 1)]);
        }
        touched
    })?;
    cpu.set_d0(1);
    Ok(format!("[graphics] Flood {rp:#x} mode {mode} at {x},{y} -> TRUE (damage {r:?})"))
}
