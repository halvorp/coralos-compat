//! AmigaOS `audio.device` (Paula 4-channel 8-bit PCM) projected onto `GuestUi` audio streams.
//!
//! `struct IOAudio` (`devices/audio.h`):
//! - `0..32`: `struct IORequest` (`io_Device` 20, `io_Unit` 24, `io_Command` 28, `io_Flags` 30, `io_Error` 31)
//! - `32`: `WORD ioa_AllocKey`
//! - `34`: `UBYTE *ioa_Data`
//! - `38`: `ULONG ioa_Length`
//! - `42`: `UWORD ioa_Period` (Paula PAL clock 3_546_895 Hz / period)
//! - `44`: `UWORD ioa_Volume` (0..=64)
//! - `46`: `UWORD ioa_Cycles` (0 = loop forever, N = repeat N times)
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{AudioFormat, AudioStreamModel, AudioStreamRef, GuestUi};
use crate::cpu::Cpu;
use crate::{machine::Amiga, Refusal};

pub const NAME: &str = "audio.device";

const LVO_BEGIN_IO: i32 = -30;
const LVO_ABORT_IO: i32 = -36;

const IO_UNIT: u32 = 24;
const IO_COMMAND: u32 = 28;
const IO_ERROR: u32 = 31;
const IOA_ALLOCKEY: u32 = 32;
const IOA_DATA: u32 = 34;
const IOA_LENGTH: u32 = 38;
const IOA_PERIOD: u32 = 42;
const IOA_VOLUME: u32 = 44;
const IOA_CYCLES: u32 = 46;

const CMD_RESET: u16 = 1;
const CMD_WRITE: u16 = 3;
const CMD_STOP: u16 = 6;
const CMD_START: u16 = 7;
const CMD_FLUSH: u16 = 8;
const ADCMD_FREE: u16 = 9;
const ADCMD_SETPREC: u16 = 10;
const ADCMD_FINISH: u16 = 11;
const ADCMD_PERVOL: u16 = 12;
const ADCMD_LOCK: u16 = 13;
const ADCMD_WAITCYCLE: u16 = 14;
const ADCMD_ALLOCATE: u16 = 32;

const ADIOERR_ALLOCFAILED: i8 = -11;
const IOERR_NOCMD: i8 = -3;
const PAL_CLOCK_HZ: u32 = 3_546_895;

#[derive(Default)]
pub struct AudioState {
    /// Bitmask of allocated Paula channels (bits 0..3).
    pub alloc_mask: u8,
    pub next_key: u16,
    /// Open `GuestUi` stream per Paula channel: `(stream_ref, rate_hz)`.
    pub channels: [Option<(AudioStreamRef, u32)>; 4],
}


fn allocate_channels<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<i8, Refusal> {
    let data = a.mem.read_u32(io + IOA_DATA).map_err(Refusal::mem("ioa_Data"))?;
    let len = a.mem.read_u32(io + IOA_LENGTH).map_err(Refusal::mem("ioa_Length"))?;
    if data == 0 || len == 0 {
        return Ok(0);
    }
    let mut chosen = 0u8;
    for i in 0..len.min(64) {
        let want = a.mem.read_u8(data + i).map_err(Refusal::mem("ioa_Data[i]"))? & 0x0f;
        if want != 0 && (a.audio.alloc_mask & want) == 0 {
            chosen = want;
            break;
        }
    }
    if chosen == 0 {
        a.mem.write_u32(io + IO_UNIT, 0).map_err(Refusal::mem("io_Unit"))?;
        return Ok(ADIOERR_ALLOCFAILED);
    }
    a.audio.alloc_mask |= chosen;
    let mut key = a.mem.read_u16(io + IOA_ALLOCKEY).map_err(Refusal::mem("ioa_AllocKey"))?;
    if key == 0 {
        a.audio.next_key = a.audio.next_key.wrapping_add(1).max(1);
        key = a.audio.next_key;
        a.mem.write_u16(io + IOA_ALLOCKEY, key).map_err(Refusal::mem("ioa_AllocKey"))?;
    }
    a.mem.write_u32(io + IO_UNIT, u32::from(chosen)).map_err(Refusal::mem("io_Unit"))?;
    Ok(0)
}

fn free_channels<U: GuestUi>(a: &mut Amiga<U>, mask: u8) {
    let m = mask & 0x0f;
    a.audio.alloc_mask &= !m;
    for ch in 0..4usize {
        if m & (1 << ch) != 0 {
            if let Some((s, _)) = a.audio.channels[ch].take() {
                let _ = a.ui.close_audio_stream(s);
            }
        }
    }
}

fn channel_stream<U: GuestUi>(a: &mut Amiga<U>, ch: usize, rate_hz: u32) -> Option<AudioStreamRef> {
    if let Some((s, r)) = a.audio.channels[ch] {
        if r == rate_hz {
            return Some(s);
        }
        let _ = a.ui.close_audio_stream(s);
        a.audio.channels[ch] = None;
    }
    let model = AudioStreamModel {
        rate_hz,
        channels: 1,
        format: AudioFormat::U8,
        label: format!("Paula ch{ch}"),
    };
    match a.ui.open_audio_stream(model) {
        Ok(s) => {
            a.audio.channels[ch] = Some((s, rate_hz));
            Some(s)
        }
        Err(_) => None,
    }
}

fn volume_to_u16(vol: u16) -> u16 {
    let v = u32::from(vol.min(64));
    ((v * 65535) / 64) as u16
}

