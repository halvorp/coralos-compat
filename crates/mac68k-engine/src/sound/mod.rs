//! The Sound Manager, first slice (Inside Macintosh VI "Sound Manager"; Executor's MIT `sound.cpp` / `soundIMVI.cpp` for the
//! routine signatures and error codes): sampled sound on a channel, played through the AUDIO seam of `guestui-abi`.
//!
//! * **Channels** are real `SndChannel` records in guest memory (`callBack` 8, `qLength` 30, `qHead` 32, `qTail` 34, `queue` 36, 128
//!   commands of 8 bytes): a program that allocated its own record is believed, a NIL one gets a record from the engine. Commands
//!   run in order the moment they are given, so the guest queue is always empty: playback order is the order PCM is queued on
//!   the channel's stream, which the HOST plays.
//! * **Streams** are the seam's ([`guestui_abi::AudioStreamModel`]): a channel opens one when its first sound has a format, and a
//!   sound of another rate / width / channel count opens another (idle ones are closed first). The engine decodes the `'snd '`
//!   resource and the sound header ([`snd`]) into PCM; it plays nothing itself. A host with no audio service refuses
//!   `open_audio_stream`, and then `SndNewChannel` answers `notEnoughHardware` (-201), as a Mac without sound hardware does.
//! * **Traps**: `SndNewChannel` ($A807), `SndDisposeChannel` ($A801), `SndAddModifier` ($A802, refused: no modifier code is
//!   modelled), `SndDoCommand` ($A803), `SndDoImmediate` ($A804), `SndPlay` ($A805), `SndControl` ($A806, refused), `SysBeep` ($A9C8)
//!   and `SoundDispatch` ($A800, [`dispatch`]).
//! * **Commands**: `nullCmd`, `quietCmd`, `flushCmd`, `pauseCmd`, `resumeCmd`, `rateCmd`, `volumeCmd`, `soundCmd`, `bufferCmd`.
//!   Every other command is refused BY NAME, none succeeds silently. NAMED LIMITS: `syncCmd` / `waitCmd` (a timed wait) are refused (`callBackCmd` runs the channel's routine: callback.rs); a looped sound (`loopEnd - loopStart > 1`) is refused; compressed (MACE /
//!   IMA) sound headers are refused; only the sampled synthesizer (and "any", 0) exists, the square-wave, wave-table and note
//!   synthesizers answer `notEnoughHardware`; a synchronous `SndPlay` (`async` FALSE) returns when the sound is QUEUED, not when it has
//!   played (the engine is not real-time: `SndChannelStatus` says `scChannelBusy` from the host's own queue length).
//! * **`SndDisposeChannel(.., quietNow = FALSE)`** lets what is queued play out: its streams go on a draining list the next sound call
//!   reaps once the host reports them idle. `quietNow` TRUE flushes first.
pub mod callback;
pub mod dispatch;
pub mod snd;

use crate::machine::{Call, Done, Mac, Regs};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{AudioFormat, AudioStreamModel, AudioStreamRef, GuestUi, UiRefusal, AUDIO_UNITY};
use retro_m68k::Cpu;
use snd::Cmd;

pub const NO_ERR: i16 = 0;
pub const NOT_ENOUGH_HARDWARE: i16 = -201;
pub const BAD_CHANNEL: i16 = -205;
pub const BAD_FORMAT: i16 = -206;
pub const PARAM_ERR: i16 = -50;
pub const SAMPLED_SYNTH: i16 = 5;

pub const SC_CALL_BACK: u32 = 8;
pub const SC_FLAGS: u32 = 28;
pub const SC_Q_LENGTH: u32 = 30;
pub const SC_Q_HEAD: u32 = 32;
pub const SC_Q_TAIL: u32 = 34;
pub const SC_QUEUE: u32 = 36;
pub const STD_Q_LENGTH: u16 = 128;
pub const SND_CHANNEL_SIZE: u32 = SC_QUEUE + 8 * STD_Q_LENGTH as u32;
/// The channel is the engine's: `SndDisposeChannel` frees the record.
const FLAG_ENGINE_ALLOCATED: u16 = 0x4000;

/// The default volume the engine reports: both channels at unity ([`AUDIO_UNITY`] in each half).
pub const FULL_VOLUME: u32 = (AUDIO_UNITY as u32) << 16 | AUDIO_UNITY as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fmt { rate: u32, channels: u8, format: AudioFormat }

