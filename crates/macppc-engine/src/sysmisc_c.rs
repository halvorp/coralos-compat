//! Small system calls with spec'd thin bodies: the Display Manager's screen-device walk (`DMGetFirstScreenDevice`, `DMGetNextScreenDevice`), the machine location in extended PRAM
//! (`ReadLocation`, `WriteLocation`) and `DebugStr`.
//! Specs (clean room): macos9-specs/InterfaceLib/{DMGetFirstScreenDevice,DMGetNextScreenDevice,ReadLocation,DebugStr}.
//! * The screen-device walk: GDevices in the device list order (`Cqd::devices`), keeping those whose gdFlags (word at +0x14) bit 15 (screenDevice) is set, and with `activeOnly` also bit 11
//!   (screenActive, 0x0800); `Next` returns the first such device AFTER `theDevice`, NULL if `theDevice` is NULL or not in the list.
//! * The machine location is 12 bytes of extended PRAM at offset 0xE4 (`ReadXPRam(0xE4, 12)`): zeros until written (latitude 0, longitude 0, gmtDelta 0 = UTC, no DST flag).
//! * `DebugStr` logs the Pascal string and returns: no debugger is installed.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

const GD_FLAGS: u32 = 0x14;
const SCREEN_DEVICE: u16 = 1 << 15;
const SCREEN_ACTIVE: u16 = 1 << 11;

#[derive(Default)]
pub struct SysState { location: [u8; 12] }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysFn { DMGetFirstScreenDevice, DMGetNextScreenDevice, ReadLocation, WriteLocation, DebugStr }

impl SysFn {
    pub fn by_name(name: &str) -> Option<SysFn> {
        use SysFn::*;
        Some(match name {
            "DMGetFirstScreenDevice" => DMGetFirstScreenDevice, "DMGetNextScreenDevice" => DMGetNextScreenDevice,
            "ReadLocation" => ReadLocation, "WriteLocation" => WriteLocation, "DebugStr" => DebugStr,
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
            ReadLocation => { let loc = self.sys.location; self.put_bytes(a0, &loc, &n)?; 0 }
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
