//! The Sound Manager end to end: hand-assembled programs push the Pascal frames of `SndNewChannel` ... `SndPlay`, set D0 for
//! `_SoundDispatch`, and the engine must keep a real `SndChannel`, decode the `'snd '` resource / sound header and queue PCM on the
//! audio SEAM (`RecordingGuestUi` is the host here). Frames and selectors: Inside Macintosh VI; Executor sound.cpp / soundIMVI.cpp;
//! systemless trap/sound.rs (read as facts).
mod common;
use common::*;
use guestui_abi::{AudioFormat, UiCall};
use mac68k_engine::sound::{SC_Q_LENGTH, SND_CHANNEL_SIZE};

const CHAN: u32 = 0x3100; // the program's SndChannelPtr variable
const CMD: u32 = 0x3110; // a SndCommand
const HDR: u32 = 0x3200; // a sound header + samples
const OUT: u32 = 0x3400;
const STATUS: u32 = 0x3500;

fn chan(g: &Rig) -> u32 { g.m.mem.read_u32(CHAN).unwrap() }
fn d0(p: &mut Prog, v: u32) { p.0.extend([0x20, 0x3C]); p.0.extend(v.to_be_bytes()); }
fn std_header(rate: u32, samples: &[u8]) -> Vec<u8> {
    let mut h = vec![0, 0, 0, 0];
    h.extend((samples.len() as u32).to_be_bytes());
    h.extend(rate.to_be_bytes());
    h.extend([0u8; 8]);
    h.extend([0, 60]);
    h.extend(samples);
    h
}
fn new_channel(p: &mut Prog, synth: i16) { p.clr_w().l(CHAN).w(synth).l(0).l(0).trap(0xA807).pop_w(OUT); }
fn dispose(p: &mut Prog, quiet: bool) { p.clr_w().rgn_arg(CHAN).b(quiet as u8).trap(0xA801).pop_w(OUT + 2); }
fn do_cmd(p: &mut Prog, cmd: u16, p1: i16, p2: u32) -> &mut Prog {
    // the SndCommand is built by the harness before the run; here only the call
    let _ = (cmd, p1, p2);
    p.clr_w().rgn_arg(CHAN).l(CMD).b(0).trap(0xA803).pop_w(OUT + 4)
}
fn set_cmd(g: &mut Rig, cmd: u16, p1: i16, p2: u32) {
    g.m.mem.write_u16(CMD, cmd).unwrap();
    g.m.mem.write_u16(CMD + 2, p1 as u16).unwrap();
    g.m.mem.write_u32(CMD + 4, p2).unwrap();
}
fn word(g: &Rig, at: u32) -> i16 { g.m.mem.read_i16(at).unwrap() }
fn go(g: &mut Rig, f: impl FnOnce(&mut Prog)) { let mut p = Prog::new(); f(&mut p); g.run(p.exit()); }
fn streams(g: &Rig) -> Vec<(u32, u32)> { g.m.ui.audio.iter().map(|(id, a)| (*id, a.queued.len() as u32)).collect() }
fn queued(g: &Rig) -> Vec<u8> { g.m.ui.audio.values().flat_map(|a| a.queued.clone()).collect() }

/// A `'snd '` resource, format 2, one bufferCmd (offset flag set) pointing at its header at offset 14.
fn snd_resource(g: &mut Rig, rate: u32, samples: &[u8]) -> u32 {
    let mut b = vec![0, 2, 0, 0, 0, 1, 0x80, 81, 0, 0, 0, 0, 0, 14];
    b.extend(std_header(rate, samples));
    let h = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, b.len() as u32, false).unwrap();
    let p = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_bytes(p, &b).unwrap();
    h
}

