//! `ColorTable` / `CTabHandle` and the Color Manager (Inside Macintosh: Imaging With QuickDraw, "Color Manager" and
//! "Color QuickDraw Reference").
//!
//! ColorTable: `ctSeed` (long) 0, `ctFlags` (word) 4, `ctSize` (word, ENTRIES - 1) 6, then `ctTable`, an array of 8-byte
//! `ColorSpec` = `value` (word) + `rgb` (RGBColor). In a device's table (`ctFlags` bit 15 set) `value` is the entry's
//! index; in any other table it is the pixel value the entry stands for. The seed of a standard table is its `'clut'` id
//! (1, 4, 8 and the grays 33, 34, 36, 40); `GetCTSeed` hands out unique ones above 1023.
//!
//! `Index2Color(index, VAR color)` and `Color2Index(color): LONGINT` work on the CURRENT DEVICE's table (`TheGDevice` ->
//! `gdPMap` -> `pmTable`). `Color2Index` is the exact entry if there is one, else the nearest ([`super::color::nearest`]).
//! An index outside the table is a named refusal (the Color Manager's reply is undocumented).
use super::color::{self, read_rgb, write_rgb, Rgb};
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const CT_SEED: u32 = 0;
pub const CT_FLAGS: u32 = 4;
pub const CT_SIZE: u32 = 6;
pub const CT_TABLE: u32 = 8;
pub const SPEC: u32 = 8;
/// `ctFlags` bit 15: a device's color table.
pub const DEVICE_TABLE: u16 = 0x8000;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use crate::qd::reg;
    reg(m, 0xAA18, "GetCTable", &[Arg::W], Some(Arg::L), get_ctable::<U>)?;
    reg(m, 0xAA24, "DisposeCTable", &[Arg::L], None, dispose_ctable::<U>)?;
    reg(m, 0xAA3F, "SetEntries", &[Arg::W, Arg::W, Arg::P], None, set_entries::<U>)?;
    reg(m, 0xAA28, "GetCTSeed", &[], Some(Arg::L), get_ct_seed::<U>)?;
    reg(m, 0xAA33, "Color2Index", &[Arg::P], Some(Arg::L), color2index::<U>)?;
    reg(m, 0xAA34, "Index2Color", &[Arg::L, Arg::P], None, index2color::<U>)?;
    reg(m, 0xAA35, "InvertColor", &[Arg::P], None, invert_color::<U>)?;
    reg(m, 0xAA36, "RealColor", &[Arg::P], Some(Arg::B), real_color::<U>)?;
    reg(m, 0xAA40, "QDError", &[], Some(Arg::W), qd_error::<U>)
}

impl<U: GuestUi> Mac<U> {
    /// A new `CTabHandle` holding `colors` (entry `i` has `value` i).
    pub fn new_ctab(&mut self, colors: &[Rgb], seed: u32, flags: u16) -> Result<u32, Refusal> {
        let (h, p) = self.alloc_handle(CT_TABLE + SPEC * colors.len() as u32, "ColorTable")?;
        let e = Refusal::mem("ColorTable");
        self.mem.write_u32(p + CT_SEED, seed).map_err(&e)?;
        self.mem.write_u16(p + CT_FLAGS, flags).map_err(&e)?;
        self.mem.write_u16(p + CT_SIZE, (colors.len() - 1) as u16).map_err(&e)?;
        for (i, c) in colors.iter().enumerate() {
            let at = p + CT_TABLE + SPEC * i as u32;
            self.mem.write_u16(at, i as u16).map_err(&e)?;
            write_rgb(&mut self.mem, at + 2, *c)?;
        }
        Ok(h)
    }

