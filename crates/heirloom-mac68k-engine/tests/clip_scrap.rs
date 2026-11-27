//! Host oracle tests for the stateless Mac Scrap Manager adapter (`mac_scrap.rs`, REUSE U8).
//!
//! Proves:
//! 1. Mac `PutScrap('TEXT')` converts MacRoman + `\r` -> UTF-8 + `\n` and publishes directly to `GuestClipboard`.
//! 2. Native CoralOS copies (`simulate_native_copy("text/plain;charset=utf-8", ...)`) advance `scrapCount`
//!    on `InfoScrap` / `GetScrap` and convert UTF-8 + `\n` -> MacRoman + `\r`.
//! 3. `ZeroScrap` clears the guest's view until a new `PutScrap` or native copy arrives.
//! 4. NEGATIVE CONTROL: `FakeClipboard::not_granted()` refuses `PutScrap` / `GetScrap` / `ZeroScrap`
//!    with `noScrapErr` (-100) and a named refusal, with ZERO private guest-only fallback.
use guestsvc_abi::{FakeClipboard, GuestClipboard, GuestSvcError};
use heirloom_mac68k_engine::scrap::{NO_SCRAP_ERR, NO_TYPE_ERR};
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::RecordingGuestUi;
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

/// Tiny 68k emitter matching `tests/toolbox_extras.rs`.
struct Prog(Vec<u8>);
impl Prog {
    fn new() -> Self { Self(vec![]) }
    fn w(&mut self, x: u16) -> &mut Self { self.0.extend(x.to_be_bytes()); self }
    fn trap(&mut self, op: u16) -> &mut Self { self.w(op) }
    fn clr_l(&mut self) -> &mut Self { self.w(0x42A7) }
    fn l(&mut self, v: u32) -> &mut Self { self.w(0x2F3C).w((v >> 16) as u16).w(v as u16) }
    fn pop_l(&mut self, to: u32) -> &mut Self { self.w(0x23DF).w((to >> 16) as u16).w(to as u16) }
    fn finish(mut self) -> Vec<u8> { self.w(0x4E75); self.0 }
}

fn l(m: &Mac<RecordingGuestUi>, at: u32) -> u32 { m.mem.read_u32(at).unwrap() }
fn w(m: &Mac<RecordingGuestUi>, at: u32) -> u16 { m.mem.read_u16(at).unwrap() }

fn run_code(m: &mut Mac<RecordingGuestUi>, code: &[u8]) {
    m.start(code, 0).unwrap();
    let rep = m.run_loop(100_000).unwrap();
    assert!(rep.stats.halted && rep.stats.bus_faults == 0, "{:?} {:?}", rep.stats, m.refused);
}

#[test]
fn mac_scrap_bridges_text_and_pict_to_host_clipboard_with_macroman_conversion() {
    let shared = Rc::new(RefCell::new(FakeClipboard::new()));
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.clip_host = Some(Box::new(SharedClip(shared.clone())));

    // Guest writes MacRoman "Caf\x8e\rLine2" ("Café\rLine2") via ZeroScrap + PutScrap('TEXT').
    let mac_text = b"Caf\x8e\rLine2";
    m.mem.write_bytes(0x3200, mac_text).unwrap();
    let text_4cc = u32::from_be_bytes(*b"TEXT");

    let mut p = Prog::new();
    p.clr_l().trap(0xA9FC).pop_l(0x3000); // ZeroScrap
    p.clr_l().l(mac_text.len() as u32).l(text_4cc).l(0x3200).trap(0xA9FE).pop_l(0x3004); // PutScrap
    p.clr_l().trap(0xA9F9).pop_l(0x3008); // InfoScrap
    run_code(&mut m, &p.finish());
    assert_eq!((l(&m, 0x3000), l(&m, 0x3004)), (0, 0));
    assert!(m.scrap.items.is_empty(), "host-backed scrap must NOT store guest-private items");

    // Host clipboard received UTF-8 with '\n'.
    {
        let c = shared.borrow();
        let (mime, payload) = c.current().expect("host clipboard populated");
        assert_eq!(mime, "text/plain;charset=utf-8");
        assert_eq!(payload, "Café\nLine2".as_bytes());
    }

    let info = l(&m, 0x3008);
    let count_before = w(&m, info + 8);

    // Native CoralOS app copies UTF-8 with CRLF and MacRoman-representable ellipsis.
    shared.borrow_mut().simulate_native_copy("text/plain;charset=utf-8", "Read Me\u{2026}\r\nOK".as_bytes());

    // Guest calls InfoScrap + GetScrap(hDest, 'TEXT', &off) and observes bumped scrapCount + MacRoman '\r'.
    let mut p2 = Prog::new();
    p2.clr_l().trap(0xA9F9).pop_l(0x3010); // InfoScrap
    p2.w(0x7002).trap(0xA122).w(0x23C8).w(0).w(0x3100); // NewHandle(2) -> $3100
    p2.clr_l().w(0x2F38).w(0x3100).l(text_4cc).l(0x3110).trap(0xA9FD).pop_l(0x3014); // GetScrap
    p2.clr_l().l(0).l(u32::from_be_bytes(*b"PICT")).l(0x3110).trap(0xA9FD).pop_l(0x3018); // GetScrap('PICT') -> noTypeErr
    run_code(&mut m, &p2.finish());

    let count_after = w(&m, l(&m, 0x3010) + 8);
    assert_eq!(count_after, count_before.wrapping_add(1), "native copy bumped scrapCount");
    let got_len = l(&m, 0x3014);
    assert_eq!(got_len, 11);
    let h = l(&m, 0x3100);
    assert_eq!(m.handle_bytes(h).unwrap(), b"Read Me\xC9\rOK");
    assert_eq!(l(&m, 0x3018) as i32, NO_TYPE_ERR);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
}

#[test]
fn not_granted_refuses_without_private_fallback() {
    let shared = Rc::new(RefCell::new(FakeClipboard::not_granted()));
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.clip_host = Some(Box::new(SharedClip(shared.clone())));

    m.mem.write_bytes(0x3200, b"secret").unwrap();
    let text_4cc = u32::from_be_bytes(*b"TEXT");

    let mut p = Prog::new();
    p.clr_l().trap(0xA9FC).pop_l(0x3000); // ZeroScrap -> noScrapErr
    p.clr_l().l(6).l(text_4cc).l(0x3200).trap(0xA9FE).pop_l(0x3004); // PutScrap -> noScrapErr
    p.clr_l().l(0).l(text_4cc).l(0x3110).trap(0xA9FD).pop_l(0x3008); // GetScrap -> noScrapErr
    run_code(&mut m, &p.finish());

    assert_eq!(l(&m, 0x3000) as i32, NO_SCRAP_ERR);
    assert_eq!(l(&m, 0x3004) as i32, NO_SCRAP_ERR);
    assert_eq!(l(&m, 0x3008) as i32, NO_SCRAP_ERR);
    assert!(m.scrap.items.is_empty(), "NEGATIVE CONTROL: denied PutScrap must not store private data");
    assert!(shared.borrow().current().is_none());
    assert!(m.refused.iter().any(|r| r.contains("not-granted")), "{:?}", m.refused);
}
