//! The Sound Manager routines Glider PRO and Warcraft III reach past the first slice: `SndControl`, `GetSoundHeaderOffset`, the uncompressed
//! forms of the compressed sound header (`NONE`, `raw `, `twos`, `sowt` -- MACE and the rest refused by name), the legacy speaker volume
//! (`SetSoundVol` / `GetSoundVol` over `SdVolume`) and the completion routine's guest order.
mod common;
use common::*;
use mac68k_engine::sound::snd;
use guestui_abi::AudioFormat;

const CHAN: u32 = 0x3100;
const CMD: u32 = 0x3110;
const HDR: u32 = 0x3200;
const OUT: u32 = 0x3400;
const OFFS: u32 = 0x3500;

fn d0(p: &mut Prog, v: u32) { p.0.extend([0x20, 0x3C]); p.0.extend(v.to_be_bytes()); }
fn word(g: &Rig, at: u32) -> i16 { g.m.mem.read_i16(at).unwrap() }
fn go(g: &mut Rig, f: impl FnOnce(&mut Prog)) { let mut p = Prog::new(); f(&mut p); g.run(p.exit()); }
fn queued(g: &Rig) -> Vec<u8> { g.m.ui.audio.values().flat_map(|a| a.queued.clone()).collect() }

fn snd_handle(g: &mut Rig, body: &[u8]) -> u32 {
    let h = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, body.len() as u32, false).unwrap();
    let p = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_bytes(p, body).unwrap();
    h
}

#[test]
fn snd_control_answers_available_for_the_sampled_synth_only_and_refuses_what_it_cannot_answer_by_name() {
    let mut g = Rig::new();
    let ctl = |g: &mut Rig, synth: i16, cmd: u16, p2: u32| -> (i16, u16) {
        g.m.mem.write_u16(CMD, cmd).unwrap(); g.m.mem.write_u16(CMD + 2, 0xFFFF).unwrap(); g.m.mem.write_u32(CMD + 4, p2).unwrap();
        go(g, |p| { p.clr_w().w(synth).l(CMD).trap(0xA806).pop_w(OUT); });
        (word(g, OUT), g.m.mem.read_u16(CMD + 2).unwrap())
    };
    assert_eq!(ctl(&mut g, 5, 24, 0), (0, 1), "availableCmd: the sampled synthesizer is available");
    assert_eq!(ctl(&mut g, 1, 24, 0), (0, 0), "availableCmd: the square-wave synthesizer is not modelled, so it is NOT available");
    assert_eq!(ctl(&mut g, 5, 24, 4), (0, 0), "availableCmd with a request for voices the sampled synth does not offer");
    g.no_refusals();
    assert_eq!(ctl(&mut g, 5, 40, 0).0, -50, "NEGATIVE: a command SndControl does not know is paramErr ...");
    assert!(g.m.refused.iter().any(|r| r.contains("SndControl")), "... and named: {:?}", g.m.refused);
}

#[test]
fn get_sound_header_offset_finds_the_first_data_command_in_either_format_and_badformat_otherwise() {
    let mut g = Rig::new();
    let call = |g: &mut Rig, h: u32| -> (i16, u32) {
        g.m.mem.write_u32(OFFS, 0xDEAD_BEEF).unwrap();
        go(g, |p| { p.clr_w().l(h).l(OFFS); d0(p, 0x0404_0018); p.trap(0xA800).pop_w(OUT); });
        (word(g, OUT), g.m.mem.read_u32(OFFS).unwrap())
    };
    // format 2: format, refCount, numCmds, then bufferCmd|0x8000 with the header at offset 14
    let f2 = snd_handle(&mut g, &[0, 2, 0, 0, 0, 1, 0x80, 81, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0]);
    assert_eq!(call(&mut g, f2), (0, 14));
    // format 1: one synth (6 bytes), then a volumeCmd and a soundCmd|0x8000 whose header is at 28
    let f1 = snd_handle(&mut g, &[0, 1, 0, 1, 0, 5, 0, 0, 0, 0, 0, 2, 0, 46, 0, 0, 0, 1, 0, 1, 0x80, 80, 0, 0, 0, 0, 0, 28]);
    assert_eq!(call(&mut g, f1), (0, 28));
    // NEGATIVE: a resource with no sampled-sound command, a wrong format word, and a handle that is not one
    let none = snd_handle(&mut g, &[0, 2, 0, 0, 0, 1, 0, 46, 0, 0, 0, 0, 0, 0]);
    assert_eq!(call(&mut g, none), (-206, 0), "badFormat, and the out slot is cleared");
    let bad = snd_handle(&mut g, &[0, 9, 0, 0, 0, 0]);
    assert_eq!(call(&mut g, bad).0, -206);
    assert_eq!(call(&mut g, 0x1234).0, -50, "not a live handle: paramErr");
}

fn cmp_header(format: &[u8; 4], comp_id: i16, bits: u16, chans: u32, frames: u32, samples: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 64];
    h[4..8].copy_from_slice(&chans.to_be_bytes());
    h[8..12].copy_from_slice(&(22254u32 << 16).to_be_bytes());
    h[20] = 0xFE;
    h[22..26].copy_from_slice(&frames.to_be_bytes());
    h[40..44].copy_from_slice(format);
    h[56..58].copy_from_slice(&comp_id.to_be_bytes());
    h[62..64].copy_from_slice(&bits.to_be_bytes());
    h.extend_from_slice(samples);
    h
}

