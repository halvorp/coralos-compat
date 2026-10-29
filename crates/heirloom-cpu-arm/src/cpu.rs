// Adapted from HexRaysSA/rax src/isa/arm/aarch32/cpu.rs and src/backend/emulator/aarch32.rs (commit 526acfa).
// Copyright (c) 2026 RAX contributors. MIT License; see crates/heirloom-cpu-arm/README.md.
//
//! The user-mode register file, the CPSR, and the fetch/execute loop.

use crate::{Config, Exit, Memory, Trap, TrapKind, Unaligned, Vfp};
use heirloom_cpu_abi::arm::{ArmContext, ArmExit, Psr};

/// One ARMv6 hardware thread in user mode.
#[derive(Clone, Debug)]
pub struct Cpu {
    /// R0-R15. `regs[15]` is the address of the next instruction to execute (not PC+8).
    pub regs: [u32; 16],
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    /// The sticky saturation flag.
    pub q: bool,
    /// The SIMD GE[3:0] flags.
    pub ge: u8,
    /// Thumb state (CPSR.T).
    pub thumb: bool,
    /// Big-endian data accesses (CPSR.E, set by SETEND BE). Instruction fetches stay little-endian.
    pub big_endian: bool,
    pub vfp: Vfp,
    /// CP15 c13 TPIDRURW: user read/write thread ID.
    pub tpidrurw: u32,
    /// CP15 c13 TPIDRURO: user read-only thread ID (the OS writes it here).
    pub tpidruro: u32,
    pub config: Config,
    /// The local exclusive monitor: (address, size) tagged by the last LDREX*.
    exclusive: Option<(u32, u8)>,
    /// Address and word of the instruction being executed.
    pub(crate) cur: u32,
    pub(crate) insn: u32,
}

/// The result of executing one instruction: `Err` stops the run loop.
pub(crate) type Step = Result<(), Exit>;

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new(Config::default())
    }
}

impl Cpu {
    pub fn new(config: Config) -> Self {
        Cpu {
            regs: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            ge: 0,
            thumb: false,
            big_endian: false,
            vfp: Vfp::default(),
            tpidrurw: 0,
            tpidruro: 0,
            config,
            exclusive: None,
            cur: 0,
            insn: 0,
        }
    }

    /// The address of the next instruction.
    pub fn pc(&self) -> u32 {
        self.regs[15]
    }

    /// Branch the CPU to `addr`; bit 0 selects Thumb, as BX does.
    pub fn set_pc_interworking(&mut self, addr: u32) {
        self.thumb = addr & 1 != 0;
        self.regs[15] = if self.thumb { addr & !1 } else { addr & !3 };
    }

    /// The CPSR as MRS reads it in user mode (mode bits = User, I/F/A clear).
    pub fn cpsr(&self) -> u32 {
        Psr { n: self.n, z: self.z, c: self.c, v: self.v, q: self.q, ge: self.ge, big_endian: self.big_endian, thumb: self.thumb }.pack()
    }

    /// Host-side CPSR write: flags, GE, E and T (the mode field is ignored; this core is user mode only).
    pub fn set_cpsr(&mut self, value: u32) {
        let p = Psr::unpack(value);
        (self.n, self.z, self.c, self.v, self.q, self.ge) = (p.n, p.z, p.c, p.v, p.q, p.ge);
        self.big_endian = p.big_endian;
        self.thumb = p.thumb;
    }

    /// Write the APSR fields MSR may change from user mode: NZCVQ (`flags`) and GE (`ge`).
    pub(crate) fn set_apsr(&mut self, value: u32, flags: bool, ge: bool) {
        if flags {
            self.n = value & (1 << 31) != 0;
            self.z = value & (1 << 30) != 0;
            self.c = value & (1 << 29) != 0;
            self.v = value & (1 << 28) != 0;
            self.q = value & (1 << 27) != 0;
        }
        if ge {
            self.ge = ((value >> 16) & 0xF) as u8;
        }
    }

    /// Forget the exclusive reservation (the host calls this on a thread switch, as CLREX would).
    pub fn clear_exclusive(&mut self) {
        self.exclusive = None;
    }

    pub(crate) fn mark_exclusive(&mut self, addr: u32, size: u8) {
        self.exclusive = Some((addr, size));
    }

    /// STREX's check: does the monitor hold (addr, size)? (STREX clears it afterwards.)
    pub(crate) fn exclusive_holds(&self, addr: u32, size: u8) -> bool {
        self.exclusive == Some((addr, size))
    }

    /// Execute up to `budget` instructions.
    pub fn run<M: Memory>(&mut self, mem: &mut M, budget: u64) -> Exit {
        for _ in 0..budget {
            if let Err(exit) = self.execute_one(mem) {
                return exit;
            }
        }
        Exit::BudgetSpent
    }

    /// [`run`](Self::run), also saying how many instructions it retired: every instruction that completed, plus the one that
    /// stopped the run unless it was refused (a Fault, Trap or Bkpt leaves the PC on it). An engine counting one per
    /// [`step`](Self::step) gets the same total from one call.
    pub fn run_counted<M: Memory>(&mut self, mem: &mut M, budget: u64) -> (Exit, u64) {
        for done in 0..budget {
            if let Err(exit) = self.execute_one(mem) {
                let stopped_on = match exit { Exit::Svc(_) | Exit::Wait => 1, _ => 0 };
                return (exit, done + stopped_on);
            }
        }
        (Exit::BudgetSpent, budget)
    }