    /// The colours of a color table by pixel value (a device table by position), 256 entries at most.
    pub fn read_ctab(&self, h: u32) -> Result<Vec<Rgb>, Refusal> {
        let p = self.deref_handle(h, "CTabHandle")?;
        let e = Refusal::mem("ColorTable");
        let (flags, n) = (self.mem.read_u16(p + CT_FLAGS).map_err(&e)?, usize::from(self.mem.read_u16(p + CT_SIZE).map_err(&e)?) + 1);
        if n > 256 { return Err(Refusal(format!("CTabHandle {h:#x}: {n} entries (an indexed table holds at most 256)"))); }
        let mut t = alloc::vec![Rgb::BLACK; n];
        for i in 0..n {
            let at = p + CT_TABLE + SPEC * i as u32;
            let value = if flags & DEVICE_TABLE != 0 { i } else { usize::from(self.mem.read_u16(at).map_err(&e)?) };
            let c = read_rgb(&self.mem, at + 2)?;
            if value < n { t[value] = c; } else { return Err(Refusal(format!("CTabHandle {h:#x}: entry {i} has value {value}, outside its {n}-entry table"))); }
        }
        Ok(t)
    }

    /// The table the current device (`TheGDevice`) uses.
    pub fn device_clut(&self) -> Result<Vec<Rgb>, Refusal> {
        let gd = self.the_gdevice()?;
        let pm = self.deref_handle(self.mem.read_u32(self.deref_handle(gd, "GDHandle")? + super::gdevice::GD_PMAP).map_err(Refusal::mem("GDevice"))?, "gdPMap")?;
        self.read_ctab(self.mem.read_u32(pm + super::pixmap::PM_TABLE).map_err(Refusal::mem("PixMap"))?)
    }
}

/// `GetCTable(ctID)`: an app's own `'clut'` resource of that id wins; ids 1 (black and white), 4, 8 and the grays 33, 34,
/// 36, 40 are the System's standard tables. Anything else is NIL with `QDError` = resNotFound (-192).
fn get_ctable<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    if let Ok(r) = m.get_resource(*b"clut", id, false) {
        let bytes = m.handle_bytes(r).unwrap_or_default();
        if bytes.len() < (CT_TABLE as usize) { return Err(Refusal(format!("GetCTable {id}: the 'clut' holds {} bytes, less than a ColorTable header", bytes.len()))); }
        let (h, p) = m.alloc_handle(bytes.len() as u32, "GetCTable")?;
        m.mem.write_bytes(p, &bytes).map_err(Refusal::mem("ColorTable"))?;
        m.cqd.qd_error = 0;
        return Ok(Done { result: h, log: format!("[quickdraw] GetCTable {id}: the application's 'clut' -> {h:#x}") });
    }
    let table = match id { 1 => Some(color::clut1()), 4 => Some(color::clut4()), 8 => Some(color::clut8()), 33 | 34 | 36 | 40 => Some(color::gray((id - 32) as u8)), _ => None };
    let Some(t) = table else {
        m.cqd.qd_error = -192;
        return Ok(Done { result: 0, log: format!("[quickdraw] GetCTable {id}: no such 'clut' (NIL, QDError -192)") });
    };
    let h = m.new_ctab(&t, id as u32, 0)?;
    m.cqd.qd_error = 0;
    Ok(Done { result: h, log: format!("[quickdraw] GetCTable {id}: the standard {}-entry table -> {h:#x}", t.len()) })
}

fn dispose_ctable<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.deref_handle(h, "DisposeCTable")?;
    m.dispose_handle(h).map_err(|e| Refusal(format!("DisposeCTable {h:#x}: memory manager error {e}")))?;
    Ok(Done { result: 0, log: format!("[quickdraw] DisposeCTable {h:#x}") })
}

