//! Floating-point loads and stores (PEM 4.2.? "Floating-Point Load and Store Instructions").
//! lfs widens a single to a double exactly; stfs rounds to single (nearest-even; the store ignores FPSCR[RN]).
use crate::cpu::Cpu;
use crate::exec::R;
use crate::fields::W;
use crate::mem::{base, rd, wr};
use crate::Host;

#[derive(Clone, Copy)]
enum Op { Lfs, Lfd, Stfs, Stfd, Stfiw }

fn access<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, op: Op, ea: u32, update: bool) -> R {
    if update && w.ra() == 0 {
        return Err(cpu.invalid(w, "update-form FP load/store with RA=0"));
    }
    let t = w.rt();
    match op {
        Op::Lfs => cpu.fpr[t] = (f32::from_bits(rd(cpu, host, ea, 4)?) as f64).to_bits(),
        Op::Lfd => {
            let hi = rd(cpu, host, ea, 4)?;
            let lo = rd(cpu, host, ea.wrapping_add(4), 4)?;
            cpu.fpr[t] = (hi as u64) << 32 | lo as u64;
        }
        Op::Stfs => wr(cpu, host, ea, 4, (f64::from_bits(cpu.fpr[t]) as f32).to_bits())?,
        Op::Stfd => {
            wr(cpu, host, ea, 4, (cpu.fpr[t] >> 32) as u32)?;
            wr(cpu, host, ea.wrapping_add(4), 4, cpu.fpr[t] as u32)?;
        }
        Op::Stfiw => wr(cpu, host, ea, 4, cpu.fpr[t] as u32)?,
    }
    if update { cpu.gpr[w.ra()] = ea; }
    Ok(())
}

/// Opcodes 48-55.
pub(crate) fn exec_d<H: Host>(host: &mut H, cpu: &mut Cpu, w: W) -> R {
    let ea = base(cpu, w).wrapping_add(w.simm());
    let (op, upd) = match w.op() {
        48 => (Op::Lfs, false), 49 => (Op::Lfs, true), 50 => (Op::Lfd, false), 51 => (Op::Lfd, true),
        52 => (Op::Stfs, false), 53 => (Op::Stfs, true), 54 => (Op::Stfd, false), _ => (Op::Stfd, true),
    };
    access(host, cpu, w, op, ea, upd)
}

/// Indexed forms in opcode 31. None if `xo` is not one.
pub(crate) fn x31<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, xo: u32) -> Option<R> {
    let (op, upd) = match xo {
        535 => (Op::Lfs, false), 567 => (Op::Lfs, true), 599 => (Op::Lfd, false), 631 => (Op::Lfd, true),
        663 => (Op::Stfs, false), 695 => (Op::Stfs, true), 727 => (Op::Stfd, false), 759 => (Op::Stfd, true),
        983 => (Op::Stfiw, false),
        _ => return None,
    };
    let ea = base(cpu, w).wrapping_add(cpu.gpr[w.rb()]);
    Some(access(host, cpu, w, op, ea, upd))
}
