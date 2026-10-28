//! Regions: real guest `Region` records behind real handles (`rgnSize` 2, `rgnBBox` 8, then data), with the pixel set
//! kept in `heirloom_raster::Region` while an operation runs.
//!
//! Format (Inside Macintosh "QuickDraw"): a rectangular or empty region is exactly 10 bytes (`rgnSize = 10`, the
//! bounding box; empty = all-zero box). Any other region follows the box with its scan lines: for each vertical
//! coordinate where the shape changes, `v` then the horizontal INVERSION POINTS of that line in increasing order, each
//! line ended by `$7FFF`, and the whole list ended by a final `$7FFF`. A point toggles "inside"; the list for line `v`
//! holds the points that differ from the line above it ([`pack`] / [`unpack`]).
use super::port::{raster, P_CLIP, P_PN_VIS, P_RGN_SAVE};
use super::target::current_port;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{read_rect, Arg};
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;
use heirloom_raster::{Rect, Region};

const END: u16 = 0x7FFF;
/// The "wide open" clip region: the largest rectangle an INTEGER pair can name.
pub const WIDE_OPEN: Rect = Rect::new(-32767, -32767, 32767, 32767);

/// Region -> record bytes.
pub fn pack(g: &Region) -> Vec<u8> {
    let b = g.bounds();
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, v: i32| out.extend_from_slice(&(v as i16).to_be_bytes());
    let spans_at = |y: i32| -> Vec<i32> { // endpoints of the spans covering row y
        let mut v: Vec<i32> = g.rects().iter().filter(|r| r.y0 <= y && y < r.y1).flat_map(|r| [r.x0, r.x1]).collect();
        v.sort_unstable();
        v
    };
    let rect_like = g.rects().len() <= 1;
    put(&mut out, 0); // rgnSize patched below
    for v in [b.y0, b.x0, b.y1, b.x1] { put(&mut out, v); }
    if !rect_like {
        let mut ys: Vec<i32> = g.rects().iter().flat_map(|r| [r.y0, r.y1]).collect();
        ys.sort_unstable();
        ys.dedup();
        let mut prev: Vec<i32> = Vec::new();
        for y in ys {
            let cur = spans_at(y);
            let diff: Vec<i32> = cur.iter().filter(|x| !prev.contains(x)).chain(prev.iter().filter(|x| !cur.contains(x))).copied().collect();
            if !diff.is_empty() {
                let mut d = diff;
                d.sort_unstable();
                put(&mut out, y);
                for x in d { put(&mut out, x); }
                put(&mut out, i32::from(END));
            }
            prev = cur;
        }
        put(&mut out, i32::from(END));
    }
    let n = out.len() as u16;
    out[0..2].copy_from_slice(&n.to_be_bytes());
    out
}

/// Record bytes -> Region, or the reason the record is malformed.
pub fn unpack(b: &[u8]) -> Result<Region, &'static str> {
    if b.len() < 10 { return Err("a Region is at least 10 bytes (rgnSize, rgnBBox)"); }
    let w = |i: usize| -> Option<i16> { b.get(i..i + 2).map(|s| i16::from_be_bytes([s[0], s[1]])) };
    if usize::from(w(0).unwrap_or(0) as u16) != b.len() { return Err("rgnSize does not match the record"); }
    let (top, left, bottom, right) = (i32::from(w(2).unwrap_or(0)), i32::from(w(4).unwrap_or(0)), i32::from(w(6).unwrap_or(0)), i32::from(w(8).unwrap_or(0)));
    let mut g = Region::new();
    if b.len() == 10 { g.or_rect(Rect::new(left, top, right, bottom)); return Ok(g); }
    let (mut i, mut inside): (usize, Vec<i32>) = (10, Vec::new());
    let mut band_start: Option<i32> = None;
    let close = |g: &mut Region, inside: &Vec<i32>, from: Option<i32>, to: i32| {
        if let Some(f) = from { for p in inside.chunks(2) { if let [a, c] = *p { g.or_rect(Rect::new(a, f, c, to)); } } }
    };
    loop {
        let v = w(i).ok_or("region data ends without its $7FFF")?;
        i += 2;
        if v == END as i16 { close(&mut g, &inside, band_start, bottom); break; }
        close(&mut g, &inside, band_start, i32::from(v));
        let y = i32::from(v);
        let mut pts: Vec<i32> = Vec::new();
        loop {
            let x = w(i).ok_or("scan line ends without its $7FFF")?;
            i += 2;
            if x == END as i16 { break; }
            pts.push(i32::from(x));
        }
        for p in pts { match inside.iter().position(|&q| q == p) { Some(k) => { inside.remove(k); } None => inside.push(p) } }
        inside.sort_unstable();
        band_start = Some(y);
    }
    Ok(g)
}

