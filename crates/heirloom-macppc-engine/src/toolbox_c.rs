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
    /// `TEToScrap()` / `TEFromScrap()` (glue in the Universal Interfaces) and `UpperString(Str255, diacSensitive)` (glue over `_UprString`): the 68k
    /// engine's shared bodies. `GetGrayRgn()` reads the GrayRgn low-memory global (spec macos9-specs/Misc/GetGrayRgn.md).
    TEToScrap, TEFromScrap, UpperString, GetGrayRgn,
    /// `GetParamText(param0..param3)`: `ParamText`'s four strings copied out (a NIL pointer skips one). `LMGetCurApName()`: the address of the
    /// CurApName low-memory Str31 (a StringPtr, not a value: the launch writes the application's name there).
    GetParamText, CurApName,
    /// The Resource Manager's map globals from its own state (heirloom-mac68k-engine resmgr): `LMGetTopMapHndl` / `LMSetTopMapHndl`, `LMGetCurMap` / `LMSetCurMap`.
    TopMapGet, TopMapSet, CurMapGet, CurMapSet,
    /// `GetIntlResource(theID)`: the 68k `IUGetIntl` (_Pack6 selector 6) body; an ID it does not provide is NIL, named in `refused`.
    GetIntlResource,
    /// `LMGetBootDrive()`: BootDrive, the working-directory reference number of the boot volume's System Folder (Inside Macintosh: Files;
    /// SysEnvirons' sysVRefNum is the same number).
    BootDrive,
    /// `DateString(dateTime, longFlag, result, intlHandle)` / `TimeString(dateTime, wantSeconds, result, intlHandle)`: the 68k IUDateString /
    /// IUTimeString text (heirloom_mac68k_engine::intl::date_text / time_text); the intl handle is not consulted (named in the log).
    DateString, TimeString,
    /// `LaunchApplication(LaunchPBPtr)`: [`crate::process_c`] (a second process in this machine, mediated by the loader and the host's launch
    /// policy). A NIL block or one that is not 'LC' is paramErr.
    LaunchApplication,
    /// `SetOutlinePreferred(Boolean)` / `GetOutlinePreferred()` (the Font Manager's outline-over-bitmap preference): recorded and answered; the
    /// text here is drawn from one bitmap face either way.
    SetOutlinePreferred, GetOutlinePreferred,
    /// `GetCurrentKeyModifiers()` (Carbon): the modifier keys held now. The GuestUi seam carries modifiers WITH each key event, not a held
    /// state between events, so the answer is "none held" (0).
    GetCurrentKeyModifiers,
    /// `LongSecondsToDate(const LongDateTime*, LongDateRec*)` / `LongDateToSeconds(const LongDateRec*, LongDateTime*)` (Script Manager, 64-bit
    /// seconds since 1904): LongDateRec = era, year, month, day, hour, minute, second, dayOfWeek (Sunday 1), dayOfYear, weekOfYear (week 1 holds
    /// January 1), pm, three reserved (14 shorts); era 0 (AD). The calendar is the one SecondsToDate uses (heirloom_mac68k_engine::intl).
    LongSecondsToDate, LongDateToSeconds,
    /// The Memory Manager's zone calls (Inside Macintosh: Memory): this machine has ONE heap zone (heirloom_mac68k_engine::lowmem::ZONE_STUB,
    /// what TheZone / ApplZone / SysZone hold), so `ApplicationZone()`, `SystemZone()` and `HandleZone(h)` answer it, `SetZone(z)` accepts it and
    /// refuses any other zone by name; `GetApplLimit()` is the limit the application heap may grow to - the top of the machine's heap (MemTop): a
    /// PowerPC program's stack is a block of its own, not the space between the heap and ApplLimit (the 68k low-memory ApplLimit keeps its 68k
    /// meaning for StackSpace); `SetApplLimit(p)` is noErr and changes nothing (logged). Photoshop 7 checks GetApplLimit() - ApplicationZone()
    /// against the 3 MB it needs.
    ApplicationZone, SystemZone, HandleZone, SetZone, GetApplLimit, SetApplLimit,
    /// `SetGrowZone(proc)`: recorded and logged; the Memory Manager here does not call a grow-zone procedure when an allocation fails (the
    /// allocation answers memFullErr as it would after a grow-zone procedure that freed nothing). Named limit.
    SetGrowZone,
    /// `RelString(str1, str2, caseSensitive, diacSensitive)` (Pascal strings): -1 / 0 / 1, the `_RelString` rule (heirloom_mac68k_engine::intl::compare_with).
    RelString,
    /// Navigation Services' availability queries (Navigation.h): the Mac OS 9.2.1 / CarbonLib 1.4 personality (gestalt_ppc.rs RULING) has
    /// Navigation Services 1.1, so `NavLibraryVersion` answers 1.1 (a NumVersion) and `NavServicesCanRun()` / `NavServicesAvailable()` TRUE; a Nav
    /// dialog call this engine does not serve yet (NavGetFile, NavPutFile, NavChooseFolder ...) is a per-call NAMED refusal, as the ruling has
    /// every missing call (Photoshop 7 refuses to start without Navigation Services).
    NavLibraryVersion, NavServicesCanRun,
    /// `LMGet<Name>` / `LMSet<Name>` over the table [`LOWMEM`]; the index is the row.
    LmGet(u8), LmSet(u8),
}

