//! Sound Manager 3.0 routines that are `_SoundDispatch` ($A800) selectors on a 68k, by C name: the SAME bodies the 68k dispatch runs
//! (heirloom-mac68k-engine `sound::dispatch`). `SndChannelStatus(chan, theLength, theStatus)` (spec macos9-specs/Sound/SoundMisc.md).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SndFn { SndChannelStatus }

impl SndFn {
    pub fn by_name(name: &str) -> Option<SndFn> { (name == "SndChannelStatus").then_some(SndFn::SndChannelStatus) }
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
        }
        Ok(())
    }
}