#[derive(Debug, Clone, Copy)]
struct Stream { r: AudioStreamRef, fmt: Fmt }

#[derive(Debug, Default)]
struct Chan { streams: Vec<Stream>, paused: bool, volume: Option<(u16, u16)>, rate: Option<u32> }

/// What the engine keeps beside the guest's `SndChannel` records.
#[derive(Default)]
pub struct SoundState {
    chans: BTreeMap<u32, Chan>,
    /// Streams whose channel was disposed with sound still queued: closed once the host says they are idle.
    draining: Vec<AudioStreamRef>,
    beep: Option<AudioStreamRef>,
    /// `Some` once a program set them; `None` reads as [`FULL_VOLUME`].
    sys_beep_volume: Option<u32>,
    default_volume: Option<u32>,
    beep_state: Option<u16>,
    next_temp: u32,
    /// Bytes ever queued per stream id (with the host's unplayed count this says how much has PLAYED; callback.rs).
    queued_total: BTreeMap<u32, u64>,
    pending: Vec<callback::Pending>,
    in_callback: bool,
}

impl SoundState {
    pub fn channels(&self) -> usize { self.chans.len() }
    pub fn master(&self) -> u32 { self.default_volume.unwrap_or(FULL_VOLUME) }
}

impl<U: GuestUi> Mac<U> {
    fn snd_open(&mut self, f: Fmt, label: &str) -> Result<AudioStreamRef, UiRefusal> {
        self.ui.open_audio_stream(AudioStreamModel { rate_hz: f.rate, channels: f.channels, format: f.format, label: String::from(label) })
    }

    /// Close the draining streams the host has finished with.
    fn snd_reap(&mut self) {
        let mut keep = Vec::new();
        for s in core::mem::take(&mut self.snd.draining) {
            match self.ui.audio_queued_bytes(s) {
                Ok(0) | Err(_) => { let _ = self.ui.close_audio_stream(s); }
                Ok(_) => keep.push(s),
            }
        }
        self.snd.draining = keep;
    }

    /// The gain of a channel's streams: the channel's own `volumeCmd` times the system default output volume, per side.
    fn snd_apply_volume(&mut self, key: u32) -> Result<(), UiRefusal> {
        let master = self.snd.master();
        let Some(c) = self.snd.chans.get(&key) else { return Ok(()) };
        let (l, r) = c.volume.unwrap_or((AUDIO_UNITY, AUDIO_UNITY));
        let gain = |own: u16, m: u32| ((u32::from(own) * (m & 0xFFFF).min(u32::from(AUDIO_UNITY))) / u32::from(AUDIO_UNITY)) as u16;
        let (gl, gr) = (gain(l, master & 0xFFFF), gain(r, master >> 16));
        let streams: Vec<AudioStreamRef> = c.streams.iter().map(|s| s.r).collect();
        for s in streams { self.ui.set_audio_volume(s, gl, gr)?; }
        Ok(())
    }

    /// A stream of format `f` on channel `key`: the channel's own if it has one, else a new one (the idle ones of other formats are closed).
    fn snd_stream_for(&mut self, key: u32, f: Fmt) -> Result<AudioStreamRef, UiRefusal> {
        if let Some(s) = self.snd.chans.get(&key).and_then(|c| c.streams.iter().find(|s| s.fmt == f)) { return Ok(s.r); }
        let existing: Vec<Stream> = self.snd.chans.get(&key).map(|c| c.streams.clone()).unwrap_or_default();
        let mut keep = Vec::new();
        for s in existing {
            if self.ui.audio_queued_bytes(s.r).map_or(true, |n| n == 0) { let _ = self.ui.close_audio_stream(s.r); } else { keep.push(s); }
        }
        let r = self.snd_open(f, "Sound Manager channel")?;
        keep.push(Stream { r, fmt: f });
        if let Some(c) = self.snd.chans.get_mut(&key) { c.streams = keep; }
        self.snd_apply_volume(key)?;
        Ok(r)
    }

