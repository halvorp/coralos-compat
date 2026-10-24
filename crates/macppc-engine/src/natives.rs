//! The PowerPC adapter of the Mixed Mode seam: [`PpcHost::glue_call`] for a [`MacPpc`].
//!
//! r3..r10 become the call's arguments (`Args::from_values` truncates Boolean/INTEGER slots, which the PowerPC ABI
//! leaves undefined above their width), the handler runs, and the result goes back in r3 (INTEGER sign-extended,
//! Boolean zero-extended). Everything that is not a Toolbox or OS trap -- the Code Fragment Manager calls an
//! application makes to find libraries, `MemError`, `BlockZero` -- is implemented here as a [`Native`].
use crate::host::{GlueOutcome, PpcHost};
use crate::machine::MacPpc;
use crate::symbols::{Binding, Native};
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::machine::UNIMPLEMENTED;
use mac68k_engine::pascal::{read_pstring_bytes, Arg};
use mac68k_engine::mixed::Return;

pub const FRAG_CONNECTION_ID_NOT_FOUND: i16 = -2801;
pub const FRAG_SYMBOL_NOT_FOUND: i16 = -2802;
pub const FRAG_LIB_NOT_FOUND: i16 = -2804;
/// CFragSymbolClass values for FindSymbol.
pub const SYMBOL_CLASS_DATA: u8 = 1;
pub const SYMBOL_CLASS_TVECTOR: u8 = 2;
/// The libraries `GetSharedLibrary` can hand out: the ones whose symbols the binding table resolves.
/// Whether the engine provides library `l`: the Toolbox libraries (one list, [`crate::glue::TOOLBOX_LIBRARIES`]) and StdCLib.
pub fn provided(l: &str) -> bool {
    crate::glue::TOOLBOX_LIBRARIES.contains(&l) || l == crate::stdclib::NAME || l == crate::stdclib::MATH_NAME || l == crate::threads::NAME || l == crate::mp::NAME || l == crate::dsp_c::NAME
}

fn err(e: i16) -> u32 { e as i32 as u32 }

impl<U: GuestUi> MacPpc<U> {
    fn finish(&mut self, name: &str, res: Result<Return, mac68k_engine::mixed::MixedError>, result: Option<Arg>, regs: &mut [u32; 32]) -> GlueOutcome {
        let r = match res { Ok(r) => r, Err(e) => return GlueOutcome::Refused(format!("{name}: {e}")) };
        if r.blocked { return GlueOutcome::Refused(format!("{name}: the handler tried to block the caller; the PowerPC run loop cannot suspend it yet")); }
        if let Some(kind) = result {
            regs[3] = match kind { Arg::B => r.value & 0xFF, Arg::W => r.value as u16 as i16 as i32 as u32, Arg::L | Arg::P => r.value };
        }
        if r.halted { GlueOutcome::Exit } else { GlueOutcome::Return }
    }

    /// The first `n` integer/pointer arguments: r3..r10, then the parameter area of the caller's frame, which starts
    /// 24 bytes above r1 (the linkage area: back chain, CR, LR, two reserved words, saved RTOC) with one word per
    /// argument (Mac OS Runtime Architectures, PowerPC calling conventions).
    fn arg_values(&self, n: usize, regs: &[u32; 32]) -> Result<alloc::vec::Vec<u32>, String> {
        (0..n).map(|i| if i < 8 { Ok(regs[3 + i]) } else {
            let at = regs[1].wrapping_add(24 + 4 * i as u32);
            self.mac.mem.read_u32(at).map_err(|e| format!("argument {i} is in the stack parameter area at {at:#x}: {e}"))
        }).collect()
    }

    /// A Pascal string argument (library or symbol names are ASCII).
    fn pstr(&self, at: u32) -> Result<String, String> {
        read_pstring_bytes(&self.mac.mem, at).map(|b| b.iter().map(|&c| char::from(c)).collect()).map_err(|e| e.0)
    }

