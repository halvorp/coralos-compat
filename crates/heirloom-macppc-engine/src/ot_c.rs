//! Open Transport's atomic utilities from `Apple;Carbon;Networking` (a Carbon application's import name for the OT client
//! library): `SInt32 OTAtomicAdd32(SInt32 toAdd, SInt32 *dest)` adds to the 32-bit word at `dest` and returns the NEW value
//! (OpenTransport.h, public API contract). The Miles Sound System calls it unguarded from its mixer, so a weak import left at 0
//! faults the first time the digital driver runs. The guest runs on one host thread and is never pre-empted inside a native call,
//! so a read-modify-write here is atomic as far as the guest can observe. NAMED LIMIT: no networking at all - every endpoint,
//! mapper and notifier call of the library stays unbound (weak-absent, a call refuses by name).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;

/// The library names a program imports Open Transport's client calls from.
pub fn provided(lib: &str) -> bool { matches!(lib, "Apple;Carbon;Networking" | "OTUtilityLib") }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtFn { OtAtomicAdd32 }

impl OtFn {
    pub fn by_name(name: &str) -> Option<OtFn> {
        match name { "OTAtomicAdd32" => Some(OtFn::OtAtomicAdd32), _ => None }
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn ot_call(&mut self, f: OtFn, regs: &mut [u32; 32]) -> Result<(), String> {
        match f {
            OtFn::OtAtomicAdd32 => {
                let (add, dest) = (regs[3], regs[4]);
                let old = self.mac.mem.read_u32(dest).map_err(|e| format!("OTAtomicAdd32: dest {dest:#x} is not guest RAM ({e})"))?;
                let new = old.wrapping_add(add);
                self.mac.mem.write_u32(dest, new).map_err(|e| format!("OTAtomicAdd32: dest {dest:#x} is not writable ({e})"))?;
                regs[3] = new;
            }
        }
        Ok(())
    }
}
