//! Library vector-table builder and the LVO -> handler dispatch table.
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

/// One vector slot: `ILLEGAL; RTS`.
pub const SLOT_LEN: u32 = 6;
pub const ILLEGAL: u16 = 0x4AFC;
pub const RTS: u16 = 0x4E75;

pub type Handler<U> = fn(&mut Amiga<U>, &mut Cpu) -> Result<String, Refusal>;
/// slot address -> (library, function, handler)
pub type Dispatch<U> = alloc::collections::BTreeMap<u32, (&'static str, &'static str, Handler<U>)>;

/// Write one `ILLEGAL; RTS` slot at `addr`.
pub fn write_slot(mem: &mut GuestMem, addr: u32) -> Result<(), retro_guest_mem::MemError> {
    mem.write_u16(addr, ILLEGAL)?;
    mem.write_u16(addr + 2, RTS)
}

/// Declarative description of a library: its base address and its LVOs.
pub struct LibraryBuilder<U: GuestUi> {
    pub name: &'static str,
    pub base: u32,
    funcs: Vec<(i32, &'static str, Option<Handler<U>>)>,
}

impl<U: GuestUi> LibraryBuilder<U> {
    pub fn new(name: &'static str, base: u32) -> Self { LibraryBuilder { name, base, funcs: vec![] } }
    /// Register an implemented function at negative offset `lvo` (e.g. -204).
    pub fn func(mut self, lvo: i32, name: &'static str, h: Handler<U>) -> Self {
        self.funcs.push((lvo, name, Some(h)));
        self
    }
    /// Register an implemented function by NAME: its vector number is looked up in the generated `lvo_names` table, so
    /// no LVO is hand-typed. A name the library does not have is a programming error (panics at registration).
    pub fn named(self, name: &'static str, h: Handler<U>) -> Self {
        let lvo = known_functions(self.name).iter().find(|(_, n)| *n == name).map(|(l, _)| *l)
            .unwrap_or_else(|| panic!("{} has no function {name} in lvo_names", self.name));
        self.func(lvo, name, h)
    }
    /// Apply a registration helper (a module's own list of functions) in the middle of a chain.
    pub fn with(self, f: impl FnOnce(Self) -> Self) -> Self { f(self) }
    /// Reserve a slot with no implementation: calling it is a named refusal, never a silent d0=0.
    pub fn stub(mut self, lvo: i32, name: &'static str) -> Self {
        self.funcs.push((lvo, name, None));
        self
    }
    /// Write the vector table into guest RAM and add the handlers to `dispatch`. Every other function the library is
    /// known to have (`lvo_names`) gets a slot too, so a call to anything not implemented is a NAMED refusal
    /// ("dos.library/Open"), never a jump into zeroed memory. The positive side gets a minimal `struct Library`:
    /// ln_Type = NT_LIBRARY (base+8), lib_Version (base+20), lib_NegSize (base+16).
    pub fn install(self, mem: &mut GuestMem, dispatch: &mut Dispatch<U>) -> Result<(), Refusal> {
        let mut neg = 0u32;
        for (lvo, name, h) in self.funcs.iter().copied() {
            let slot = self.base.wrapping_add(lvo as u32);
            write_slot(mem, slot).map_err(Refusal::mem("library vector"))?;
            dispatch.insert(slot, (self.name, name, h.unwrap_or(unimplemented_lvo::<U>)));
            neg = neg.max(lvo.unsigned_abs());
        }
        for &(lvo, name) in known_functions(self.name) {
            let slot = self.base.wrapping_add(lvo as u32);
            if dispatch.contains_key(&slot) { continue; }
            write_slot(mem, slot).map_err(Refusal::mem("library vector"))?;
            dispatch.insert(slot, (self.name, name, unimplemented_lvo::<U>));
            neg = neg.max(lvo.unsigned_abs());
        }
        mem.write_u8(self.base + 8, NT_LIBRARY).map_err(Refusal::mem("Library.ln_Type"))?;
        mem.write_u16(self.base + 16, neg as u16).map_err(Refusal::mem("Library.lib_NegSize"))?;
        mem.write_u16(self.base + 20, LIB_VERSION).map_err(Refusal::mem("Library.lib_Version"))?;
        Ok(())
    }
}

/// `struct Node.ln_Type` of a library (exec/nodes.h NT_LIBRARY).
pub const NT_LIBRARY: u8 = 9;
/// The version every bridged library reports (AmigaOS 3.1 = 40): programs refuse to run on older ones.
pub const LIB_VERSION: u16 = 40;

/// The functions `library` (e.g. "dos.library") is known to have, from the generated table; empty if unknown.
pub fn known_functions(library: &str) -> &'static [(i32, &'static str)] {
    crate::lvo_names::LIBRARIES.iter().find(|(n, _)| n.eq_ignore_ascii_case(library)).map(|(_, f)| *f).unwrap_or(&[])
}

/// The table's own spelling of `library`, if it is known (OpenLibrary keeps a `&'static str` name).
pub fn known_library(library: &str) -> Option<&'static str> {
    crate::lvo_names::LIBRARIES.iter().find(|(n, _)| n.eq_ignore_ascii_case(library)).map(|(n, _)| *n)
}

/// The prefix every unimplemented-call refusal starts with (the census runner keys on it).
pub const UNIMPLEMENTED: &str = "unimplemented";

fn unimplemented_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if a.stop_on_unimplemented { cpu.halt(); }
    Err(Refusal(UNIMPLEMENTED.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use guestui_abi::RecordingGuestUi;

    #[test]
    fn install_writes_illegal_rts_and_registers() {
        let mut mem = GuestMem::new(0x10000);
        let mut d: Dispatch<RecordingGuestUi> = Default::default();
        LibraryBuilder::new("x.library", 0x2000).stub(-30, "A").stub(-36, "B").install(&mut mem, &mut d).unwrap();
        assert_eq!(mem.read_u16(0x2000 - 30).unwrap(), ILLEGAL);
        assert_eq!(mem.read_u16(0x2000 - 28).unwrap(), RTS);
        assert_eq!(d.len(), 2);
        assert_eq!(d[&(0x2000 - 36)].1, "B");
    }

    #[test]
    fn vector_outside_ram_is_refused() {
        let mut mem = GuestMem::new(0x1000);
        let mut d: Dispatch<RecordingGuestUi> = Default::default();
        assert!(LibraryBuilder::new("x.library", 0x2000).stub(-30, "A").install(&mut mem, &mut d).is_err());
    }
}
