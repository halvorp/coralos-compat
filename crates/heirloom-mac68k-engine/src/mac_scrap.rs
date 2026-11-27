//! Stateless Classic Mac Scrap Manager adapter over CoralOS `clipboardd` (`guestsvc_abi::GuestClipboard`, REUSE U8).
//!
//! `ZeroScrap`, `PutScrap`, `GetScrap`, `LoadScrap`, `UnloadScrap`, `InfoScrap`.
//! When `Mac::clip_host` is attached (always true on CoralOS via `OsVTable::attach_clipboard`),
//! every read and write crosses [`guestsvc_abi::GuestClipboard`] directly with NO guest-private fallback:
//! - `'TEXT'` <-> `text/plain;charset=utf-8` (Mac OS Roman <-> UTF-8 via `crate::macroman`, `\r` <-> `\n`)
//! - `'PICT'` <-> `application/x-mac-pict`
//! - any other 4CC <-> `application/x-mac-scrap-<8 lowercase hex bytes>`
//! - `scrapCount` tracks `GuestClipboard::generation()` (backed on CoralOS by C12 `Watch` + doorbell, zero polling)
//!   so native CoralOS copies immediately invalidate classic and Carbon `ScrapRef`s.
//! - On `NotGranted` (or any host refusal), `PutScrap` / `GetScrap` record a named refusal and return an error
//!   without silently storing or serving private guest-only data.
use crate::cpu::Cpu;
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestsvc_abi::GuestSvcError;
use heirloom_ui_abi::GuestUi;

pub const NO_TYPE_ERR: i32 = -102;
/// `noScrapErr` (-100): returned when the host clipboard refuses (`NotGranted` / `Lost` / `Stalled`).
pub const NO_SCRAP_ERR: i32 = -100;

/// MIME type used for plain UTF-8 text on CoralOS `clipboardd`.
pub const MIME_TEXT_UTF8: &str = "text/plain;charset=utf-8";
/// MIME type used for Classic Mac `'PICT'` payloads.
pub const MIME_MAC_PICT: &str = "application/x-mac-pict";

/// Bookkeeping for the Mac Scrap Manager (`scrapCount` and lazy `PScrapStuff` guest pointer).
///
/// When `Mac::clip_host` is attached, `items` is NEVER read or written: all payloads live in
/// CoralOS `clipboardd`. `items` exists solely for standalone host unit tests that construct
/// `Mac::new(ui)` without attaching a host clipboard.
#[derive(Default)]
pub struct Scrap {
    pub items: Vec<([u8; 4], Vec<u8>)>,
    /// `scrapCount`: bumps whenever `ZeroScrap` runs or the host clipboard generation advances.
    pub count: i16,
    /// Last host `GuestClipboard::generation()` observed by this machine.
    pub last_host_gen: Option<u64>,
    /// If `ZeroScrap` ran while connected to a host clipboard, the generation at which it was cleared
    /// (so a subsequent `GetScrap` before any `PutScrap` or native copy returns `noTypeErr`).
    pub cleared_at_gen: Option<u64>,
    /// The `PScrapStuff` guest pointer returned by `InfoScrap` (lazily allocated, kept current).
    info: u32,
}

impl Scrap {
    pub fn zero(&mut self) {
        self.items.clear();
        self.count = self.count.wrapping_add(1);
    }
    pub fn put(&mut self, ty: [u8; 4], data: Vec<u8>) {
        match self.items.iter_mut().find(|(t, _)| *t == ty) {
            Some(e) => e.1 = data,
            None => self.items.push((ty, data)),
        }
    }
    pub fn get(&self, ty: [u8; 4]) -> Option<&[u8]> {
        self.items.iter().find(|(t, _)| *t == ty).map(|(_, d)| d.as_slice())
    }
    pub fn size(&self) -> u32 {
        self.items.iter().map(|(_, d)| d.len() as u32 + 8).sum()
    }
}

/// Convert a Mac 4CC + guest bytes into a CoralOS `(mime, payload)`.
pub fn mac_to_host(ty: [u8; 4], data: &[u8]) -> (String, Vec<u8>) {
    if ty == *b"TEXT" {
        let utf8 = crate::macroman::decode(data).replace('\r', "\n");
        (String::from(MIME_TEXT_UTF8), utf8.into_bytes())
    } else if ty == *b"PICT" {
        (String::from(MIME_MAC_PICT), data.to_vec())
    } else {
        (
            format!("application/x-mac-scrap-{:02x}{:02x}{:02x}{:02x}", ty[0], ty[1], ty[2], ty[3]),
            data.to_vec(),
        )
    }
}

