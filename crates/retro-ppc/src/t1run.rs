//! Tier 1's block executor: run the decoded instructions of one block, leaving exactly tier 0's state.
//!
//! At every exit it leaves what tier 0 leaves: `pc` (the faulting instruction on a trap, else where execution goes),
//! the time base (tier 0 ticks it once per retired instruction; it is also exact before every Generic instruction,
//! the only ones that can read it), the trace ring (every pc, when tracing). Between instructions `cpu.pc` is only
//! stored when an instruction can read it (Generic, the FP handlers); loads and stores name their own pc in a trap.
//! A block ends early when an instruction wrote the page it was decoded from (self-modifying code, `icbi`): the rest
//! of it is stale, so execution continues at the next pc through a fresh lookup, which re-decodes.
use crate::cpu::{Cpu, Trap, XER_CA};
use crate::fields::W;
use crate::t1op::{Op, K};
use crate::{branch, exec, fpmem, fpu, int, Host};

#[inline(always)]
fn ld<H: Host>(host: &mut H, pc: u32, ea: u32, n: u32) -> Result<u32, Trap> {
    let m = host.mem();
    let r = match n { 1 => m.read_u8(ea).map(u32::from), 2 => m.read_u16(ea).map(u32::from), _ => m.read_u32(ea) };
    r.map_err(|_| Trap::BusFault { pc, addr: ea, write: false })
}
#[inline(always)]
fn st<H: Host>(host: &mut H, pc: u32, ea: u32, n: u32, v: u32) -> Result<(), Trap> {
    let m = host.mem();
    let r = match n { 1 => m.write_u8(ea, v as u8), 2 => m.write_u16(ea, v as u16), _ => m.write_u32(ea, v) };
    r.map_err(|_| Trap::BusFault { pc, addr: ea, write: true })
}
#[inline(always)]
fn ca(cpu: &mut Cpu, c: bool) { cpu.xer = (cpu.xer & !XER_CA) | if c { XER_CA } else { 0 }; }

/// What an instruction did to the flow.
enum Flow { Next, Jump(u32) }