    /// Queue a decoded sound on channel `key`: the OSErr and the log text.
    fn snd_play_sound(&mut self, key: u32, s: snd::Sound) -> Result<(i16, String), Refusal> {
        if s.looped { return Err(Refusal(String::from("a looped sound (loopEnd - loopStart > 1) is not played: no continuous feed is modelled"))); }
        let rate = self.snd.chans.get(&key).and_then(|c| c.rate).unwrap_or(s.rate_hz);
        let f = Fmt { rate, channels: s.channels, format: s.format };
        let stream = match self.snd_stream_for(key, f) {
            Ok(r) => r,
            Err(UiRefusal::Unsupported(w)) => return Ok((NOT_ENOUGH_HARDWARE, format!("no audio out in this host ({w}): notEnoughHardware"))),
            Err(e) => return Err(Refusal(format!("the host refused the audio stream: {e:?}"))),
        };
        let frame = (s.channels as usize) * s.format.bytes_per_sample() as usize;
        let whole = s.pcm.len() / frame * frame;
        self.ui.queue_audio(stream, &s.pcm[..whole]).map_err(|e| Refusal(format!("the host refused queued audio: {e:?}")))?;
        *self.snd.queued_total.entry(stream.id).or_insert(0) += whole as u64;
        Ok((NO_ERR, format!("{} Hz {}ch {:?}, {} bytes queued", rate, s.channels, s.format, whole)))
    }

    /// The header at `addr` and its samples, read from guest memory (sized from the header itself).
    fn snd_read_header(&self, addr: u32) -> Result<snd::Sound, Refusal> {
        let e = Refusal::mem("sound header");
        let head = self.mem.bytes(self.mem.span(addr, 64.min((self.mem.len() as u32).saturating_sub(self.mem.strip(addr)))).map_err(&e)?).map_err(&e)?.to_vec();
        let total = match head.get(20) {
            Some(0xFF) => {
                let (ch, frames, size) = (u32::from_be_bytes([head[4], head[5], head[6], head[7]]), u32::from_be_bytes([head[22], head[23], head[24], head[25]]), u32::from(u16::from_be_bytes([head[48], head[49]])));
                64 + ch.saturating_mul(frames).saturating_mul(size / 8) as usize
            }
            Some(_) => 22 + u32::from_be_bytes([head[4], head[5], head[6], head[7]]) as usize,
            None => return Err(Refusal(String::from("sound header: shorter than its encode byte"))),
        };
        let bytes = self.mem.bytes(self.mem.span(addr, total.min((self.mem.len() as u32).saturating_sub(self.mem.strip(addr)) as usize) as u32).map_err(&e)?).map_err(&e)?.to_vec();
        snd::decode_header(&bytes, |p, n| self.mem.span(p, n).ok().and_then(|s| self.mem.bytes(s).ok()).map(<[u8]>::to_vec))
    }

    fn snd_key(&self, chan: u32) -> Option<u32> { self.snd.chans.contains_key(&chan).then_some(chan) }

    /// Run one command on channel `key`. Err = refused by name (the caller turns it into the trap's refusal).
    fn snd_exec(&mut self, key: u32, c: Cmd) -> Result<(i16, String), Refusal> {
        let ui_err = |e: UiRefusal| Refusal(format!("the host refused the audio call: {e:?}"));
        let streams = |m: &Self| m.snd.chans.get(&key).map(|c| c.streams.iter().map(|s| s.r).collect::<Vec<_>>()).unwrap_or_default();
        match c.cmd {
            snd::NULL_CMD => Ok((NO_ERR, String::from("nullCmd"))),
            snd::QUIET_CMD | snd::FLUSH_CMD => {
                for s in streams(self) { self.ui.flush_audio(s).map_err(ui_err)?; }
                Ok((NO_ERR, format!("{} (queued sound dropped)", if c.cmd == snd::QUIET_CMD { "quietCmd" } else { "flushCmd" })))
            }
            snd::PAUSE_CMD | snd::RESUME_CMD => {
                let on = c.cmd == snd::PAUSE_CMD;
                if let Some(ch) = self.snd.chans.get_mut(&key) { ch.paused = on; }
                for s in streams(self) { self.ui.pause_audio(s, on).map_err(ui_err)?; }
                Ok((NO_ERR, String::from(if on { "pauseCmd" } else { "resumeCmd" })))
            }
            snd::RATE_CMD => {
                let hz = snd::hz(c.param2);
                if let Some(ch) = self.snd.chans.get_mut(&key) { ch.rate = (hz != 0).then_some(hz); }
                Ok((NO_ERR, format!("rateCmd {hz} Hz for the next sounds")))
            }
            snd::VOLUME_CMD => {
                let (l, r) = ((c.param2 & 0xFFFF) as u16, (c.param2 >> 16) as u16);
                let (l, r) = (l.min(AUDIO_UNITY), r.min(AUDIO_UNITY));
                if let Some(ch) = self.snd.chans.get_mut(&key) { ch.volume = Some((l, r)); }
                self.snd_apply_volume(key).map_err(ui_err)?;
                Ok((NO_ERR, format!("volumeCmd left {l:#x} right {r:#x}")))
            }
            snd::SOUND_CMD | snd::BUFFER_CMD => {
                let s = self.snd_read_header(c.param2)?;
                let (err, text) = self.snd_play_sound(key, s)?;
                Ok((err, format!("{} {text}", if c.cmd == snd::SOUND_CMD { "soundCmd" } else { "bufferCmd" })))
            }
            snd::CALL_BACK_CMD => self.snd_queue_callback(key, c),
            other => Err(Refusal(format!("{} (command {other}) is not implemented", cmd_name(other)))),
        }
    }
}

