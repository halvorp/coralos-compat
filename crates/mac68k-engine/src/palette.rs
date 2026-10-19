//! The Palette Manager (Inside Macintosh: Advanced Color Imaging, "Palette Manager"), the part that does not need a colour
//! display. This machine's QuickDraw is one bit deep, so there is no colour lookup table to load: a palette is an honest,
//! real data structure the program can build, read and change, it is associated with a window, and activating it changes
//! nothing on the screen (a 1-bit port renders every colour as black or white anyway).
//!
//! **The record** is the documented one, in guest memory: `pmEntries` (2), 14 bytes of private fields, then per entry a
//! `ColorInfo` of 16 bytes: `ciRGB` (6), `ciUsage` (2), `ciTolerance` (2), 6 private bytes (the size is read off a corpus
//! application's own `'pltt'`: 272 bytes for 16 entries). A `'pltt'` resource has the same
//! layout, so `GetNewPalette` is a copy. `NewPalette` fills the entries from a `ColorTable` when one is given (a `ColorSpec` is a
//! 2-byte value then an RGBColor; the table header is `ctSeed` 4, `ctFlags` 2, `ctSize` 2 = entries - 1).
//!
//! **Not modelled, by name:** `AnimateEntry` / `AnimatePalette` (no hardware table to animate), `CTab2Palette`, `Palette2CTab`,
//! `CopyPalette`, and the dispatch `PaletteDispatch` selectors: the census shows each as the next missing trap. `PmForeColor`
//! and `PmBackColor` are accepted and logged (a 1-bit port draws the black/white nearest to the entry anyway).
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const HEADER: u32 = 16;
const ENTRY: u32 = 16;

/// Palettes associated with windows (the real Manager keeps them in the window's auxiliary record).
#[derive(Default)]
pub struct Palettes { pub by_window: BTreeMap<u32, u32> }

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    use Arg::{L, W};
    m.register(0xAA90, &[], None, |_, _, c| ok(c, "colour tables are not loaded: this display is 1 bit deep"))?;
    m.register(0xAA91, &[W, L, W, W], Some(L), new_palette::<U>)?;
    m.register(0xAA92, &[W], Some(L), get_new_palette::<U>)?;
    m.register(0xAA93, &[L], None, dispose_palette::<U>)?;
    m.register(0xAA94, &[L], None, |_, _, c| ok(c, "nothing to install: a 1-bit port has no colour table"))?;
    m.register(0xAA95, &[L, L, W], None, set_palette::<U>)?;
    m.register(0xAA96, &[L], Some(L), get_palette::<U>)?;
    m.register(0xAA97, &[W], None, |_, _, c| ok(c, "a 1-bit port draws the nearest of black and white"))?;
    m.register(0xAA98, &[W], None, |_, _, c| ok(c, "a 1-bit port draws the nearest of black and white"))?;
    m.register(0xAA9B, &[L, W, L], None, get_entry_color::<U>)?;
    m.register(0xAA9C, &[L, W, L], None, set_entry_color::<U>)?;
    m.register(0xAA9D, &[L, W, L, L], None, get_entry_usage::<U>)?;
    m.register(0xAA9E, &[L, W, W, W], None, set_entry_usage::<U>)
}

fn ok(c: &Call, why: &str) -> Result<Done, Refusal> { Ok(Done { result: 0, log: format!("[palette] {} ({why})", c.name) }) }

impl<U: GuestUi> Mac<U> {
    /// A palette handle's base address and entry count.
    fn palette_at(&self, h: u32, what: &str) -> Result<(u32, u16), Refusal> {
        let b = self.mm.block(h).ok_or_else(|| Refusal(format!("{what}: {h:#x} is not a PaletteHandle")))?;
        let n = self.mem.read_u16(b.ptr).map_err(Refusal::mem("PaletteRecord"))?;
        if b.size < HEADER + u32::from(n) * ENTRY { return Err(Refusal(format!("{what}: {h:#x} holds {} bytes, too few for its {n} entries", b.size))); }
        Ok((b.ptr, n))
    }

    fn entry_at(&self, h: u32, entry: u16, what: &str) -> Result<u32, Refusal> {
        let (base, n) = self.palette_at(h, what)?;
        if entry >= n { return Err(Refusal(format!("{what}: entry {entry} is outside the palette's {n}"))); }
        Ok(base + HEADER + u32::from(entry) * ENTRY)
    }

    fn make_palette(&mut self, entries: u16) -> Result<(u32, u32), Refusal> {
        let size = HEADER + u32::from(entries) * ENTRY;
        let h = self.mm.new_handle(&mut self.mem, &mut self.heap, size, true).map_err(|e| Refusal(format!("NewPalette: no memory for {entries} entries (memFullErr {e})")))?;
        let p = self.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal("NewPalette: no block".into()))?;
        self.mem.write_u16(p, entries).map_err(Refusal::mem("PaletteRecord"))?;
        Ok((h, p))
    }
}

