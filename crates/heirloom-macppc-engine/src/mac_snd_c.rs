//! Sound Manager routines that are `_SoundDispatch` ($A800) selectors on a 68k, by C name: the SAME bodies the 68k dispatch runs
//! (heirloom-mac68k-engine `sound::dispatch`). `SndChannelStatus(chan, theLength, theStatus)` (spec macos9-specs/Sound/SoundMisc.md).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

use heirloom_mac68k_engine::sound::SOUND_MANAGER_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SndFn { SndChannelStatus, SndSoundManagerVersion, GetDefaultOutputVolume, SetDefaultOutputVolume, GetSoundVol, SetSoundVol }

impl SndFn {
    pub fn by_name(name: &str) -> Option<SndFn> {
        Some(match name {
            "SndChannelStatus" => SndFn::SndChannelStatus,
            "SndSoundManagerVersion" => SndFn::SndSoundManagerVersion,
            "GetDefaultOutputVolume" => SndFn::GetDefaultOutputVolume,
            "SetDefaultOutputVolume" => SndFn::SetDefaultOutputVolume,
            "GetSoundVol" => SndFn::GetSoundVol,
            "SetSoundVol" => SndFn::SetSoundVol,
            _ => return None,
        })
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
            // DSpGetVersion convention). The shared Sound Manager's one version (heirloom-mac68k-engine sound::SOUND_MANAGER_VERSION).
            SndFn::SndSoundManagerVersion => {
                let buf = regs[3];
                self.mac.mem.write_u32(buf, SOUND_MANAGER_VERSION).map_err(|_| format!("SndSoundManagerVersion: the result buffer {buf:#x} leaves guest RAM"))?;
                let v = SOUND_MANAGER_VERSION.to_be_bytes();
                self.mac.log.push(format!("[sound] SndSoundManagerVersion -> {:x}.{:x}.{:x} stage {:#x}", v[0], v[1] >> 4, v[1] & 0xF, v[2]));
            }
            SndFn::GetDefaultOutputVolume => {
                let ptr = regs[3];
                let v = heirloom_mac68k_engine::sound::dispatch::get_default_output_volume(&self.mac);
                if ptr != 0 {
                    self.mac.mem.write_u32(ptr, v).map_err(|_| format!("GetDefaultOutputVolume: {ptr:#x} leaves guest RAM"))?;
                }
                self.mac.log.push(format!("[sound] GetDefaultOutputVolume -> {v:#010x}"));
                regs[3] = 0;
            }
            SndFn::SetDefaultOutputVolume => {
                let v = regs[3];
                heirloom_mac68k_engine::sound::dispatch::set_default_output_volume(&mut self.mac, v).map_err(|r| r.0)?;
                self.mac.log.push(format!("[sound] SetDefaultOutputVolume {v:#010x}"));
                regs[3] = 0;
            }
            SndFn::GetSoundVol => {
                let ptr = regs[3];
                let v = heirloom_mac68k_engine::sound::dispatch::get_default_output_volume(&self.mac);
                let lvl = (((v & 0xFFFF).min(0x0100) * 7 + 0x80) / 0x0100).min(7) as u16;
                if ptr != 0 {
                    self.mac.mem.write_u16(ptr, lvl).map_err(|_| format!("GetSoundVol: {ptr:#x} leaves guest RAM"))?;
                }
                self.mac.log.push(format!("[sound] GetSoundVol -> {lvl}"));
                regs[3] = 0;
            }
            SndFn::SetSoundVol => {
                let lvl = (regs[3] as u16 & 7) as u32;
                let side = if lvl == 0 { 0 } else { (lvl * 0x0100 + 3) / 7 };
                let v = (side << 16) | side;
                heirloom_mac68k_engine::sound::dispatch::set_default_output_volume(&mut self.mac, v).map_err(|r| r.0)?;
                self.mac.log.push(format!("[sound] SetSoundVol {lvl} ({v:#010x})"));
                regs[3] = 0;
            }
        }
        Ok(())
    }
}