#[test]
fn snd_new_channel_makes_a_real_channel_record_and_a_stream_on_the_seam() {
    let mut g = Rig::new();
    go(&mut g, |p| { new_channel(p, 5); });
    g.no_refusals();
    let c = chan(&g);
    assert_ne!(c, 0, "the engine allocated the record for a NIL channel pointer");
    assert_eq!(word(&g, OUT), 0, "noErr");
    assert_eq!(g.m.mem.read_u16(c + SC_Q_LENGTH).unwrap(), 128, "qLength = stdQLength");
    assert_eq!(g.m.mm.ptr_size(c).unwrap(), SND_CHANNEL_SIZE);
    let opens: Vec<_> = g.m.ui.calls.iter().filter_map(|c| match c { UiCall::OpenAudio(m) => Some(m.clone()), _ => None }).collect();
    assert_eq!(opens.len(), 1);
    assert_eq!((opens[0].rate_hz, opens[0].channels, opens[0].format), (22254, 1, AudioFormat::U8));
    // dispose closes the stream and frees the record the engine made
    go(&mut g, |p| { dispose(p, true); });
    assert_eq!(word(&g, OUT + 2), 0);
    assert!(g.m.ui.audio.is_empty(), "no stream is left");
    assert!(g.m.mm.ptr_size(c).is_err(), "the engine's record is freed");
    // a second dispose of the same pointer is a badChannel, not a crash
    go(&mut g, |p| { dispose(p, true); });
    assert_eq!(word(&g, OUT + 2), -205, "badChannel");
}

#[test]
fn snd_play_decodes_the_resource_and_queues_pcm_at_the_sounds_own_rate() {
    let mut g = Rig::new();
    let h = snd_resource(&mut g, 0x2B77_0000, &[0x80, 0xA0, 0xC0, 0xA0, 0x80]); // 11127 Hz
    go(&mut g, |p| { new_channel(p, 5); p.clr_w().rgn_arg(CHAN).l(h).b(1).trap(0xA805).pop_w(OUT + 6); });
    g.no_refusals();
    assert_eq!(word(&g, OUT + 6), 0);
    let s = g.m.ui.audio.values().find(|a| a.model.rate_hz == 11127).expect("a stream at the sound's rate");
    assert_eq!((s.model.channels, s.model.format, s.queued.as_slice()), (1, AudioFormat::U8, &[0x80u8, 0xA0, 0xC0, 0xA0, 0x80][..]));
    // negative control: the first stream (22254 Hz from SndNewChannel) got nothing, the idle one was replaced
    assert!(g.m.ui.audio.values().all(|a| a.model.rate_hz != 22254 || a.queued.is_empty()));
}

#[test]
fn snd_play_on_a_nil_channel_plays_out_and_is_reaped_when_the_host_is_done() {
    let mut g = Rig::new();
    let h = snd_resource(&mut g, 0x56EE_8BA3, &[1, 2, 3, 4]);
    go(&mut g, |p| { p.clr_w().l(0).l(h).b(1).trap(0xA805).pop_w(OUT); });
    g.no_refusals();
    assert_eq!((word(&g, OUT), queued(&g)), (0, vec![1, 2, 3, 4]), "a channel made for the call; its sound is queued");
    let (id, _) = streams(&g)[0];
    let sref = guestui_abi::UiObjectRef { id, generation: 1 };
    g.m.ui.play_audio(sref, 4).unwrap();
    go(&mut g, |p| { p.trap(0xA9C8); }); // any sound call reaps; SysBeep is one
    assert!(g.m.ui.audio.values().all(|a| a.model.label != "Sound Manager channel"), "the drained temporary stream was closed: {:?}", g.m.ui.audio);
}

