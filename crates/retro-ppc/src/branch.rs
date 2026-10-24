//! Branch, condition-register and system-call instructions (PEM 4.2.3 "Branch Processor Instructions").
use crate::cpu::{Cpu, Trap};
use crate::exec::R;
use crate::fields::W;
use crate::Host;

/// BO / BI evaluation of bc, bclr, bcctr (PEM Figure 4-? "BO operand encodings"). BO bit 0 (value 0x10) = ignore
/// the condition; bit 1 (0x08) = the CR bit value to test; bit 2 (0x04) = do not decrement CTR; bit 3 (0x02) =
/// branch when CTR == 0 instead of != 0.
pub(crate) fn take_branch(cpu: &mut Cpu, bo: u32, bi: u32) -> bool {
    if bo & 0x04 == 0 { cpu.ctr = cpu.ctr.wrapping_sub(1); }
    let ctr_ok = bo & 0x04 != 0 || ((cpu.ctr != 0) ^ (bo & 0x02 != 0));
    let cond_ok = bo & 0x10 != 0 || (cpu.cr_bit(bi) == (bo & 0x08 != 0));
    ctr_ok && cond_ok
}

pub(crate) fn exec<H: Host>(host: &mut H, cpu: &mut Cpu, w: W) -> R {
    let pc = cpu.pc;
    match w.op() {
        18 => { // b, ba, bl, bla
            let li = (((w.0 & 0x03FF_FFFC) << 6) as i32 >> 6) as u32;
            let target = if w.aa() { li } else { pc.wrapping_add(li) };
            if w.lk() { cpu.lr = pc.wrapping_add(4); }
            cpu.next = target;
        }
        16 => { // bc
            let bd = (w.0 & 0xFFFC) as u16 as i16 as i32 as u32;
            let go = take_branch(cpu, w.rt() as u32, w.ra() as u32);
            if w.lk() { cpu.lr = pc.wrapping_add(4); }
            if go { cpu.next = if w.aa() { bd } else { pc.wrapping_add(bd) }; }
        }
        17 => { // sc
            if w.0 & 2 == 0 { return Err(cpu.invalid(w, "sc with reserved bit 30 clear")); }
            if !host.syscall(cpu) { return Err(Trap::Syscall { pc }); }
        }
        _ => return xl(cpu, w),
    }
    Ok(())
}

/// Opcode 19: bclr, bcctr and the condition-register logical instructions.
fn xl(cpu: &mut Cpu, w: W) -> R {
    let pc = cpu.pc;
    let (bt, ba, bb) = (w.rt() as u32, w.ra() as u32, w.rb() as u32);
    let crop = |cpu: &mut Cpu, f: fn(bool, bool) -> bool| {
        let v = f(cpu.cr_bit(ba), cpu.cr_bit(bb));
        cpu.set_cr_bit(bt, v);
    };
    match w.xo() {
        16 | 528 => { // bclr / bcctr: the target is read BEFORE LK overwrites LR and BEFORE CTR is decremented
            let ctr_form = w.xo() == 528;
            if ctr_form && w.rt() & 0x04 == 0 { return Err(cpu.invalid(w, "bcctr with BO decrementing CTR")); }
            let target = if ctr_form { cpu.ctr } else { cpu.lr } & !3;
            let go = take_branch(cpu, w.rt() as u32, w.ra() as u32);
            if w.lk() { cpu.lr = pc.wrapping_add(4); }
            if go { cpu.next = target; }
        }
        0 => { let v = cpu.cr_field(w.bfa()); cpu.set_cr_field(w.bf(), v) } // mcrf
        33 => crop(cpu, |a, b| !(a | b)),     // crnor
        129 => crop(cpu, |a, b| a & !b),      // crandc
        193 => crop(cpu, |a, b| a ^ b),       // crxor
        225 => crop(cpu, |a, b| !(a & b)),    // crnand
        257 => crop(cpu, |a, b| a & b),       // crand
        289 => crop(cpu, |a, b| !(a ^ b)),    // creqv
        417 => crop(cpu, |a, b| a | !b),      // crorc
        449 => crop(cpu, |a, b| a | b),       // cror
        150 => {}                              // isync: nothing to synchronise in an interpreter
        50 => return Err(cpu.privileged(w, "rfi")),
        _ => return Err(cpu.unimpl(w, "opcode 19 extended")),
    }
    Ok(())
}
