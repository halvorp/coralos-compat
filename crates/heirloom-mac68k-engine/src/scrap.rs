//! The Scrap Manager (Inside Macintosh I, "Scrap Manager"): `ZeroScrap`, `PutScrap`, `GetScrap`,
//! `LoadScrap`, `UnloadScrap`, `InfoScrap`. The desk scrap is an IN-MEMORY list of (type, bytes); the scrap file on disk is
//! never needed, so `LoadScrap` and `UnloadScrap` succeed trivially (the scrap is always in memory). The host clipboard
//! joins this list later through [`Scrap`]; nothing here reaches outside the guest.
//!
//! `PutScrap(length, type, source)` adds (or replaces) the entry of that type; `GetScrap(hDest, type, VAR offset)` returns its
//! length (and, when `hDest` is non-NIL, resizes the handle and copies the data) or `noTypeErr` (-102); `ZeroScrap` empties the
//! scrap and bumps `scrapCount`, which is how programs notice a change. `InfoScrap` answers a PScrapStuff in guest memory.
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const NO_TYPE_ERR: i32 = -102;

/// The desk scrap.
#[derive(Default)]
pub struct Scrap {
    pub items: Vec<([u8; 4], Vec<u8>)>,
    /// scrapCount: changes whenever the scrap is zeroed.
    pub count: i16,
    /// The PScrapStuff the program was given (lazily allocated, kept current).
    info: u32,
}

impl Scrap {
    pub fn zero(&mut self) { self.items.clear(); self.count = self.count.wrapping_add(1); }
    pub fn put(&mut self, ty: [u8; 4], data: Vec<u8>) {
        match self.items.iter_mut().find(|(t, _)| *t == ty) { Some(e) => e.1 = data, None => self.items.push((ty, data)) }
    }
    pub fn get(&self, ty: [u8; 4]) -> Option<&[u8]> { self.items.iter().find(|(t, _)| *t == ty).map(|(_, d)| d.as_slice()) }
    pub fn size(&self) -> u32 { self.items.iter().map(|(_, d)| d.len() as u32 + 8).sum() }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9FC, &[], Some(Arg::L), zero_scrap::<U>)?;
    m.register(0xA9FE, &[Arg::L, Arg::L, Arg::P], Some(Arg::L), put_scrap::<U>)?;
    m.register(0xA9FD, &[Arg::P, Arg::L, Arg::P], Some(Arg::L), get_scrap::<U>)?;
    m.register(0xA9FA, &[], Some(Arg::L), resident::<U>)?; // UnloadScrap
    m.register(0xA9FB, &[], Some(Arg::L), resident::<U>)?; // LoadScrap
    m.register(0xA9F9, &[], Some(Arg::L), info_scrap::<U>)
}

fn ty_str(t: [u8; 4]) -> alloc::string::String { t.iter().map(|&b| char::from(b)).collect() }

fn zero_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    m.scrap.zero();
    Ok(Done { result: 0, log: format!("[scrap] ZeroScrap (scrapCount {})", m.scrap.count) })
}

fn put_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (len, ty, src) = (c.args.get(0), c.args.get(1).to_be_bytes(), c.args.get(2));
    let data = m.mem.span(src, len).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("PutScrap source"))?;
    m.scrap.put(ty, data);
    Ok(Done { result: 0, log: format!("[scrap] PutScrap '{}' {len} bytes", ty_str(ty)) })
}

fn get_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, ty, off) = (c.args.get(0), c.args.get(1).to_be_bytes(), c.args.get(2));
    let Some(data) = m.scrap.get(ty).map(<[u8]>::to_vec) else {
        return Ok(Done { result: NO_TYPE_ERR as u32, log: format!("[scrap] GetScrap '{}' -> noTypeErr", ty_str(ty)) });
    };
    if h != 0 {
        m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, data.len() as u32).map_err(|e| Refusal(format!("GetScrap: cannot size the destination handle (OSErr {e})")))?;
        let p = m.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("GetScrap: {h:#x} is not a handle")))?;
        m.mem.write_bytes(p, &data).map_err(Refusal::mem("GetScrap destination"))?;
    }
    if off != 0 { m.mem.write_u32(off, 0).map_err(Refusal::mem("GetScrap offset"))?; }
    Ok(Done { result: data.len() as u32, log: format!("[scrap] GetScrap '{}' -> {} bytes", ty_str(ty), data.len()) })
}

/// LoadScrap / UnloadScrap: the scrap lives in memory, so there is nothing to move to or from a file.
fn resident<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[scrap] {} (the scrap is always in memory)", c.name) })
}

/// PScrapStuff: scrapSize (4), scrapHandle (4, NIL: no handle is exposed), scrapCount (2), scrapState (2: 1 = in memory), scrapName (4, NIL).
fn info_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    if m.scrap.info == 0 { m.scrap.info = m.mm.new_ptr(&mut m.mem, &mut m.heap, 16, true).map_err(|e| Refusal(format!("InfoScrap: no memory (OSErr {e})")))?; }
    let p = m.scrap.info;
    let e = Refusal::mem("PScrapStuff");
    m.mem.write_u32(p, m.scrap.size()).map_err(&e)?;
    m.mem.write_u32(p + 4, 0).map_err(&e)?;
    m.mem.write_u16(p + 8, m.scrap.count as u16).map_err(&e)?;
    m.mem.write_u16(p + 10, 1).map_err(&e)?;
    m.mem.write_u32(p + 12, 0).map_err(&e)?;
    Ok(Done { result: p, log: format!("[scrap] InfoScrap -> {p:#x} ({} types, count {})", m.scrap.items.len(), m.scrap.count) })
}