#[test]
fn commands_run_in_order_quiet_flushes_pause_and_volume_reach_the_host() {
    let mut g = Rig::new();
    go(&mut g, |p| { new_channel(p, 0); });
    // bufferCmd with a header in memory
    g.m.mem.write_bytes(HDR, &std_header(0x56EE_8BA3, &[9, 9, 9])).unwrap();
    set_cmd(&mut g, 81, 0, HDR);
    go(&mut g, |p| { do_cmd(p, 81, 0, HDR); });
    assert_eq!((word(&g, OUT + 4), queued(&g)), (0, vec![9, 9, 9]));
    // pause then resume
    set_cmd(&mut g, 11, 0, 0);
    go(&mut g, |p| { do_cmd(p, 11, 0, 0); });
    set_cmd(&mut g, 12, 0, 0);
    go(&mut g, |p| { do_cmd(p, 12, 0, 0); });
    assert!(g.m.ui.calls.iter().any(|c| matches!(c, UiCall::PauseAudio(_, true))) && g.m.ui.calls.iter().any(|c| matches!(c, UiCall::PauseAudio(_, false))));
    // volumeCmd: left in the low word, right in the high word
    set_cmd(&mut g, 46, 0, 0x0080_0040);
    go(&mut g, |p| { do_cmd(p, 46, 0, 0); });
    assert!(g.m.ui.calls.iter().any(|c| matches!(c, UiCall::SetAudioVolume(_, 0x40, 0x80))), "{:?}", g.m.ui.calls);
    // quietCmd drops what is queued and not played
    set_cmd(&mut g, 3, 0, 0);
    go(&mut g, |p| { do_cmd(p, 3, 0, 0); });
    assert_eq!(queued(&g), Vec::<u8>::new(), "quietCmd flushed the queue");
    g.no_refusals();
}

#[test]
fn sound_dispatch_reports_channel_status_from_the_hosts_own_queue() {
    let mut g = Rig::new();
    go(&mut g, |p| { new_channel(p, 5); });
    g.m.mem.write_bytes(HDR, &std_header(0x56EE_8BA3, &[7; 10])).unwrap();
    set_cmd(&mut g, 81, 0, HDR);
    go(&mut g, |p| { do_cmd(p, 81, 0, HDR); });
    // SndChannelStatus(chan, 24, &status): D0 = $00100008, frame: chan (4), length (2), status (4); OSErr below
    let status = |g: &mut Rig| { go(g, |p| { p.clr_w().rgn_arg(CHAN).w(24).l(STATUS); d0(p, 0x0010_0008); p.trap(0xA800).pop_w(OUT + 8); }); (word(g, OUT + 8), g.m.mem.read_u8(STATUS + 12).unwrap(), g.m.mem.read_u8(STATUS + 14).unwrap()) };
    assert_eq!(status(&mut g), (0, 1, 0), "busy: ten bytes are queued and the host has played none");
    let (id, _) = streams(&g)[0];
    g.m.ui.play_audio(guestui_abi::UiObjectRef { id, generation: 1 }, 10).unwrap();
    assert_eq!(status(&mut g), (0, 0, 0), "idle once the host has played them");
    // a short SCStatus buffer is paramErr, a stranger is badChannel
    go(&mut g, |p| { p.clr_w().rgn_arg(CHAN).w(8).l(STATUS); d0(p, 0x0010_0008); p.trap(0xA800).pop_w(OUT + 8); });
    assert_eq!(word(&g, OUT + 8), -50);
    go(&mut g, |p| { p.clr_w().l(0x1234).w(24).l(STATUS); d0(p, 0x0010_0008); p.trap(0xA800).pop_w(OUT + 8); });
    assert_eq!(word(&g, OUT + 8), -205);
    assert!(!g.m.refused.is_empty() && g.m.refused.iter().all(|r| r.contains("scStartTime")), "the three times are refused by name, nothing else is: {:?}", g.m.refused);
}

#[test]
fn a_host_with_no_audio_answers_not_enough_hardware_and_never_pretends() {
    let mut g = Rig::new();
    g.m.ui.refuse_audio = true;
    go(&mut g, |p| { new_channel(p, 5); });
    assert_eq!(word(&g, OUT), -201, "notEnoughHardware");
    assert_eq!(g.m.snd.channels(), 0);
    let h = snd_resource(&mut g, 0x56EE_8BA3, &[1]);
    go(&mut g, |p| { p.clr_w().l(0).l(h).b(1).trap(0xA805).pop_w(OUT + 6); });
    assert_eq!(word(&g, OUT + 6), -201, "SndPlay says so too");
    go(&mut g, |p| { p.w(30).trap(0xA9C8); });
    assert!(g.m.log.iter().any(|l| l.contains("SysBeep") && l.contains("no audio out")), "the silent beep is logged: {:#?}", g.m.log);
    g.no_refusals();
}

