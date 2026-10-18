//! Integer loads and stores, load/store multiple and string, reservations, cache hints (PEM 4.2.3).
//!
//! Plain loads and stores may be unaligned (the 6xx handles them in big-endian mode). lwarx, stwcx., lmw and stmw
//! require word alignment and fault with [`Trap::Alignment`] otherwise. Bytes outside guest RAM are a
//! [`Trap::BusFault`]; a faulting multi-register instruction may have written some registers (the architecture
//! allows partial completion), but pc stays at it.
use crate::cpu::{Cpu, Trap};
use crate::exec::R;
use crate::fields::W;
use crate::Host;

/// Cache line size of the 601/603/G3 (dcbz clears one line).
pub const LINE: u32 = 32;

pub(crate) fn rd<H: Host>(cpu: &Cpu, host: &mut H, addr: u32, n: u32) -> Result<u32, Trap> {
    let m = host.mem();
    let r = match n { 1 => m.read_u8(addr).map(u32::from), 2 => m.read_u16(addr).map(u32::from), _ => m.read_u32(addr) };
    r.map_err(|_| Trap::BusFault { pc: cpu.pc, addr, write: false })
}
pub(crate) fn wr<H: Host>(cpu: &Cpu, host: &mut H, addr: u32, n: u32, v: u32) -> R {
    let m = host.mem();
    let r = match n { 1 => m.write_u8(addr, v as u8), 2 => m.write_u16(addr, v as u16), _ => m.write_u32(addr, v) };
    r.map_err(|_| Trap::BusFault { pc: cpu.pc, addr, write: true })
}

#[derive(Clone, Copy, PartialEq)]
enum Op { Lbz, Lhz, Lha, Lwz, Stb, Sth, Stw, Lhbr, Lwbr, Sthbr, Stwbr }

impl Op {
    fn store(self) -> bool { matches!(self, Op::Stb | Op::Sth | Op::Stw | Op::Sthbr | Op::Stwbr) }
    fn size(self) -> u32 {
        match self { Op::Lbz | Op::Stb => 1, Op::Lhz | Op::Lha | Op::Sth | Op::Lhbr | Op::Sthbr => 2, _ => 4 }
    }
}

/// Effective address base: (rA|0).
pub(crate) fn base(cpu: &Cpu, w: W) -> u32 { if w.ra() == 0 { 0 } else { cpu.gpr[w.ra()] } }

fn access<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, op: Op, ea: u32, update: bool) -> R {
    if update && (w.ra() == 0 || (!op.store() && w.ra() == w.rt())) {
        return Err(cpu.invalid(w, "update-form load/store with RA=0 or RA=RT"));
    }
    let t = w.rt();
    match op {
        Op::Lbz | Op::Lhz | Op::Lwz => cpu.gpr[t] = rd(cpu, host, ea, op.size())?,
        Op::Lha => cpu.gpr[t] = rd(cpu, host, ea, 2)? as u16 as i16 as i32 as u32,
        Op::Lhbr => cpu.gpr[t] = (rd(cpu, host, ea, 2)? as u16).swap_bytes() as u32,
        Op::Lwbr => cpu.gpr[t] = rd(cpu, host, ea, 4)?.swap_bytes(),
        Op::Stb | Op::Sth | Op::Stw => wr(cpu, host, ea, op.size(), cpu.gpr[t])?,
        Op::Sthbr => wr(cpu, host, ea, 2, (cpu.gpr[t] as u16).swap_bytes() as u32)?,
        Op::Stwbr => wr(cpu, host, ea, 4, cpu.gpr[t].swap_bytes())?,
    }
    if update { cpu.gpr[w.ra()] = ea; }
    Ok(())
}

/// D-form loads/stores, opcodes 32-47.
pub(crate) fn exec_d<H: Host>(host: &mut H, cpu: &mut Cpu, w: W) -> R {
    let ea = base(cpu, w).wrapping_add(w.simm());
    let (op, upd) = match w.op() {
        32 => (Op::Lwz, false), 33 => (Op::Lwz, true), 34 => (Op::Lbz, false), 35 => (Op::Lbz, true),
        36 => (Op::Stw, false), 37 => (Op::Stw, true), 38 => (Op::Stb, false), 39 => (Op::Stb, true),
        40 => (Op::Lhz, false), 41 => (Op::Lhz, true), 42 => (Op::Lha, false), 43 => (Op::Lha, true),
        44 => (Op::Sth, false), 45 => (Op::Sth, true),
        46 => return lmw(host, cpu, w, ea),
        _ => return stmw(host, cpu, w, ea),
    };
    access(host, cpu, w, op, ea, upd)
}

fn aligned4(cpu: &Cpu, ea: u32, what: &'static str) -> R {
    if ea & 3 != 0 { Err(Trap::Alignment { pc: cpu.pc, addr: ea, what }) } else { Ok(()) }
}

fn lmw<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32) -> R {
    aligned4(cpu, ea, "lmw")?;
    if w.ra() >= w.rt() { return Err(cpu.invalid(w, "lmw with RA in the loaded range")); }
    for (i, r) in (w.rt()..32).enumerate() {
        cpu.gpr[r] = rd(cpu, host, ea.wrapping_add(4 * i as u32), 4)?;
    }
    Ok(())
}
fn stmw<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32) -> R {
    aligned4(cpu, ea, "stmw")?;
    for (i, r) in (w.rt()..32).enumerate() {
        wr(cpu, host, ea.wrapping_add(4 * i as u32), 4, cpu.gpr[r])?;
    }
    Ok(())
}

