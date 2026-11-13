//! The Sound Manager for PowerPC programs: the routines with no trap of their own (`SetSoundVol`, `GetSoundVol`, `SndChannelStatus`,
//! `GetSoundHeaderOffset`), `SysBeep` through the audio seam (it was logged only), and a channel's completion routine entered as PowerPC code at
//! the next safepoint, in the order the guest queued them. The engine behind all of it is mac68k-engine's `sound` module: one implementation.
use guestui_abi::{RecordingGuestUi, UiObjectRef};
use mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use retro_ppc::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const VAR: u32 = 0x3000;
const CMD: u32 = 0x3040;
const HDR: u32 = 0x3100;
const OUT: u32 = 0x3200;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind_from("InterfaceLib", name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind_from("InterfaceLib", name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
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
fn put_cmd(m: &mut M, at: u32, cmd: u16, p1: i16, p2: u32) {
    m.mac.mem.write_u16(at, cmd).unwrap(); m.mac.mem.write_u16(at + 2, p1 as u16).unwrap(); m.mac.mem.write_u32(at + 4, p2).unwrap();
}

#[test]
fn set_and_get_sound_vol_are_the_legacy_speaker_level_in_sdvolume() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "GetSoundVol", &[OUT]), 0);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 7, "a fresh machine is at full volume");
    call(&mut m, "SetSoundVol", &[3]);
    call(&mut m, "GetSoundVol", &[OUT]);
    assert_eq!((m.mac.mem.read_u16(OUT).unwrap(), m.mac.mem.read_u8(0x260).unwrap() & 7), (3, 3));
    // NEGATIVE: a level past the scale is clamped, and a NIL result pointer is not written through
    call(&mut m, "SetSoundVol", &[200]);
    call(&mut m, "GetSoundVol", &[OUT]);
    assert_eq!(m.mac.mem.read_u16(OUT).unwrap(), 7);
    call(&mut m, "GetSoundVol", &[0]);
}

#[test]
fn snd_channel_status_and_get_sound_header_offset_answer_a_powerpc_caller_with_osErr_in_r3() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "SndChannelStatus", &[0x1234, 24, OUT]), -205, "badChannel, sign-extended");
    let snd = [0, 2, 0, 0, 0, 1, 0x80, 81, 0, 0, 0, 0, 0, 14, 0, 0, 0, 0];
    let h = m.mac.mm.new_handle(&mut m.mac.mem, &mut m.mac.heap, snd.len() as u32, false).unwrap();
    let p = m.mac.mm.block(h).unwrap().ptr;
    m.mac.mem.write_bytes(p, &snd).unwrap();
    assert_eq!(call(&mut m, "GetSoundHeaderOffset", &[h, OUT]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 14);
    assert_eq!(call(&mut m, "GetSoundHeaderOffset", &[h, 0]), -50, "NEGATIVE: a NIL offset pointer is paramErr");
    // a live channel reports busy while the host holds unplayed bytes
    assert_eq!(call(&mut m, "SndNewChannel", &[VAR, 5, 0, 0]), 0);
    let chan = m.mac.mem.read_u32(VAR).unwrap();
    m.mac.mem.write_bytes(HDR, &std_header(0x56EE_8BA3, &[7; 10])).unwrap();
    put_cmd(&mut m, CMD, 81, 0, HDR);
    assert_eq!(call(&mut m, "SndDoCommand", &[chan, CMD, 0]), 0);
    assert_eq!(call(&mut m, "SndChannelStatus", &[chan, 24, OUT]), 0);
    assert_eq!(m.mac.mem.read_u8(OUT + 12).unwrap(), 1, "scChannelBusy");
}

#[test]
fn sysbeep_from_a_powerpc_program_is_played_through_the_audio_seam() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "SysBeep", &[30]);
    let queued: usize = m.mac.ui.audio.values().map(|a| a.queued.len()).sum();
    assert!(queued > 1000, "a beep of {queued} bytes reached the host");
    // NEGATIVE: a host with no audio is logged by name, never pretended
    let mut quiet = MacPpc::new(RecordingGuestUi::new()).unwrap();
    quiet.mac.ui.refuse_audio = true;
    call(&mut quiet, "SysBeep", &[30]);
    assert!(quiet.mac.log.iter().any(|l| l.contains("SysBeep") && l.contains("no audio out")), "{:#?}", quiet.mac.log);
}

/// A machine with a channel whose completion routine is the PowerPC procedure at `CODE`, and `n` bytes of sound followed by `callBackCmd`s.
fn channel_with_callbacks(callbacks: &[(i16, u32)]) -> (M, u32, UiObjectRef) { channel_with(callbacks, None) }