/// One decoded instruction at `ipc` (`tb` = the time base before it).
#[inline(always)]
fn one<H: Host>(host: &mut H, cpu: &mut Cpu, op: &Op, ipc: u32, tb: u64) -> Result<Flow, Trap> {
    let (d, a, b) = (op.d as usize & 31, op.a as usize & 31, op.b as usize & 31);
    let g = &mut cpu.gpr;
    match op.k {
        K::Nop => {}
        K::Li => g[d] = op.imm,
        K::Addi => g[d] = g[a].wrapping_add(op.imm),
        K::Addic | K::AddicRc => {
            let (v, c, _) = int::add_core(g[a], op.imm, 0);
            g[d] = v;
            ca(cpu, c);
            if op.k == K::AddicRc { cpu.set_cr0(v) }
        }
        K::Subfic => { let (v, c, _) = int::add_core(!g[a], op.imm, 1); g[d] = v; ca(cpu, c) }
        K::Mulli => g[d] = (g[a] as i32).wrapping_mul(op.imm as i32) as u32,
        K::Ori => g[d] = g[a] | op.imm,
        K::Xori => g[d] = g[a] ^ op.imm,
        K::AndiRc => { let v = g[a] & op.imm; g[d] = v; cpu.set_cr0(v) }
        K::Rlwinm => g[d] = g[a].rotate_left(b as u32) & op.imm,
        K::RlwinmRc => { let v = g[a].rotate_left(b as u32) & op.imm; g[d] = v; cpu.set_cr0(v) }
        K::Cmpi => { let o = (g[a] as i32).cmp(&(op.imm as i32)); cpu.set_cr_compare(d as u32, o) }
        K::Cmpli => { let o = g[a].cmp(&op.imm); cpu.set_cr_compare(d as u32, o) }
        K::Cmp => { let o = (g[a] as i32).cmp(&(g[b] as i32)); cpu.set_cr_compare(d as u32, o) }
        K::Cmpl => { let o = g[a].cmp(&g[b]); cpu.set_cr_compare(d as u32, o) }
        K::Or => g[d] = g[a] | g[b],
        K::OrRc => { let v = g[a] | g[b]; g[d] = v; cpu.set_cr0(v) }
        K::And => g[d] = g[a] & g[b],
        K::AndRc => { let v = g[a] & g[b]; g[d] = v; cpu.set_cr0(v) }
        K::Xor => g[d] = g[a] ^ g[b],
        K::Andc => g[d] = g[a] & !g[b],
        K::Nor => g[d] = !(g[a] | g[b]),
        K::Add => g[d] = g[a].wrapping_add(g[b]),
        K::AddRc => { let v = g[a].wrapping_add(g[b]); g[d] = v; cpu.set_cr0(v) }
        K::Subf => g[d] = g[b].wrapping_sub(g[a]),
        K::SubfRc => { let v = g[b].wrapping_sub(g[a]); g[d] = v; cpu.set_cr0(v) }
        K::Neg => g[d] = g[a].wrapping_neg(),
        K::Mullw => g[d] = (g[a] as i32).wrapping_mul(g[b] as i32) as u32,
        K::Extsh => g[d] = g[a] as u16 as i16 as i32 as u32,
        K::Extsb => g[d] = g[a] as u8 as i8 as i32 as u32,
        K::Srawi | K::SrawiRc => {
            let (v, c) = int::sra(g[a], b as u32);
            g[d] = v;
            ca(cpu, c);
            if op.k == K::SrawiRc { cpu.set_cr0(v) }
        }
        K::Slw => { let s = g[b]; g[d] = if s & 0x20 != 0 { 0 } else { g[a] << (s & 31) } }
        K::Srw => { let s = g[b]; g[d] = if s & 0x20 != 0 { 0 } else { g[a] >> (s & 31) } }
        K::Mflr => g[d] = cpu.lr,
        K::Mtlr => cpu.lr = g[d],
        K::Mfctr => g[d] = cpu.ctr,
        K::Mtctr => cpu.ctr = g[d],
        K::Lwz => { let ea = g[a].wrapping_add(op.imm); g[d] = ld(host, ipc, ea, 4)? }
        K::Lbz => { let ea = g[a].wrapping_add(op.imm); g[d] = ld(host, ipc, ea, 1)? }
        K::Lhz => { let ea = g[a].wrapping_add(op.imm); g[d] = ld(host, ipc, ea, 2)? }
        K::Lha => { let ea = g[a].wrapping_add(op.imm); g[d] = ld(host, ipc, ea, 2)? as u16 as i16 as i32 as u32 }
        K::Lwzu => { let ea = g[a].wrapping_add(op.imm); let v = ld(host, ipc, ea, 4)?; let g = &mut cpu.gpr; g[d] = v; g[a] = ea }
        K::Lbzu => { let ea = g[a].wrapping_add(op.imm); let v = ld(host, ipc, ea, 1)?; let g = &mut cpu.gpr; g[d] = v; g[a] = ea }
        K::Lwzx => { let ea = g[a].wrapping_add(g[b]); g[d] = ld(host, ipc, ea, 4)? }
        K::Lbzx => { let ea = g[a].wrapping_add(g[b]); g[d] = ld(host, ipc, ea, 1)? }
        K::Lfs => { let ea = g[a].wrapping_add(op.imm); cpu.fpr[d] = fpmem::single_to_double(ld(host, ipc, ea, 4)?) }
        K::Lfd => {
            let ea = g[a].wrapping_add(op.imm);
            let hi = ld(host, ipc, ea, 4)?;
            let lo = ld(host, ipc, ea.wrapping_add(4), 4)?;
            cpu.fpr[d] = (hi as u64) << 32 | lo as u64;
        }
        K::Fp59 => { cpu.pc = ipc; fpu::exec_59(cpu, W(op.w))? }
        K::Fp63 => { cpu.pc = ipc; fpu::exec_63(cpu, W(op.w))? }
        K::Stw => { let ea = g[a].wrapping_add(op.imm); let v = g[d]; st(host, ipc, ea, 4, v)? }
        K::Stb => { let ea = g[a].wrapping_add(op.imm); let v = g[d]; st(host, ipc, ea, 1, v)? }
        K::Sth => { let ea = g[a].wrapping_add(op.imm); let v = g[d]; st(host, ipc, ea, 2, v)? }
        K::Stwu => { let ea = g[a].wrapping_add(op.imm); let v = g[d]; st(host, ipc, ea, 4, v)?; cpu.gpr[a] = ea }
        K::Stbu => { let ea = g[a].wrapping_add(op.imm); let v = g[d]; st(host, ipc, ea, 1, v)?; cpu.gpr[a] = ea }
        K::Stwx => { let ea = g[a].wrapping_add(g[b]); let v = g[d]; st(host, ipc, ea, 4, v)? }
        K::Stbx => { let ea = g[a].wrapping_add(g[b]); let v = g[d]; st(host, ipc, ea, 1, v)? }
        K::Stfs => { let ea = g[a].wrapping_add(op.imm); let v = fpmem::double_to_single(cpu.fpr[d]); st(host, ipc, ea, 4, v)? }
        K::Stfd => {
            let ea = g[a].wrapping_add(op.imm);
            let f = cpu.fpr[d];
            st(host, ipc, ea, 4, (f >> 32) as u32)?;
            st(host, ipc, ea.wrapping_add(4), 4, f as u32)?;
        }
        K::Generic | K::GenericEnd | K::Sc => {
            // exactly tier 0's step around `execute`
            cpu.pc = ipc;
            cpu.tb = tb;
            cpu.next = ipc.wrapping_add(4);
            cpu.redirected = false;
            exec::execute(host, cpu, W(op.w))?;
            if op.k != K::Generic || cpu.redirected { return Ok(Flow::Jump(if cpu.redirected { cpu.pc } else { cpu.next })); }
        }
        K::B => return Ok(Flow::Jump(op.imm)),
        K::Bl => { cpu.lr = ipc.wrapping_add(4); return Ok(Flow::Jump(op.imm)) }
        K::Bc | K::Bcl => {
            let go = branch::take_branch(cpu, d as u32, a as u32);
            if op.k == K::Bcl { cpu.lr = ipc.wrapping_add(4); }
            return Ok(Flow::Jump(if go { op.imm } else { ipc.wrapping_add(4) }));
        }
        K::Bclr | K::Bclrl | K::Bcctr | K::Bcctrl => {
            let target = if matches!(op.k, K::Bcctr | K::Bcctrl) { cpu.ctr } else { cpu.lr } & !3;
            let go = branch::take_branch(cpu, d as u32, a as u32);
            if matches!(op.k, K::Bclrl | K::Bcctrl) { cpu.lr = ipc.wrapping_add(4); }
            return Ok(Flow::Jump(if go { target } else { ipc.wrapping_add(4) }));
        }
    }
    Ok(Flow::Next)
}

