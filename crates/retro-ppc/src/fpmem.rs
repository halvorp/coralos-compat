//! Floating-point loads and stores (PEM 4.2.? "Floating-Point Load and Store Instructions").
//! lfs widens a single to a double exactly; stfs truncates to single per the PEM store conversion (see below).
use crate::cpu::Cpu;
use crate::exec::R;
use crate::fields::W;
use crate::mem::{base, rd, wr};
use crate::Host;

#[derive(Clone, Copy)]
enum Op { Lfs, Lfd, Stfs, Stfd, Stfiw }

/// lfs: widen exactly. A NaN keeps its payload and its quiet bit (a host `as` cast would quiet a signalling NaN).
fn single_to_double(s: u32) -> u64 {
    let x = f32::from_bits(s);
    if x.is_nan() { (s as u64 & 0x8000_0000) << 32 | 0x7FF << 52 | (s as u64 & 0x007F_FFFF) << 29 } else { (x as f64).to_bits() }
}
/// stfs: the PEM store conversion (stfs pseudocode, "Floating-point store single"), which TRUNCATES, never rounds,
/// and does not look at FPSCR[RN]; a program wanting a rounded single does `frsp` first.
///  * FRS[1:11] > 896 (a normal single exponent, infinity, NaN) or FRS = +-0: WORD[0:1] <- FRS[0:1],
///    WORD[2:31] <- FRS[5:34]. A NaN keeps sign, quiet bit and the top 22 payload bits unchanged (no quieting).
///  * 874 <= FRS[1:11] <= 896 (a single denormal): the implicit 1 and the fraction are shifted right by 897-exp and
///    the low bits are dropped.
///  * smaller magnitudes are architecturally undefined; we store a signed zero.
fn double_to_single(d: u64) -> u32 {
    let exp = (d >> 52 & 0x7FF) as u32;
    let sign = (d >> 32) as u32 & 0x8000_0000;
    if exp > 896 || d << 1 == 0 {
        sign | (d >> 32) as u32 & 0x4000_0000 | (d >> 29) as u32 & 0x3FFF_FFFF
    } else if exp >= 874 {
        sign | ((1u64 << 52 | d & ((1 << 52) - 1)) >> (897 - exp + 29)) as u32
    } else {
        sign
    }
}

fn access<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, op: Op, ea: u32, update: bool) -> R {
    if update && w.ra() == 0 {
        return Err(cpu.invalid(w, "update-form FP load/store with RA=0"));
    }
    let t = w.rt();
    match op {
        Op::Lfs => cpu.fpr[t] = single_to_double(rd(cpu, host, ea, 4)?),
        Op::Lfd => {
            let hi = rd(cpu, host, ea, 4)?;
            let lo = rd(cpu, host, ea.wrapping_add(4), 4)?;
            cpu.fpr[t] = (hi as u64) << 32 | lo as u64;
        }
        Op::Stfs => wr(cpu, host, ea, 4, double_to_single(cpu.fpr[t]))?,
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
