//! Integer arithmetic, logical, rotate, shift and compare (PEM 4.2.1, 4.2.2, 4.2.4, 4.2.5).
use crate::cpu::{Cpu, XER_CA, XER_OV, XER_SO};
use crate::exec::R;
use crate::fields::{mask, W};

/// a + b + cin with PowerPC's CA (carry out of bit 0) and OV (signed overflow) as PEM defines them for addc/adde.
pub(crate) fn add_core(a: u32, b: u32, cin: u32) -> (u32, bool, bool) {
    let sum = a as u64 + b as u64 + cin as u64;
    let r = sum as u32;
    (r, sum >> 32 != 0, ((a ^ r) & (b ^ r)) >> 31 != 0)
}

/// Write rD and apply the OE / Rc / CA side effects shared by every XO-form instruction.
fn finish(cpu: &mut Cpu, w: W, rd: usize, r: u32, ca: Option<bool>, ov: bool) {
    cpu.gpr[rd] = r;
    if let Some(c) = ca { cpu.xer = (cpu.xer & !XER_CA) | if c { XER_CA } else { 0 }; }
    if w.oe() {
        cpu.xer = (cpu.xer & !XER_OV) | if ov { XER_OV | XER_SO } else { 0 };
    }
    if w.rc() { cpu.set_cr0(r); }
}

fn ca_in(cpu: &Cpu) -> u32 { (cpu.xer & XER_CA != 0) as u32 }
fn ra0(cpu: &Cpu, w: W) -> u32 { if w.ra() == 0 { 0 } else { cpu.gpr[w.ra()] } }

/// Compare into CR field `bf`: LT/GT/EQ plus XER[SO] (PEM cmp/cmpl).
fn compare(cpu: &mut Cpu, bf: u32, ord: core::cmp::Ordering) {
    let so = (cpu.xer & XER_SO != 0) as u32;
    let f = match ord { core::cmp::Ordering::Less => 8, core::cmp::Ordering::Greater => 4, core::cmp::Ordering::Equal => 2 };
    cpu.set_cr_field(bf, f | so);
}

/// The TO-field test of tw/twi: bits 0x10 <, 0x08 >, 0x04 =, 0x02 <u, 0x01 >u (PEM tw).
fn trap_cond(to: u32, a: u32, b: u32) -> bool {
    let (sa, sb) = (a as i32, b as i32);
    (to & 0x10 != 0 && sa < sb) || (to & 0x08 != 0 && sa > sb) || (to & 4 != 0 && a == b)
        || (to & 2 != 0 && a < b) || (to & 1 != 0 && a > b)
}

pub(crate) fn twi(cpu: &mut Cpu, w: W) -> R {
    if trap_cond(w.rt() as u32, cpu.gpr[w.ra()], w.simm()) { Err(crate::cpu::Trap::TrapWord { pc: cpu.pc, word: w.0 }) } else { Ok(()) }
}
pub(crate) fn tw(cpu: &mut Cpu, w: W) -> R {
    if trap_cond(w.rt() as u32, cpu.gpr[w.ra()], cpu.gpr[w.rb()]) { Err(crate::cpu::Trap::TrapWord { pc: cpu.pc, word: w.0 }) } else { Ok(()) }
}