fn cmd_name(c: u16) -> &'static str {
    match c {
        snd::WAIT_CMD => "waitCmd", snd::CALL_BACK_CMD => "callBackCmd", snd::SYNC_CMD => "syncCmd", snd::RE_INIT_CMD => "reInitCmd",
        snd::AVAILABLE_CMD => "availableCmd", snd::VERSION_CMD => "versionCmd", snd::GET_VOLUME_CMD => "getVolumeCmd", snd::GET_RATE_CMD => "getRateCmd",
        26 => "totalLoadCmd", 27 => "loadCmd", 40 => "freqDurationCmd", 41 => "restCmd", 42 => "freqCmd", 43 => "ampCmd", 44 => "timbreCmd", 45 => "getAmpCmd",
        60 => "waveTableCmd", 61 => "phaseCmd", 20 => "initCmd", 21 => "freeCmd", 22 => "totalLoad", 83 => "bufferCmd-related", _ => "an unnamed command",
    }
}

fn read_cmd<U: GuestUi>(m: &Mac<U>, at: u32) -> Result<Cmd, Refusal> {
    let e = Refusal::mem("SndCommand");
    Ok(Cmd { cmd: m.mem.read_u16(at).map_err(&e)?, param1: m.mem.read_i16(at + 2).map_err(&e)?, param2: m.mem.read_u32(at + 4).map_err(&e)? })
}

fn err_done(err: i16, log: String) -> Result<Done, Refusal> { Ok(Done { result: err as u16 as u32, log: format!("[sound] {log}") }) }

/// `FUNCTION SndNewChannel(VAR chan: SndChannelPtr; synth: INTEGER; init: LONGINT; userRoutine: SndCallBackProcPtr): OSErr`.
fn snd_new_channel<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.snd_reap();
    let (pp, synth, init, user) = (c.args.get(0), c.args.get(1) as i16, c.args.get(2), c.args.get(3));
    if synth != 0 && synth != SAMPLED_SYNTH {
        return err_done(NOT_ENOUGH_HARDWARE, format!("SndNewChannel synth {synth} (init {init:#x}): only the sampled synthesizer exists here (notEnoughHardware)"));
    }
    // does the host have an audio service? Open the channel's first stream at the Mac's usual rate and see.
    let f = Fmt { rate: 22_254, channels: 1, format: AudioFormat::U8 };
    let first = match m.snd_open(f, "Sound Manager channel") {
        Ok(r) => r,
        Err(UiRefusal::Unsupported(w)) => return err_done(NOT_ENOUGH_HARDWARE, format!("SndNewChannel: no audio out in this host ({w}): notEnoughHardware")),
        Err(e) => return Err(Refusal(format!("SndNewChannel: the host refused the audio stream: {e:?}"))),
    };
    let mut p = m.mem.read_u32(pp).map_err(Refusal::mem("SndChannelPtr"))?;
    let mut flags = 0u16;
    if p == 0 {
        p = m.mm.new_ptr(&mut m.mem, &mut m.heap, SND_CHANNEL_SIZE, true).map_err(|e| Refusal(format!("SndNewChannel: no memory for the channel (memFullErr {e})")))?;
        m.mem.write_u32(pp, p).map_err(Refusal::mem("SndChannelPtr"))?;
        flags = FLAG_ENGINE_ALLOCATED;
    }
    let e = Refusal::mem("SndChannel");
    m.mem.write_u32(p + SC_CALL_BACK, user).map_err(&e)?;
    m.mem.write_u16(p + SC_FLAGS, flags).map_err(&e)?;
    m.mem.write_u16(p + SC_Q_LENGTH, STD_Q_LENGTH).map_err(&e)?;
    m.mem.write_u16(p + SC_Q_HEAD, 0).map_err(&e)?;
    m.mem.write_u16(p + SC_Q_TAIL, 0).map_err(&e)?;
    m.snd.chans.insert(p, Chan { streams: alloc::vec![Stream { r: first, fmt: f }], ..Chan::default() });
    m.snd_apply_volume(p).map_err(|e| Refusal(format!("SndNewChannel: {e:?}")))?;
    err_done(NO_ERR, format!("SndNewChannel {p:#x} synth {synth}{}", if user != 0 { " (its callBack routine runs for each callBackCmd, after the sound queued before it has played)" } else { "" }))
}

