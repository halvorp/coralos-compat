//! The File Manager's parameter-block calls as C functions: `PBReadSync(ParmBlkPtr)`, `PBHGetCatInfoSync(HParmBlkPtr)`, `PBGetFPosAsync` ... Each is the 68k trap (or `_HFSDispatch`
//! selector) the Universal Interfaces' glue issues, with the block pointer where the trap wants A0 and the OSErr in r3 where it wants D0, run through the SAME handler:
//! `Mac::os_call_regs` for the plain traps (HFS form = trap bit 9) and `heirloom_mac68k_engine::fm::traps::call_selector` for the `HFSDispatch` family.
//! `...Async` completes before it returns (the volume is memory or a stream read in full): `ioResult` is already the final result when the program polls it. An asynchronous call
//! with a completion routine (`ioCompletion` != 0, Warcraft III's MPQ reads) is completed the same way and its routine is QUEUED: it runs, with the parameter block in r3, at the
//! next glue call (a safepoint, [`MacPpc::io_safepoint`]), never inside the call that issued it - on a real Mac it runs at interrupt time after the call has returned, so a program
//! that sets up state after `PB...Async` returns still sees its completion afterwards. Not provided, so unbound by name: PBCatSearch,
//! PBUnmountVol, the Desktop Database calls (PBDT*), PBHCopyFile.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::fm::pb::IO_COMPLETION;
use heirloom_mac68k_engine::fm::traps::sel;
use heirloom_mac68k_engine::mixed::trap_opcode;
use heirloom_mac68k_engine::osconv::Regs;

/// What a `PB...` name runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbCall { kind: Kind, hfs: bool, async_call: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind { Trap(u8), Selector(u16), NoAccessControl, OpenDeny { rsrc: bool } }

/// (C name without `PB`/`H`/`Sync`/`Async`, trap name): the plain traps. The `H` prefix selects the HFS form of the same trap.
const TRAPS: &[(&str, &str)] = &[
    ("Read", "Read"), ("Write", "Write"), ("GetEOF", "GetEOF"), ("SetEOF", "SetEOF"), ("GetFPos", "GetFPos"), ("SetFPos", "SetFPos"), ("Close", "Close"), ("FlushFile", "FlushFile"),
    ("Allocate", "Allocate"), ("Open", "Open"), ("OpenRF", "OpenRF"), ("Create", "Create"), ("Delete", "Delete"), ("Rename", "Rename"), ("GetFInfo", "GetFileInfo"),
    ("SetFInfo", "SetFileInfo"), ("SetFLock", "SetFilLock"), ("RstFLock", "RstFilLock"), ("GetVInfo", "GetVolInfo"), ("GetVol", "GetVol"), ("SetVol", "SetVol"), ("FlushVol", "FlushVol"),
];
/// The `_HFSDispatch` selectors by C name (all HFS-only, except `OpenDF`'s two forms).
const SELECTORS: &[(&str, u16)] = &[("OpenWD", sel::OPEN_WD), ("CloseWD", sel::CLOSE_WD), ("CatMove", sel::CAT_MOVE), ("DirCreate", sel::DIR_CREATE), ("GetWDInfo", sel::GET_WD_INFO),
    ("GetFCBInfo", sel::GET_FCB_INFO), ("GetCatInfo", sel::GET_CAT_INFO), ("GetVolParms", sel::GET_VOL_PARMS), ("SetCatInfo", sel::SET_CAT_INFO), ("OpenDF", sel::OPEN_DF),
    ("XGetVolInfo", sel::XGET_VOL_INFO)];