/// Convert a CoralOS `(mime, payload)` into Mac guest bytes for requested 4CC `ty`, if compatible.
pub fn host_to_mac(ty: [u8; 4], mime: &str, payload: &[u8]) -> Option<Vec<u8>> {
    if ty == *b"TEXT" {
        if mime == "text/plain" || mime.starts_with("text/plain;") {
            let s = String::from_utf8_lossy(payload);
            let mac_lines = s.replace("\r\n", "\r").replace('\n', "\r");
            return Some(crate::macroman::encode_lossy(&mac_lines));
        }
        return None;
    }
    if ty == *b"PICT" {
        return (mime == MIME_MAC_PICT).then(|| payload.to_vec());
    }
    let want = format!("application/x-mac-scrap-{:02x}{:02x}{:02x}{:02x}", ty[0], ty[1], ty[2], ty[3]);
    (mime == want).then(|| payload.to_vec())
}

impl<U: GuestUi> Mac<U> {
    /// Observe the host clipboard's current generation (C12 Watch / doorbell backed, zero polling)
    /// and advance `scrapCount` if a native app changed the clipboard since we last looked.
    pub fn scrap_sync_from_host(&mut self) -> Result<(), GuestSvcError> {
        let Some(host) = self.clip_host.as_mut() else {
            return Ok(());
        };
        let gen = host.generation()?;
        let changed = match self.scrap.last_host_gen {
            Some(prev) => prev != gen,
            None => gen != 0,
        };
        if changed {
            self.scrap.count = self.scrap.count.wrapping_add(1);
            self.scrap.cleared_at_gen = None;
        }
        self.scrap.last_host_gen = Some(gen);
        Ok(())
    }

    /// Clear the desk scrap and advance `scrapCount`. When `clip_host` is attached, records the
    /// cleared generation without touching any guest-private buffer.
    pub fn scrap_zero(&mut self) -> Result<(), GuestSvcError> {
        if let Some(host) = self.clip_host.as_mut() {
            let gen = host.generation()?;
            self.scrap.last_host_gen = Some(gen);
            self.scrap.cleared_at_gen = Some(gen);
            self.scrap.count = self.scrap.count.wrapping_add(1);
            return Ok(());
        }
        self.scrap.zero();
        Ok(())
    }

    /// Publish `(ty, data)` to the host clipboard when attached (never falling back to private state on error),
    /// or to the standalone test buffer when no host clipboard is attached.
    pub fn scrap_put(&mut self, ty: [u8; 4], data: Vec<u8>) -> Result<(), GuestSvcError> {
        if let Some(host) = self.clip_host.as_mut() {
            let (mime, payload) = mac_to_host(ty, &data);
            let gen = host.copy(&mime, &payload)?;
            self.scrap.last_host_gen = Some(gen);
            self.scrap.cleared_at_gen = None;
            return Ok(());
        }
        self.scrap.put(ty, data);
        Ok(())
    }

    /// Read flavor `ty` from the host clipboard when attached, or from the standalone test buffer when detached.
    pub fn scrap_get(&mut self, ty: [u8; 4]) -> Result<Option<Vec<u8>>, GuestSvcError> {
        if self.clip_host.is_some() {
            self.scrap_sync_from_host()?;
            if self.scrap.cleared_at_gen.is_some() && self.scrap.cleared_at_gen == self.scrap.last_host_gen {
                return Ok(None);
            }
            let entry = self.clip_host.as_mut().unwrap().paste()?;
            return Ok(entry.and_then(|(mime, payload)| host_to_mac(ty, &mime, &payload)));
        }
        Ok(self.scrap.get(ty).map(<[u8]>::to_vec))
    }

    /// Approximate scrap size in bytes for `InfoScrap` (`PScrapStuff.scrapSize`).
    pub fn scrap_size(&mut self) -> u32 {
        if self.clip_host.is_some() {
            if self.scrap_sync_from_host().is_err() {
                return 0;
            }
            if self.scrap.cleared_at_gen.is_some() && self.scrap.cleared_at_gen == self.scrap.last_host_gen {
                return 0;
            }
            return match self.clip_host.as_mut().unwrap().paste() {
                Ok(Some((mime, payload))) => {
                    if let Some(t) = host_to_mac(*b"TEXT", &mime, &payload) {
                        t.len() as u32 + 8
                    } else {
                        payload.len() as u32 + 8
                    }
                }
                _ => 0,
            };
        }
        self.scrap.size()
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9FC, &[], Some(Arg::L), zero_scrap::<U>)?;
    m.register(0xA9FE, &[Arg::L, Arg::L, Arg::P], Some(Arg::L), put_scrap::<U>)?;
    m.register(0xA9FD, &[Arg::P, Arg::L, Arg::P], Some(Arg::L), get_scrap::<U>)?;
    m.register(0xA9FA, &[], Some(Arg::L), resident::<U>)?; // UnloadScrap
    m.register(0xA9FB, &[], Some(Arg::L), resident::<U>)?; // LoadScrap
    m.register(0xA9F9, &[], Some(Arg::L), info_scrap::<U>)
}