/// `FUNCTION SndDisposeChannel(chan: SndChannelPtr; quietNow: BOOLEAN): OSErr`.
fn snd_dispose_channel<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.snd_reap();
    let (p, quiet) = (c.args.get(0), c.args.get(1) as u8 != 0);
    let Some(ch) = m.snd.chans.remove(&p) else { return err_done(BAD_CHANNEL, format!("SndDisposeChannel {p:#x}: not a channel (badChannel)")) };
    let mut draining = 0;
    for s in ch.streams {
        if quiet { let _ = m.ui.flush_audio(s.r); }
        if !quiet && m.ui.audio_queued_bytes(s.r).is_ok_and(|n| n > 0) { m.snd.draining.push(s.r); draining += 1; } else { let _ = m.ui.close_audio_stream(s.r); }
    }
    let flags = m.mem.read_u16(p + SC_FLAGS).unwrap_or(0);
    if flags & FLAG_ENGINE_ALLOCATED != 0 { let _ = m.mm.dispose_ptr(&mut m.heap, p); }
    err_done(NO_ERR, format!("SndDisposeChannel {p:#x} quietNow {quiet}{}", if draining > 0 { format!(" ({draining} stream(s) play out)") } else { String::new() }))
}

fn do_command<U: GuestUi>(m: &mut Mac<U>, chan: u32, cmd_at: u32, name: &str) -> Result<Done, Refusal> {
    m.snd_reap();
    let Some(key) = m.snd_key(chan) else { return err_done(BAD_CHANNEL, format!("{name} {chan:#x}: not a channel (badChannel)")) };
    let cmd = read_cmd(m, cmd_at)?;
    let (err, text) = m.snd_exec(key, cmd)?;
    err_done(err, format!("{name} {chan:#x}: {text}"))
}

/// `FUNCTION SndDoCommand(chan; cmd: SndCommand; noWait: BOOLEAN): OSErr`: commands run as they are given (see the module note).
fn snd_do_command<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { do_command(m, c.args.get(0), c.args.get(1), "SndDoCommand") }
/// `FUNCTION SndDoImmediate(chan; cmd: SndCommand): OSErr`.
fn snd_do_immediate<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { do_command(m, c.args.get(0), c.args.get(1), "SndDoImmediate") }