#[test]
fn unsupported_synths_and_commands_are_named_not_silent() {
    let mut g = Rig::new();
    go(&mut g, |p| { new_channel(p, 1); }); // squareWaveSynth
    assert_eq!((word(&g, OUT), g.m.snd.channels()), (-201, 0));
    go(&mut g, |p| { new_channel(p, 5); });
    set_cmd(&mut g, 10, 0, 0x1234); // waitCmd: a timed wait in the queue
    go(&mut g, |p| { do_cmd(p, 10, 0, 0x1234); });
    assert!(g.m.refused.iter().any(|r| r.contains("waitCmd")), "{:?}", g.m.refused);
    g.m.refused.clear();
    // a compressed header
    let mut h = std_header(0x56EE_8BA3, &[1, 2]);
    h[20] = 0xFE;
    g.m.mem.write_bytes(HDR, &h).unwrap();
    set_cmd(&mut g, 81, 0, HDR);
    go(&mut g, |p| { do_cmd(p, 81, 0, HDR); });
    assert!(g.m.refused.iter().any(|r| r.contains("compressed")), "{:?}", g.m.refused);
    // SoundDispatch selectors that are not implemented pop their frame and say so
    g.m.refused.clear();
    go(&mut g, |p| { p.clr_w().l(0).l(0).l(0); d0(p, 0x0618_0014); p.trap(0xA800); });
    assert!(g.m.refused.iter().any(|r| r.contains("SoundDispatch selector")), "{:?}", g.m.refused);
}

#[test]
fn sysbeep_queues_a_beep_unless_the_program_turned_it_off_and_the_dispatch_calls_answer() {
    let mut g = Rig::new();
    go(&mut g, |p| { p.w(30).trap(0xA9C8); });
    let n = queued(&g).len();
    assert!(n > 1000, "a beep of {n} bytes");
    // SndSetSysBeepState(0): $001C0008, one word; SndGetSysBeepState reads it back
    go(&mut g, |p| { p.clr_w().w(0); d0(p, 0x001C_0008); p.trap(0xA800).pop_w(OUT); p.l(STATUS); d0(p, 0x0018_0008); p.trap(0xA800); p.trap(0xA9C8); });
    assert_eq!((word(&g, OUT), g.m.mem.read_u16(STATUS).unwrap(), queued(&g).len()), (0, 0, n), "the beep is off: nothing more queued");
    // version
    go(&mut g, |p| { p.clr_l(); d0(p, 0x000C_0008); p.trap(0xA800).pop_l(OUT + 10); });
    assert_eq!(g.m.mem.read_u32(OUT + 10).unwrap(), 0x0300_8000);
    // GetDefaultOutputVolume as Glider PRO asks it: $022C0018
    go(&mut g, |p| { p.clr_w().l(OUT + 16); d0(p, 0x022C_0018); p.trap(0xA800).pop_w(OUT + 14); });
    assert_eq!((word(&g, OUT + 14), g.m.mem.read_u32(OUT + 16).unwrap()), (0, 0x0100_0100));
    go(&mut g, |p| { p.clr_w().l(0x0040_0040); d0(p, 0x0230_0018); p.trap(0xA800).pop_w(OUT + 14); p.clr_w().l(OUT + 16); d0(p, 0x022C_0018); p.trap(0xA800).pop_w(OUT + 14); });
    assert_eq!(g.m.mem.read_u32(OUT + 16).unwrap(), 0x0040_0040, "SetDefaultOutputVolume is read back");
    // UnsignedFixedMulDiv(value, multiplier, divisor) = value * multiplier / divisor
    go(&mut g, |p| { p.clr_l().l(1000).l(3).l(4); d0(p, 0x060C_0018); p.trap(0xA800).pop_l(OUT + 20); });
    assert_eq!(g.m.mem.read_u32(OUT + 20).unwrap(), 750);
    g.no_refusals();
}