impl PbCall {
    pub fn by_name(name: &str) -> Option<PbCall> {
        // the Desktop Manager's synchronous calls have no Sync suffix (PBDTGetPath)
        if name == "PBDTGetPath" { return Some(PbCall { kind: Kind::Selector(heirloom_mac68k_engine::fm::traps::sel::DT_GET_PATH), hfs: true, async_call: false }); }
        // PBHGetDirAccess / PBHSetDirAccess (_HFSDispatch $32 / $33): AppleShare access privileges. These volumes are local and do not have them
        // (PBHGetVolParms reports vMAttrib 0: bAccessCntl clear, which a program is meant to test): paramErr on a volume that exists, nsvErr else
        if let Some(rest) = name.strip_prefix("PBH").and_then(|r| r.strip_suffix("Sync").map(|c| (c, false)).or_else(|| r.strip_suffix("Async").map(|c| (c, true)))) {
            if matches!(rest.0, "GetDirAccess" | "SetDirAccess") { return Some(PbCall { kind: Kind::NoAccessControl, hfs: true, async_call: rest.1 }); }
        }
        // PBHOpenDeny / PBHOpenRFDeny (_HFSDispatch $38 / $39): the HFS Open / OpenRF with the access the block's ioDenyModes asks for
        if let Some((rest, async_call)) = name.strip_prefix("PBH").and_then(|r| r.strip_suffix("Sync").map(|c| (c, false)).or_else(|| r.strip_suffix("Async").map(|c| (c, true)))) {
            if rest == "OpenDeny" || rest == "OpenRFDeny" { return Some(PbCall { kind: Kind::OpenDeny { rsrc: rest == "OpenRFDeny" }, hfs: true, async_call }); }
        }
        let rest = name.strip_prefix("PB")?;
        let (core, async_call) = match (rest.strip_suffix("Sync"), rest.strip_suffix("Async")) { (Some(c), _) => (c, false), (_, Some(c)) => (c, true), _ => return None };
        let (hfs, base) = match core.strip_prefix('H') { Some(b) if TRAPS.iter().any(|(n, _)| *n == b) || SELECTORS.iter().any(|(n, _)| *n == b) => (true, b), _ => (false, core) };
        if let Some(i) = TRAPS.iter().position(|(n, _)| *n == base) { return Some(PbCall { kind: Kind::Trap(i as u8), hfs, async_call }); }
        SELECTORS.iter().find(|(n, _)| *n == base).map(|&(_, s)| PbCall { kind: Kind::Selector(s), hfs, async_call })
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// Run `c` on the parameter block `pb`; the OSErr for r3.
    pub(crate) fn pb_call(&mut self, c: PbCall, pb: u32) -> Result<i16, String> {
        let completion = if c.async_call { self.mac.mem.read_u32(pb.wrapping_add(IO_COMPLETION)).map_err(|_| format!("PB call: the parameter block at {pb:#x} leaves guest RAM"))? } else { 0 };
        let e = self.pb_run(c, pb)?;
        if completion != 0 {
            // the shared trap body queued its 68k-convention call (D0/A0); this PowerPC caller runs the routine itself (r3 = block)
            self.mac.take_async_completion();
            self.rt.io_done.push_back((pb, completion));
        }
        Ok(e)
    }

    /// At a safepoint (`call_pc` is the glue call about to run): the oldest queued I/O completion routine is entered with its parameter block; the interrupted call runs again
    /// when it returns. `Ok(true)` = the CPU now runs the routine.
    pub(crate) fn io_safepoint(&mut self, cpu: &mut crate::cpu::Cpu, call_pc: u32) -> Result<bool, String> {
        if !self.rt.callbacks.is_empty() { return Ok(false); }
        let Some((pb, upp)) = self.rt.io_done.pop_front() else { return Ok(false) };
        let saved = crate::callback::Saved::of(cpu, call_pc);
        let sp = cpu.gpr[1];
        self.enter_upp(cpu, upp, &[pb], sp).map_err(|e| format!("I/O completion routine {upp:#x} for the parameter block {pb:#x}: {e}"))?;
        self.mac.log.push(format!("[file] completion routine {upp:#x} for the parameter block {pb:#x} runs at this safepoint"));
        self.rt.callbacks.push(crate::callback::Frame { saved, cont: crate::callback::Continuation::IoCompletion });
        Ok(true)
    }

    fn pb_run(&mut self, c: PbCall, pb: u32) -> Result<i16, String> {
        match c.kind {
            Kind::Trap(i) => {
                let base = trap_opcode(TRAPS[i as usize].1).ok_or_else(|| format!("{}: no trap word", TRAPS[i as usize].1))?;
                let op = base | if c.hfs { 0x0200 } else { 0 } | if c.async_call { 0x0400 } else { 0 };
                let (regs, _) = self.mac.os_call_regs(Regs { opcode: op, d0: 0, a0: pb, d1: 0, a1: 0 }).map_err(|e| format!("{}: {e}", TRAPS[i as usize].0))?;
                Ok(regs.d0 as u16 as i16)
            }
            // ioDenyModes (26, a word over HIOParam's ioVersNum / ioPermssn): its access bits (dmRd 1, dmWr 2) are the open's permission (0 =
            // fsCurPerm); its deny bits (dmNoneDenyRd 0x10, dmNoneDenyWr 0x20) are NOT enforced against other openers (one program opens these
            // local volumes' files at a time here: named). The word is put back after the open.
            Kind::OpenDeny { rsrc } => {
                let at = pb.wrapping_add(26);
                let modes = self.mac.mem.read_u16(at).map_err(|_| format!("PBHOpenDeny: the block {pb:#x} leaves guest RAM"))?;
                let _ = self.mac.mem.write_u16(at, modes & 3);
                let i = TRAPS.iter().position(|(n, _)| *n == if rsrc { "OpenRF" } else { "Open" }).expect("in the table") as u8;
                let r = self.pb_run(PbCall { kind: Kind::Trap(i), hfs: true, async_call: c.async_call }, pb);
                let _ = self.mac.mem.write_u16(at, modes);
                self.mac.log.push(format!("[file] PBHOpen{}Deny ioDenyModes {modes:#x} -> {r:?} (deny bits not enforced: one opener)", if rsrc { "RF" } else { "" }));
                r
            }
            Kind::NoAccessControl => {
                let vref = self.mac.mem.read_u16(pb.wrapping_add(heirloom_mac68k_engine::fm::pb::IO_VREF_NUM)).map_err(|_| format!("PBHGet/SetDirAccess: the block {pb:#x} leaves guest RAM"))? as i16;
                let e: i16 = if self.mac.fm.volume_of_vref(vref).is_ok() { -50 } else { -35 };
                let _ = self.mac.mem.write_u16(pb.wrapping_add(heirloom_mac68k_engine::fm::pb::IO_RESULT), e as u16);
                self.mac.log.push(format!("[file] PBH(Get|Set)DirAccess vRef {vref} -> {e} (no access privileges on a local volume)"));
                Ok(e)
            }
            Kind::Selector(s) => match heirloom_mac68k_engine::fm::traps::call_selector(&mut self.mac, s, pb, c.async_call, c.hfs) {
                Some(Ok(e)) => {
                    let name = SELECTORS.iter().find(|(_, x)| *x == s).map_or("?", |(n, _)| n);
                    let np = self.mac.mem.read_u32(pb.wrapping_add(heirloom_mac68k_engine::fm::pb::IO_NAME_PTR)).unwrap_or(0);
                    let shown = if np == 0 { String::new() } else { heirloom_mac68k_engine::pascal::read_pstring_bytes(&self.mac.mem, np).map(|b| format!(" \"{}\"", heirloom_mac68k_engine::fm::name_of(&b))).unwrap_or_default() };
                    self.mac.log.push(format!("[file] PB{}{name}{shown} -> {e}", if c.hfs { "H" } else { "" }));
                    Ok(e)
                }
                Some(Err(r)) => Err(r.0),
                None => Err(format!("HFSDispatch selector {s:#x} is not modelled")),
            },
        }
    }
}