/// `FUNCTION SndPlay(chan: SndChannelPtr; sndHdl: Handle; async: BOOLEAN): OSErr`: every command of the `'snd '` resource, in order.
fn snd_play<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.snd_reap();
    let (chan, h, async_) = (c.args.get(0), c.args.get(1), c.args.get(2) as u8 != 0);
    let blk = m.mm.block(h).ok_or_else(|| Refusal(format!("SndPlay: {h:#x} is not a live handle")))?;
    let (base, size) = (blk.ptr, blk.size);
    let body = m.mem.bytes(m.mem.span(base, size).map_err(Refusal::mem("'snd '"))?).map_err(Refusal::mem("'snd '"))?.to_vec();
    let cmds = snd::parse_snd(&body)?;
    let (key, temp) = if chan == 0 {
        // a NIL channel: the Sound Manager makes one for this sound and disposes it afterwards (what is queued still plays)
        m.snd.next_temp += 1;
        let key = 0xFFFF_0000 + m.snd.next_temp;
        m.snd.chans.insert(key, Chan::default());
        (key, true)
    } else {
        match m.snd_key(chan) { Some(k) => (k, false), None => return err_done(BAD_CHANNEL, format!("SndPlay {chan:#x}: not a channel (badChannel)")) }
    };
    let mut log = String::new();
    let mut result = Ok(NO_ERR);
    for (mut cmd, by_offset) in cmds {
        if by_offset { cmd.param2 = base.wrapping_add(cmd.param2); }
        match m.snd_exec(key, cmd) {
            Ok((NO_ERR, t)) => { log.push_str(&t); log.push_str("; "); }
            Ok((err, t)) => { log.push_str(&t); result = Ok(err); break; }
            Err(e) => { result = Err(e); break; }
        }
    }
    if temp {
        if let Some(ch) = m.snd.chans.remove(&key) { for s in ch.streams { m.snd.draining.push(s.r); } }
    }
    let err = result?;
    err_done(err, format!("SndPlay {chan:#x} {size}-byte 'snd ': {log}{}", if async_ { "" } else { "(synchronous: returns when the sound is queued, not when it has played)" }))
}

/// `FUNCTION SndAddModifier(...)` / `SndControl(...)`: no modifier or synthesizer resource code runs in the engine.
fn snd_unmodelled<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    Err(Refusal(format!("{} needs a modifier / synthesizer ('snth') code resource run as guest code, which is not modelled", c.name)))
}

/// `PROCEDURE SysBeep(duration: INTEGER)`: a short beep through a stream of its own, at the system beep volume; muted when the program
/// turned the beep off with `SndSetSysBeepState`. A host without audio is logged, not hidden (the call has no result).
fn sys_beep<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    m.snd_reap();
    if m.snd.beep_state.unwrap_or(1) == 0 { return Ok(Done { result: 0, log: String::from("[sound] SysBeep (the system beep is off)") }); }
    let f = Fmt { rate: 22_254, channels: 1, format: AudioFormat::U8 };
    let stream = match m.snd.beep {
        Some(s) => s,
        None => match m.snd_open(f, "System beep") {
            Ok(s) => { m.snd.beep = Some(s); s }
            Err(UiRefusal::Unsupported(w)) => return Ok(Done { result: 0, log: format!("[sound] SysBeep: no audio out in this host ({w}); the beep is silent") }),
            Err(e) => return Err(Refusal(format!("SysBeep: the host refused the audio stream: {e:?}"))),
        },
    };
    let v = m.snd.sys_beep_volume.unwrap_or(FULL_VOLUME);
    let (l, r) = (((v & 0xFFFF) as u16).min(AUDIO_UNITY), ((v >> 16) as u16).min(AUDIO_UNITY));
    m.ui.set_audio_volume(stream, l, r).map_err(|e| Refusal(format!("SysBeep: {e:?}")))?;
    m.ui.queue_audio(stream, &snd::beep(800, 150, 22_254)).map_err(|e| Refusal(format!("SysBeep: {e:?}")))?;
    Ok(Done { result: 0, log: format!("[sound] SysBeep (duration {}: ignored, as a System 7 Mac does) 800 Hz 150 ms", c.args.get(0) as i16) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA807, &[Arg::P, Arg::W, Arg::L, Arg::L], Some(Arg::W), snd_new_channel::<U>)?;
    m.register(0xA801, &[Arg::P, Arg::B], Some(Arg::W), snd_dispose_channel::<U>)?;
    m.register(0xA803, &[Arg::P, Arg::P, Arg::B], Some(Arg::W), snd_do_command::<U>)?;
    m.register(0xA804, &[Arg::P, Arg::P], Some(Arg::W), snd_do_immediate::<U>)?;
    m.register(0xA805, &[Arg::P, Arg::P, Arg::B], Some(Arg::W), snd_play::<U>)?;
    m.register(0xA802, &[Arg::P, Arg::L, Arg::W, Arg::L], Some(Arg::W), snd_unmodelled::<U>)?;
    m.register(0xA806, &[Arg::W, Arg::P], Some(Arg::W), snd_unmodelled::<U>)?;
    m.register(0xA9C8, &[Arg::W], None, sys_beep::<U>)?;
    m.register_os(0xA800, dispatch::sound_dispatch::<U>)
}