impl<U: GuestUi> Mac<U> {
    /// The pixel set of region handle `h`.
    pub fn read_region(&self, h: u32) -> Result<Region, Refusal> {
        let blk = self.mm.block(h).ok_or_else(|| Refusal(format!("{h:#x} is not a region handle the Memory Manager owns")))?;
        let bytes = self.mem.bytes(self.mem.span(blk.ptr, blk.size).map_err(Refusal::mem("Region"))?).map_err(Refusal::mem("Region"))?;
        unpack(bytes).map_err(|why| Refusal(format!("region {h:#x}: {why}")))
    }

    /// Store `g` into region handle `h`, resizing the handle to the record.
    pub fn write_region(&mut self, h: u32, g: &Region) -> Result<(), Refusal> {
        let bytes = pack(g);
        self.mm.set_handle_size(&mut self.mem, &mut self.heap, h, bytes.len() as u32).map_err(|e| Refusal(format!("region {h:#x}: SetHandleSize failed (memFullErr {e})")))?;
        let ptr = self.mm.block(h).ok_or_else(|| Refusal(format!("{h:#x} lost its block")))?.ptr;
        self.mem.write_bytes(ptr, &bytes).map_err(Refusal::mem("Region"))
    }

    pub fn new_region(&mut self, g: &Region) -> Result<u32, Refusal> {
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, 10, true).map_err(|e| Refusal(format!("NewRgn: memFullErr {e}")))?;
        self.write_region(h, g)?;
        Ok(h)
    }
}

fn done(log: &str) -> Result<Done, Refusal> { Ok(Done { result: 0, log: format!("[quickdraw] {log}") }) }
fn flag(name: &str, b: bool) -> Result<Done, Refusal> { Ok(Done { result: u32::from(b), log: format!("[quickdraw] {name} -> {b}") }) }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use super::reg;
    reg(m, 0xA8D8, "NewRgn", &[], Some(Arg::L), new_rgn::<U>)?;
    reg(m, 0xA8D9, "DisposeRgn", &[Arg::L], None, dispose_rgn::<U>)?;
    reg(m, 0xA8DA, "OpenRgn", &[], None, open_rgn::<U>)?;
    reg(m, 0xA8DB, "CloseRgn", &[Arg::L], None, close_rgn::<U>)?;
    reg(m, 0xA8DC, "CopyRgn", &[Arg::L, Arg::L], None, copy_rgn::<U>)?;
    reg(m, 0xA8DD, "SetEmptyRgn", &[Arg::L], None, set_empty_rgn::<U>)?;
    reg(m, 0xA8DE, "SetRecRgn", &[Arg::L, Arg::W, Arg::W, Arg::W, Arg::W], None, set_rect_rgn::<U>)?;
    reg(m, 0xA8DF, "RectRgn", &[Arg::L, Arg::L], None, rect_rgn::<U>)?;
    reg(m, 0xA8E0, "OffsetRgn", &[Arg::L, Arg::W, Arg::W], None, offset_rgn::<U>)?;
    reg(m, 0xA8E1, "InsetRgn", &[Arg::L, Arg::W, Arg::W], None, inset_rgn::<U>)?;
    reg(m, 0xA8E4, "SectRgn", &[Arg::L, Arg::L, Arg::L], None, sect_rgn::<U>)?;
    reg(m, 0xA8E5, "UnionRgn", &[Arg::L, Arg::L, Arg::L], None, union_rgn::<U>)?;
    reg(m, 0xA8E6, "DiffRgn", &[Arg::L, Arg::L, Arg::L], None, diff_rgn::<U>)?;
    reg(m, 0xA8E7, "XOrRgn", &[Arg::L, Arg::L, Arg::L], None, xor_rgn::<U>)?;
    reg(m, 0xA8E8, "PtInRgn", &[Arg::L, Arg::L], Some(Arg::B), pt_in_rgn::<U>)?;
    reg(m, 0xA8E9, "RectInRgn", &[Arg::L, Arg::L], Some(Arg::B), rect_in_rgn::<U>)?;
    reg(m, 0xA8E3, "EqualRgn", &[Arg::L, Arg::L], Some(Arg::B), equal_rgn::<U>)?;
    reg(m, 0xA8E2, "EmptyRgn", &[Arg::L], Some(Arg::B), empty_rgn::<U>)?;
    reg(m, 0xA87B, "ClipRect", &[Arg::L], None, clip_rect::<U>)?;
    reg(m, 0xA879, "SetClip", &[Arg::L], None, set_clip::<U>)?;
    reg(m, 0xA87A, "GetClip", &[Arg::L], None, get_clip::<U>)
}

