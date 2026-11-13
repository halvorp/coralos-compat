//! The Sound Manager for PowerPC programs. The channel, the `'snd '` decoder and the audio seam are mac68k-engine's `sound` module (ONE
//! implementation for both processors); this file is the PowerPC half only:
//! * the routines that have no trap (InterfaceLib names of `_SoundDispatch` selectors and legacy glue): `SetSoundVol`, `GetSoundVol`,
//!   `SndChannelStatus`, `GetSoundHeaderOffset`; an `OSErr` result goes back in r3 sign-extended;
//! * a channel's completion routine (`callBackCmd`), entered as PowerPC code through its routine descriptor at the next glue call (a safepoint,
//!   the way a Time Manager task runs) and never nested; each completion runs once, in the order the guest queued them.
use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SndFn { SetSoundVol, GetSoundVol, SndChannelStatus, GetSoundHeaderOffset }

impl SndFn {
    pub fn by_name(name: &str) -> Option<SndFn> {
        use SndFn::*;
        Some(match name {
            "SetSoundVol" => SetSoundVol, "GetSoundVol" => GetSoundVol, "SndChannelStatus" => SndChannelStatus, "GetSoundHeaderOffset" => GetSoundHeaderOffset,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn snd_call(&mut self, f: SndFn, cpu: &mut Cpu) -> Result<(), String> {
        use SndFn::*;
        let (a0, a1, a2) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0));
        let e = |r: mac68k_engine::Refusal| format!("{f:?}: {}", r.0);
        let err = |code: i16| i32::from(code) as u32;
        cpu.gpr[3] = match f {
            SetSoundVol => { self.mac.set_sound_vol(a0 as u16 as i16).map_err(e)?; 0 }
            GetSoundVol => {
                // GetSoundVol(short *level): a NIL or unwritable pointer is left alone (the routine has no result)
                let level = self.mac.get_sound_vol() as u16;
                if a0 != 0 { let _ = self.mac.mem.write_u16(a0, level); }
                0
            }
            SndChannelStatus => err(self.mac.snd_channel_status(a0, a1 as u16 as i16, a2).map_err(e)?),
            GetSoundHeaderOffset => err(self.mac.snd_get_header_offset(a0, a1).map_err(e)?),
        };
        Ok(())
    }

    /// At a safepoint (`call_pc` is the glue call about to run): if a channel's completion routine is due, save the interrupted call and enter the
    /// routine with r3 = the channel and r4 = the `SndCommand`. `Ok(true)` = the CPU now runs the routine.
    pub(crate) fn snd_safepoint(&mut self, cpu: &mut Cpu, call_pc: u32) -> Result<bool, String> {
        if !self.rt.callbacks.is_empty() { return Ok(false); }
        let Some(d) = self.mac.snd_take_due() else { return Ok(false) };
        let saved = Saved::of(cpu, call_pc);
        let sp = cpu.gpr[1];
        if let Err(why) = self.enter_upp(cpu, d.routine, &[d.chan, d.cmd], sp) {
            // refused by name; the program loses this completion, the host loses nothing, the next one still runs
            self.mac.finish_snd_callback(d.cmd);
            self.mac.note_refusal("SndNewChannel", format!("the completion routine {:#x} of channel {:#x} cannot run: {why}", d.routine, d.chan));
            return Ok(false);
        }
        self.rt.callbacks.push(Frame { saved, cont: Continuation::SndCallBack(d.cmd) });
        Ok(true)
    }

    /// The completion routine returned: its command record is freed and the interrupted call is entered again.
    pub(crate) fn snd_callback_returned(&mut self, cpu: &mut Cpu, frame: Frame, cmd: u32) {
        self.mac.finish_snd_callback(cmd);
        frame.saved.restore(cpu);
    }
}