/// `PROCEDURE SetEntries(start, count: INTEGER; aTable: CSpecArray)` ($AA3F): write `count + 1` colour specs into the CURRENT DEVICE's
/// table (Inside Macintosh: Imaging With QuickDraw, "Color Manager"). With `start` >= 0 the specs fill entries `start ..= start + count` in
/// order and their `value` words are ignored; with `start` = -1 each spec's `value` is the entry it sets. The table gets a new seed, and
/// every colour window sharing it is shown again with the new colours (qd `refresh_palettes`). An entry past the table is a named refusal
/// (the Color Manager's reply is undocumented); the entries before it are written.
fn set_entries<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (start, count, specs) = (c.args.get(0) as u16 as i16, c.args.get(1) as u16 as i16, c.args.get(2));
    if count < 0 { return Err(Refusal(format!("SetEntries: count {count} is negative"))); }
    let gd = m.the_gdevice()?;
    let pm = m.deref_handle(m.mem.read_u32(m.deref_handle(gd, "GDHandle")? + super::gdevice::GD_PMAP).map_err(Refusal::mem("GDevice"))?, "gdPMap")?;
    let ct = m.mem.read_u32(pm + super::pixmap::PM_TABLE).map_err(Refusal::mem("PixMap"))?;
    let tp = m.deref_handle(ct, "the device's colour table")?;
    let e = Refusal::mem("ColorSpec");
    let entries = u32::from(m.mem.read_u16(tp + CT_SIZE).map_err(&e)?) + 1;
    for i in 0..=count as u32 {
        let spec = specs.wrapping_add(SPEC * i);
        let index = if start < 0 { u32::from(m.mem.read_u16(spec).map_err(&e)?) } else { start as u32 + i };
        if index >= entries { return Err(Refusal(format!("SetEntries: entry {index} is past the device's {entries}-entry table"))); }
        let rgb = read_rgb(&m.mem, spec + 2)?;
        write_rgb(&mut m.mem, tp + CT_TABLE + SPEC * index + 2, rgb)?;
    }
    let seed = m.unique_seed();
    m.mem.write_u32(tp + CT_SEED, seed).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[quickdraw] SetEntries start {start} count {count}: {} colour(s) set in the device table", count + 1) })
}

fn get_ct_seed<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let s = m.unique_seed();
    Ok(Done { result: s, log: format!("[quickdraw] GetCTSeed -> {s}") })
}

fn color2index<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rgb = read_rgb(&m.mem, c.args.get(0))?;
    let i = color::nearest(&m.device_clut()?, rgb);
    Ok(Done { result: u32::from(i), log: format!("[quickdraw] Color2Index ({:04x},{:04x},{:04x}) -> {i}", rgb.r, rgb.g, rgb.b) })
}

fn index2color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (index, out) = (c.args.get(0), c.args.get(1));
    let t = m.device_clut()?;
    let rgb = *t.get(index as usize).ok_or_else(|| Refusal(format!("Index2Color: index {index} is outside the current device's {}-entry table", t.len())))?;
    write_rgb(&mut m.mem, out, rgb)?;
    Ok(Done { result: 0, log: format!("[quickdraw] Index2Color {index} -> ({:04x},{:04x},{:04x})", rgb.r, rgb.g, rgb.b) })
}

fn invert_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = c.args.get(0);
    let rgb = read_rgb(&m.mem, at)?.inverted();
    write_rgb(&mut m.mem, at, rgb)?;
    Ok(Done { result: 0, log: format!("[quickdraw] InvertColor -> ({:04x},{:04x},{:04x})", rgb.r, rgb.g, rgb.b) })
}

/// `RealColor(color): BOOLEAN`: TRUE when the colour is exactly an entry of the current device's table.
fn real_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let rgb = read_rgb(&m.mem, c.args.get(0))?;
    let real = m.device_clut()?.contains(&rgb);
    Ok(Done { result: u32::from(real), log: format!("[quickdraw] RealColor ({:04x},{:04x},{:04x}) -> {real}", rgb.r, rgb.g, rgb.b) })
}

fn qd_error<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: m.cqd.qd_error as u16 as u32, log: format!("[quickdraw] QDError -> {}", m.cqd.qd_error) })
}
