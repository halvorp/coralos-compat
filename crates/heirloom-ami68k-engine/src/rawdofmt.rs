//! exec RawDoFmt and dos VFPrintf / VPrintf: the engine's one formatter (fmt.rs) with the output going to a guest
//! PutChProc (RawDoFmt) or a file (VFPrintf).
//!
//! RawDoFmt(a0 = template, a1 = dataStream, a2 = PutChProc, a3 = PutChData) calls the proc once per character with the
//! character in D0 and A3 = PutChData (A3 as the proc left it comes back on the next call, which is what makes the
//! classic `move.b d0,(a3)+` string-copy proc work), then once with 0 (the terminator), and returns the address after the arguments the
//! template used. The proc is called through guest_call.rs, so it may trash registers. A NULL PutChProc would be the
//! serial debug port on a real machine; the engine has none, so that is a named refusal.
//! VFPrintf(d1 = file, d2 = template, d3 = argArray) -> the number of characters written, or -1; VPrintf(d1 = template,
//! d2 = argArray) is VFPrintf(Output()). The argArray is a RawDoFmt data stream.
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::guest_call::{self, Next};
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("RawDoFmt", raw_do_fmt::<U>)
}

/// The dos.library half: VFPrintf and VPrintf.
pub fn register_dos<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("VFPrintf", vf_printf::<U>).named("VPrintf", v_printf::<U>)
}

fn emit<U: GuestUi>(bytes: Vec<u8>, i: usize, proc_: u32, data: u32, ret: u32) -> Next<U> {
    let Some(&ch) = bytes.get(i) else { return Next::Return(ret) };
    Next::Call { target: proc_, d0: ch as u32, aregs: alloc::vec![(3, data)], then: Box::new(move |_, cpu: &mut Cpu, _| Ok(emit(bytes, i + 1, proc_, cpu.areg(3), ret))) }
}

fn raw_do_fmt<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tpl, data, proc_, pdata) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if proc_ == 0 { return Err(Refusal("RawDoFmt: PutChProc is NULL (the serial debug port is not modelled)".into())); }
    let t = crate::guest_str::read_bytes_z(&a.mem, tpl, 8192).map_err(Refusal::mem("RawDoFmt.template"))?;
    let f = crate::fmt::format(&a.mem, &t, data, false)?;
    let line = format!("[exec] RawDoFmt \"{}\" -> {} characters", String::from_utf8_lossy(&t), f.bytes.len());
    let mut out = f.bytes;
    out.push(0);
    guest_call::start(a, cpu, emit(out, 0, proc_, pdata, data + f.used))?;
    Ok(line)
}

fn vfprintf_to<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, fh: u32, tpl: u32, args: u32, what: &str) -> Result<String, Refusal> {
    let t = crate::guest_str::read_bytes_z(&a.mem, tpl, 8192).map_err(Refusal::mem("VFPrintf.template"))?;
    let f = crate::fmt::format(&a.mem, &t, args, false)?;
    let ok = crate::dos::write_to(a, fh, &f.bytes)?;
    cpu.set_d0(if ok { f.bytes.len() as u32 } else { u32::MAX });
    Ok(format!("[dos] {what} \"{}\" -> {}", String::from_utf8_lossy(&t), if ok { f.bytes.len() as i64 } else { -1 }))
}

fn vf_printf<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (fh, tpl, args) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    vfprintf_to(a, cpu, fh, tpl, args, "VFPrintf")
}

fn v_printf<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tpl, args) = (cpu.dreg(1), cpu.dreg(2));
    let fh = a.mem.read_u32(a.sched.cur + 160).map_err(Refusal::mem("pr_COS"))?;
    vfprintf_to(a, cpu, fh, tpl, args, "VPrintf")
}
