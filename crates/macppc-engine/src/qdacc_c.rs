//! Carbon QuickDraw accessors and the small QuickDraw/OS routines the Installer and the games import weakly (`GetQDGlobalsArrow`, `GetQDGlobalsScreenBits`, `GetPortClipRegion`, `GetGlobalMouse`,
//! `DeltaPoint`, `GetIndPattern`, `SetA5`, `GetTime` ...). Specs (clean room): macos9-specs/Quickdraw/{PortAccessors,GetGlobalMouse,GetPortClipRegion,SetPortClipRegion,
//! GetPortPenLocation,GetQDGlobalsScreenBits,DeltaPoint,GetIndPattern,SetA5,SetCurrentA5,GetTime,RestoreDeviceClut}. The QD globals are the real record at `thePort - offset` (quickdraw.rs):
//! the accessors copy out of it, so a Carbon program that never sees the record still reads what a Classic one would. The arrow cursor, the five standard patterns
//! (`GetQDGlobalsBlack/White/Gray/LightGray/DarkGray`) and `randSeed` (`Get/SetQDGlobalsRandomSeed`) follow the same pattern as the spec'd screenBits (copy out of the globals; not in a spec,
//! the public Carbon contract). `SetA5`/`SetCurrentA5` keep an emulated register value (a native PowerPC program has no A5 world; the old value is returned). `RestoreDeviceClut` is a no-op
//! on this display (spec: direct synthetic display). Left unbound: `OpenCPicture`, `OpColor` (no handler behind the trap yet).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::quickdraw::{OFF_ARROW, OFF_BLACK, OFF_DK_GRAY, OFF_GRAY, OFF_LT_GRAY, OFF_RAND_SEED, OFF_SCREEN_BITS, OFF_WHITE};
use mac68k_engine::qd::port::{P_CLIP, P_PN_LOC};
use retro_ppc::Cpu;