/// [`channel_with_callbacks`] with `routine` as the SndNewChannel `userRoutine` instead of a descriptor of the procedure at `CODE`.
fn channel_with(callbacks: &[(i16, u32)], routine: Option<u32>) -> (M, u32, UiObjectRef) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    let upp = new_routine_descriptor(&mut m.mac, TV, 0x2C0, ISA_PPC).unwrap();
    assert_eq!(call(&mut m, "SndNewChannel", &[VAR, 5, 0, routine.unwrap_or(upp)]), 0);
    let chan = m.mac.mem.read_u32(VAR).unwrap();
    m.mac.mem.write_bytes(HDR, &std_header(0x56EE_8BA3, &[7; 10])).unwrap();
    put_cmd(&mut m, CMD, 81, 0, HDR);
    assert_eq!(call(&mut m, "SndDoCommand", &[chan, CMD, 0]), 0);
    for &(p1, p2) in callbacks {
        put_cmd(&mut m, CMD, 13, p1, p2);
        assert_eq!(call(&mut m, "SndDoCommand", &[chan, CMD, 0]), 0);
    }
    let id = *m.mac.ui.audio.keys().next().expect("a stream");
    (m, chan, UiObjectRef { id, generation: 1 })
}

#[test]
fn a_callback_cmd_runs_the_powerpc_routine_at_a_safepoint_once_the_sound_has_played_in_queue_order() {
    let (mut m, chan, stream) = channel_with_callbacks(&[(11, 0xAAAA), (22, 0xBBBB)]);
    m.bind_from("InterfaceLib", "TickCount");
    let id = m.glue.intern(&mut m.mac, "CFM", "TickCount").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let fresh = || { let mut c = Cpu::new(0x5000, SP); c.gpr[2] = 0x6000; c.gpr[3] = 0x77; c };
    // the host has played nothing: the glue call is just the call
    let mut cpu = fresh();
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE, "not due while ten bytes are unplayed");
    m.mac.ui.play_audio(stream, 10).unwrap();
    // due: the first routine is entered with r3 = the channel, r4 = a SndCommand {callBackCmd, param1, param2}
    let mut cpu = fresh();
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, cpu.gpr[2], cpu.gpr[3]), (CODE, TOC, chan));
    let cmd = cpu.gpr[4];
    assert_eq!((m.mac.mem.read_u16(cmd).unwrap(), m.mac.mem.read_u16(cmd + 2).unwrap(), m.mac.mem.read_u32(cmd + 4).unwrap()), (13, 11, 0xAAAA));
    assert_eq!(m.callback_depth(), 1);
    // it returns: the interrupted call is entered again with its registers, and the record is freed
    let ret = cpu.lr;
    assert!(m.native(&mut cpu, ret));
    assert_eq!((cpu.pc, cpu.gpr[3], cpu.gpr[2]), (stub, 0x77, 0x6000));
    assert!(m.mac.mm.ptr_size(cmd).is_err(), "the SndCommand record is freed");
    // the second routine follows at the next safepoint, with ITS parameters; then nothing is left
    let mut cpu = fresh();
    assert!(m.native(&mut cpu, stub));
    assert_eq!((cpu.pc, m.mac.mem.read_u16(cpu.gpr[4] + 2).unwrap(), m.mac.mem.read_u32(cpu.gpr[4] + 4).unwrap()), (CODE, 22, 0xBBBB));
    let ret = cpu.lr;
    m.native(&mut cpu, ret);
    let mut cpu = fresh();
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE, "NEGATIVE: each completion runs exactly once");
}

#[test]
fn a_completion_routine_this_engine_cannot_run_is_refused_by_name_and_does_not_block_the_next() {
    // the channel's userRoutine is a raw pointer (not a routine descriptor)
    let (mut m, _, stream) = channel_with(&[(1, 0)], Some(TV));
    m.mac.ui.play_audio(stream, 10).unwrap();
    m.bind_from("InterfaceLib", "TickCount");
    let id = m.glue.intern(&mut m.mac, "CFM", "TickCount").unwrap();
    let stub = m.glue.entry(id).unwrap().stub;
    let mut cpu = Cpu::new(0x5000, SP);
    cpu.gpr[2] = 0x6000;
    m.native(&mut cpu, stub);
    assert_ne!(cpu.pc, CODE);
    assert!(m.mac.refused.iter().any(|r| r.contains("completion routine")), "{:?}", m.mac.refused);
    assert_eq!(m.callback_depth(), 0);
}
