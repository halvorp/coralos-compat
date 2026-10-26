// Adapted from HexRaysSA/rax src/isa/arm/aarch32/instructions/coproc.rs (pl0_cp15) (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! Coprocessor dispatch: cp10/cp11 go to VFP, cp15 serves what ARMv6 user mode may reach (the c13 thread
//! ID registers and the c7 barrier operations); everything else is a named trap.

use crate::a32::{bit, bits};
use crate::cpu::{Cpu, Step};
use crate::{Memory, TrapKind};

impl Cpu {
    /// cccc 110x xxxx: LDC/STC and MCRR/MRRC.
    pub(crate) fn a32_coprocessor_load_store<M: Memory>(&mut self, mem: &mut M, insn: u32) -> Step {
        let cp = bits(insn, 11, 8) as u8;
        match cp {
            10 | 11 if self.config.vfp => {
                if bits(insn, 24, 21) == 0b0010 {
                    self.vfp_move_two(insn)
                } else {
                    self.vfp_load_store(mem, insn)
                }
            }
            15 if bits(insn, 24, 21) == 0b0010 => Err(self.trap(TrapKind::Cp15 {
                read: bit(insn, 20),
                opc1: bits(insn, 7, 4) as u8,
                crn: 0,
                crm: bits(insn, 3, 0) as u8,
                opc2: 0,
            })),
            15 => Err(self.undefined()), // no LDC/STC on CP15
            _ => Err(self.trap(TrapKind::Coprocessor { cp })),
        }
    }

    /// cccc 1110 xxxx: CDP and MCR/MRC.
    pub(crate) fn a32_coprocessor(&mut self, insn: u32) -> Step {
        let cp = bits(insn, 11, 8) as u8;
        match cp {
            10 | 11 if self.config.vfp => {
                if bit(insn, 4) {
                    self.vfp_move_core(insn)
                } else {
                    self.vfp_data_processing(insn)
                }
            }
            15 if bit(insn, 4) => self.cp15_transfer(insn),
            15 => Err(self.undefined()),
            _ => Err(self.trap(TrapKind::Coprocessor { cp })),
        }
    }

    /// MCR/MRC p15 from user mode.
    fn cp15_transfer(&mut self, insn: u32) -> Step {
        let read = bit(insn, 20);
        let (opc1, crn, rt, opc2, crm) =
            (bits(insn, 23, 21), bits(insn, 19, 16), bits(insn, 15, 12), bits(insn, 7, 5), bits(insn, 3, 0));
        let refuse = |cpu: &Self| {
            Err(cpu.trap(TrapKind::Cp15 { read, opc1: opc1 as u8, crn: crn as u8, crm: crm as u8, opc2: opc2 as u8 }))
        };
        if opc1 != 0 {
            return refuse(self);
        }
        match (crn, crm, opc2, read) {
            (13, 0, 2, true) => self.cp15_read(rt, self.tpidrurw),
            (13, 0, 2, false) => {
                self.tpidrurw = self.r(rt);
                Ok(())
            }
            (13, 0, 3, true) => self.cp15_read(rt, self.tpidruro),
            // c7: Flush Prefetch Buffer (ISB), Data Synchronization Barrier, Data Memory Barrier.
            // A single-threaded interpreter is already ordered: these complete immediately.
            (7, 5, 4, false) | (7, 10, 4, false) | (7, 10, 5, false) => Ok(()),
            _ => refuse(self),
        }
    }

    fn cp15_read(&mut self, rt: u32, value: u32) -> Step {
        if rt == 15 {
            return Err(self.unpredictable()); // MRC to APSR_nzcv is not meaningful for these registers
        }
        self.set_r(rt, value);
        Ok(())
    }
}
