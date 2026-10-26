//! Explicit bindings for a program's OWN window definition (HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING).
//!
//! A `procID` selects `WDEF id = procID >> 4`. The system WDEFs (0 and 1) have native roles ([`crate::toolbox::role_of`]). Any other
//! WDEF is guest code that draws the frame, and the engine never runs it to draw chrome (all guest UI goes through the seam), so a
//! custom window is presented natively ONLY when a [`Binding`] names that exact definition. A binding is keyed by what the definition
//! IS, not by what the program is called or what number it chose: the resource id, its byte length and an FNV-1a 64-bit digest of its
//! bytes (the digest is a fact about the resource; no definition code is stored or copied). A WDEF with no binding is refused by name,
//! as before.
//!
//! The first entry is MacPaint 2.1F's floating palette (WDEF 34, procID 544 = variation 0): a small-title-bar window that floats above
//! the document windows. It is bound to the seam's `WindowRole::Utility` (a palette window), not resizable. The CoralOS host mapping
//! this requires is `WindowRole::Utility` drawn as a palette frame (kit gap 15: dialog is framed, utility is not yet); a host without
//! a utility role refuses `open_window` by name and the program gets the refusal.
use crate::machine::Mac;
use crate::Refusal;
use alloc::format;
use heirloom_ui_abi::{GuestUi, WindowRole};

/// One bound window definition.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    pub name: &'static str,
    pub id: i16,
    pub len: usize,
    pub fnv1a64: u64,
    pub role: WindowRole,
    pub resizable: bool,
}

/// The bound definitions.
pub const BINDINGS: &[Binding] = &[
    Binding { name: "MacPaint 2.1F floating palette", id: 34, len: 724, fnv1a64: 0x9b57_374e_27d9_8836, role: WindowRole::Utility, resizable: false },
];

/// FNV-1a, 64 bits.
pub fn fnv1a64(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl<U: GuestUi> Mac<U> {
    /// The native role of a window with this `procID`: a system WDEF's, else the bound role of the program's own definition. Reads the
    /// `WDEF` resource without loading it (no `ResErr` is set, no handle made).
    pub(crate) fn window_role(&self, proc_id: u16) -> Result<(WindowRole, bool), Refusal> { self.window_role_in(BINDINGS, proc_id) }

    /// [`Mac::window_role`] against an explicit binding table (the production table is [`BINDINGS`]).
    pub fn window_role_in(&self, bindings: &[Binding], proc_id: u16) -> Result<(WindowRole, bool), Refusal> {
        let unbound = match crate::toolbox::role_of(proc_id) { Ok(r) => return Ok(r), Err(e) => e };
        let id = (proc_id >> 4) as i16;
        for rn in self.rm.chain(false) {
            let Some(f) = self.rm.file(rn) else { continue };
            let Some(i) = f.fork.find(*b"WDEF", id) else { continue };
            let Some(bytes) = f.fork.data(i) else { continue };
            let sum = fnv1a64(bytes);
            return match bindings.iter().find(|b| b.id == id && b.len == bytes.len() && b.fnv1a64 == sum) {
                Some(b) => Ok((b.role, b.resizable)),
                None => Err(Refusal(format!("{} (WDEF {id} is {} bytes, digest {sum:#018x}: no binding entry names it)", unbound.0, bytes.len()))),
            };
        }
        Err(unbound)
    }
}
