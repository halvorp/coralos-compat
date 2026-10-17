//! Library vector-table builder and the LVO -> handler dispatch table.
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
pub type Dispatch<U> = std::collections::HashMap<u32, (&'static str, &'static str, Handler<U>)>;

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
    /// Reserve a slot with no implementation: calling it is a named refusal, never a silent d0=0.
    pub fn stub(mut self, lvo: i32, name: &'static str) -> Self {
        self.funcs.push((lvo, name, None));
        self
    }
    /// Write the vector table into guest RAM and add the handlers to `dispatch`.
    pub fn install(self, mem: &mut GuestMem, dispatch: &mut Dispatch<U>) -> Result<(), Refusal> {
        for (lvo, name, h) in self.funcs {
            let slot = self.base.wrapping_add(lvo as u32);
            write_slot(mem, slot).map_err(Refusal::mem("library vector"))?;
            dispatch.insert(slot, (self.name, name, h.unwrap_or(unimplemented_lvo::<U>)));
        }
        Ok(())
    }
}

fn unimplemented_lvo<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> {
    Err(Refusal("no bridge implementation".into()))
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
