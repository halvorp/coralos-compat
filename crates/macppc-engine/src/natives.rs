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
pub const PROVIDED_LIBRARIES: &[&str] = &["InterfaceLib"];

fn err(e: i16) -> u32 { e as i32 as u32 }

impl<U: GuestUi> MacPpc<U> {
    fn finish(&mut self, name: &str, res: Result<Return, mac68k_engine::mixed::MixedError>, result: Option<Arg>, regs: &mut [u32; 32]) -> GlueOutcome {
        let r = match res { Ok(r) => r, Err(e) => return GlueOutcome::Refused(format!("{name}: {e}")) };
        if r.blocked { return GlueOutcome::Refused(format!("{name}: the handler tried to block the caller; the PowerPC run loop cannot suspend it yet")); }
        if let Some(kind) = result {
            regs[3] = match kind { Arg::B => r.value & 0xFF, Arg::W => r.value as u16 as i16 as i32 as u32, Arg::L => r.value };
        }
        if r.halted { GlueOutcome::Exit } else { GlueOutcome::Return }
    }

    /// A Pascal string argument (library or symbol names are ASCII).
    fn pstr(&self, at: u32) -> Result<String, String> {
        read_pstring_bytes(&self.mac.mem, at).map(|b| b.iter().map(|&c| char::from(c)).collect()).map_err(|e| e.0)
    }

    fn native(&mut self, n: Native, regs: &mut [u32; 32]) -> GlueOutcome {
        let a = |i: usize| regs[3 + i];
        match n {
            Native::Exit => GlueOutcome::Exit,
            Native::WaitNextEvent | Native::GetNextEvent => {
                // eventMask, EventRecord*, [sleep, mouseRgn]: the sleep is dropped so the call never suspends the caller
                // (a poll; the PowerPC run loop cannot idle yet, see docs/MACPPC-PLUMBING.md), and a non-zero sleep is logged
                let name = if n == Native::WaitNextEvent { "WaitNextEvent" } else { "GetNextEvent" };
                let Some(op) = mac68k_engine::mixed::trap_opcode("WaitNextEvent") else { return GlueOutcome::Refused("WaitNextEvent has no trap word".into()) };
                if n == Native::WaitNextEvent && a(2) != 0 { self.mac.log.push(format!("[cfm] WaitNextEvent sleep={} ticks polled, not slept", a(2))); }
                let r = self.mac.pascal_call(op, &[a(0), a(1), 0, 0]);
                self.finish(name, r, Some(Arg::B), regs)
            }
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
                if !PROVIDED_LIBRARIES.contains(&name.as_str()) {
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
                let b = self.bind(&name);
                let (addr, class) = match &b {
                    Binding::Unbound(_) => { self.mac.log.push(format!("[cfm] FindSymbol {lib}/{name} -> fragSymbolNotFound")); regs[3] = err(FRAG_SYMBOL_NOT_FOUND); return GlueOutcome::Return; }
                    Binding::Qd => match self.qd_block() { Ok(p) => (p, SYMBOL_CLASS_DATA), Err(e) => return GlueOutcome::Refused(format!("FindSymbol: {e}")) },
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
                if layout.len() > 8 { return GlueOutcome::Refused(format!("{symbol}: {} arguments need the PowerPC stack parameter area (only r3..r10 are handled)", layout.len())); }
                let values: alloc::vec::Vec<u32> = regs[3..3 + layout.len()].to_vec();
                let r = self.mac.pascal_call(op, &values);
                self.finish(symbol, r, result, regs)
            }
            Binding::Os(op, sig) => {
                let values: alloc::vec::Vec<u32> = regs[3..3 + sig.args.len()].to_vec();
                let r = self.mac.os_call(op, sig, &values);
                let result = sig.result.map(|_| Arg::L);
                self.finish(symbol, r, result, regs)
            }
            Binding::Native(n) => self.native(n, regs),
            Binding::Qd => GlueOutcome::Refused(format!("{symbol} is a data symbol, it cannot be called")),
            Binding::Unbound(why) => {
                let line = format!("{UNIMPLEMENTED} import {symbol}: {why}");
                self.mac.log.push(format!("[toolbox] REFUSED: {line}"));
                self.mac.refused.push(line);
                GlueOutcome::Refused(format!("NEXT-NEEDED {symbol}: {why}"))
            }
        }
    }

    fn glue_symbol_at(&self, pc: u32) -> Option<String> { self.glue.at_stub(pc).map(|e| e.symbol.clone()) }
}