/// Called from `OpenDevice("audio.device", ...)`: allocates channels immediately if `ioa_Length != 0`.
pub fn open<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<i8, Refusal> {
    allocate_channels(a, io)
}

/// Called from `CloseDevice`: releases any channels still owned by `io_Unit`.
pub fn close<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<(), Refusal> {
    let unit = a.mem.read_u32(io + IO_UNIT).map_err(Refusal::mem("io_Unit"))? as u8;
    free_channels(a, unit);
    Ok(())
}

/// Execute an `audio.device` IORequest and complete it cleanly (`Ok(true)` = completed immediately).
pub fn start<U: GuestUi>(a: &mut Amiga<U>, io: u32) -> Result<bool, Refusal> {
    let cmd = a.mem.read_u16(io + IO_COMMAND).map_err(Refusal::mem("io_Command"))?;
    let unit = (a.mem.read_u32(io + IO_UNIT).map_err(Refusal::mem("io_Unit"))? & 0x0f) as u8;
    let err = match cmd {
        ADCMD_ALLOCATE => allocate_channels(a, io)?,
        ADCMD_FREE => {
            free_channels(a, unit);
            a.mem.write_u32(io + IO_UNIT, 0).map_err(Refusal::mem("io_Unit"))?;
            0
        }
        CMD_WRITE => {
            let data = a.mem.read_u32(io + IOA_DATA).map_err(Refusal::mem("ioa_Data"))?;
            let len = a.mem.read_u32(io + IOA_LENGTH).map_err(Refusal::mem("ioa_Length"))?;
            let period = a.mem.read_u16(io + IOA_PERIOD).map_err(Refusal::mem("ioa_Period"))?.max(124);
            let vol = a.mem.read_u16(io + IOA_VOLUME).map_err(Refusal::mem("ioa_Volume"))?;
            let cycles = a.mem.read_u16(io + IOA_CYCLES).map_err(Refusal::mem("ioa_Cycles"))?;
            let rate_hz = (PAL_CLOCK_HZ / u32::from(period)).clamp(4_000, 48_000);
            if data != 0 && len > 0 {
                let n = (len as usize).min(262_144);
                let mut pcm = Vec::with_capacity(n);
                for i in 0..n {
                    let s = a.mem.read_u8(data + i as u32).map_err(Refusal::mem("audio pcm"))?;
                    pcm.push(s ^ 0x80); // signed 8-bit Paula -> unsigned 8-bit GuestUi
                }
                let gvol = volume_to_u16(vol);
                for ch in 0..4usize {
                    if unit & (1 << ch) != 0 {
                        if let Some(s) = channel_stream(a, ch, rate_hz) {
                            let _ = a.ui.set_audio_volume(s, gvol, gvol);
                            if cycles == 0 {
                                let _ = a.ui.queue_audio_looped(s, &pcm, 0, pcm.len() as u32);
                            } else {
                                let _ = a.ui.queue_audio(s, &pcm);
                            }
                        }
                    }
                }
            }
            0
        }
        ADCMD_PERVOL => {
            let vol = a.mem.read_u16(io + IOA_VOLUME).map_err(Refusal::mem("ioa_Volume"))?;
            let gvol = volume_to_u16(vol);
            for ch in 0..4usize {
                if unit & (1 << ch) != 0 {
                    if let Some((s, _)) = a.audio.channels[ch] {
                        let _ = a.ui.set_audio_volume(s, gvol, gvol);
                    }
                }
            }
            0
        }
        CMD_FLUSH | ADCMD_FINISH | CMD_RESET => {
            for ch in 0..4usize {
                if unit & (1 << ch) != 0 {
                    if let Some((s, _)) = a.audio.channels[ch] {
                        let _ = a.ui.flush_audio(s);
                    }
                }
            }
            0
        }
        CMD_STOP => {
            for ch in 0..4usize {
                if unit & (1 << ch) != 0 {
                    if let Some((s, _)) = a.audio.channels[ch] {
                        let _ = a.ui.pause_audio(s, true);
                    }
                }
            }
            0
        }
        CMD_START => {
            for ch in 0..4usize {
                if unit & (1 << ch) != 0 {
                    if let Some((s, _)) = a.audio.channels[ch] {
                        let _ = a.ui.pause_audio(s, false);
                    }
                }
            }
            0
        }
        ADCMD_SETPREC | ADCMD_LOCK | ADCMD_WAITCYCLE => 0,
        _ => IOERR_NOCMD,
    };
    crate::timer::complete_io(a, io, err)?;
    Ok(true)
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.func(LVO_BEGIN_IO, "BeginIO", begin_io::<U>)
     .func(LVO_ABORT_IO, "AbortIO", abort_io::<U>)
}

fn begin_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    start(a, io)?;
    let err = a.mem.read_u8(io + IO_ERROR).map_err(Refusal::mem("io_Error"))? as i8;
    cpu.set_d0(err as i32 as u32);
    Ok(format!("[audio.device] BeginIO {io:#x} -> {err}"))
}

fn abort_io<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let io = cpu.areg(1);
    let unit = (a.mem.read_u32(io + IO_UNIT).map_err(Refusal::mem("io_Unit"))? & 0x0f) as u8;
    for ch in 0..4usize {
        if unit & (1 << ch) != 0 {
            if let Some((s, _)) = a.audio.channels[ch] {
                let _ = a.ui.flush_audio(s);
            }
        }
    }
    cpu.set_d0(0);
    Ok(format!("[audio.device] AbortIO {io:#x}"))
}
