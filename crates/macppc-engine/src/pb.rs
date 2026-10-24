//! The File Manager's parameter-block calls as C functions: `PBReadSync(ParmBlkPtr)`, `PBHGetCatInfoSync(HParmBlkPtr)`, `PBGetFPosAsync` ... Each is the 68k trap (or `_HFSDispatch`
//! selector) the Universal Interfaces' glue issues, with the block pointer where the trap wants A0 and the OSErr in r3 where it wants D0, run through the SAME handler:
//! `Mac::os_call_regs` for the plain traps (HFS form = trap bit 9) and `mac68k_engine::fm::traps::call_selector` for the `HFSDispatch` family.
//! `...Async` completes before it returns (the volume is memory): `ioResult` is already the final result when the program polls it. An asynchronous call with a completion routine
//! (`ioCompletion` != 0) is refused BY NAME: running it needs the callback machine and no corpus program needs it yet. Not provided, so unbound by name: PBCatSearch,
//! PBXGetVolInfo, PBUnmountVol, the Desktop Database calls (PBDT*), PBHCopyFile.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::fm::pb::IO_COMPLETION;
use mac68k_engine::fm::traps::sel;
use mac68k_engine::mixed::trap_opcode;
use mac68k_engine::osconv::Regs;

/// What a `PB...` name runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PbCall { kind: Kind, hfs: bool, async_call: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind { Trap(u8), Selector(u16) }

/// (C name without `PB`/`H`/`Sync`/`Async`, trap name): the plain traps. The `H` prefix selects the HFS form of the same trap.
const TRAPS: &[(&str, &str)] = &[
    ("Read", "Read"), ("Write", "Write"), ("GetEOF", "GetEOF"), ("SetEOF", "SetEOF"), ("GetFPos", "GetFPos"), ("SetFPos", "SetFPos"), ("Close", "Close"), ("FlushFile", "FlushFile"),
    ("Allocate", "Allocate"), ("Open", "Open"), ("OpenRF", "OpenRF"), ("Create", "Create"), ("Delete", "Delete"), ("Rename", "Rename"), ("GetFInfo", "GetFileInfo"),
    ("SetFInfo", "SetFileInfo"), ("SetFLock", "SetFilLock"), ("RstFLock", "RstFilLock"), ("GetVInfo", "GetVolInfo"), ("GetVol", "GetVol"), ("SetVol", "SetVol"), ("FlushVol", "FlushVol"),
];
/// The `_HFSDispatch` selectors by C name (all HFS-only, except `OpenDF`'s two forms).
const SELECTORS: &[(&str, u16)] = &[("OpenWD", sel::OPEN_WD), ("CloseWD", sel::CLOSE_WD), ("CatMove", sel::CAT_MOVE), ("DirCreate", sel::DIR_CREATE), ("GetWDInfo", sel::GET_WD_INFO),
    ("GetFCBInfo", sel::GET_FCB_INFO), ("GetCatInfo", sel::GET_CAT_INFO), ("GetVolParms", sel::GET_VOL_PARMS), ("SetCatInfo", sel::SET_CAT_INFO), ("OpenDF", sel::OPEN_DF)];

impl PbCall {
    pub fn by_name(name: &str) -> Option<PbCall> {
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
        if c.async_call && self.mac.mem.read_u32(pb.wrapping_add(IO_COMPLETION)).map_err(|_| format!("PB call: the parameter block at {pb:#x} leaves guest RAM"))? != 0 {
            return Err(String::from("PB...Async with a completion routine: running it needs the callback machine (ioCompletion != 0 is refused)"));
        }
        match c.kind {
            Kind::Trap(i) => {
                let base = trap_opcode(TRAPS[i as usize].1).ok_or_else(|| format!("{}: no trap word", TRAPS[i as usize].1))?;
                let op = base | if c.hfs { 0x0200 } else { 0 } | if c.async_call { 0x0400 } else { 0 };
                let (regs, _) = self.mac.os_call_regs(Regs { opcode: op, d0: 0, a0: pb, d1: 0, a1: 0 }).map_err(|e| format!("{}: {e}", TRAPS[i as usize].0))?;
                Ok(regs.d0 as u16 as i16)
            }
            Kind::Selector(s) => match mac68k_engine::fm::traps::call_selector(&mut self.mac, s, pb, c.async_call, c.hfs) {
                Some(Ok(e)) => Ok(e),
                Some(Err(r)) => Err(r.0),
                None => Err(format!("HFSDispatch selector {s:#x} is not modelled")),
            },
        }
    }
}
