//! Data accesses: the ARMv6 alignment rules of [`Unaligned`] and CPSR.E byte order, on top of the host's
//! [`Memory`]. Every guest load and store goes through here.

use crate::cpu::{Cpu, Unaligned};
use crate::{Exit, Fault, Memory, TrapKind};

/// Which alignment rule an access follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// LDR/STR/LDRH/STRH/LDRSH (and the T variants): unaligned allowed when U=1.
    Plain,
    /// LDM/STM, LDRD/STRD, SWP and VFP: word alignment required (U=1), ignored address bits (U=0).
    Multi,
    /// LDREX*/STREX*: natural alignment always required.
    Exclusive,
}

impl Cpu {
    fn fault(&self, fault: Fault) -> Exit {
        Exit::Fault { pc: self.cur, fault }
    }

    fn misaligned(&self, addr: u32, write: bool) -> Exit {
        self.trap(TrapKind::Unaligned { addr, write })
    }

    /// The address an access of `size` bytes actually uses, or the alignment trap. `None` means "compose
    /// from bytes" (an allowed unaligned access).
    fn place(&self, addr: u32, size: u32, kind: Kind, write: bool) -> Result<Option<u32>, Exit> {
        let need = if kind == Kind::Multi { 4 } else { size };
        if addr & (need - 1) == 0 {
            return Ok(Some(addr));
        }
        match (kind, self.config.unaligned) {
            (Kind::Exclusive, _) | (_, Unaligned::Strict) => Err(self.misaligned(addr, write)),
            (Kind::Multi, Unaligned::Armv6) => Err(self.misaligned(addr, write)),
            (Kind::Plain, Unaligned::Armv6) => Ok(None),
            (_, Unaligned::Legacy) => Ok(Some(addr & !(need - 1))),
        }
    }

    /// Check a multi-word transfer's base alignment before any of its accesses run.
    pub(crate) fn check_multi(&self, addr: u32, write: bool) -> Result<u32, Exit> {
        Ok(self.place(addr, 4, Kind::Multi, write)?.unwrap_or(addr))
    }

    pub(crate) fn ld8<M: Memory>(&self, mem: &mut M, addr: u32) -> Result<u32, Exit> {
        mem.read_u8(addr).map(u32::from).map_err(|f| self.fault(f))
    }

    pub(crate) fn st8<M: Memory>(&self, mem: &mut M, addr: u32, value: u32) -> Result<(), Exit> {
        mem.write_u8(addr, value as u8).map_err(|f| self.fault(f))
    }

    fn order16(&self, v: u16) -> u16 {
        if self.big_endian {
            v.swap_bytes()
        } else {
            v
        }
    }

    fn order32(&self, v: u32) -> u32 {
        if self.big_endian {
            v.swap_bytes()
        } else {
            v
        }
    }

    /// Load a halfword, zero-extended.
    pub(crate) fn ld16<M: Memory>(&self, mem: &mut M, addr: u32, kind: Kind) -> Result<u32, Exit> {
        let raw = match self.place(addr, 2, kind, false)? {
            Some(a) => mem.read_u16(a).map_err(|f| self.fault(f))?,
            None => {
                let lo = self.ld8(mem, addr)?;
                let hi = self.ld8(mem, addr.wrapping_add(1))?;
                (lo | hi << 8) as u16
            }
        };
        Ok(self.order16(raw) as u32)
    }

    pub(crate) fn st16<M: Memory>(&self, mem: &mut M, addr: u32, value: u32, kind: Kind) -> Result<(), Exit> {
        let v = self.order16(value as u16);
        match self.place(addr, 2, kind, true)? {
            Some(a) => mem.write_u16(a, v).map_err(|f| self.fault(f)),
            None => {
                self.st8(mem, addr, v as u32)?;
                self.st8(mem, addr.wrapping_add(1), (v >> 8) as u32)
            }
        }
    }

    /// Load a word. A U=0 unaligned `Plain` load returns the aligned word rotated, as ARMv5 did.
    pub(crate) fn ld32<M: Memory>(&self, mem: &mut M, addr: u32, kind: Kind) -> Result<u32, Exit> {
        let raw = match self.place(addr, 4, kind, false)? {
            Some(a) => mem.read_u32(a).map_err(|f| self.fault(f))?,
            None => {
                let mut v = 0;
                for i in 0..4 {
                    v |= self.ld8(mem, addr.wrapping_add(i))? << (8 * i);
                }
                v
            }
        };
        let v = self.order32(raw);
        if kind == Kind::Plain && self.config.unaligned == Unaligned::Legacy {
            return Ok(v.rotate_right(8 * (addr & 3)));
        }
        Ok(v)
    }

    pub(crate) fn st32<M: Memory>(&self, mem: &mut M, addr: u32, value: u32, kind: Kind) -> Result<(), Exit> {
        let v = self.order32(value);
        match self.place(addr, 4, kind, true)? {
            Some(a) => mem.write_u32(a, v).map_err(|f| self.fault(f)),
            None => {
                for i in 0..4 {
                    self.st8(mem, addr.wrapping_add(i), v >> (8 * i))?;
                }
                Ok(())
            }
        }
    }
}
