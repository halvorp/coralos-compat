//! `_SoundDispatch` ($A800): the Sound Manager 3.0 routines past the original seven traps. The selector is in D0 (not on the stack):
//! bits 24-31 the parameter size in WORDS (the callee pops that many bytes), bits 16-23 the routine, bits 0-15 the family. The
//! routine byte alone is not unique (`SndSoundManagerVersion` and `UnsignedFixedMulDiv` are both $0C), so the parameter size is part
//! of the key; the three selectors Inside Macintosh VI lists with a zero size byte are tabled below. Facts: AMS `ams-snd`
//! `SoundDispatch.cc` (D0 low word = API, high word = call), systemless `trap/sound.rs` (the routine / frame table, read as facts).
//!
//! Implemented: `SndSoundManagerVersion` (reports 3.0, final), `UnsignedFixedMulDiv`, `SndChannelStatus`, `SndGetSysBeepState`,
//! `SndSetSysBeepState`, `GetSysBeepVolume`, `SetSysBeepVolume`, `GetDefaultOutputVolume`, `SetDefaultOutputVolume`. Any other selector pops
//! its frame (so the program's stack stays whole) and is refused by name: its OSErr slot is left untouched.
use super::*;

/// (selector with a zero size byte, parameter bytes) from Inside Macintosh VI-576.
fn zero_size_params(d0: u32) -> Option<u32> {
    match d0 { 0x0010_0008 => Some(10), 0x0014_0008 => Some(6), 0x0018_0008 => Some(4), 0x001C_0008 => Some(2), 0x0020_0008 => Some(8), _ => None }
}

fn name_of(routine: u32, words: u32) -> &'static str {
    match (routine, words) {
        (0x00, 13) => "SndStartFilePlay", (0x04, 2) => "SndPauseFilePlay", (0x08, 3) => "SndStopFilePlay", (0x10, _) => "SndChannelStatus", (0x14, _) => "SndManagerStatus",
        (0x20, _) => "SndPlayDoubleBuffer", (0x18, 5) => "SPBOpenDevice", (0x04, 4) => "GetSoundHeaderOffset", (0x08, 1) => "SndGetInfo/SndSetInfo family", _ => "an unnamed routine",
    }
}

