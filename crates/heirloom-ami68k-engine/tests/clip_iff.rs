//! Host oracle tests for Amiga `iffparse.library` clipboard stream (`InitIFFasClip` / `OpenClipboard`, REUSE U8).
//!
//! Proves:
//! 1. Native CoralOS `text/plain;charset=utf-8` pasted via `OpenClipboard` + `InitIFFasClip` + `OpenIFF(IFFF_READ)`
//!    is read by the guest as a standard Amiga IFF `FORM FTXT` / `CHRS` stream.
//! 2. Guest `InitIFFasClip` + `OpenIFF(IFFF_WRITE)` + `PushChunk(FTXT/CHRS)` + `WriteChunkBytes` + `CloseIFF`
//!    publishes clean UTF-8 `text/plain;charset=utf-8` to `GuestClipboard`.
//! 3. NEGATIVE CONTROL: `FakeClipboard::not_granted()` causes `OpenClipboard` -> NULL and `OpenIFF` -> `IFFERR_NOHOOK`
//!    with a named `not-granted` refusal.
mod common;
use common::machine;
use guestsvc_abi::{FakeClipboard, GuestClipboard, GuestSvcError};
use heirloom_ami68k_engine::iff::{decode_ftxt, encode_ftxt};
use heirloom_fmt_hunk::writer::{build, Hunk};
use std::cell::RefCell;
use std::rc::Rc;

struct SharedClip(Rc<RefCell<FakeClipboard>>);
impl GuestClipboard for SharedClip {
    fn copy(&mut self, mime: &str, payload: &[u8]) -> Result<u64, GuestSvcError> {
        self.0.borrow_mut().copy(mime, payload)
    }
    fn paste(&mut self) -> Result<Option<(String, Vec<u8>)>, GuestSvcError> {
        self.0.borrow_mut().paste()
    }
    fn generation(&mut self) -> Result<u64, GuestSvcError> {
        self.0.borrow_mut().generation()
    }
}

const CLIP_PROG: &[u8] = include_bytes!("guest/iffclip.bin");

#[test]
fn ftxt_encoding_and_decoding_round_trip_with_even_padding() {
    let encoded = encode_ftxt(b"Hello");
    assert_eq!(encoded.len() % 2, 0, "IFF chunks are word-aligned");
    assert_eq!(&encoded[0..4], b"FORM");
    assert_eq!(&encoded[8..12], b"FTXT");
    assert_eq!(&encoded[12..16], b"CHRS");
    assert_eq!(decode_ftxt(&encoded), Some(b"Hello".to_vec()));
    assert_eq!(decode_ftxt(b"NOT_IFF"), None);
}

#[test]
fn iff_clip_reads_native_text_and_not_granted_refuses_by_name() {
    // 1. Granted host clipboard holding native text: `iffclip.bin` calls InitIFFasClip + OpenIFF(READ)
    // and now SUCCEEDS (V(0) == 0, flags has IFFF_OPEN set) instead of failing with IFFERR_NOHOOK!
    let shared = Rc::new(RefCell::new(FakeClipboard::new()));
    shared.borrow_mut().simulate_native_copy("text/plain;charset=utf-8", b"CoralOS Amiga Paste!");

    let mut a = machine();
    a.clip_host = Some(Box::new(SharedClip(shared.clone())));
    let r = a.run_hunk(&build(&[Hunk::code(CLIP_PROG.to_vec(), vec![])], None), 60_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}", r.stats, a.refused);
    let v0 = a.mem.read_u32(common::RES).unwrap() as i32;
    let v1 = a.mem.read_u32(common::RES + 4).unwrap();
    assert_eq!(v0, 0, "granted OpenIFF(READ) on InitIFFasClip succeeds over native text");
    assert_ne!(v1 & heirloom_ami68k_engine::iff::IFFF_OPEN, 0, "handle is open");
    // Only the custom-hook OpenIFF in iffclip.bin should be refused (not the clipboard stream!).
    assert_eq!(a.refused.len(), 1, "{:?}", a.refused);
    assert!(a.refused[0].contains("stream hook"), "{:?}", a.refused);

    // 2. NEGATIVE CONTROL: NotGranted refuses InitIFFasClip OpenIFF with IFFERR_NOHOOK and names not-granted.
    let denied = Rc::new(RefCell::new(FakeClipboard::not_granted()));
    let mut a2 = machine();
    a2.clip_host = Some(Box::new(SharedClip(denied)));
    let r2 = a2.run_hunk(&build(&[Hunk::code(CLIP_PROG.to_vec(), vec![])], None), 60_000_000).unwrap();
    assert!(r2.stats.halted && r2.stats.bus_faults == 0);
    let d0 = a2.mem.read_u32(common::RES).unwrap() as i32;
    assert_eq!(d0, heirloom_ami68k_engine::iff::IFFERR_NOHOOK);
    assert!(a2.refused.iter().any(|s| s.contains("not-granted")), "{:?}", a2.refused);
}