fn new_palette<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (entries, ctab, usage, tol) = (c.args.get(0) as u16, c.args.get(1), c.args.get(2) as u16, c.args.get(3) as u16);
    let (h, p) = m.make_palette(entries)?;
    let e = Refusal::mem("NewPalette");
    let src = if ctab == 0 { None } else { m.mm.block(ctab).map(|b| b.ptr) };
    for i in 0..u32::from(entries) {
        let at = p + HEADER + i * ENTRY;
        if let Some(t) = src {
            // ctTable starts at 8; the table may hold fewer colours than the palette asks for (the rest stay black)
            let count = m.mem.read_u16(t + 6).map_err(&e)?.wrapping_add(1);
            if i < u32::from(count) {
                let rgb = m.mem.span(t + 8 + i * 8 + 2, 6).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(&e)?;
                m.mem.write_bytes(at, &rgb).map_err(&e)?;
            }
        }
        m.mem.write_u16(at + 6, usage).and_then(|_| m.mem.write_u16(at + 8, tol)).map_err(&e)?;
    }
    Ok(Done { result: h, log: format!("[palette] NewPalette {entries} entries (usage {usage:#x}, tolerance {tol}){} -> {h:#x}", if src.is_some() { " from a colour table" } else { "" }) })
}

fn get_new_palette<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let id = c.args.get(0) as u16 as i16;
    let Ok(r) = m.get_resource(*b"pltt", id, false) else { return Ok(Done { result: 0, log: format!("[palette] GetNewPalette {id}: no 'pltt' resource") }) };
    let bytes = m.handle_bytes(r).unwrap_or_default();
    let n = bytes.get(0..2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
    if bytes.len() < (HEADER + u32::from(n) * ENTRY) as usize { return Err(Refusal(format!("GetNewPalette {id}: the 'pltt' holds {} bytes, too few for {n} entries", bytes.len()))); }
    let (h, p) = m.make_palette(n)?;
    m.mem.write_bytes(p, &bytes[..(HEADER + u32::from(n) * ENTRY) as usize]).map_err(Refusal::mem("PaletteRecord"))?;
    Ok(Done { result: h, log: format!("[palette] GetNewPalette {id}: {n} entries -> {h:#x}") })
}

fn dispose_palette<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let h = c.args.get(0);
    m.palette_at(h, "DisposePalette")?;
    m.pal.by_window.retain(|_, &mut p| p != h);
    m.dispose_handle(h).map_err(|e| Refusal(format!("DisposePalette: {e}")))?;
    Ok(Done { result: 0, log: format!("[palette] DisposePalette {h:#x}") })
}

fn set_palette<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (win, h, updates) = (c.args.get(0), c.args.get(1), c.args.get(2));
    if !m.windows.contains_key(&win) { return Err(Refusal(format!("NSetPalette: {win:#x} is not an open window"))); }
    if h == 0 { m.pal.by_window.remove(&win); } else { m.palette_at(h, "NSetPalette")?; m.pal.by_window.insert(win, h); }
    Ok(Done { result: 0, log: format!("[palette] NSetPalette window {win:#x} palette {h:#x} (updates {updates:#06x}; nothing to install on a 1-bit display)") })
}

fn get_palette<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    if !m.windows.contains_key(&win) { return Err(Refusal(format!("GetPalette: {win:#x} is not an open window"))); }
    let h = m.pal.by_window.get(&win).copied().unwrap_or(0);
    Ok(Done { result: h, log: format!("[palette] GetPalette window {win:#x} -> {h:#x}{}", if h == 0 { " (none set)" } else { "" }) })
}

fn get_entry_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = m.entry_at(c.args.get(0), c.args.get(1) as u16, "GetEntryColor")?;
    let rgb = m.mem.span(at, 6).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("PaletteRecord"))?;
    m.mem.write_bytes(c.args.get(2), &rgb).map_err(Refusal::mem("GetEntryColor"))?;
    Ok(Done { result: 0, log: format!("[palette] GetEntryColor {:#x} #{}", c.args.get(0), c.args.get(1) as u16) })
}
fn set_entry_color<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = m.entry_at(c.args.get(0), c.args.get(1) as u16, "SetEntryColor")?;
    let rgb = m.mem.span(c.args.get(2), 6).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("SetEntryColor"))?;
    m.mem.write_bytes(at, &rgb).map_err(Refusal::mem("PaletteRecord"))?;
    Ok(Done { result: 0, log: format!("[palette] SetEntryColor {:#x} #{}", c.args.get(0), c.args.get(1) as u16) })
}
fn get_entry_usage<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = m.entry_at(c.args.get(0), c.args.get(1) as u16, "GetEntryUsage")?;
    let e = Refusal::mem("GetEntryUsage");
    let (u, t) = (m.mem.read_u16(at + 6).map_err(&e)?, m.mem.read_u16(at + 8).map_err(&e)?);
    m.mem.write_u16(c.args.get(2), u).and_then(|_| m.mem.write_u16(c.args.get(3), t)).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[palette] GetEntryUsage {:#x} #{} -> usage {u:#x} tolerance {t}", c.args.get(0), c.args.get(1) as u16) })
}
fn set_entry_usage<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let at = m.entry_at(c.args.get(0), c.args.get(1) as u16, "SetEntryUsage")?;
    let e = Refusal::mem("SetEntryUsage");
    m.mem.write_u16(at + 6, c.args.get(2) as u16).and_then(|_| m.mem.write_u16(at + 8, c.args.get(3) as u16)).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[palette] SetEntryUsage {:#x} #{}", c.args.get(0), c.args.get(1) as u16) })
}