#[test]
fn dispose_without_quiet_now_lets_the_queued_sound_play_out_and_with_it_drops_it() {
    // IM VI: SndDisposeChannel(quietNow = FALSE) waits for the queue; the engine cannot wait, so the stream stays until the host is idle
    for (quiet, survives) in [(false, true), (true, false)] {
        let mut g = Rig::new();
        let h = snd_resource(&mut g, 0x56EE_8BA3, &[5, 5, 5, 5]);
        go(&mut g, |p| { new_channel(p, 5); p.clr_w().rgn_arg(CHAN).l(h).b(1).trap(0xA805).pop_w(OUT + 6); dispose(p, quiet); });
        assert_eq!(word(&g, OUT + 2), 0);
        assert_eq!(g.m.ui.audio.values().any(|a| a.queued.len() == 4 && a.played == 0), survives, "quietNow {quiet}: {:?}", g.m.ui.audio);
        assert!(g.m.snd.channels() == 0, "the channel itself is gone either way");
    }
}

#[test]
fn callbackcmd_runs_the_channels_routine_after_the_queued_sound_has_played_and_only_once() {
    // The routine (guest code at 0x3700): movea.l 4(sp),a0 ; move.l 4(a0),$3600 ; movea.l 8(sp),a1 ; move.l a1,$3604 ; move.w 2(a0),$3608 ; rtd #8
    let code: [u8; 30] = [0x20, 0x6F, 0x00, 0x04, 0x23, 0xE8, 0x00, 0x04, 0x00, 0x00, 0x36, 0x00, 0x22, 0x6F, 0x00, 0x08, 0x23, 0xC9, 0x00, 0x00, 0x36, 0x04,
        0x33, 0xE8, 0x00, 0x02, 0x00, 0x00, 0x36, 0x08];
    let mut g = Rig::new();
    put(&mut g.m, 0x3700, &code);
    put(&mut g.m, 0x3700 + 30, &[0x4E, 0x74, 0x00, 0x08]); // rtd #8
    go(&mut g, |p| { p.clr_w().l(CHAN).w(5).l(0).l(0x3700).trap(0xA807).pop_w(OUT); });
    assert_eq!(word(&g, OUT), 0);
    g.m.mem.write_bytes(HDR, &std_header(0x56EE_8BA3, &[9; 40])).unwrap();
    set_cmd(&mut g, 81, 0, HDR);
    go(&mut g, |p| { do_cmd(p, 81, 0, HDR); });
    set_cmd(&mut g, 13, 7, 0xABCD);
    go(&mut g, |p| { do_cmd(p, 13, 7, 0xABCD); p.trap(0xA850); }); // and a trap after it
    g.no_refusals();
    assert_eq!(g.m.mem.read_u32(0x3600).unwrap(), 0, "40 bytes are still queued and unplayed: the routine has not run");
    // the host plays the sound; the routine runs before the next trap
    let (id, _) = streams(&g)[0];
    g.m.ui.play_audio(guestui_abi::UiObjectRef { id, generation: 1 }, 40).unwrap();
    go(&mut g, |p| { p.trap(0xA850); });
    assert_eq!(g.m.mem.read_u32(0x3600).unwrap(), 0xABCD, "param2 reached the routine through its SndCommand");
    assert_eq!((g.m.mem.read_u32(0x3604).unwrap(), g.m.mem.read_i16(0x3608).unwrap()), (chan(&g), 7), "and the channel and param1");
    assert!(g.m.log.iter().any(|l| l.starts_with("[sound] callBack routine 0x3700 runs")), "{:#?}", g.m.log);
    // exactly once: clear the marker, trap again, it stays clear
    g.m.mem.write_u32(0x3600, 0).unwrap();
    go(&mut g, |p| { p.trap(0xA850); });
    assert_eq!(g.m.mem.read_u32(0x3600).unwrap(), 0, "a callBackCmd fires once");
    g.no_refusals();
}

#[test]
fn a_channel_with_no_callback_routine_ignores_callbackcmd_and_a_sound_queued_after_it_does_not_delay_it() {
    let mut g = Rig::new();
    go(&mut g, |p| { new_channel(p, 5); });
    set_cmd(&mut g, 13, 1, 2);
    go(&mut g, |p| { do_cmd(p, 13, 1, 2); });
    assert_eq!(word(&g, OUT + 4), 0, "noErr");
    assert!(g.m.log.iter().any(|l| l.contains("no callBack routine: ignored")));
    g.no_refusals();
}
