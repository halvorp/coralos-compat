//! Fetch, decode and dispatch by primary opcode (PEM chapter 8 "Instruction Set", table of opcodes).
use crate::cpu::{Cpu, Trap};
use crate::fields::W;
use crate::{branch, fpmem, fpu, int, mem, x31, Host};

pub(crate) type R = Result<(), Trap>;

impl Cpu {
    pub(crate) fn unimpl(&self, w: W, what: &'static str) -> Trap { Trap::Unimplemented { pc: self.pc, word: w.0, what } }
    pub(crate) fn invalid(&self, w: W, why: &'static str) -> Trap { Trap::InvalidForm { pc: self.pc, word: w.0, why } }
    pub(crate) fn privileged(&self, w: W, what: &'static str) -> Trap { Trap::Privileged { pc: self.pc, word: w.0, what } }
}

pub(crate) fn step<H: Host>(host: &mut H, cpu: &mut Cpu) -> R {
    let pc = cpu.pc;
    cpu.note_pc(pc);
    cpu.redirected = false;
    if cpu.natives.contains(&pc) {
        if !host.native(cpu, pc) { return Err(Trap::NativeUnhandled { addr: pc }); }
        if !cpu.redirected { cpu.pc = cpu.lr & !3; }
        cpu.tb = cpu.tb.wrapping_add(1);
        return Ok(());
    }
    let word = host.mem().read_u32(pc).map_err(|_| Trap::BusFault { pc, addr: pc, write: false })?;
    cpu.next = pc.wrapping_add(4);
    execute(host, cpu, W(word))?;
    cpu.tb = cpu.tb.wrapping_add(1);
    if !cpu.redirected { cpu.pc = cpu.next; }
    Ok(())
}

fn execute<H: Host>(host: &mut H, cpu: &mut Cpu, w: W) -> R {
    match w.op() {
        3 => int::twi(cpu, w),
        7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 20 | 21 | 23 | 24..=29 => int::exec_d(cpu, w),
        16..=18 | 19 => branch::exec(host, cpu, w),
        31 => x31::exec(host, cpu, w),
        32..=47 => mem::exec_d(host, cpu, w),
        48..=55 => fpmem::exec_d(host, cpu, w),
        59 => fpu::exec_59(cpu, w),
        63 => fpu::exec_63(cpu, w),
        _ => Err(cpu.unimpl(w, crate::names::primary(w.op()))),
    }
}