/// D-form integer opcodes 7-15, 20-21, 23-29.
pub(crate) fn exec_d(cpu: &mut Cpu, w: W) -> R {
    let (t, a) = (w.rt(), w.ra());
    match w.op() {
        7 => cpu.gpr[t] = (cpu.gpr[a] as i32).wrapping_mul(w.simm() as i32) as u32, // mulli
        8 => { // subfic
            let (r, ca, _) = add_core(!cpu.gpr[a], w.simm(), 1);
            cpu.gpr[t] = r;
            cpu.xer = (cpu.xer & !XER_CA) | if ca { XER_CA } else { 0 };
        }
        10 => { let o = cpu.gpr[a].cmp(&w.uimm()); compare(cpu, w.bf(), o) } // cmpli
        11 => { let o = (cpu.gpr[a] as i32).cmp(&(w.simm() as i32)); compare(cpu, w.bf(), o) } // cmpi
        12 | 13 => { // addic / addic.
            let (r, ca, _) = add_core(cpu.gpr[a], w.simm(), 0);
            cpu.gpr[t] = r;
            cpu.xer = (cpu.xer & !XER_CA) | if ca { XER_CA } else { 0 };
            if w.op() == 13 { cpu.set_cr0(r); }
        }
        14 => cpu.gpr[t] = ra0(cpu, w).wrapping_add(w.simm()),                  // addi
        15 => cpu.gpr[t] = ra0(cpu, w).wrapping_add(w.uimm() << 16),            // addis
        20 => { // rlwimi
            let m = mask(w.mb(), w.me());
            let r = cpu.gpr[t].rotate_left(w.sh());
            let v = (r & m) | (cpu.gpr[a] & !m);
            cpu.gpr[a] = v;
            if w.rc() { cpu.set_cr0(v); }
        }
        21 => { // rlwinm
            let v = cpu.gpr[t].rotate_left(w.sh()) & mask(w.mb(), w.me());
            cpu.gpr[a] = v;
            if w.rc() { cpu.set_cr0(v); }
        }
        23 => { // rlwnm
            let v = cpu.gpr[t].rotate_left(cpu.gpr[w.rb()] & 31) & mask(w.mb(), w.me());
            cpu.gpr[a] = v;
            if w.rc() { cpu.set_cr0(v); }
        }
        24 => cpu.gpr[a] = cpu.gpr[t] | w.uimm(),          // ori
        25 => cpu.gpr[a] = cpu.gpr[t] | w.uimm() << 16,    // oris
        26 => cpu.gpr[a] = cpu.gpr[t] ^ w.uimm(),          // xori
        27 => cpu.gpr[a] = cpu.gpr[t] ^ w.uimm() << 16,    // xoris
        28 => { let v = cpu.gpr[t] & w.uimm(); cpu.gpr[a] = v; cpu.set_cr0(v) }        // andi.
        29 => { let v = cpu.gpr[t] & w.uimm() << 16; cpu.gpr[a] = v; cpu.set_cr0(v) }  // andis.
        _ => return Err(cpu.unimpl(w, "D-form integer")),
    }
    Ok(())
}

