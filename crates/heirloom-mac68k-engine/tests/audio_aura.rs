//! REUSE U16: Mac Sound Manager over engine-owned `guestsvc_abi::GuestAudio` (CoralOS Aura.engine).
//!
//! Proves that when `Mac::audio_host` is attached, every Sound Manager playback operation (`SndNewChannel`,
//! `SndPlay`, looped `bufferCmd`, `volumeCmd`, `pauseCmd`/`resumeCmd`, `flushCmd`, `SndChannelStatus`,
//! `SysBeep`, `SndDisposeChannel`) goes exclusively through `GuestAudio` with ZERO audio calls on `GuestUi`,
//! and that `FakeAudio::not_granted()` maps cleanly to `notEnoughHardware` (`-201`).
mod common;
use common::*;
use guestsvc_abi::{FakeAudio, GuestAudio, GuestAudioFormat, GuestAudioModel, GuestSvcError};
use heirloom_ui_abi::UiCall;
use std::cell::RefCell;
use std::rc::Rc;

fn audio_ui_calls(g: &Rig) -> usize {
    g.m.ui.calls.iter().filter(|c| matches!(
        c,
        UiCall::OpenAudio(_)
            | UiCall::QueueAudio { .. }
            | UiCall::QueueAudioLooped { .. }
            | UiCall::FlushAudio(_)
            | UiCall::PauseAudio { .. }
            | UiCall::SetAudioVolume { .. }
            | UiCall::CloseAudio(_)
    )).count()
}

const CHAN: u32 = 0x3100;
const CMD: u32 = 0x3110;
const OUT: u32 = 0x3400;
const STATUS: u32 = 0x3500;

struct SharedAudio(Rc<RefCell<FakeAudio>>);

impl GuestAudio for SharedAudio {
    fn open_stream(&mut self, model: &GuestAudioModel) -> Result<u32, GuestSvcError> {
        self.0.borrow_mut().open_stream(model)
    }
    fn queue_pcm(&mut self, stream: u32, pcm: &[u8]) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().queue_pcm(stream, pcm)
    }
    fn queue_looped(&mut self, stream: u32, pcm: &[u8], a: u32, b: u32) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().queue_looped(stream, pcm, a, b)
    }
    fn flush(&mut self, stream: u32) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().flush(stream)
    }
    fn pause(&mut self, stream: u32, paused: bool) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().pause(stream, paused)
    }
    fn set_volume(&mut self, stream: u32, left: u16, right: u16) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().set_volume(stream, left, right)
    }
    fn queued_bytes(&mut self, stream: u32) -> Result<u32, GuestSvcError> {
        self.0.borrow_mut().queued_bytes(stream)
    }
    fn close_stream(&mut self, stream: u32) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().close_stream(stream)
    }
}

fn std_header(rate: u32, samples: &[u8]) -> Vec<u8> {
    let mut h = vec![0, 0, 0, 0];
    h.extend((samples.len() as u32).to_be_bytes());
    h.extend(rate.to_be_bytes());
    h.extend([0u8; 8]);
    h.extend([0, 60]);
    h.extend(samples);
    h
}

fn snd_resource(g: &mut Rig, rate: u32, samples: &[u8]) -> u32 {
    let mut b = vec![0, 2, 0, 0, 0, 1, 0x80, 81, 0, 0, 0, 0, 0, 14];
    b.extend(std_header(rate, samples));
    let h = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, b.len() as u32, false).unwrap();
    let p = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_bytes(p, &b).unwrap();
    h
}

fn go(g: &mut Rig, f: impl FnOnce(&mut Prog)) {
    let mut p = Prog::new();
    f(&mut p);
    g.run(p.exit());
}

fn d0(p: &mut Prog, v: u32) {
    p.0.extend([0x20, 0x3C]);
    p.0.extend(v.to_be_bytes());
}

fn set_cmd(g: &mut Rig, cmd: u16, p1: i16, p2: u32) {
    g.m.mem.write_u16(CMD, cmd).unwrap();
    g.m.mem.write_u16(CMD + 2, p1 as u16).unwrap();
    g.m.mem.write_u32(CMD + 4, p2).unwrap();
}