/// Run `ops` (a block decoded at `pc` from a page of generation `gen`; possibly cut short by the budget). Returns the
/// instructions retired; a trap is put in `trap` (pc left at the faulting instruction). `stale_check` false is the
/// planted mutant of the negative control: writes to the block's own page are ignored.
#[inline]
pub(crate) fn run<H: Host>(host: &mut H, cpu: &mut Cpu, ops: &[Op], pc: u32, gen: u64, stale_check: bool, trap: &mut Option<Trap>) -> u64 {
    let tb0 = cpu.tb;
    let tracing = cpu.tracing();
    let mut bumps = if stale_check { host.mem().code_bumps() } else { 0 };
    let mut ipc = pc;
    let mut i = 0usize;
    while i < ops.len() {
        let op = &ops[i];
        if tracing { cpu.note_pc(ipc); }
        match one(host, cpu, op, ipc, tb0.wrapping_add(i as u64)) {
            Ok(Flow::Next) => {}
            Ok(Flow::Jump(t)) => {
                cpu.pc = t;
                cpu.tb = tb0.wrapping_add(i as u64 + 1);
                return i as u64 + 1;
            }
            Err(t) => return fault(cpu, ipc, tb0, i, t, trap),
        }
        i += 1;
        ipc = ipc.wrapping_add(4);
        if stale_check && op.may_write() {
            let now = host.mem().code_bumps();
            if now != bumps {
                if host.mem().code_gen(pc) != gen { break; }
                bumps = now;
            }
        }
    }
    cpu.pc = ipc;
    cpu.tb = tb0.wrapping_add(i as u64);
    i as u64
}

#[cold]
fn fault(cpu: &mut Cpu, ipc: u32, tb0: u64, i: usize, t: Trap, trap: &mut Option<Trap>) -> u64 {
    cpu.pc = ipc;
    cpu.tb = tb0.wrapping_add(i as u64);
    *trap = Some(t);
    i as u64
}
