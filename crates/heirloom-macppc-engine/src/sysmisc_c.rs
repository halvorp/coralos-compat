//! Small system calls with spec'd thin bodies: the Display Manager's screen-device walk (`DMGetFirstScreenDevice`, `DMGetNextScreenDevice`), the machine location in extended PRAM
//! (`ReadLocation`, `WriteLocation`) and `DebugStr`.
//! Specs (clean room): macos9-specs/InterfaceLib/{DMGetFirstScreenDevice,DMGetNextScreenDevice,ReadLocation,DebugStr}.
//! * The screen-device walk: GDevices in the device list order (`Cqd::devices`), keeping those whose gdFlags (word at +0x14) bit 15 (screenDevice) is set, and with `activeOnly` also bit 11
//!   (screenActive, 0x0800); `Next` returns the first such device AFTER `theDevice`, NULL if `theDevice` is NULL or not in the list.
//! * The machine location is 12 bytes of extended PRAM at offset 0xE4 (`ReadXPRam(0xE4, 12)`): zeros until written (latitude 0, longitude 0, gmtDelta 0 = UTC, no DST flag).
//! * `DebugStr` logs the Pascal string and returns: no debugger is installed.
//! * UTCUtils (Mac OS 9 DateTimeUtils.h): `ConvertLocalTimeToUTC` / `ConvertUTCToLocalTime` (UInt32 seconds since 1904) and
//!   `ConvertUTCToLocalDateTime` / `ConvertLocalToUTCDateTime` (UTCDateTime / LocalDateTime: highSeconds 2, lowSeconds 4, fraction 2) move
//!   by the machine location's gmtDelta (the signed low 24 bits of its third long, seconds east of UTC; IM Operating System Utilities),
//!   the same offset `ReadLocation` reports. A NULL result pointer is paramErr.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const GD_FLAGS: u32 = 0x14;
const SCREEN_DEVICE: u16 = 1 << 15;
const SCREEN_ACTIVE: u16 = 1 << 11;

