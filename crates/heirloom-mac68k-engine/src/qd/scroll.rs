//! `ScrollRect(dstRect, dh, dv, updateRgn)` (QuickDraw Bitmaps.a `ScrollRect`, the same arithmetic): the pixels inside
//! `dstRect` ∩ `visRgn` ∩ `clipRgn` (`srcRgn`) move by (`dh`, `dv`) to `dstRgn` = `srcRgn` offset; the area they vacate,
//! `srcRgn` − `dstRgn`, is painted with the background pattern and returned in `updateRgn`. Bits moved past `dstRect`'s edge
//! are lost. A hidden pen (`pnVis < 0`) or a zero offset changes nothing and sets `updateRgn` empty.
//! The move is exact in pixels and so is recorded as a pixel op; the erase is an ordinary background fill.
use super::draw::paint;
use super::port::*;
use super::target::{self, clip_region, current_port};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::blit::blit_within;
use heirloom_raster::paint::Clip;
use heirloom_raster::Region;

fn scroll_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (rect, dh, dv, upd) = (raster(read_rect(&m.mem, c.args.get(0))?), i32::from(c.args.get(1) as i16), i32::from(c.args.get(2) as i16), c.args.get(3));
    let port = current_port(m)?;
    super::picture::note_drawing(m, port);
    let log = format!("[quickdraw] ScrollRect {} by ({dh},{dv})", super::rect_str(&rect));
    if m.mm.block(upd).is_none() { return Err(Refusal(format!("ScrollRect: updateRgn {upd:#x} is not a region handle"))); }
    if m.mem.read_i16(port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis"))? < 0 || (dh == 0 && dv == 0) {
        m.write_region(upd, &Region::new())?;
        return Ok(Done { result: 0, log: format!("{log} (nothing moves: hidden pen or zero offset)") });
    }
    let mut vacated = Region::new();
    let mut origin = (0, 0);
    target::with(m, port, |m, t| {
        origin = t.local0;
        let mut src = Region::from_rect(t.to_pixels(rect));
        src.and_region(&clip_region(m, t)?);
        let mut dst = src.clone();
        dst.translate(dh, dv);
        vacated = src.clone();
        vacated.clear_region(&dst);
        let d = t.to_pixels(rect);
        let (inside, wide) = (|x: i32, y: i32| dst.contains(x, y), Clip::rect(t.bm.rect()));
        Ok(blit_within(&mut t.bm, d.x0, d.y0, d.x0 - dh, d.y0 - dv, d.width(), d.height(), super::copybits::minterm(0), 0xFF, Some(&inside), &wide))
    })?;
    vacated.translate(origin.0, origin.1);
    m.write_region(upd, &vacated)?;
    let pat = read_port_pat(&m.mem, port, PatSlot::Bk)?;
    paint(m, port, &vacated, PAT_COPY, &pat)?;
    Ok(Done { result: 0, log })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    super::reg(m, 0xA8EF, "ScrollRect", &[Arg::L, Arg::W, Arg::W, Arg::L], None, scroll_rect::<U>)
}