    /// The thread's whole state as the CPU contract publishes it (heirloom-cpu-abi::arm::ArmContext), with `exit` as the
    /// record of why it last stopped.
    pub fn to_context(&self, exit: &Exit) -> ArmContext {
        let (exclusive_addr, exclusive_size) = match self.exclusive { Some((a, s)) => (a, s as u32), None => (0, 0) };
        ArmContext {
            regs: self.regs,
            cpsr: self.cpsr(),
            vfp_s: self.vfp.s,
            fpscr: self.vfp.fpscr,
            tpidrurw: self.tpidrurw,
            tpidruro: self.tpidruro,
            unaligned: self.config.unaligned.to_raw(),
            vfp: self.config.vfp as u32,
            exclusive_addr,
            exclusive_size,
            exit: ArmExit::encode(exit),
        }
    }

    /// A CPU in the state `ctx` names; `None` when it names a configuration or reservation no core has (refused, not guessed).
    pub fn from_context(ctx: &ArmContext) -> Option<Cpu> {
        let unaligned = Unaligned::from_raw(ctx.unaligned)?;
        let vfp = match ctx.vfp { 0 => false, 1 => true, _ => return None };
        let exclusive = match ctx.exclusive_size {
            0 => None,
            s @ (1 | 2 | 4 | 8) => Some((ctx.exclusive_addr, s as u8)),
            _ => return None,
        };
        let mut cpu = Cpu::new(Config { unaligned, vfp });
        cpu.regs = ctx.regs;
        cpu.set_cpsr(ctx.cpsr);
        cpu.vfp.s = ctx.vfp_s;
        cpu.vfp.fpscr = ctx.fpscr;
        cpu.tpidrurw = ctx.tpidrurw;
        cpu.tpidruro = ctx.tpidruro;
        cpu.exclusive = exclusive;
        Some(cpu)
    }

    /// Execute one instruction: `run(mem, 1)`. A normally completed instruction returns `BudgetSpent`.
    pub fn step<M: Memory>(&mut self, mem: &mut M) -> Exit {
        self.run(mem, 1)
    }

    fn execute_one<M: Memory>(&mut self, mem: &mut M) -> Step {
        let pc = self.regs[15];
        self.cur = pc;
        let result = if self.thumb {
            self.execute_thumb(mem, pc)
        } else {
            self.execute_a32(mem, pc)
        };
        if let Err(Exit::Fault { .. } | Exit::Trap(_) | Exit::Bkpt(_)) = result {
            // Not executed: the PC goes back onto the instruction. (Handlers check and access memory
            // before they write any register, so nothing else needs undoing.)
            self.regs[15] = self.cur;
        }
        result
    }

    fn execute_a32<M: Memory>(&mut self, mem: &mut M, pc: u32) -> Step {
        self.insn = 0;
        if pc & 3 != 0 {
            return Err(self.trap(TrapKind::Unpredictable));
        }
        let insn = mem.fetch_u32(pc).map_err(|fault| Exit::Fault { pc, fault })?;
        self.insn = insn;
        self.regs[15] = pc.wrapping_add(4);
        let cond = insn >> 28;
        if cond == 0xF {
            return self.a32_unconditional(insn);
        }
        if !crate::alu::condition_passed(cond, self.n, self.z, self.c, self.v) {
            return Ok(());
        }
        self.a32(mem, insn)
    }

    fn execute_thumb<M: Memory>(&mut self, mem: &mut M, pc: u32) -> Step {
        self.insn = 0;
        let hw = mem.fetch_u16(pc).map_err(|fault| Exit::Fault { pc, fault })?;
        self.insn = hw as u32;
        self.regs[15] = pc.wrapping_add(2);
        self.thumb16(mem, hw)
    }

    /// Read a register as an operand: R15 reads as the current instruction + 8 (A32) or + 4 (Thumb).
    #[inline]
    pub(crate) fn r(&self, n: u32) -> u32 {
        if n == 15 {
            self.cur.wrapping_add(if self.thumb { 4 } else { 8 })
        } else {
            self.regs[n as usize]
        }
    }

    /// Write R0-R14. (R15 writes go through one of the *_write_pc helpers.)
    #[inline]
    pub(crate) fn set_r(&mut self, n: u32, value: u32) {
        debug_assert!(n < 15);
        self.regs[n as usize] = value;
    }

    /// BranchWritePC: a branch that keeps the instruction set.
    pub(crate) fn branch_write_pc(&mut self, addr: u32) {
        self.regs[15] = if self.thumb { addr & !1 } else { addr & !3 };
    }

    /// BXWritePC (also LoadWritePC on ARMv5T+): bit 0 selects Thumb; an ARM target with bit 1 set is
    /// UNPREDICTABLE and refused.
    pub(crate) fn bx_write_pc(&mut self, addr: u32) -> Step {
        if addr & 1 != 0 {
            self.thumb = true;
            self.regs[15] = addr & !1;
        } else if addr & 2 != 0 {
            return Err(self.trap(TrapKind::Unpredictable));
        } else {
            self.thumb = false;
            self.regs[15] = addr;
        }
        Ok(())
    }

    /// Write a result register that may be R15 (data-processing: ALUWritePC = BranchWritePC on ARMv6).
    pub(crate) fn write_alu(&mut self, d: u32, value: u32) {
        if d == 15 {
            self.branch_write_pc(value);
        } else {
            self.set_r(d, value);
        }
    }

    pub(crate) fn set_nz(&mut self, value: u32) {
        self.n = value >> 31 != 0;
        self.z = value == 0;
    }

    pub(crate) fn trap(&self, kind: TrapKind) -> Exit {
        Exit::Trap(Trap { pc: self.cur, insn: self.insn, thumb: self.thumb, kind })
    }

    pub(crate) fn undefined(&self) -> Exit {
        self.trap(TrapKind::Undefined)
    }

    pub(crate) fn unpredictable(&self) -> Exit {
        self.trap(TrapKind::Unpredictable)
    }
}