#[derive(Default)]
pub struct SysState { location: [u8; 12], /// the Font Manager's SetOutlinePreferred (toolbox_c.rs)
    pub(crate) outline_preferred: bool,
    /// the Font Manager's SetPreserveGlyph (toolbox_c.rs)
    pub(crate) preserve_glyph: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysFn { ConvertLocalTimeToUTC, ConvertUTCToLocalTime, ConvertUTCToLocalDateTime, ConvertLocalToUTCDateTime, DMGetFirstScreenDevice, DMGetNextScreenDevice, ReadLocation, WriteLocation, DebugStr, DMGetDisplayIDByGDevice, UpdateSystemActivity, RegisterComponentResourceFile, OpenADefaultComponent }

impl SysFn {
    pub fn by_name(name: &str) -> Option<SysFn> {
        use SysFn::*;
        Some(match name {
            "DMGetFirstScreenDevice" => DMGetFirstScreenDevice, "DMGetNextScreenDevice" => DMGetNextScreenDevice,
            "ReadLocation" => ReadLocation, "ConvertLocalTimeToUTC" => ConvertLocalTimeToUTC, "ConvertUTCToLocalTime" => ConvertUTCToLocalTime, "ConvertUTCToLocalDateTime" => ConvertUTCToLocalDateTime, "ConvertLocalToUTCDateTime" => ConvertLocalToUTCDateTime, "WriteLocation" => WriteLocation, "DMGetDisplayIDByGDevice" => DMGetDisplayIDByGDevice, "DebugStr" => DebugStr, "UpdateSystemActivity" => UpdateSystemActivity, "RegisterComponentResourceFile" => RegisterComponentResourceFile, "OpenADefaultComponent" => OpenADefaultComponent,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn qualifies(&self, gd: u32, active_only: bool) -> bool {
        let Some(p) = self.mac.mm.block(gd).map(|b| b.ptr) else { return false };
        let f = self.mac.mem.read_u16(p + GD_FLAGS).unwrap_or(0);
        f & SCREEN_DEVICE != 0 && (!active_only || f & SCREEN_ACTIVE != 0)
    }

    pub(crate) fn sys_call(&mut self, f: SysFn, cpu: &mut Cpu) -> Result<(), String> {
        use SysFn::*;
        let (a0, a1) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0));
        let n = format!("{f:?}");
        let r: u32 = match f {
            DMGetFirstScreenDevice => {
                let list = self.mac.cqd.devices.clone();
                list.into_iter().find(|&d| self.qualifies(d, a0 & 0xFF != 0)).unwrap_or(0)
            }
            DMGetNextScreenDevice => {
                let list = self.mac.cqd.devices.clone();
                match list.iter().position(|&d| d == a0) {
                    Some(i) if a0 != 0 => list[i + 1..].iter().copied().find(|&d| self.qualifies(d, a1 & 0xFF != 0)).unwrap_or(0),
                    _ => 0,
                }
            }
            // DMGetDisplayIDByGDevice(device, &displayID, failToMain) -> OSErr (spec: macos9-specs/Misc/DMGetDisplayIDByGDevice): the main display is id 1; a device that is not a
            // screen is kDMGenErr (-71130) unless failToMain substitutes the main device; a NULL out pointer is paramErr
            DMGetDisplayIDByGDevice => {
                let a2 = arg(self, cpu, 2)?;
                if a1 == 0 { (-50i32) as u32 } else if self.qualifies(a0, false) || a2 & 0xFF != 0 {
                    self.mac.mem.write_u32(a1, 1).map_err(|_| format!("{n}: the result pointer leaves guest RAM"))?; 0
                } else { (-71130i32) as u32 }
            }
            // UpdateSystemActivity(activity) -> OSErr (Power Manager, CarbonLib 1.0): "the user is active, do not idle-sleep". The guest has no
            // idle-sleep timer here (the host owns power), so there is nothing to reset: noErr. Activity types 0..4 are the defined ones.
            UpdateSystemActivity => if a0 & 0xFF <= 4 { 0 } else { (-50i32) as u32 },
            // RegisterComponentResourceFile(resRefNum, global) -> short (Component Manager): the number of components registered from the file's 'thng'
            // resources. This engine runs no third-party components (QuickTime's are refused per call, qt_c.rs): it registers none and says so (0), which is
            // the honest count; a program that needs one of them finds it missing at the call that asks for it.
            RegisterComponentResourceFile => { self.mac.log.push(format!("[component] RegisterComponentResourceFile file {} -> 0 components (none are run here)", a0 as i16)); 0 }
            // OpenADefaultComponent(componentType, componentSubType, &ci) -> OSErr: no component is registered here (see above), so there is none to open:
            // *ci = NIL and invalidComponentID (-3000), the Component Manager's "no such component"; named in the log with the type asked for.
            OpenADefaultComponent => {
                let out = arg(self, cpu, 2)?;
                if out != 0 { self.mac.mem.write_u32(out, 0).map_err(|_| format!("{n}: the result pointer leaves guest RAM"))?; }
                let fc = |v: u32| String::from_utf8_lossy(&v.to_be_bytes()).into_owned();
                self.mac.log.push(format!("[component] OpenADefaultComponent '{}'/'{}' -> invalidComponentID (no component is run here)", fc(a0), fc(a1)));
                (-3000i32) as u32
            }
            ReadLocation => { let loc = self.sys.location; self.put_bytes(a0, &loc, &n)?; 0 }
            ConvertLocalTimeToUTC | ConvertUTCToLocalTime => if a1 == 0 { (-50i32) as u32 } else {
                let d = if f == ConvertUTCToLocalTime { self.gmt_delta() } else { -self.gmt_delta() };
                self.mac.mem.write_u32(a1, (a0 as i64 + d) as u32).map_err(|_| format!("{n}: the result pointer leaves guest RAM"))?; 0
            },
            ConvertUTCToLocalDateTime | ConvertLocalToUTCDateTime => if a1 == 0 || a0 == 0 { (-50i32) as u32 } else {
                let b = self.get_bytes(a0, 8, &n)?;
                let secs = (u64::from(u16::from_be_bytes([b[0], b[1]])) << 32 | u64::from(u32::from_be_bytes([b[2], b[3], b[4], b[5]]))) as i64;
                let d = if f == ConvertUTCToLocalDateTime { self.gmt_delta() } else { -self.gmt_delta() };
                let t = (secs + d).max(0) as u64;
                let mut o = [0u8; 8];
                o[0..2].copy_from_slice(&((t >> 32) as u16).to_be_bytes());
                o[2..6].copy_from_slice(&(t as u32).to_be_bytes());
                o[6..8].copy_from_slice(&b[6..8]);
                self.put_bytes(a1, &o, &n)?; 0
            },
            WriteLocation => { let b = self.get_bytes(a0, 12, &n)?; self.sys.location.copy_from_slice(&b); 0 }
            DebugStr => {
                let len = self.mac.mem.read_u8(a0).map_err(|_| format!("DebugStr: the string at {a0:#x} leaves guest RAM"))?;
                let text = self.get_bytes(a0 + 1, u32::from(len), &n)?;
                self.mac.log.push(format!("[debugstr] {}", String::from_utf8_lossy(&text)));
                0
            }
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// The machine location's gmtDelta: seconds east of UTC, the signed low 24 bits of its third long (0 until WriteLocation).
    fn gmt_delta(&self) -> i64 { let l = &self.sys.location; i64::from((i32::from_be_bytes([l[8], l[9], l[10], l[11]]) << 8) >> 8) }
}