#[derive(Default)]
pub struct QaState { a5: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaFn {
    GetQDGlobalsArrow, GetQDGlobalsScreenBits, GetQDGlobalsRandomSeed, SetQDGlobalsRandomSeed, GetQDGlobalsBlack, GetQDGlobalsWhite, GetQDGlobalsGray, GetQDGlobalsLightGray, GetQDGlobalsDarkGray,
    GetGlobalMouse, GetPortClipRegion, SetPortClipRegion, GetPortPenLocation, DeltaPoint, GetIndPattern, SetA5, SetCurrentA5, GetTime, RestoreDeviceClut,
}

impl QaFn {
    pub fn by_name(name: &str) -> Option<QaFn> {
        use QaFn::*;
        Some(match name {
            "GetQDGlobalsArrow" => GetQDGlobalsArrow, "GetQDGlobalsScreenBits" => GetQDGlobalsScreenBits, "GetQDGlobalsRandomSeed" => GetQDGlobalsRandomSeed,
            "SetQDGlobalsRandomSeed" => SetQDGlobalsRandomSeed, "GetQDGlobalsBlack" => GetQDGlobalsBlack, "GetQDGlobalsWhite" => GetQDGlobalsWhite, "GetQDGlobalsGray" => GetQDGlobalsGray,
            "GetQDGlobalsLightGray" => GetQDGlobalsLightGray, "GetQDGlobalsDarkGray" => GetQDGlobalsDarkGray, "GetGlobalMouse" => GetGlobalMouse, "GetPortClipRegion" => GetPortClipRegion,
            "SetPortClipRegion" => SetPortClipRegion, "GetPortPenLocation" => GetPortPenLocation, "DeltaPoint" => DeltaPoint, "GetIndPattern" => GetIndPattern, "SetA5" => SetA5,
            "SetCurrentA5" => SetCurrentA5, "GetTime" => GetTime, "RestoreDeviceClut" => RestoreDeviceClut,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn qa_trap(&mut self, name: &str, v: &[u32]) -> Result<u32, String> {
        let op = mac68k_engine::mixed::trap_opcode(name).ok_or_else(|| format!("{name} is not a trap"))?;
        self.mac.pascal_call(op, v).map(|r| r.value).map_err(|e| format!("{name}: {e:?}"))
    }
    fn qa_copy(&mut self, from: u32, to: u32, n: u32, who: &str) -> Result<(), String> {
        let b = self.get_bytes(from, n, who)?;
        self.put_bytes(to, &b, who)
    }

    pub(crate) fn qa_call(&mut self, f: QaFn, cpu: &mut Cpu) -> Result<(), String> {
        use QaFn::*;
        let n = format!("{f:?}");
        let n = n.as_str();
        let (a0, a1, a2) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0));
        let globals = self.mac.qd_globals; // the address of thePort; the other globals lie below it
        let need = |g: u32| if g == 0 { Err(format!("{n}: QuickDraw is not initialised (no globals)")) } else { Ok(()) };
        // the current port, or NIL
        let current = |s: &Self| if s.mac.qd_globals == 0 { 0 } else { s.mac.mem.read_u32(s.mac.qd_globals).unwrap_or(0) };
        let r: u32 = match f {
            GetQDGlobalsArrow => { need(globals)?; self.qa_copy(globals - OFF_ARROW, a0, 68, n)?; a0 }
            GetQDGlobalsScreenBits => { need(globals)?; self.qa_copy(globals - OFF_SCREEN_BITS, a0, 14, n)?; a0 }
            GetQDGlobalsRandomSeed => { need(globals)?; self.mac.mem.read_u32(globals - OFF_RAND_SEED).map_err(|_| format!("{n}: randSeed leaves guest RAM"))? }
            SetQDGlobalsRandomSeed => { need(globals)?; self.mac.mem.write_u32(globals - OFF_RAND_SEED, a0).map_err(|_| format!("{n}: randSeed leaves guest RAM"))?; 0 }
            GetQDGlobalsBlack | GetQDGlobalsWhite | GetQDGlobalsGray | GetQDGlobalsLightGray | GetQDGlobalsDarkGray => {
                need(globals)?;
                let off = match f { GetQDGlobalsBlack => OFF_BLACK, GetQDGlobalsWhite => OFF_WHITE, GetQDGlobalsGray => OFF_GRAY, GetQDGlobalsLightGray => OFF_LT_GRAY, _ => OFF_DK_GRAY };
                self.qa_copy(globals - off, a0, 8, n)?; a0
            }
            // the processed mouse (low memory Mouse, 0x0830): v then h
            GetGlobalMouse => { let p = self.mac.mem.read_u32(mac68k_engine::lowmem::MOUSE).unwrap_or(0); self.mac.mem.write_u32(a0, p).map_err(|_| format!("{n}: the Point leaves guest RAM"))?; 0 }
            // a NIL region (or no port at all) sets the QuickDraw error latch to paramErr and changes nothing; a NIL port is the current port; the region is COPIED
            GetPortClipRegion | SetPortClipRegion => {
                let port = if a0 == 0 { current(self) } else { a0 };
                if a1 == 0 || port == 0 { self.mac.cqd.qd_error = -50; return Ok(cpu.gpr[3] = if f == GetPortClipRegion { a1 } else { 0 }); }
                let clip = self.mac.mem.read_u32(port + P_CLIP).map_err(|_| format!("{n}: the port {port:#x} leaves guest RAM"))?;
                if f == GetPortClipRegion { self.qa_trap("CopyRgn", &[clip, a1])?; a1 } else { self.qa_trap("CopyRgn", &[a1, clip])?; 0 }
            }
            GetPortPenLocation => {
                let port = if a0 == 0 { current(self) } else { a0 };
                if a1 == 0 || port == 0 { self.mac.cqd.qd_error = -50; a1 } else { self.qa_copy(port + P_PN_LOC, a1, 4, n)?; a1 }
            }
            // DeltaPoint(ptA, ptB): (A.v - B.v) << 16 | (A.h - B.h), each 16-bit wrap-around
            DeltaPoint => { let v = ((a0 >> 16) as u16).wrapping_sub((a1 >> 16) as u16); let h = (a0 as u16).wrapping_sub(a1 as u16); u32::from(v) << 16 | u32::from(h) }
            // GetIndPattern(Pattern *, patListID, index): the index'th pattern (1-based) of 'PAT#'; the destination is untouched on any failure
            GetIndPattern => {
                let (id, idx) = (a1 as u16 as i16, a2 as u16 as i16);
                if idx >= 1 {
                    if let Ok(h) = self.mac.get_resource(*b"PAT#", id, false) {
                        if let Some(b) = self.mac.handle_bytes(h) {
                            let count = b.get(0..2).map_or(0, |c| u16::from_be_bytes([c[0], c[1]]));
                            let at = 2 + 8 * (idx as usize - 1);
                            if idx as u16 <= count && b.len() >= at + 8 { let pat = b[at..at + 8].to_vec(); self.put_bytes(a0, &pat, n)?; }
                        }
                    }
                }
                0
            }
            SetA5 => { let old = self.qa.a5; self.qa.a5 = a0; old }
            SetCurrentA5 => { let old = self.qa.a5; self.qa.a5 = self.mac.mem.read_u32(mac68k_engine::lowmem::CURRENT_A5).unwrap_or(0); old }
            GetTime => { let secs = self.mac.mem.read_u32(mac68k_engine::lowmem::TIME).unwrap_or(0); self.qa_trap("SecondsToDate", &[secs, a0])?; 0 }
            RestoreDeviceClut => 0,
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