#[test]
fn sound_manager_routes_all_playback_through_guest_audio_without_touching_ui() {
    let mut g = Rig::new();
    let aud = Rc::new(RefCell::new(FakeAudio::new()));
    g.m.audio_host = Some(Box::new(SharedAudio(aud.clone())));

    // SndNewChannel + SndPlay
    let h = snd_resource(&mut g, 0x2B77_0000, &[0x80, 0xA0, 0xC0, 0xA0, 0x80]); // 11127 Hz
    go(&mut g, |p| {
        p.clr_w().l(CHAN).w(5).l(0).l(0).trap(0xA807).pop_w(OUT);
        p.clr_w().rgn_arg(CHAN).l(h).b(1).trap(0xA805).pop_w(OUT + 2);
    });
    g.no_refusals();
    assert_eq!(g.m.mem.read_i16(OUT).unwrap(), 0);
    assert_eq!(g.m.mem.read_i16(OUT + 2).unwrap(), 0);
    assert!(g.m.ui.audio.is_empty(), "GuestUi must receive zero audio streams when audio_host is attached");
    assert_eq!(audio_ui_calls(&g), 0, "GuestUi must receive zero audio calls when audio_host is attached");

    let stream_id = {
        let a = aud.borrow();
        assert_eq!(a.streams().len(), 1);
        let (&id, s) = a.streams().iter().next().unwrap();
        assert_eq!(s.model.rate_hz, 11127);
        assert_eq!(s.model.channels, 1);
        assert_eq!(s.model.format, GuestAudioFormat::U8);
        assert_eq!(s.queued.as_slice(), &[0x80, 0xA0, 0xC0, 0xA0, 0x80]);
        id
    };

    // volumeCmd + pauseCmd + SndChannelStatus (scChannelBusy=1, scChannelPaused=1)
    set_cmd(&mut g, heirloom_mac68k_engine::sound::snd::VOLUME_CMD, 0, 0x0080_0040); // volumeCmd: right=0x80, left=0x40
    go(&mut g, |p| { p.clr_w().rgn_arg(CHAN).l(CMD).b(0).trap(0xA803).pop_w(OUT + 4); });
    set_cmd(&mut g, heirloom_mac68k_engine::sound::snd::PAUSE_CMD, 0, 0); // pauseCmd
    go(&mut g, |p| { p.clr_w().rgn_arg(CHAN).l(CMD).b(0).trap(0xA803).pop_w(OUT + 6); });
    go(&mut g, |p| {
        p.clr_w().rgn_arg(CHAN).w(24).l(STATUS);
        d0(p, 0x0010_0008); // SndChannelStatus
        p.trap(0xA800).pop_w(OUT + 8);
    });
    assert_eq!(g.m.mem.read_i16(OUT + 8).unwrap(), 0);
    assert_eq!(g.m.mem.read_u8(STATUS + 12).unwrap(), 1, "scChannelBusy");
    assert_eq!(g.m.mem.read_u8(STATUS + 14).unwrap(), 1, "scChannelPaused");
    assert_eq!(aud.borrow().streams()[&stream_id].volume, (0x0040, 0x0080));
    assert!(aud.borrow().streams()[&stream_id].paused);

    // flushCmd + SysBeep + SndDisposeChannel
    set_cmd(&mut g, 4, 0, 0); // flushCmd
    go(&mut g, |p| {
        p.clr_w().rgn_arg(CHAN).l(CMD).b(0).trap(0xA803).pop_w(OUT + 10);
        p.w(10).trap(0xA9C8); // SysBeep
        p.clr_w().rgn_arg(CHAN).b(1).trap(0xA801).pop_w(OUT + 12);
    });
    assert_eq!(g.m.mem.read_i16(OUT + 12).unwrap(), 0);
    assert!(aud.borrow().streams().contains_key(&stream_id) == false, "disposed channel closed its Aura stream");
    assert!(!aud.borrow().all_queued().is_empty(), "SysBeep queued PCM on Aura");
    assert!(g.m.ui.audio.is_empty(), "GuestUi still received zero audio streams");
}

#[test]
fn negative_control_not_granted_audio_returns_not_enough_hardware_and_touches_no_ui() {
    let mut g = Rig::new();
    let aud = Rc::new(RefCell::new(FakeAudio::not_granted()));
    g.m.audio_host = Some(Box::new(SharedAudio(aud.clone())));

    go(&mut g, |p| {
        p.clr_w().l(CHAN).w(5).l(0).l(0).trap(0xA807).pop_w(OUT);
        p.w(10).trap(0xA9C8); // SysBeep silent when audio is not granted
    });
    g.no_refusals();
    assert_eq!(g.m.mem.read_i16(OUT).unwrap(), -201, "notEnoughHardware when (audio) is not granted");
    assert!(aud.borrow().streams().is_empty());
    assert!(g.m.ui.audio.is_empty());
    assert_eq!(audio_ui_calls(&g), 0);
}