fn new_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let h = m.new_region(&Region::new())?;
    Ok(Done { result: h, log: format!("[quickdraw] NewRgn -> {h:#x}") })
}
fn dispose_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.read_region(h)?; // a non-region handle is refused by name, not freed
    m.dispose_handle(h).map_err(|e| Refusal(format!("DisposeRgn {h:#x}: memory manager error {e}")))?;
    done(&format!("DisposeRgn {h:#x}"))
}
fn open_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    if m.qd.rgn_open.contains_key(&port) { return Err(Refusal("OpenRgn: a region is already open on this port (nested OpenRgn is not supported)".into())); }
    let h = m.new_region(&Region::new())?;
    m.mem.write_u32(port.wrapping_add(P_RGN_SAVE), h).map_err(Refusal::mem("rgnSave"))?;
    let vis = m.mem.read_i16(port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis"))?; // OpenRgn hides the pen
    m.mem.write_u16(port.wrapping_add(P_PN_VIS), vis.wrapping_sub(1) as u16).map_err(Refusal::mem("pnVis"))?;
    m.qd.rgn_open.insert(port, Region::new());
    done(&format!("OpenRgn on {port:#x}: rgnSave {h:#x}"))
}
fn close_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let port = current_port(m)?;
    let g = m.qd.rgn_open.remove(&port).ok_or_else(|| Refusal("CloseRgn: no region is open on the current port".into()))?;
    let (dst, save) = (c.args.get(0), m.mem.read_u32(port.wrapping_add(P_RGN_SAVE)).map_err(Refusal::mem("rgnSave"))?);
    m.write_region(dst, &g)?;
    if save != 0 && save != dst { m.dispose_handle(save).map_err(|e| Refusal(format!("CloseRgn: freeing the scratch region failed ({e})")))?; }
    m.mem.write_u32(port.wrapping_add(P_RGN_SAVE), 0).map_err(Refusal::mem("rgnSave"))?;
    let vis = m.mem.read_i16(port.wrapping_add(P_PN_VIS)).map_err(Refusal::mem("pnVis"))?;
    m.mem.write_u16(port.wrapping_add(P_PN_VIS), vis.wrapping_add(1) as u16).map_err(Refusal::mem("pnVis"))?;
    done(&format!("CloseRgn -> {dst:#x} ({} rect(s))", g.rects().len()))
}
fn copy_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let g = m.read_region(c.args.get(0))?;
    m.write_region(c.args.get(1), &g)?;
    done("CopyRgn")
}
fn set_empty_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.read_region(c.args.get(0))?;
    m.write_region(c.args.get(0), &Region::new())?;
    done("SetEmptyRgn")
}
fn set_rect_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let a = &c.args;
    m.read_region(a.get(0))?;
    let r = Rect::new(i32::from(a.get(1) as i16), i32::from(a.get(2) as i16), i32::from(a.get(3) as i16), i32::from(a.get(4) as i16));
    m.write_region(a.get(0), &Region::from_rect(r))?; // an empty rect makes an empty region
    done("SetRectRgn")
}
fn rect_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.read_region(c.args.get(0))?;
    let r = raster(read_rect(&m.mem, c.args.get(1))?);
    m.write_region(c.args.get(0), &Region::from_rect(r))?;
    done("RectRgn")
}
fn offset_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let mut g = m.read_region(c.args.get(0))?;
    g.translate(i32::from(c.args.get(1) as i16), i32::from(c.args.get(2) as i16));
    m.write_region(c.args.get(0), &g)?;
    done("OffsetRgn")
}
fn inset_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let g = m.read_region(c.args.get(0))?;
    let (dh, dv) = (i32::from(c.args.get(1) as i16), i32::from(c.args.get(2) as i16));
    m.write_region(c.args.get(0), &super::inset::inset_region(&g, dh, dv))?;
    done("InsetRgn")
}