/// XO-form arithmetic of opcode 31, `x` = XO with the OE bit removed. None if `x` is not an XO-form instruction.
pub(crate) fn arith(cpu: &mut Cpu, w: W, x: u32) -> Option<R> {
    let (d, a, b) = (w.rt(), cpu.gpr[w.ra()], cpu.gpr[w.rb()]);
    let ca = ca_in(cpu);
    match x {
        266 => { let (r, _, o) = add_core(a, b, 0); finish(cpu, w, d, r, None, o) }                  // add
        10 => { let (r, c, o) = add_core(a, b, 0); finish(cpu, w, d, r, Some(c), o) }               // addc
        138 => { let (r, c, o) = add_core(a, b, ca); finish(cpu, w, d, r, Some(c), o) }             // adde
        202 => { let (r, c, o) = add_core(a, 0, ca); finish(cpu, w, d, r, Some(c), o) }             // addze
        234 => { let (r, c, o) = add_core(a, u32::MAX, ca); finish(cpu, w, d, r, Some(c), o) }      // addme
        40 => { let (r, _, o) = add_core(!a, b, 1); finish(cpu, w, d, r, None, o) }                 // subf
        8 => { let (r, c, o) = add_core(!a, b, 1); finish(cpu, w, d, r, Some(c), o) }               // subfc
        136 => { let (r, c, o) = add_core(!a, b, ca); finish(cpu, w, d, r, Some(c), o) }            // subfe
        200 => { let (r, c, o) = add_core(!a, 0, ca); finish(cpu, w, d, r, Some(c), o) }            // subfze
        232 => { let (r, c, o) = add_core(!a, u32::MAX, ca); finish(cpu, w, d, r, Some(c), o) }     // subfme
        104 => { let (r, _, o) = add_core(!a, 0, 1); finish(cpu, w, d, r, None, o) }                // neg
        235 => { // mullw
            let p = (a as i32 as i64) * (b as i32 as i64);
            finish(cpu, w, d, p as u32, None, p != p as i32 as i64)
        }
        75 => { let p = (a as i32 as i64) * (b as i32 as i64); finish(cpu, w, d, (p >> 32) as u32, None, false) } // mulhw
        11 => { let p = a as u64 * b as u64; finish(cpu, w, d, (p >> 32) as u32, None, false) }                 // mulhwu
        491 => { // divw: see the lib.rs note on undefined results
            let (r, o) = if b == 0 { (if (a as i32) < 0 { u32::MAX } else { 0 }, true) }
                else if a == 0x8000_0000 && b == u32::MAX { (if (a as i32) < 0 { u32::MAX } else { 0 }, true) }
                else { ((a as i32 / b as i32) as u32, false) };
            finish(cpu, w, d, r, None, o)
        }
        459 => { let (r, o) = if b == 0 { (u32::MAX, true) } else { (a / b, false) }; finish(cpu, w, d, r, None, o) } // divwu
        _ => return None,
    }
    Some(Ok(()))
}

/// X-form logical, shift and compare instructions of opcode 31 (`xo` is the full 10-bit XO). None if not one of them.
pub(crate) fn logical(cpu: &mut Cpu, w: W, xo: u32) -> Option<R> {
    let (s, a, b) = (cpu.gpr[w.rt()], w.ra(), cpu.gpr[w.rb()]);
    let r = match xo {
        28 => s & b, 60 => s & !b, 444 => s | b, 412 => s | !b, 316 => s ^ b,
        124 => !(s | b), 476 => !(s & b), 284 => !(s ^ b),
        24 => if b & 0x20 != 0 { 0 } else { s << (b & 31) },     // slw
        536 => if b & 0x20 != 0 { 0 } else { s >> (b & 31) },    // srw
        26 => s.leading_zeros(),                                 // cntlzw
        922 => s as u16 as i16 as i32 as u32,                    // extsh
        954 => s as u8 as i8 as i32 as u32,                      // extsb
        792 => { // sraw
            let n = b & 0x3F;
            let (r, ca) = sra(s, n);
            cpu.xer = (cpu.xer & !XER_CA) | if ca { XER_CA } else { 0 };
            r
        }
        824 => { // srawi
            let (r, ca) = sra(s, w.sh());
            cpu.xer = (cpu.xer & !XER_CA) | if ca { XER_CA } else { 0 };
            r
        }
        0 => { let o = (cpu.gpr[w.ra()] as i32).cmp(&(b as i32)); compare(cpu, w.bf(), o); return Some(Ok(())) }  // cmp
        32 => { let o = cpu.gpr[w.ra()].cmp(&b); compare(cpu, w.bf(), o); return Some(Ok(())) }                  // cmpl
        4 => return Some(tw(cpu, w)),
        _ => return None,
    };
    cpu.gpr[a] = r;
    if w.rc() { cpu.set_cr0(r); }
    Some(Ok(()))
}

/// Arithmetic right shift by `n` (0-63) with PowerPC's CA: set when the source is negative and a 1 bit was shifted out.
fn sra(s: u32, n: u32) -> (u32, bool) {
    let neg = (s as i32) < 0;
    if n >= 32 { return (if neg { u32::MAX } else { 0 }, neg); }
    let r = ((s as i32) >> n) as u32;
    (r, neg && n != 0 && s & ((1u32 << n) - 1) != 0)
}
