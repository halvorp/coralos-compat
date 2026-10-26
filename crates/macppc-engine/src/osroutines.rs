//! Operating System routines a PowerPC program calls by C prototype, whose 68k traps take their arguments in D0/A0/A1 and may
//! return TWO values (`PurgeSpace`: D0 and A0) or pack two arguments in one register (`FlushEvents`: D0 = stopMask << 16 | eventMask).
//! The generic [`mac68k_engine::mixed::OsSig`] table maps one-in-one-out Memory Manager calls; the routines here need pointers
//! written back or a combined register, so each is a few lines over [`Mac::os_call_regs`], running the SAME handler as the 68k
//! A-line bridge (prototypes: Universal Interfaces `Events.h`, `Timer.h`/`OSUtils.h`, `MacMemory.h`, `Gestalt.h`).
//! `SysEnvirons(versionRequested, theWorld)` is `D0.W = version, A0 = record` into the 68k `_SysEnvirons` handler; r3 = its OSErr.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::mixed::trap_opcode;
use mac68k_engine::osconv::Regs;

/// `gestaltSysArchitecture`: 1 = 68k, 2 = PowerPC (Gestalt.h; executor src/gestalt.cpp:146 answers `gestaltPowerPC` for it).
/// gestaltUndefSelectorErr.
pub const GESTALT_UNDEF_SELECTOR_ERR: i32 = -5551;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsRoutine { HLockHi, MaxBlock, TempFreeMem, TempMaxMem, FlushEvents, Delay, PurgeSpace, MaxMem, MoveHHi, HandToHand, PtrToHand, PtrAndHand, HandAndHand, Gestalt, SysEnvirons }

impl OsRoutine {
    pub fn by_name(name: &str) -> Option<OsRoutine> {
        use OsRoutine::*;
        Some(match name {
            "HLockHi" => HLockHi, "MaxBlock" => MaxBlock, "TempFreeMem" => TempFreeMem, "TempMaxMem" => TempMaxMem, "FlushEvents" => FlushEvents, "Delay" => Delay, "PurgeSpace" => PurgeSpace, "MaxMem" => MaxMem, "MoveHHi" => MoveHHi,
            "HandToHand" => HandToHand, "PtrToHand" => PtrToHand, "PtrAndHand" => PtrAndHand, "HandAndHand" => HandAndHand, "Gestalt" => Gestalt, "SysEnvirons" => SysEnvirons,
            _ => return None,
        })
    }
    /// How many C arguments the prototype takes.
    pub fn arg_count(self) -> usize {
        use OsRoutine::*;
        match self { MaxMem | MoveHHi | HandToHand | TempMaxMem | HLockHi => 1, MaxBlock | TempFreeMem => 0, FlushEvents | Delay | PurgeSpace | HandAndHand | Gestalt | SysEnvirons => 2, PtrToHand | PtrAndHand => 3 }
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// One routine; the value to put in r3 (0 for `void` routines). A pointer outside guest RAM is a refusal BY NAME.
    pub(crate) fn os_routine(&mut self, c: OsRoutine, a: &[u32]) -> Result<u32, String> {
        use OsRoutine::*;
        let name = format!("{c:?}");
        let store = |m: &mut MacPpc<U>, at: u32, v: u32| m.mac.mem.write_u32(at, v).map_err(|_| format!("{name}: result pointer {at:#x} is outside guest RAM"));
        if c == Gestalt {
            let sel = a[0].to_be_bytes();
            let shown: String = sel.iter().map(|&b| char::from(b)).collect();
            if let Some(v) = crate::gestalt_ppc::answer(sel).or_else(|| matches!(&sel, b"ram " | b"lram").then_some(self.mac.mem.len() as u32)) {
                store(self, a[1], v)?;
                self.mac.log.push(format!("[toolbox] Gestalt '{shown}' -> {v:#x} (PowerPC profile)"));
                return Ok(0);
            }
            if let Some(why) = crate::gestalt_ppc::unanswered(sel) {
                self.mac.log.push(format!("[toolbox] Gestalt '{shown}': gestaltUndefSelectorErr, unanswered for a PowerPC program: {why}"));
                return Ok(GESTALT_UNDEF_SELECTOR_ERR as u32);
            }
        }
        // Memory Manager queries with no trap handler in the shared core: the PowerPC process has the one heap, no temporary-memory partition
        // (Temp* answer the same free space), so `MaxBlock` is the largest free block now, `TempFreeMem` the total free bytes.
        match c {
            // HLockHi(h) is the Memory Manager glue `MoveHHi(h); HLock(h)` (blocks never move here, so the first only validates the handle)
            HLockHi => {
                for step in ["MoveHHi", "HLock"] {
                    let op = trap_opcode(step).ok_or_else(|| format!("{step}: no trap word"))?;
                    let (out, ret) = self.mac.os_call_regs(Regs { opcode: op, d0: 0, a0: a[0], d1: 0, a1: 0 }).map_err(|e| format!("HLockHi: {e}"))?;
                    let _ = (out, ret);
                }
                return Ok(0);
            }
            MaxBlock => return Ok(self.mac.heap.available(true)),
            TempFreeMem => return Ok(self.mac.heap.available(false)),
            TempMaxMem => { store(self, a[0], 0)?; return Ok(self.mac.heap.available(true)); } // *grow = 0: the heap cannot grow
            _ => {}
        }
        let opcode = trap_opcode(&name).ok_or_else(|| format!("{name}: no trap word"))?;
        let mut regs = Regs { opcode, d0: 0, a0: 0, d1: 0, a1: 0 };
        match c {
            FlushEvents => regs.d0 = (a[1] & 0xFFFF) << 16 | (a[0] & 0xFFFF),
            Delay => regs.a0 = a[0],
            PurgeSpace | MaxBlock | TempFreeMem | TempMaxMem | HLockHi => {}
            MaxMem => {}
            MoveHHi => regs.a0 = a[0],
            HandToHand => regs.a0 = self.mac.mem.read_u32(a[0]).map_err(|_| format!("{name}: handle pointer {:#x} is outside guest RAM", a[0]))?,
            PtrToHand => { regs.a0 = a[0]; regs.d0 = a[2]; }
            PtrAndHand => { regs.a0 = a[0]; regs.a1 = a[1]; regs.d0 = a[2]; }
            HandAndHand => { regs.a0 = a[0]; regs.a1 = a[1]; }
            Gestalt => regs.d0 = a[0],
            SysEnvirons => { regs.d0 = a[0] & 0xFFFF; regs.a0 = a[1]; }
        }
        let (out, ret) = self.mac.os_call_regs(regs).map_err(|e| format!("{name}: {e}"))?;
        if ret.blocked { return Err(format!("{name}: the handler tried to block the caller")); }
        let err = out.d0 as u16 as i16 as i32 as u32;
        match c {
            MaxBlock | TempFreeMem | TempMaxMem | HLockHi => unreachable!("answered above"),
            FlushEvents | MoveHHi => Ok(0),
            Delay => { store(self, a[1], out.d0)?; Ok(0) }
            PurgeSpace => { store(self, a[0], out.d0)?; store(self, a[1], out.a0)?; Ok(0) }
            MaxMem => { store(self, a[0], out.a0)?; Ok(out.d0) }
            HandToHand | PtrToHand => { if out.d0 as u16 == 0 { store(self, if c == HandToHand { a[0] } else { a[1] }, out.a0)?; } Ok(err) }
            PtrAndHand | HandAndHand | SysEnvirons => Ok(err),
            Gestalt => { if out.d0 as u16 == 0 { store(self, a[1], out.a0)?; } Ok(err) }
        }
    }
}
