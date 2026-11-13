//! The Sound Manager routines in VALUE form: `SndChannelStatus`, `GetSoundHeaderOffset`, `SndControl` and the legacy speaker volume
//! (`SetSoundVol` / `GetSoundVol`). The 68k adapters (`_SoundDispatch`, `_SndControl`) and the PowerPC imports of the same names call
//! these, so there is ONE implementation per routine whichever processor the program runs on.
//!
//! Return codes follow Inside Macintosh: Sound: `noErr`, `paramErr` (-50: a bad pointer or buffer), `badChannel` (-205), `badFormat` (-206).
use super::*;
use crate::lowmem::SD_VOLUME;

/// `availableCmd`, `versionCmd`, `totalLoadCmd`, `loadCmd`: the commands `SndControl` answers (Inside Macintosh: Sound 2-134).
const TOTAL_LOAD_CMD: u16 = 26;
const LOAD_CMD: u16 = 27;
/// The legacy speaker volume scale, `SdVolume` bits 0-2.
pub const SD_VOLUME_MAX: u8 = 7;

impl<U: GuestUi> Mac<U> {
    /// `SndChannelStatus(chan, theLength, VAR theStatus)`: `scChannelBusy` / `scChannelPaused` come from the host's own queue; the three
    /// times are NOT modelled (they read 0) and the call says so.
    pub fn snd_channel_status(&mut self, chan: u32, len: i16, status: u32) -> Result<i16, Refusal> {
        self.snd_reap();
        let Some(key) = self.snd_key(chan) else { return Ok(BAD_CHANNEL) };
        if len < 24 { return Ok(PARAM_ERR); }
        let (streams, paused) = self.snd.chans.get(&key).map(|c| (c.streams.iter().map(|s| s.r).collect::<Vec<_>>(), c.paused)).unwrap_or_default();
        let busy = streams.iter().any(|s| self.ui.audio_queued_bytes(*s).is_ok_and(|n| n > 0));
        // SCStatus, 24 bytes: scStartTime, scEndTime, scCurrentTime, scChannelBusy, scChannelDisposed, scChannelPaused, scUnused, scChannelAttributes,
        // scCPULoad. The three times are REFUSED BY NAME below: what clock they count and from when is not documented in a source we have, so the
        // engine does not invent one; they read 0.
        let mut rec = [0u8; 24];
        rec[12] = u8::from(busy);
        rec[14] = u8::from(paused);
        if self.mem.write_bytes(status, &rec).is_err() { return Ok(PARAM_ERR); }
        self.note_refusal("SndChannelStatus", String::from("scStartTime, scEndTime and scCurrentTime are not modelled (they read 0); scChannelBusy and scChannelPaused are real"));
        Ok(NO_ERR)
    }

    /// `GetSoundHeaderOffset(sndHandle, VAR offset)`: the byte offset, inside the `'snd '` resource, of the first sampled-sound header (the
    /// `param2` of its first `soundCmd` / `bufferCmd` that carries the offset flag). A resource with none, or in no known format, is `badFormat`
    /// and the offset is cleared; a handle that is not live, or an unwritable `offset`, is `paramErr`.
    pub fn snd_get_header_offset(&mut self, h: u32, out: u32) -> Result<i16, Refusal> {
        if out == 0 || self.mem.write_u32(out, 0).is_err() { return Ok(PARAM_ERR); }
        let Some((base, size)) = self.mm.block(h).map(|b| (b.ptr, b.size)) else { return Ok(PARAM_ERR) };
        let body = self.mem.span(base, size).and_then(|s| self.mem.bytes(s)).map(<[u8]>::to_vec).map_err(Refusal::mem("'snd '"))?;
        let found = snd::parse_snd(&body).ok().and_then(|cmds| cmds.into_iter().find(|(c, by_offset)| *by_offset && matches!(c.cmd, snd::SOUND_CMD | snd::BUFFER_CMD)).map(|(c, _)| c.param2));
        let Some(offset) = found else { return Ok(BAD_FORMAT) };
        if self.mem.write_u32(out, offset).is_err() { return Ok(PARAM_ERR); }
        Ok(NO_ERR)
    }

    /// `SndControl(id, VAR cmd)`: `availableCmd` (param1 = 1 when the synthesizer `id` can be used: here only the sampled one, which is the
    /// only one that exists), and `totalLoadCmd` / `loadCmd` (0 percent: the engine synthesises nothing). Any other command is `paramErr`
    /// and named: a synthesizer's version and `initCmd` options are the 'snth' code resource's to answer, and none runs here.
    pub fn snd_control(&mut self, id: i16, cmd: u32) -> Result<i16, Refusal> {
        let (Ok(c), Ok(p2)) = (self.mem.read_u16(cmd), self.mem.read_u32(cmd.wrapping_add(4))) else { return Ok(PARAM_ERR) };
        let answer = match c & !snd::OFFSET_FLAG {
            snd::AVAILABLE_CMD => u16::from(id == SAMPLED_SYNTH && p2 == 0),
            TOTAL_LOAD_CMD | LOAD_CMD => 0,
            other => {
                self.note_refusal("SndControl", format!("command {other} for synthesizer {id} is not modelled (only availableCmd, totalLoadCmd and loadCmd are answered)"));
                return Ok(PARAM_ERR);
            }
        };
        if self.mem.write_u16(cmd.wrapping_add(2), answer).is_err() { return Ok(PARAM_ERR); }
        Ok(NO_ERR)
    }

    /// `GetSoundVol`: the legacy speaker level, 0-7: the low three bits of the low-memory global `SdVolume`.
    pub fn get_sound_vol(&self) -> i16 { i16::from(self.mem.read_u8(SD_VOLUME).unwrap_or(SD_VOLUME_MAX) & SD_VOLUME_MAX) }

    /// `SetSoundVol(level)`: clamp to 0-7, store in `SdVolume`'s low bits (the high bits stay) and apply to every open stream and the beep.
    pub fn set_sound_vol(&mut self, level: i16) -> Result<(), Refusal> {
        let level = level.clamp(0, i16::from(SD_VOLUME_MAX)) as u8;
        let old = self.mem.read_u8(SD_VOLUME).unwrap_or(0);
        self.mem.write_u8(SD_VOLUME, (old & !SD_VOLUME_MAX) | level).map_err(Refusal::mem("SdVolume"))?;
        let keys: Vec<u32> = self.snd.chans.keys().copied().collect();
        for k in keys { self.snd_apply_volume(k).map_err(|e| Refusal(format!("SetSoundVol: {e:?}")))?; }
        Ok(())
    }

    /// `level / 7` of `gain`: the legacy speaker level scales every stream the Sound Manager owns.
    pub(super) fn snd_legacy_scale(&self, gain: u16) -> u16 { ((u32::from(gain) * self.get_sound_vol() as u32) / u32::from(SD_VOLUME_MAX)) as u16 }
}

/// The trap adapter for `FUNCTION SndControl(id: INTEGER; VAR cmd: SndCommand): OSErr` ($A806).
pub(super) fn snd_control_trap<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let err = m.snd_control(c.args.get(0) as i16, c.args.get(1))?;
    Ok(Done { result: err as u16 as u32, log: format!("[sound] SndControl synth {} cmd {:#x} -> {err}", c.args.get(0) as i16, c.args.get(1)) })
}