/// (accessor suffix, address, bytes): the low-memory globals programs read through the Universal Interfaces' `LMGet`/`LMSet` calls.
pub const LOWMEM: &[(&str, u32, u32)] = &[("ResLoad", 0x0A5E, 1), ("MBarHeight", 0x0BAA, 2), ("CurDirStore", 0x0398, 4), ("Ticks", 0x016A, 4), ("DoubleTime", 0x02F0, 4), ("CaretTime", 0x02F4, 4), ("SFSaveDisk", 0x0214, 2), ("ApplLimit", 0x0130, 4),
    ("ResErr", 0x0A60, 2), ("RndSeed", 0x0156, 4), ("SysFontFam", 0x0BA6, 2), ("TESysJust", 0x0BAC, 2), ("HiliteMode", 0x0938, 1),
    // FractEnable (Inside Macintosh: Text, the Font Manager: fractional widths on when nonzero)
    ("FractEnable", 0x0BF4, 1),
    // Inside Macintosh / LowMem.h addresses of more globals programs read and write by accessor; the engine keeps those it uses (WindowList, GrayRgn,
    // TheZone, CurApRefNum ...) current in the same words, the rest are plain storage a program may set and read back
    ("ROMMapInsert", 0x0B9E, 1), ("TmpResLoad", 0x0B9F, 1), ("SysFontSize", 0x0BA8, 2), ("KeyThresh", 0x018E, 2), ("CurApRefNum", 0x0900, 2),
    ("CurActivate", 0x0A64, 4), ("CurDeactive", 0x0A68, 4), ("CrsrBusy", 0x08CD, 1), ("SysEvtMask", 0x0144, 2), ("WindowList", 0x09D6, 4),
    ("CurStackBase", 0x0908, 4), ("GhostWindow", 0x0A84, 4), ("TheZone", 0x0118, 4), ("GZRootHnd", 0x0328, 4), ("GZMoveHnd", 0x0330, 4),
    ("CurJTOffset", 0x0934, 2), ("LastSPExtra", 0x0B4C, 4), ("GrayRgn", 0x09EE, 4), ("TEScrpLength", 0x0AB0, 2)];

