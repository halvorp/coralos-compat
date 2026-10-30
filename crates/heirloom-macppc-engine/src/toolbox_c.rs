//! Toolbox routines that are LIBRARY GLUE, not traps, in the Universal Interfaces: string-list and string-copy utilities (`GetIndString`,
//! `CopyPascalStringToC`, `c2pstr` ...), Binary-Decimal conversion (`NumToString`: a `_Pack7` selector on the 68k), low-memory accessors
//! (`LMGetResLoad` ...), and the clocks (`GetDateTime`, `Microseconds`, Driver Services' `UpTime`). A 68k program reaches the same behaviour through a trap or its own inline
//! code; the engine's 68k core has no handler by this NAME, so a PowerPC program that imports it needs these.
//! Units: `UpTime()` is an `AbsoluteTime` (two words); here one unit is a nanosecond of the machine's tick clock (60 Hz), so `AbsoluteToNanoseconds` is the identity.
//! `EqualString` folds ASCII case only when `caseSens` is false and compares diacritical characters as bytes (the script-aware folding of the Text Utilities is not done).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TbFn {
    GetIndString, CopyPascalStringToC, CopyCStringToPascal, C2PStr, P2CStr, EqualString, NumToString, StringToNum,
    FindFolder, SetEventMask, GetDateTime, GetDblTime, GetCaretTime, Microseconds, UpTime, AbsoluteToNanoseconds,
    /// `LMGet<Name>` / `LMSet<Name>` over the table [`LOWMEM`]; the index is the row.
    LmGet(u8), LmSet(u8),
}

/// (accessor suffix, address, bytes): the low-memory globals programs read through the Universal Interfaces' `LMGet`/`LMSet` calls.
pub const LOWMEM: &[(&str, u32, u32)] = &[("ResLoad", 0x0A5E, 1), ("MBarHeight", 0x0BAA, 2), ("CurDirStore", 0x0398, 4), ("Ticks", 0x016A, 4), ("DoubleTime", 0x02F0, 4), ("CaretTime", 0x02F4, 4), ("SFSaveDisk", 0x0214, 2), ("ApplLimit", 0x0130, 4)];