fn ty_str(t: [u8; 4]) -> String {
    t.iter().map(|&b| char::from(b)).collect()
}

fn zero_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    if let Err(e) = m.scrap_zero() {
        m.note_refusal("ZeroScrap", format!("host clipboard {}", e.name()));
        return Ok(Done { result: NO_SCRAP_ERR as u32, log: format!("[scrap] ZeroScrap REFUSED: {}", e.name()) });
    }
    Ok(Done { result: 0, log: format!("[scrap] ZeroScrap (scrapCount {})", m.scrap.count) })
}

fn put_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (len, ty, src) = (c.args.get(0), c.args.get(1).to_be_bytes(), c.args.get(2));
    let data = m.mem.span(src, len).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(Refusal::mem("PutScrap source"))?;
    if let Err(e) = m.scrap_put(ty, data) {
        m.note_refusal("PutScrap", format!("host clipboard {}", e.name()));
        return Ok(Done { result: NO_SCRAP_ERR as u32, log: format!("[scrap] PutScrap '{}' REFUSED: {}", ty_str(ty), e.name()) });
    }
    Ok(Done { result: 0, log: format!("[scrap] PutScrap '{}' {len} bytes", ty_str(ty)) })
}

fn get_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, ty, off) = (c.args.get(0), c.args.get(1).to_be_bytes(), c.args.get(2));
    let data = match m.scrap_get(ty) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return Ok(Done { result: NO_TYPE_ERR as u32, log: format!("[scrap] GetScrap '{}' -> noTypeErr", ty_str(ty)) });
        }
        Err(e) => {
            m.note_refusal("GetScrap", format!("host clipboard {}", e.name()));
            return Ok(Done { result: NO_SCRAP_ERR as u32, log: format!("[scrap] GetScrap '{}' REFUSED: {}", ty_str(ty), e.name()) });
        }
    };
    if h != 0 {
        m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, data.len() as u32).map_err(|e| Refusal(format!("GetScrap: cannot size the destination handle (OSErr {e})")))?;
        let p = m.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("GetScrap: {h:#x} is not a handle")))?;
        m.mem.write_bytes(p, &data).map_err(Refusal::mem("GetScrap destination"))?;
    }
    if off != 0 {
        m.mem.write_u32(off, 0).map_err(Refusal::mem("GetScrap offset"))?;
    }
    Ok(Done { result: data.len() as u32, log: format!("[scrap] GetScrap '{}' -> {} bytes", ty_str(ty), data.len()) })
}

/// LoadScrap / UnloadScrap: the scrap lives in CoralOS `clipboardd`, so there is nothing to move to or from a file.
fn resident<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Ok(Done { result: 0, log: format!("[scrap] {} (the scrap is host-backed)", c.name) })
}

/// PScrapStuff: scrapSize (4), scrapHandle (4, NIL: no handle is exposed), scrapCount (2), scrapState (2: 1 = in memory), scrapName (4, NIL).
fn info_scrap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    let size = m.scrap_size();
    if m.scrap.info == 0 {
        m.scrap.info = m.mm.new_ptr(&mut m.mem, &mut m.heap, 16, true).map_err(|e| Refusal(format!("InfoScrap: no memory (OSErr {e})")))?;
    }
    let p = m.scrap.info;
    let e = Refusal::mem("PScrapStuff");
    m.mem.write_u32(p, size).map_err(&e)?;
    m.mem.write_u32(p + 4, 0).map_err(&e)?;
    m.mem.write_u16(p + 8, m.scrap.count as u16).map_err(&e)?;
    m.mem.write_u16(p + 10, 1).map_err(&e)?;
    m.mem.write_u32(p + 12, 0).map_err(&e)?;
    Ok(Done { result: p, log: format!("[scrap] InfoScrap -> {p:#x} (size {size}, count {})", m.scrap.count) })
}