impl TbFn {
    pub fn by_name(name: &str) -> Option<TbFn> {
        use TbFn::*;
        Some(match name {
            "GetIndString" => GetIndString, "CopyPascalStringToC" => CopyPascalStringToC, "CopyCStringToPascal" => CopyCStringToPascal,
            "C2PStr" | "c2pstr" => C2PStr, "P2CStr" | "p2cstr" => P2CStr, "EqualString" => EqualString, "NumToString" => NumToString, "StringToNum" => StringToNum,
            "FindFolder" => FindFolder, "SetEventMask" => SetEventMask, "GetDateTime" => GetDateTime, "GetDblTime" => GetDblTime, "GetCaretTime" => GetCaretTime, "Microseconds" => Microseconds, "UpTime" => UpTime,
            "AbsoluteToNanoseconds" => AbsoluteToNanoseconds,
            "TEToScrap" => TEToScrap, "TEFromScrap" => TEFromScrap, "UpperString" => UpperString, "GetGrayRgn" => GetGrayRgn,
            "GetParamText" => GetParamText, "LMGetCurApName" => CurApName, "LMGetTopMapHndl" => TopMapGet, "LMSetTopMapHndl" => TopMapSet, "LMGetCurMap" => CurMapGet, "LMSetCurMap" => CurMapSet, "GetIntlResource" => GetIntlResource, "LMGetBootDrive" => BootDrive, "DateString" => DateString, "TimeString" => TimeString, "LaunchApplication" => LaunchApplication, "SetOutlinePreferred" => SetOutlinePreferred, "GetOutlinePreferred" => GetOutlinePreferred, "GetCurrentKeyModifiers" => GetCurrentKeyModifiers, "LongSecondsToDate" => LongSecondsToDate, "LongDateToSeconds" => LongDateToSeconds,
            "ApplicationZone" => ApplicationZone, "SystemZone" => SystemZone, "HandleZone" => HandleZone, "SetZone" => SetZone, "GetApplLimit" => GetApplLimit, "SetApplLimit" => SetApplLimit, "SetGrowZone" => SetGrowZone, "RelString" => RelString,
            "NavLibraryVersion" => NavLibraryVersion, "NavServicesCanRun" | "NavServicesAvailable" => NavServicesCanRun,
            // GetMBarHeight() / GetSysFont() are the Universal Interfaces' readers of MBarHeight and SysFontFam (Menus.h, Fonts.h)
            "GetMBarHeight" => return TbFn::by_name("LMGetMBarHeight"),
            "GetSysFont" => return TbFn::by_name("LMGetSysFontFam"),
            // GetSysDirection / SetSysDirection: TESysJust, the system's line direction (0 = left to right, the Roman script's)
            "GetSysDirection" => return TbFn::by_name("LMGetTESysJust"),
            "SetSysDirection" => return TbFn::by_name("LMSetTESysJust"),
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

    /// `f`'s name as `{f:?}` writes it, made once per distinct call (a game calls a handful, many times a frame).
    fn tb_name(&mut self, f: TbFn) -> alloc::rc::Rc<str> {
        if let Some((_, n)) = self.rt.tb_names.iter().find(|(g, _)| *g == f) { return n.clone(); }
        let n: alloc::rc::Rc<str> = alloc::rc::Rc::from(format!("{f:?}"));
        self.rt.tb_names.push((f, n.clone()));
        n
    }

    pub(crate) fn tb_call(&mut self, f: TbFn, cpu: &mut Cpu) -> Result<(), String> {
        use TbFn::*;
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let name = self.tb_name(f);
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
            TEToScrap => self.mac.te_to_scrap().map_err(|e| e.0)? as i32 as u32,
            TEFromScrap => self.mac.te_from_scrap().map_err(|e| e.0)? as i32 as u32,
            UpperString => {
                let mut t = self.pstr_bytes(a0, &name)?;
                heirloom_mac68k_engine::strcmp::upper_string(&mut t, a1 & 0xFF != 0);
                self.put_bytes(a0 + 1, &t, &name)?;
                0
            }
            GetParamText => {
                for (i, p) in [a0, a1, a2, a3].into_iter().enumerate() {
                    if p == 0 { continue }
                    let s = self.mac.tr.dlg.params[i].clone();
                    self.put_pstr(p, &s, &name)?;
                }
                0
            }
            CurApName => heirloom_mac68k_engine::lowmem::CUR_AP_NAME,
            SetOutlinePreferred => { self.sys.outline_preferred = a0 & 0xFF != 0; 0 }
            GetOutlinePreferred => u32::from(self.sys.outline_preferred),
            GetCurrentKeyModifiers => 0,
            RelString => {
                let (x, y) = (self.pstr_bytes(a0, &name)?, self.pstr_bytes(a1, &name)?);
                heirloom_mac68k_engine::intl::compare_with(&x, &y, a2 & 0xFF != 0, a3 & 0xFF != 0) as u32
            }
            SetGrowZone => { self.mac.log.push(format!("[mem] SetGrowZone {a0:#x}: recorded; not called on a failed allocation here (memFullErr as after a grow-zone procedure that freed nothing)")); 0 }
            NavLibraryVersion => 0x0110_8000,
            NavServicesCanRun => 1,
            ApplicationZone | SystemZone => heirloom_mac68k_engine::lowmem::ZONE_STUB,
            HandleZone => { if self.mac.mm.block(a0).is_none() { self.mac.log.push(format!("[mem] HandleZone {a0:#x}: not a handle (answered the one zone)")); } heirloom_mac68k_engine::lowmem::ZONE_STUB }
            SetZone => {
                if a0 != heirloom_mac68k_engine::lowmem::ZONE_STUB { return Err(format!("SetZone {a0:#x}: the only heap zone is {:#x}", heirloom_mac68k_engine::lowmem::ZONE_STUB)); }
                return Ok(());
            }
            GetApplLimit => self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::MEM_TOP).map_err(|_| format!("{name}: MemTop"))?,
            SetApplLimit => { self.mac.log.push(format!("[mem] SetApplLimit {a0:#x}: noErr (the one zone is not bounded by a stack limit here)")); 0 }
            LongSecondsToDate => {
                let secs = (u64::from(self.mac.mem.read_u32(a0).map_err(|_| format!("{name}: the LongDateTime leaves guest RAM"))?) << 32
                    | u64::from(self.mac.mem.read_u32(a0 + 4).map_err(|_| format!("{name}: the LongDateTime leaves guest RAM"))?)) as i64;
                let (y, mo, d, wd, h, mi, s) = heirloom_mac68k_engine::intl::civil_long(secs);
                let doy = heirloom_mac68k_engine::intl::mac_seconds_long(y, i64::from(mo), i64::from(d), 0, 0, 0).div_euclid(86_400)
                    - heirloom_mac68k_engine::intl::mac_seconds_long(y, 1, 1, 0, 0, 0).div_euclid(86_400) + 1;
                let jan1_wd = (i64::from(wd as u32) - (doy - 1)).rem_euclid(7);
                let week = (doy - 1 + jan1_wd) / 7 + 1;
                let f = [0, y as i16, mo as i16, d as i16, h as i16, mi as i16, s as i16, wd as i16 + 1, doy as i16, week as i16, i16::from(h >= 12), 0, 0, 0];
                for (i, v) in f.iter().enumerate() { self.mac.mem.write_u16(a1 + 2 * i as u32, *v as u16).map_err(|_| format!("{name}: the LongDateRec leaves guest RAM"))?; }
                0
            }
            LongDateToSeconds => {
                let g = |m: &Self, i: u32| m.mac.mem.read_u16(a0 + 2 * i).map(|v| i64::from(v as i16)).map_err(|_| format!("{name}: the LongDateRec leaves guest RAM"));
                let secs = heirloom_mac68k_engine::intl::mac_seconds_long(g(self, 1)?, g(self, 2)?, g(self, 3)?, g(self, 4)?, g(self, 5)?, g(self, 6)?);
                self.mac.mem.write_u32(a1, (secs >> 32) as u32).and_then(|_| self.mac.mem.write_u32(a1 + 4, secs as u32)).map_err(|_| format!("{name}: the LongDateTime leaves guest RAM"))?;
                0
            }
            LaunchApplication => return self.launch_application(a0, cpu), // process_c.rs: a second process, mediated
            DateString | TimeString => {
                let text = if f == DateString { heirloom_mac68k_engine::intl::date_text(a0, a1 as u8)? } else { heirloom_mac68k_engine::intl::time_text(a0, a1 & 0xFF != 0) };
                self.put_pstr(a2, text.as_bytes(), &name)?;
                self.mac.log.push(format!("[pack6] {name} {a0} -> \"{text}\"{}", if a3 != 0 { " (the intl handle is not consulted)" } else { "" }));
                0
            }
            TopMapGet => self.mac.top_map_handle().map_err(|e| format!("{name}: {}", e.0))?,
            TopMapSet => { if let Err(e) = self.mac.set_top_map_handle(a0) { self.mac.note_refusal(&name, e.0); } 0 }
            CurMapGet => self.mac.rm.cur as u16 as u32,
            CurMapSet => { if let Err(e) = self.mac.set_cur_map(a0 as u16 as i16) { self.mac.note_refusal(&name, e.0); } 0 }
            BootDrive => self.mac.fm.system_folder_wd().map_err(|e| format!("{name}: the boot volume's System Folder (OSErr {e})"))? as u16 as u32,
            GetIntlResource => match heirloom_mac68k_engine::intl::intl_resource(&mut self.mac, a0 as u16 as i16) {
                Ok((h, log)) => { self.mac.log.push(log); h }
                Err(e) => { self.mac.note_refusal("GetIntlResource", e.0); 0 }
            },
            GetGrayRgn => self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::GRAY_RGN).map_err(|_| format!("{name}: low memory leaves guest RAM"))?,
            NumToString => { let t = heirloom_mac68k_engine::pack7::num_text(a0 as i32); self.put_pstr(a1, t.as_bytes(), &name)?; 0 }
            StringToNum => { let s = self.pstr_bytes(a0, &name)?; let n = heirloom_mac68k_engine::pack7::parse_num(&s); self.mac.mem.write_u32(a1, n as u32).map_err(|_| format!("{name}: result pointer {a1:#x} leaves guest RAM"))?; 0 }
            // FindFolder(vRefNum, folderType, createFolder, &foundVRefNum, &foundDirID): the Folder Manager is the 68k engine's (fm/alias.rs), folders are made on first use
            FindFolder => {
                let (vref, ty) = (a0 as u16 as i16, a1.to_be_bytes());
                let found = self.mac.fm.find_folder(vref, ty);
                self.mac.log.push(format!("[file] FindFolder vRef {vref} '{}' -> {found:?}", ty.iter().map(|&b| char::from(b)).collect::<String>()));
                match found {
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
            // AbsoluteTime / Nanoseconds are STRUCTS returned by value: the Mac OS PowerPC ABI passes the caller's result buffer in r3 and expects it
            // back (the measured DSpGetVersion convention); one unit = one nanosecond of the tick clock, so the conversion is the identity
            UpTime => {
                let ns = self.mac.ev.ticks_total * 1_000_000_000 / 60;
                self.mac.mem.write_u32(a0, (ns >> 32) as u32).and_then(|_| self.mac.mem.write_u32(a0 + 4, ns as u32)).map_err(|_| format!("{name}: result buffer {a0:#x} leaves guest RAM"))?;
                cpu.gpr[3] = a0; return Ok(());
            }
            AbsoluteToNanoseconds => { // (buffer r3, AbsoluteTime by value r4/r5)
                self.mac.mem.write_u32(a0, a1).and_then(|_| self.mac.mem.write_u32(a0 + 4, a2)).map_err(|_| format!("{name}: result buffer {a0:#x} leaves guest RAM"))?;
                cpu.gpr[3] = a0; return Ok(());
            }
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