pub fn sound_dispatch<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, regs: &mut Regs) -> Result<String, Refusal> {
    m.snd_reap();
    let (d0, sp) = (regs.d0, cpu.areg(7));
    let (words, routine, family) = (d0 >> 24, (d0 >> 16) & 0xFF, d0 & 0xFFFF);
    // the size byte counts words; the five selectors Inside Macintosh lists with a zero byte are tabled, any other zero is a routine with no parameters
    let params = zero_size_params(d0).unwrap_or(words * 2);
    let e = Refusal::mem("SoundDispatch frame");
    let arg32 = |m: &Mac<U>, off: u32| m.mem.read_u32(sp + off).map_err(Refusal::mem("SoundDispatch frame"));
    let ret_w = |m: &mut Mac<U>, cpu: &mut Cpu, v: u16| -> Result<(), Refusal> { m.mem.write_u16(sp + params, v).map_err(Refusal::mem("SoundDispatch result"))?; cpu.set_areg(7, sp + params); Ok(()) };
    let ret_l = |m: &mut Mac<U>, cpu: &mut Cpu, v: u32| -> Result<(), Refusal> { m.mem.write_u32(sp + params, v).map_err(Refusal::mem("SoundDispatch result"))?; cpu.set_areg(7, sp + params); Ok(()) };
    let text = match (routine, words, family) {
        (0x0C, 0, _) => { ret_l(m, cpu, 0x0300_8000)?; String::from("SndSoundManagerVersion -> 3.0 final") }
        (0x0C, 6, _) => {
            let (div, mul, val) = (arg32(m, 0)?, arg32(m, 4)?, arg32(m, 8)?);
            let r = if div == 0 { u32::MAX } else { ((u64::from(val) * u64::from(mul)) / u64::from(div)).min(u64::from(u32::MAX)) as u32 };
            ret_l(m, cpu, r)?;
            format!("UnsignedFixedMulDiv({val:#x}, {mul:#x}, {div:#x}) -> {r:#x}")
        }
        (0x10, _, 0x0008) => {
            let (status, len, chan) = (arg32(m, 0)?, m.mem.read_u16(sp + 4).map_err(&e)? as i16, arg32(m, 6)?);
            let err = match m.snd_key(chan) {
                None => BAD_CHANNEL,
                Some(_) if len < 24 => PARAM_ERR,
                Some(key) => {
                    let (streams, paused) = m.snd.chans.get(&key).map(|c| (c.streams.iter().map(|s| s.r).collect::<Vec<_>>(), c.paused)).unwrap_or_default();
                    let busy = streams.iter().any(|s| m.ui.audio_queued_bytes(*s).is_ok_and(|n| n > 0));
                    // SCStatus, 24 bytes: scStartTime, scEndTime, scCurrentTime, scChannelBusy, scChannelDisposed, scChannelPaused,
                    // scUnused, scChannelAttributes, scCPULoad. The three times are REFUSED BY NAME below: what clock they count and
                    // from when is not documented in a source we have, so the engine does not invent one; they read 0.
                    let mut rec = [0u8; 24];
                    rec[12] = u8::from(busy);
                    rec[14] = u8::from(paused);
                    m.mem.write_bytes(status, &rec).map_err(Refusal::mem("SCStatus"))?;
                    NO_ERR
                }
            };
            ret_w(m, cpu, err as u16)?;
            if err == NO_ERR { m.note_refusal("SndChannelStatus", String::from("scStartTime, scEndTime and scCurrentTime are not modelled (they read 0); scChannelBusy and scChannelPaused are real")); }
            format!("SndChannelStatus {chan:#x} -> {err}")
        }
        (0x18, 0, 0x0008) | (0x18, _, 0x0008) => {
            let p = arg32(m, 0)?;
            m.mem.write_u16(p, m.snd.beep_state.unwrap_or(1)).map_err(Refusal::mem("sysBeepState"))?;
            cpu.set_areg(7, sp + params);
            String::from("SndGetSysBeepState")
        }
        (0x1C, _, 0x0008) => {
            let st = m.mem.read_u16(sp).map_err(&e)?;
            m.snd.beep_state = Some(st);
            ret_w(m, cpu, NO_ERR as u16)?;
            format!("SndSetSysBeepState {st}")
        }
        (0x24, 2, _) | (0x2C, 2, _) => {
            let (p, sys) = (arg32(m, 0)?, routine == 0x24);
            let v = if sys { m.snd.sys_beep_volume } else { m.snd.default_volume }.unwrap_or(FULL_VOLUME);
            m.mem.write_u32(p, v).map_err(Refusal::mem("volume"))?;
            ret_w(m, cpu, NO_ERR as u16)?;
            format!("{} -> {v:#010x}", if sys { "GetSysBeepVolume" } else { "GetDefaultOutputVolume" })
        }
        (0x28, 2, _) | (0x30, 2, _) => {
            let (v, sys) = (arg32(m, 0)?, routine == 0x28);
            if sys { m.snd.sys_beep_volume = Some(v); } else {
                m.snd.default_volume = Some(v);
                let keys: Vec<u32> = m.snd.chans.keys().copied().collect();
                for k in keys { m.snd_apply_volume(k).map_err(|e| Refusal(format!("SetDefaultOutputVolume: {e:?}")))?; }
            }
            ret_w(m, cpu, NO_ERR as u16)?;
            format!("{} {v:#010x}", if sys { "SetSysBeepVolume" } else { "SetDefaultOutputVolume" })
        }
        _ => {
            cpu.set_areg(7, sp + params);
            return Err(Refusal(format!("SoundDispatch selector {d0:#010x} ({}) is not implemented (its {params}-byte frame was popped; a result slot, if it has one, is untouched)", name_of(routine, words))));
        }
    };
    Ok(format!("[sound] {text}"))
}
