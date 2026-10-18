//! Shared harness: an LLVM-assembled instruction oracle, and a tiny machine that runs a program to a native exit.
#![allow(dead_code)]
pub mod oracle;
pub mod programs;
use retro_guest_mem::GuestMem;
use retro_ppc::{resume, Cpu, Host, RunStats};

pub const CODE: u32 = 0x1000;
pub const EXIT: u32 = 0xF000;
pub const STACK: u32 = 0x8000;

/// The encoding LLVM produced for this assembler line (committed in oracle.rs; regenerate with tools/ppc-regen.py).
pub fn asm(src: &str) -> u32 {
    oracle::ORACLE.iter().find(|(s, _)| *s == src).unwrap_or_else(|| panic!("asm({src:?}) is not in oracle.rs: run tools/ppc-regen.py")).1
}

pub struct Vm {
    pub mem: GuestMem,
    pub natives: Vec<u32>,
    pub syscalls: Vec<u32>,
    /// What a handled `sc` does: set r3 to this.
    pub sc_result: u32,
}

impl Host for Vm {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.natives.push(addr);
        if addr == EXIT { cpu.halt(); }
        true
    }
    fn syscall(&mut self, cpu: &mut Cpu) -> bool {
        self.syscalls.push(cpu.gpr[0]);
        cpu.gpr[3] = self.sc_result;
        true
    }
}

pub fn vm() -> Vm { Vm { mem: GuestMem::new(1 << 20), natives: vec![], syscalls: vec![], sc_result: 0 } }

/// Load `words` at CODE, return to EXIT through LR (bound native) after them, run, and hand back everything.
pub fn run_words(words: &[u32], setup: impl FnOnce(&mut Cpu, &mut Vm)) -> (Cpu, Vm, RunStats) {
    let mut v = vm();
    for (i, w) in words.iter().enumerate() { v.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    v.mem.write_u32(CODE + 4 * words.len() as u32, asm("blr")).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    c.lr = EXIT;
    c.bind_native(EXIT);
    setup(&mut c, &mut v);
    let st = resume(&mut v, &mut c, 1_000_000);
    (c, v, st)
}

/// Assemble-by-oracle and run a straight-line list of instructions.
pub fn prog(lines: &[&str], setup: impl FnOnce(&mut Cpu, &mut Vm)) -> (Cpu, Vm, RunStats) {
    let words: Vec<u32> = lines.iter().map(|l| asm(l)).collect();
    run_words(&words, setup)
}

/// Run `lines` with only a register preset; panics unless it ended cleanly at EXIT.
pub fn go(lines: &[&str], regs: &[(usize, u32)]) -> Cpu {
    let (c, _, st) = prog(lines, |c, _| for &(r, v) in regs { c.gpr[r] = v });
    assert!(st.halted && st.trap.is_none(), "{lines:?}: {st:?}");
    c
}

/// [`go`] with XER preset too (carry / summary-overflow tests).
pub fn gox(lines: &[&str], regs: &[(usize, u32)], xer: u32) -> Cpu {
    let (c, _, st) = prog(lines, |c, _| { c.xer = xer; for &(r, v) in regs { c.gpr[r] = v } });
    assert!(st.halted && st.trap.is_none(), "{lines:?}: {st:?}");
    c
}