fn binary<U: GuestUi>(m: &mut Mac<U>, c: &Call, name: &str, op: fn(&mut Region, &Region)) -> Result<Done, Refusal> {
    let (mut a, b) = (m.read_region(c.args.get(0))?, m.read_region(c.args.get(1))?);
    op(&mut a, &b);
    m.write_region(c.args.get(2), &a)?;
    done(name)
}
fn sect_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { binary(m, c, "SectRgn", Region::and_region) }
fn union_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { binary(m, c, "UnionRgn", Region::or_region) }
fn diff_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { binary(m, c, "DiffRgn", Region::clear_region) }
fn xor_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { binary(m, c, "XorRgn", Region::xor_region) }

fn pt_in_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (v, h) = ((c.args.get(0) >> 16) as i16, c.args.get(0) as i16);
    flag("PtInRgn", m.read_region(c.args.get(1))?.contains(i32::from(h), i32::from(v)))
}
fn rect_in_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let r = raster(read_rect(&m.mem, c.args.get(0))?);
    let mut g = m.read_region(c.args.get(1))?;
    g.and_rect(r);
    flag("RectInRgn", !g.is_empty())
}
fn equal_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    flag("EqualRgn", m.read_region(c.args.get(0))? == m.read_region(c.args.get(1))?)
}
fn empty_rgn<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { flag("EmptyRgn", m.read_region(c.args.get(0))?.is_empty()) }

fn port_rgn<U: GuestUi>(m: &mut Mac<U>, off: u32, what: &str) -> Result<u32, Refusal> {
    let port = current_port(m)?;
    let h = m.mem.read_u32(port.wrapping_add(off)).map_err(Refusal::mem("GrafPort"))?;
    if h == 0 { Err(Refusal(format!("{what}: the current port has no region there (port {port:#x} was not initialised by OpenPort/NewWindow)"))) } else { Ok(h) }
}
fn clip_rect<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = port_rgn(m, P_CLIP, "ClipRect")?;
    let r = raster(read_rect(&m.mem, c.args.get(0))?);
    m.write_region(h, &Region::from_rect(r))?;
    done("ClipRect")
}
fn set_clip<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = port_rgn(m, P_CLIP, "SetClip")?;
    let g = m.read_region(c.args.get(0))?;
    m.write_region(h, &g)?;
    done("SetClip")
}
fn get_clip<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = port_rgn(m, P_CLIP, "GetClip")?;
    let g = m.read_region(h)?;
    m.write_region(c.args.get(0), &g)?;
    done("GetClip")
}