#[test]
fn compressed_headers_that_are_really_pcm_decode_and_the_codecs_are_refused_by_their_names() {
    let none = |h: &[u8]| snd::decode_header(h, |_, _| None);
    // NONE / twos: signed big-endian 16-bit; sowt: little-endian; raw : offset-binary 8-bit
    let s = none(&cmp_header(b"twos", -1, 16, 1, 2, &[0x01, 0x02, 0xFF, 0xFE])).unwrap();
    assert_eq!((s.format, s.channels, s.rate_hz, s.pcm), (AudioFormat::S16Le, 1, 22254, vec![0x02, 0x01, 0xFE, 0xFF]));
    let s = none(&cmp_header(b"sowt", -1, 16, 2, 1, &[0x01, 0x02, 0x03, 0x04])).unwrap();
    assert_eq!((s.format, s.channels, s.pcm), (AudioFormat::S16Le, 2, vec![0x01, 0x02, 0x03, 0x04]));
    let s = none(&cmp_header(b"raw ", -1, 8, 1, 3, &[0x80, 0x90, 0x70])).unwrap();
    assert_eq!((s.format, s.pcm), (AudioFormat::U8, vec![0x80, 0x90, 0x70]));
    let s = none(&cmp_header(b"NONE", 0, 16, 1, 1, &[0x00, 0x10])).unwrap();
    assert_eq!((s.format, s.pcm), (AudioFormat::S16Le, vec![0x10, 0x00]));
    // NEGATIVE CONTROLS, by name: MACE 3:1 / 6:1 (compressionID 3 / 4), IMA 4:1, u-law; and samples that run past the resource
    for (fmt, id, name) in [(b"MAC3", 3, "MACE 3:1"), (b"MAC6", 4, "MACE 6:1"), (b"ima4", -1, "ima4"), (b"ulaw", -1, "ulaw")] {
        let e = none(&cmp_header(fmt, id, 8, 1, 4, &[0; 4])).unwrap_err().0;
        assert!(e.contains(name) && e.contains("compressed"), "{name}: {e}");
    }
    let e = none(&cmp_header(b"twos", -1, 16, 1, 100, &[0; 4])).unwrap_err().0;
    assert!(e.contains("run past"), "{e}");
    let e = none(&cmp_header(b"twos", -1, 16, 3, 1, &[0; 6])).unwrap_err().0;
    assert!(e.contains("3 channels"), "{e}");
}

#[test]
fn a_compressed_pcm_header_plays_through_bufferCmd_and_a_codec_one_is_refused_in_the_trap() {
    let mut g = Rig::new();
    go(&mut g, |p| { p.clr_w().l(CHAN).w(5).l(0).l(0).trap(0xA807).pop_w(OUT); });
    let play = |g: &mut Rig, h: &[u8]| {
        g.m.mem.write_bytes(HDR, h).unwrap();
        g.m.mem.write_u16(CMD, 81).unwrap(); g.m.mem.write_u16(CMD + 2, 0).unwrap(); g.m.mem.write_u32(CMD + 4, HDR).unwrap();
        let chan = g.m.mem.read_u32(CHAN).unwrap();
        go(g, |p| { p.clr_w().l(chan).l(CMD).b(0).trap(0xA803).pop_w(OUT + 2); });
    };
    play(&mut g, &cmp_header(b"twos", -1, 16, 1, 2, &[0x01, 0x02, 0x03, 0x04]));
    g.no_refusals();
    assert_eq!((word(&g, OUT + 2), queued(&g)), (0, vec![0x02, 0x01, 0x04, 0x03]));
    play(&mut g, &cmp_header(b"MAC3", 3, 8, 1, 4, &[0; 4]));
    assert!(g.m.refused.iter().any(|r| r.contains("MACE 3:1")), "{:?}", g.m.refused);
}

#[test]
fn the_legacy_speaker_volume_is_sdvolume_and_scales_the_default_output_gain() {
    let mut g = Rig::new();
    // SetSoundVol / GetSoundVol are Pascal glue in InterfaceLib, not traps: the engine exposes them to the PowerPC import and to hosts.
    assert_eq!(g.m.get_sound_vol(), 7, "a fresh machine is at full volume");
    assert_eq!(g.m.mem.read_u8(0x260).unwrap() & 7, 7, "SdVolume is the same byte");
    g.m.set_sound_vol(3).unwrap();
    assert_eq!((g.m.get_sound_vol(), g.m.mem.read_u8(0x260).unwrap() & 7), (3, 3));
    g.m.set_sound_vol(99).unwrap();
    assert_eq!(g.m.get_sound_vol(), 7, "a level above 7 is clamped to the top of the 0-7 scale");
    // the level reaches the host: a channel's stream gets level/7 of unity per side
    go(&mut g, |p| { p.clr_w().l(CHAN).w(5).l(0).l(0).trap(0xA807).pop_w(OUT); });
    g.m.ui.calls.clear();
    g.m.set_sound_vol(0).unwrap();
    let vols: Vec<_> = g.m.ui.calls.iter().filter_map(|c| match c { guestui_abi::UiCall::SetAudioVolume(_, l, r) => Some((*l, *r)), _ => None }).collect();
    assert_eq!(vols, vec![(0, 0)], "level 0 silences the open stream");
}