impl TbFn {
    pub fn by_name(name: &str) -> Option<TbFn> {
        use TbFn::*;
        Some(match name {
            "GetIndString" => GetIndString, "CopyPascalStringToC" => CopyPascalStringToC, "CopyCStringToPascal" => CopyCStringToPascal,
            "C2PStr" | "c2pstr" => C2PStr, "P2CStr" | "p2cstr" => P2CStr, "EqualString" => EqualString, "NumToString" => NumToString, "StringToNum" => StringToNum,
            "FindFolder" => FindFolder, "SetEventMask" => SetEventMask, "GetDateTime" => GetDateTime, "GetDblTime" => GetDblTime, "GetCaretTime" => GetCaretTime, "Microseconds" => Microseconds, "UpTime" => UpTime,
            "AbsoluteToNanoseconds" => AbsoluteToNanoseconds,
            n => {
                let (get, rest) = if let Some(r) = n.strip_prefix("LMGet") { (true, r) } else if let Some(r) = n.strip_prefix("LMSet") { (false, r) } else { return None };
                let i = LOWMEM.iter().position(|(s, _, _)| *s == rest)? as u8;
                if get { LmGet(i) } else { LmSet(i) }
            }
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn pstr_bytes(&self, at: u32, who: &str) -> Result<Vec<u8>, String> {
        let n = self.mac.mem.read_u8(at).map_err(|_| format!("{who}: the Pascal string at {at:#x} leaves guest RAM"))?;
        self.get_bytes(at + 1, u32::from(n), who)
    }
    fn put_pstr(&mut self, at: u32, s: &[u8], who: &str) -> Result<(), String> {
        let n = s.len().min(255);
        let mut b = alloc::vec![n as u8]; b.extend_from_slice(&s[..n]);
        self.put_bytes(at, &b, who)
    }

    pub(crate) fn tb_call(&mut self, f: TbFn, cpu: &mut Cpu) -> Result<(), String> {
        use TbFn::*;
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let _ = a3;
        let name = format!("{f:?}");
        let r: u32 = match f {
            // GetIndString(Str255 theString, short strListID, short index): the index'th string of the 'STR#' resource; an empty string if there is none (ResError is set)
            GetIndString => {
                let (list, index) = (a1 as u16 as i16, a2 as u16 as i16);
                let out = self.mac.get_resource(*b"STR#", list, false).ok().and_then(|h| self.mac.handle_bytes(h)).and_then(|b| {
                    let count = u16::from_be_bytes([*b.first()?, *b.get(1)?]) as usize;
                    if index < 1 || index as usize > count { return None; }
                    let mut at = 2;
                    for _ in 1..index { at += 1 + *b.get(at)? as usize; }
                    let n = *b.get(at)? as usize;
                    b.get(at + 1..at + 1 + n).map(<[u8]>::to_vec)
                });
                self.put_pstr(a0, out.as_deref().unwrap_or(&[]), &name)?;
                0
            }
            CopyPascalStringToC => { let mut s = self.pstr_bytes(a0, &name)?; s.push(0); self.put_bytes(a1, &s, &name)?; 0 }
            CopyCStringToPascal => { let s = self.cstr(a0, &name)?; self.put_pstr(a1, &s, &name)?; 0 }
            C2PStr => { let s = self.cstr(a0, &name)?; let n = s.len().min(255); let mut b = alloc::vec![n as u8]; b.extend_from_slice(&s[..n]); self.put_bytes(a0, &b, &name)?; a0 }
            P2CStr => { let s = self.pstr_bytes(a0, &name)?; let mut b = s; b.push(0); self.put_bytes(a0, &b, &name)?; a0 }
            EqualString => {
                let (x, y) = (self.pstr_bytes(a0, &name)?, self.pstr_bytes(a1, &name)?);
                let case_sens = a2 & 0xFF != 0;
                u32::from(if case_sens { x == y } else { x.eq_ignore_ascii_case(&y) })
            }
            NumToString => { let t = heirloom_mac68k_engine::pack7::num_text(a0 as i32); self.put_pstr(a1, t.as_bytes(), &name)?; 0 }
            StringToNum => { let s = self.pstr_bytes(a0, &name)?; let n = heirloom_mac68k_engine::pack7::parse_num(&s); self.mac.mem.write_u32(a1, n as u32).map_err(|_| format!("{name}: result pointer {a1:#x} leaves guest RAM"))?; 0 }
            // FindFolder(vRefNum, folderType, createFolder, &foundVRefNum, &foundDirID): the Folder Manager is the 68k engine's (fm/alias.rs), folders are made on first use
            FindFolder => {
                let (vref, ty) = (a0 as u16 as i16, a1.to_be_bytes());
                match self.mac.fm.find_folder(vref, ty) {
                    Ok((v, d)) => {
                        let dir_p = arg(self, cpu, 4)?;
                        self.mac.mem.write_u16(a3, v as u16).and_then(|_| self.mac.mem.write_u32(dir_p, d)).map_err(|_| format!("{name}: result pointers leave guest RAM"))?;
                        0
                    }
                    Err(e) => e as i32 as u32,
                }
            }
            // SetEventMask(mask) is the obsolete setter of the SysEvtMask low-memory word (0x144) that the Event Manager consults
            SetEventMask => { self.mac.mem.write_u16(0x0144, a0 as u16).map_err(|_| String::from("SetEventMask: low memory leaves guest RAM"))?; 0 }
            GetDateTime => { let t = self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::TIME).unwrap_or(0); self.mac.mem.write_u32(a0, t).map_err(|_| format!("{name}: result pointer {a0:#x} leaves guest RAM"))?; 0 }
            GetDblTime => { let v = self.mac.mem.read_u32(0x02F0).unwrap_or(0); if v == 0 { 32 } else { v } }
            GetCaretTime => { let v = self.mac.mem.read_u32(0x02F4).unwrap_or(0); if v == 0 { 32 } else { v } }
            Microseconds => { // UnsignedWide* (hi, lo)
                let us = self.mac.ev.ticks_total * 1_000_000 / 60;
                self.mac.mem.write_u32(a0, (us >> 32) as u32).and_then(|_| self.mac.mem.write_u32(a0 + 4, us as u32)).map_err(|_| format!("{name}: result pointer {a0:#x} leaves guest RAM"))?;
                0
            }
            // AbsoluteTime is two words (hi, lo) returned in r3/r4; one unit = one nanosecond of the tick clock
            UpTime => { let ns = self.mac.ev.ticks_total * 1_000_000_000 / 60; cpu.gpr[3] = (ns >> 32) as u32; cpu.gpr[4] = ns as u32; return Ok(()); }
            AbsoluteToNanoseconds => { cpu.gpr[3] = a0; cpu.gpr[4] = a1; return Ok(()); }
            LmGet(i) => {
                let (_, at, n) = LOWMEM[i as usize];
                match n { 1 => u32::from(self.mac.mem.read_u8(at).unwrap_or(0)), 2 => u32::from(self.mac.mem.read_u16(at).unwrap_or(0)), _ => self.mac.mem.read_u32(at).unwrap_or(0) }
            }
            LmSet(i) => {
                let (_, at, n) = LOWMEM[i as usize];
                let w = match n { 1 => self.mac.mem.write_u8(at, a0 as u8), 2 => self.mac.mem.write_u16(at, a0 as u16), _ => self.mac.mem.write_u32(at, a0) };
                w.map_err(|_| format!("{name}: low memory {at:#x} leaves guest RAM"))?;
                0
            }
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