/// lswi/lswx: `n` bytes into consecutive registers starting at RT (wrapping r31 -> r0), big-endian packed, the
/// unused low bytes of the last register zeroed (PEM lswi).
fn load_string<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32, n: u32) -> R {
    let mut r = w.rt();
    let mut i = 0;
    while i < n {
        let mut v = 0u32;
        for k in 0..4 {
            let b = if i + k < n { rd(cpu, host, ea.wrapping_add(i + k), 1)? } else { 0 };
            v = v << 8 | b;
        }
        cpu.gpr[r] = v;
        r = (r + 1) & 31;
        i += 4;
    }
    Ok(())
}
/// lswi/lswx are invalid when RA (nonzero) or, for lswx, RB is one of the registers being loaded (PEM lswi/lswx),
/// the range running RT.. for ceil(n/4) registers and wrapping r31 -> r0.
fn string_overlap(cpu: &Cpu, w: W, n: u32, check_rb: bool) -> Option<Trap> {
    let count = n.div_ceil(4);
    let loaded = |r: usize| (r as u32).wrapping_sub(w.rt() as u32) & 31 < count;
    if (w.ra() != 0 && loaded(w.ra())) || (check_rb && loaded(w.rb())) {
        return Some(cpu.invalid(w, "lswi/lswx with RA or RB inside the loaded register range"));
    }
    None
}
fn store_string<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32, n: u32) -> R {
    let mut r = w.rt();
    for i in 0..n {
        let v = cpu.gpr[r];
        wr(cpu, host, ea.wrapping_add(i), 1, v >> (24 - 8 * (i & 3)))?;
        if i & 3 == 3 { r = (r + 1) & 31; }
    }
    Ok(())
}

/// Indexed / reservation / string / cache-management members of opcode 31. None if `xo` is not one of them.
pub(crate) fn x31<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, xo: u32) -> Option<R> {
    let ea = base(cpu, w).wrapping_add(cpu.gpr[w.rb()]);
    let (op, upd) = match xo {
        23 => (Op::Lwz, false), 55 => (Op::Lwz, true), 87 => (Op::Lbz, false), 119 => (Op::Lbz, true),
        151 => (Op::Stw, false), 183 => (Op::Stw, true), 215 => (Op::Stb, false), 247 => (Op::Stb, true),
        279 => (Op::Lhz, false), 311 => (Op::Lhz, true), 343 => (Op::Lha, false), 375 => (Op::Lha, true),
        407 => (Op::Sth, false), 439 => (Op::Sth, true),
        534 => (Op::Lwbr, false), 790 => (Op::Lhbr, false), 662 => (Op::Stwbr, false), 918 => (Op::Sthbr, false),
        20 => return Some(lwarx(host, cpu, w, ea)),
        150 => return Some(stwcx(host, cpu, w, ea)),
        597 => {
            let n = if w.rb() == 0 { 32 } else { w.rb() as u32 };
            if let Some(e) = string_overlap(cpu, w, n, false) { return Some(Err(e)); }
            return Some(load_string(host, cpu, w, base(cpu, w), n));
        }
        725 => { let n = if w.rb() == 0 { 32 } else { w.rb() as u32 }; return Some(store_string(host, cpu, w, base(cpu, w), n)) }
        533 => {
            let n = cpu.xer & 0x7F;
            if let Some(e) = string_overlap(cpu, w, n, true) { return Some(Err(e)); }
            return Some(load_string(host, cpu, w, ea, n));
        }
        661 => return Some(store_string(host, cpu, w, ea, cpu.xer & 0x7F)),
        1014 => { // dcbz: zero the 32-byte line containing EA
            let line = ea & !(LINE - 1);
            return Some(host.mem().zero(line, LINE).map_err(|_| Trap::BusFault { pc: cpu.pc, addr: line, write: true }));
        }
        // Cache / ordering hints: no cache and a single in-order CPU, so there is nothing to do. dcbi is supervisor.
        54 | 86 | 246 | 278 | 982 | 598 | 854 => return Some(Ok(())),
        566 => return Some(Err(cpu.privileged(w, "tlbsync"))),
        470 => return Some(Err(cpu.privileged(w, "dcbi"))),
        _ => return None,
    };
    Some(access(host, cpu, w, op, ea, upd))
}

fn lwarx<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32) -> R {
    aligned4(cpu, ea, "lwarx")?;
    cpu.gpr[w.rt()] = rd(cpu, host, ea, 4)?;
    cpu.reservation = Some(ea);
    Ok(())
}

/// stwcx.: stores only if the reservation from the last lwarx is for this very address; CR0 = 0b00 EQ SO. The
/// reservation is cleared either way. (Always Rc=1; Rc=0 is an invalid form.)
fn stwcx<H: Host>(host: &mut H, cpu: &mut Cpu, w: W, ea: u32) -> R {
    if !w.rc() { return Err(cpu.invalid(w, "stwcx. with Rc=0")); }
    aligned4(cpu, ea, "stwcx.")?;
    let ok = cpu.reservation.take() == Some(ea);
    if ok { wr(cpu, host, ea, 4, cpu.gpr[w.rt()])?; }
    let so = (cpu.xer & crate::cpu::XER_SO != 0) as u32;
    cpu.set_cr_field(0, if ok { 2 } else { 0 } | so);
    Ok(())
}