    fn native(&mut self, n: Native, regs: &mut [u32; 32]) -> GlueOutcome {
        let a = |i: usize| regs[3 + i];
        match n {
            Native::Exit => { self.mac.log.push("[toolbox] ExitToShell".into()); GlueOutcome::Exit }
            Native::SysBeep => {
                // SysBeep(short duration): the GuestUi trait has no beep or sound call, so it is LOGGED, not played; the call
                // is accepted so programs that beep when they finish keep running. Missing piece: a UI-side beep event.
                self.mac.log.push(format!("[toolbox] SysBeep duration {} (logged only: GuestUi has no beep)", a(0) as u16 as i16));
                GlueOutcome::Return
            }
            Native::InitDone => GlueOutcome::Return,
            Native::NewRoutineDescriptor | Native::NewFatRoutineDescriptor => {
                use mac68k_engine::mixed_mode::{new_fat_routine_descriptor, new_routine_descriptor};
                // NewRoutineDescriptor(theProc, theProcInfo, theISA) / NewFatRoutineDescriptor(theM68kProc, thePowerPCProc, theProcInfo).
                // The ISA argument is an ISAType (a byte); NIL is the documented answer when there is no memory.
                let made = if n == Native::NewRoutineDescriptor { new_routine_descriptor(&mut self.mac, a(0), a(1), a(2) as u8) } else { new_fat_routine_descriptor(&mut self.mac, a(0), a(1), a(2)) };
                regs[3] = match made {
                    Ok(upp) => { self.mac.log.push(format!("[mixedmode] {n:?} proc={:#x} procInfo={:#x} -> {upp:#x}", a(0), a(1))); upp }
                    Err(e) => { self.mac.log.push(format!("[mixedmode] {n:?}: {e} -> NIL")); 0 }
                };
                GlueOutcome::Return
            }
            Native::NewUpp => {
                regs[3] = match mac68k_engine::mixed_mode::new_routine_descriptor(&mut self.mac, a(0), 0, mac68k_engine::mixed_mode::ISA_PPC) {
                    Ok(upp) => upp,
                    Err(e) => { self.mac.log.push(format!("[mixedmode] NewUPP: {e} -> NIL")); 0 }
                };
                GlueOutcome::Return
            }
            Native::DisposeUpp => { let _ = mac68k_engine::mixed_mode::dispose_routine_descriptor(&mut self.mac, a(0)); GlueOutcome::Return }
            Native::DisposeRoutineDescriptor => match mac68k_engine::mixed_mode::dispose_routine_descriptor(&mut self.mac, a(0)) {
                Ok(()) => { self.mac.log.push(format!("[mixedmode] DisposeRoutineDescriptor {:#x}", a(0))); GlueOutcome::Return }
                Err(e) => GlueOutcome::Refused(format!("{e}")),
            },
            Native::TrapAddress(q) => match mac68k_engine::traptable::trap_address(&mut self.mac, q.opcode(a(1)), a(0)) {
                Ok((at, line)) => { self.mac.log.push(line); regs[3] = at; GlueOutcome::Return }
                Err(e) => GlueOutcome::Refused(format!("{n:?}: {}", e.0)),
            },
            Native::HasDepth => {
                // HasDepth(gd, depth, whichFlags, flags): INTEGER; only the device and depth matter to the screen
                regs[3] = u32::from(mac68k_engine::cqd::depth::has_depth(&self.mac, a(0), a(1) as u16)) as u16 as i16 as i32 as u32;
                GlueOutcome::Return
            }
            Native::SetDepth => { regs[3] = mac68k_engine::cqd::depth::set_depth(&mut self.mac, a(0), a(1) as u16) as i32 as u32; GlueOutcome::Return }
            Native::SetJmp | Native::CExit | Native::BreakPoint | Native::CallbackReturn => GlueOutcome::Refused(format!("{n:?} needs the CPU registers and is served by the run loop")),
            Native::WaitNextEvent | Native::GetNextEvent => {
                // eventMask, EventRecord*, [sleep, mouseRgn]. The handler is polled with sleep 0 (it pumps the GuestUi and
                // returns a queued event or the null event). A null event for a caller that asked to sleep (WaitNextEvent
                // sleep != 0) is `Idle`: the run loop parks the CPU on this very call and retries it when a matching event is
                // queued or the sleep's ticks are up (`wait_deadline`: the host's clock then wakes it, and the retry returns the null
                // event, as the 68k engine's blocked call does). GetNextEvent never waits (Inside Macintosh): with nothing pending
                // it returns FALSE with a null event at once, so idle loops built on it keep running.
                let name = if n == Native::WaitNextEvent { "WaitNextEvent" } else { "GetNextEvent" };
                let Some(op) = mac68k_engine::mixed::trap_opcode("WaitNextEvent") else { return GlueOutcome::Refused("WaitNextEvent has no trap word".into()) };
                let timed_out = core::mem::take(&mut self.rt.wait_timed_out);
                let sleep = if n == Native::WaitNextEvent { a(2) } else { 0 };
                let r = self.mac.pascal_call(op, &[a(0), a(1), 0, 0]);
                if sleep != 0 && !timed_out && matches!(r, Ok(rr) if rr.value == 0 && !rr.halted) {
                    self.rt.wait_mask = Some(a(0) as u16);
                    // the same rule as the 68k engine's blocked call (events.rs): at least one tick, and a sleep of 0x50000+ ticks is "until an event"
                    self.rt.wait_deadline = (sleep < 0x0005_0000).then(|| self.mac.ev.ticks.wrapping_add(sleep.max(1)));
                    self.mac.log.push(format!("[cfm] {name} mask {:#06x}: no event, the caller waits", a(0) as u16));
                    return GlueOutcome::Idle;
                }
                self.finish(name, r, Some(Arg::B), regs)
            }
            Native::StandardAlert => {
                let args = match self.arg_values(5, regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("StandardAlert: {e}")) };
                match self.standard_alert(&args) { Ok(None) => GlueOutcome::Idle, Ok(Some(e)) => { regs[3] = e; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::ModalDialog => {
                // filterProc, VAR itemHit. The shared handler returns a queued hit or asks to block; a blocked call parks the CPU
                // ON the call and the run loop retries it once the front dialog has a hit (`Mac::modal_ready`).
                let Some(op) = mac68k_engine::mixed::trap_opcode("ModalDialog") else { return GlueOutcome::Refused("ModalDialog has no trap word".into()) };
                let r = self.mac.pascal_call(op, &[a(0), a(1)]);
                if matches!(r, Ok(rr) if rr.blocked) {
                    self.rt.wait_dialog = true;
                    self.mac.log.push("[cfm] ModalDialog: no item hit yet, the caller waits".into());
                    return GlueOutcome::Idle;
                }
                self.finish("ModalDialog", r, None, regs)
            }
            Native::File(c) => {
                let nargs = match c { crate::files::FileCall::HOpen | crate::files::FileCall::HOpenDF | crate::files::FileCall::HOpenRF | crate::files::FileCall::HCreate => 5, _ => 4 };
                let args = match self.arg_values(nargs, regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{c:?}: {e}")) };
                match self.file_call(c, &args) { Ok(e) => { regs[3] = err(e); GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Os(c) => {
                let args = match self.arg_values(c.arg_count(), regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{c:?}: {e}")) };
                match self.os_routine(c, &args) { Ok(v) => { regs[3] = v; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Process(sel) => {
                let n = mac68k_engine::procmgr::arg_count::<U>(sel).unwrap_or(0);
                let args = match self.arg_values(n, regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("Process Manager routine {sel:#x}: {e}")) };
                match mac68k_engine::procmgr::call_selector(&mut self.mac, sel, &args) {
                    Ok((v, kind)) => { regs[3] = match kind { Some(Arg::W) => v as u16 as i16 as i32 as u32, Some(Arg::B) => v & 0xFF, _ => v }; GlueOutcome::Return }
                    Err(e) => GlueOutcome::Refused(format!("Process Manager routine {sel:#x}: {}", e.0)),
                }
            }
            Native::Tb(f) => {
                // these need arguments and a result in registers only: run them on a scratch register file
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.tb_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Cf(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.cf_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Cfx(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.cfx_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Mn(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.mn_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Sys(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.sys_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Dsp(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.dsp_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Tm(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.tm_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Tsm(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.tsm_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Sc(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.sc_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Cx(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.cx_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Gw(f) => {
                let mut cpu = retro_ppc::Cpu::new(0, regs[1]);
                cpu.gpr = *regs;
                match self.gw_call(f, &mut cpu) { Ok(()) => { *regs = cpu.gpr; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Pb(c) => {
                let args = match self.arg_values(1, regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("PB call: {e}")) };
                match self.pb_call(c, args[0]) { Ok(e) => { regs[3] = err(e); GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Alias(c) => {
                let args = match self.arg_values(c.arg_count(), regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{}: {e}", c.name())) };
                match self.alias_call(c, &args) { Ok(v) => { regs[3] = v; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::Spec(c) => {
                let args = match self.arg_values(c.arg_count(), regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{}: {e}", c.name())) };
                match self.spec_call(c, &args) { Ok(v) => { regs[3] = v; GlueOutcome::Return } Err(e) => GlueOutcome::Refused(e) }
            }
            Native::C(_) | Native::Math(_) | Native::Stdio(_) | Native::Thread(_) | Native::Mp(_) | Native::Carbon(_) | Native::Ae(_) => GlueOutcome::Refused(format!("{n:?} needs the CPU registers and is served by the run loop")),
            Native::MemError => { regs[3] = err(self.mac.mem_error()); GlueOutcome::Return }
            Native::BlockZero => {
                let (dest, n) = (a(0), a(1));
                if self.mac.mem.zero(dest, n).is_err() {
                    let why = format!("BlockZero: {n} bytes at {dest:#x} leave guest RAM");
                    self.mac.log.push(format!("[toolbox] {why}"));
                    self.mac.refused.push(why);
                }
                GlueOutcome::Return
            }
            Native::GetSharedLibrary => {
                let (name_p, conn_p, main_p, err_p) = (a(0), a(3), a(4), a(5));
                let name = match self.pstr(name_p) { Ok(n) => n, Err(e) => return GlueOutcome::Refused(format!("GetSharedLibrary: {e}")) };
                if !provided(name.as_str()) {
                    if err_p != 0 { let _ = self.mac.mem.write_u8(err_p, name.len().min(255) as u8).and_then(|_| self.mac.mem.write_bytes(err_p + 1, &name.as_bytes()[..name.len().min(255)])); }
                    self.mac.log.push(format!("[cfm] GetSharedLibrary \"{name}\" -> fragLibNotFound"));
                    regs[3] = err(FRAG_LIB_NOT_FOUND);
                    return GlueOutcome::Return;
                }
                let id = match self.connections.iter().position(|c| *c == name) { Some(i) => i, None => { self.connections.push(name.clone()); self.connections.len() - 1 } } as u32 + 1;
                let ok = self.mac.mem.write_u32(conn_p, id).is_ok() && (main_p == 0 || self.mac.mem.write_u32(main_p, 0).is_ok());
                self.mac.log.push(format!("[cfm] GetSharedLibrary \"{name}\" -> connection {id}"));
                regs[3] = if ok { 0 } else { err(-50) };
                GlueOutcome::Return
            }
            Native::FindSymbol => {
                let (conn, name_p, addr_p, class_p) = (a(0), a(1), a(2), a(3));
                let Some(lib) = conn.checked_sub(1).and_then(|i| self.connections.get(i as usize)).filter(|n| !n.is_empty()).cloned() else {
                    regs[3] = err(FRAG_CONNECTION_ID_NOT_FOUND);
                    return GlueOutcome::Return;
                };
                let name = match self.pstr(name_p) { Ok(n) => n, Err(e) => return GlueOutcome::Refused(format!("FindSymbol: {e}")) };
                let b = self.bind_from(&lib, &name);
                let (addr, class) = match &b {
                    Binding::Unbound(_) => { self.mac.log.push(format!("[cfm] FindSymbol {lib}/{name} -> fragSymbolNotFound")); regs[3] = err(FRAG_SYMBOL_NOT_FOUND); return GlueOutcome::Return; }
                    Binding::Qd | Binding::CData(_) => match self.data_block(&name, &b) { Ok(p) => (p, SYMBOL_CLASS_DATA), Err(e) => return GlueOutcome::Refused(format!("FindSymbol: {e}")) },
                    _ => match self.glue.intern(&mut self.mac, &lib, &name) { Ok(id) => (self.glue.entry(id).map_or(0, |e| e.tvector), SYMBOL_CLASS_TVECTOR), Err(e) => return GlueOutcome::Refused(format!("FindSymbol: {e}")) },
                };
                let ok = self.mac.mem.write_u32(addr_p, addr).is_ok() && (class_p == 0 || self.mac.mem.write_u8(class_p, class).is_ok());
                self.mac.log.push(format!("[cfm] FindSymbol {lib}/{name} -> {addr:#x}"));
                regs[3] = if ok { 0 } else { err(-50) };
                GlueOutcome::Return
            }
            Native::CloseConnection => {
                let p = a(0);
                let id = self.mac.mem.read_u32(p).unwrap_or(0);
                if id == 0 || self.connections.get(id as usize - 1).map_or(true, |n| n.is_empty()) { regs[3] = err(FRAG_CONNECTION_ID_NOT_FOUND); return GlueOutcome::Return; }
                // a closed connection keeps its slot (ids are never reused) but has no name, so FindSymbol refuses it
                self.connections[id as usize - 1].clear();
                let _ = self.mac.mem.write_u32(p, 0);
                regs[3] = 0;
                GlueOutcome::Return
            }
        }
    }
}

impl<U: GuestUi> PpcHost for MacPpc<U> {
    fn glue_call(&mut self, symbol: &str, regs: &mut [u32; 32]) -> GlueOutcome {
        let Some(b) = self.bindings.get(symbol).cloned() else {
            return GlueOutcome::Refused(format!("{symbol}: no binding was recorded for this glue stub"));
        };
        match b {
            Binding::Pascal(op) => {
                let Some((layout, result)) = self.mac.pascal_signature(op) else { return GlueOutcome::Refused(format!("{symbol}: registered with the other calling convention")) };
                let values = match self.arg_values(layout.len(), regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{symbol}: {e}")) };
                let r = self.mac.pascal_call(op, &values);
                self.finish(symbol, r, result, regs)
            }
            Binding::Os(op, sig) => {
                let values = match self.arg_values(sig.args.len(), regs) { Ok(v) => v, Err(e) => return GlueOutcome::Refused(format!("{symbol}: {e}")) };
                let r = self.mac.os_call(op, sig, &values);
                let result = sig.result.map(|_| Arg::L);
                self.finish(symbol, r, result, regs)
            }
            Binding::Native(n) => self.native(n, regs),
            Binding::Qd | Binding::CData(_) => GlueOutcome::Refused(format!("{symbol} is a data symbol, it cannot be called")),
            Binding::Export(a) => GlueOutcome::Refused(format!("{symbol}: an export at {a:#x} is called through its own TVector, never through glue")),
            Binding::Unbound(why) => {
                let line = format!("{UNIMPLEMENTED} import {symbol}: {why}");
                self.mac.log.push(format!("[toolbox] REFUSED: {line}"));
                self.mac.refused.push(line);
                GlueOutcome::Refused(format!("NEXT-NEEDED {symbol}: {why}"))
            }
        }
    }

    fn glue_symbol_at(&self, pc: u32) -> Option<String> { self.glue.at_stub(pc).map(|e| e.key()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guestui_abi::RecordingGuestUi;

    #[test]
    fn arguments_past_r10_come_from_the_parameter_area_above_the_linkage_area() {
        let m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        let sp = mac68k_engine::machine::STACK_TOP - 256;
        let mut m = m;
        // parameter word i (i >= 8) lives at sp + 24 + 4*i
        m.mac.mem.write_u32(sp + 24 + 4 * 8, 0xAAAA_0008).unwrap();
        m.mac.mem.write_u32(sp + 24 + 4 * 9, 0xAAAA_0009).unwrap();
        let mut regs = [0u32; 32];
        regs[1] = sp;
        for i in 0..8 { regs[3 + i] = 100 + i as u32; }
        assert_eq!(m.arg_values(10, &regs).unwrap(), [100, 101, 102, 103, 104, 105, 106, 107, 0xAAAA_0008, 0xAAAA_0009]);
        // negative control: only r3..r10 are consulted for 8 arguments, and a stack pointer outside RAM is a named error
        assert_eq!(m.arg_values(8, &regs).unwrap().len(), 8);
        regs[1] = 0xF000_0000;
        assert!(m.arg_values(9, &regs).unwrap_err().contains("argument 8 is in the stack parameter area"));
    }
}
