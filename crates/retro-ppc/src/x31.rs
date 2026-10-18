//! Opcode 31 dispatch: routes each extended opcode to the concern that owns it.
use crate::cpu::Cpu;
use crate::exec::R;
use crate::fields::W;
use crate::{fpmem, int, mem, sys, Host};

pub(crate) fn exec<H: Host>(host: &mut H, cpu: &mut Cpu, w: W) -> R {
    let xo = w.xo();
    // Exact X-form matches first: a few X-form numbers equal an XO-form number once OE (bit 0x200) is masked.
    if let Some(r) = int::logical(cpu, w, xo) { return r; }
    if let Some(r) = mem::x31(host, cpu, w, xo) { return r; }
    if let Some(r) = fpmem::x31(host, cpu, w, xo) { return r; }
    if let Some(r) = sys::x31(cpu, w, xo) { return r; }
    if let Some(r) = int::arith(cpu, w, xo & 0x1FF) { return r; }
    Err(cpu.unimpl(w, crate::names::x31(xo)))
}
