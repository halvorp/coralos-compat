//! Sound Manager 3.0 routines that are `_SoundDispatch` ($A800) selectors on a 68k, by C name: the SAME bodies the 68k dispatch runs
//! (heirloom-mac68k-engine `sound::dispatch`). `SndChannelStatus(chan, theLength, theStatus)` (spec macos9-specs/Sound/SoundMisc.md).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

/// NumVersion 3.0.0 final (major 3, minor/bug 0, stage 0x80 finalStage, nonRel 0).
pub const SOUND_MANAGER_3_0: u32 = 0x0300_8000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SndFn { SndChannelStatus, SndSoundManagerVersion }

impl SndFn {
    pub fn by_name(name: &str) -> Option<SndFn> {
        match name { "SndChannelStatus" => Some(SndFn::SndChannelStatus), "SndSoundManagerVersion" => Some(SndFn::SndSoundManagerVersion), _ => None }
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn snd_call(&mut self, f: SndFn, regs: &mut [u32; 32]) -> Result<(), String> {
        match f {
            SndFn::SndChannelStatus => {
                let (chan, len, status) = (regs[3], regs[4] as u16 as i16, regs[5]);
                let e = heirloom_mac68k_engine::sound::dispatch::channel_status(&mut self.mac, chan, len, status).map_err(|r| r.0)?;
                self.mac.log.push(format!("[sound] SndChannelStatus {chan:#x} -> {e}"));
                regs[3] = e as i32 as u32;
            }
            // SndSoundManagerVersion() -> NumVersion, a struct returned by value: the caller's buffer in r3, returned in r3 (the measured
            // DSpGetVersion convention). 3.0.0 final: the Sound Manager 3.0 API level the shared sound dispatch serves, no higher.
            SndFn::SndSoundManagerVersion => {
                let buf = regs[3];
                self.mac.mem.write_u32(buf, SOUND_MANAGER_3_0).map_err(|_| format!("SndSoundManagerVersion: the result buffer {buf:#x} leaves guest RAM"))?;
                self.mac.log.push(String::from("[sound] SndSoundManagerVersion -> 3.0.0"));
            }
        }
        Ok(())
    }
}
