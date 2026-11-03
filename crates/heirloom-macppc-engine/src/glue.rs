//! Glue: one 16-byte cell per imported function. `[0..8)` is the TVector `{code = stub, toc = 0}` an import relocates
//! to; `[8..12)` is the stub itself, a `blr`, and its address is what the CPU's hook recognises. Cells come in chunks
//! of [`CHUNK_CELLS`] taken from the guest heap, so the table grows with the number of distinct imports.
use crate::error::CfmError;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::Mac;

pub const CELL: u32 = 16;
pub const CHUNK_CELLS: u32 = 256;
/// `blr`: what the stub holds, so a CPU without the hook still returns to its caller (with r3 unchanged).
pub const BLR: u32 = 0x4E80_0020;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlueEntry { pub library: String, pub symbol: String, pub tvector: u32, pub stub: u32 }

/// `keys[id]` is entry `id`'s binding key, made once at intern: the run loop looks it up on every glue call, and building it
/// per call (a String, a `format!` for a non-Toolbox library) was a measured share of a Warcraft III frame (doc 22 phase 5).
#[derive(Default)]
pub struct Glue {
    chunks: Vec<u32>,
    entries: Vec<GlueEntry>,
    keys: Vec<Rc<str>>,
    by_key: BTreeMap<(String, String), usize>,
    /// `id_at_stub` answers by stub address, direct-mapped (a cell's stub never moves and its id never changes once interned, so a
    /// remembered answer stays true): every glue call asks, twice with the stage clock on, and the chunk walk was ~46 host
    /// instructions a call. (pc, id + 1); 0 = empty.
    memo: [core::cell::Cell<(u32, u32)>; 32],
}

/// The libraries whose exports are the Macintosh Toolbox under the Universal Interfaces' C names: InterfaceLib itself, CarbonLib, and the
/// Mac OS 8.6+ sub-libraries the Toolbox was split into (the import-library set of a Mac OS 9 SDK, Retro68 `ImportLibraries/`: AppearanceLib,
/// ControlsLib, DialogsLib, MenusLib, WindowsLib, NavigationLib; metamage_1 `Roles/libcarbon/A-line.confd/PPC.conf` imports the same four).
/// One name table serves them all, so an import resolves by name whichever of them the fragment names; a name the table cannot serve is
/// NEXT-NEEDED with the library in its message.
pub const TOOLBOX_LIBRARIES: &[&str] = &["InterfaceLib", "CarbonLib", "AppearanceLib", "ControlsLib", "DialogsLib", "MenusLib", "WindowsLib", "NavigationLib"];

/// The key a symbol's [`crate::symbols::Binding`] is stored under: the bare name for the Toolbox libraries and the
/// engine's own pseudo-library, `Library::name` for every other provided library, so two libraries exporting one name
/// keep separate bindings.
pub fn binding_key(library: &str, symbol: &str) -> String {
    if library == "CFM" || TOOLBOX_LIBRARIES.contains(&library) { symbol.into() } else { alloc::format!("{library}::{symbol}") }
}

impl GlueEntry { pub fn key(&self) -> String { binding_key(&self.library, &self.symbol) } }

impl Glue {
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
    pub fn entry(&self, id: usize) -> Option<&GlueEntry> { self.entries.get(id) }

    /// The cell for `library/symbol`, allocating it (and a chunk when the last one is full) on first use.
    pub fn intern<U: GuestUi>(&mut self, mac: &mut Mac<U>, library: &str, symbol: &str) -> Result<usize, CfmError> {
        if let Some(&id) = self.by_key.get(&(library.into(), symbol.into())) { return Ok(id); }
        let id = self.entries.len();
        if id as u32 / CHUNK_CELLS >= self.chunks.len() as u32 {
            let bytes = CELL * CHUNK_CELLS;
            let base = mac.mm.new_ptr(&mut mac.mem, &mut mac.heap, bytes, true).map_err(|_| CfmError::NoMemory { what: "a glue chunk", bytes })?;
            self.chunks.push(base);
        }
        let tv = self.chunks[id / CHUNK_CELLS as usize] + CELL * (id as u32 % CHUNK_CELLS);
        let stub = tv + 8;
        let m = |e| CfmError::Memory(alloc::format!("glue cell at {tv:#x}: {e}"));
        mac.mem.write_u32(tv, stub).map_err(m)?;
        mac.mem.write_u32(tv + 4, 0).map_err(m)?;
        mac.mem.write_u32(stub, BLR).map_err(m)?;
        self.entries.push(GlueEntry { library: library.into(), symbol: symbol.into(), tvector: tv, stub });
        self.keys.push(Rc::from(binding_key(library, symbol)));
        self.by_key.insert((library.into(), symbol.into()), id);
        Ok(id)
    }

    /// The cell whose STUB is at `pc` (not its TVector), if any.
    pub fn at_stub(&self, pc: u32) -> Option<&GlueEntry> { self.id_at_stub(pc).and_then(|id| self.entries.get(id)) }

    /// The binding key of cell `id` (shared, not rebuilt).
    pub fn key_of(&self, id: usize) -> Option<Rc<str>> { self.keys.get(id).cloned() }

    /// The binding key of the cell whose STUB is at `pc` (shared, not rebuilt), if any.
    pub fn key_at_stub(&self, pc: u32) -> Option<Rc<str>> { self.id_at_stub(pc).and_then(|id| self.keys.get(id)).cloned() }

    /// The id of the cell whose STUB is at `pc`, if any.
    pub fn id_at_stub(&self, pc: u32) -> Option<usize> {
        let slot = &self.memo[((pc / CELL) % 32) as usize];
        let (p, id) = slot.get();
        if p == pc && id != 0 { return Some(id as usize - 1); }
        let found = self.id_at_stub_walk(pc);
        if let Some(id) = found.filter(|&id| id < u32::MAX as usize) { slot.set((pc, id as u32 + 1)); }
        found
    }

    fn id_at_stub_walk(&self, pc: u32) -> Option<usize> {
        for (ci, &base) in self.chunks.iter().enumerate() {
            let size = CELL * CHUNK_CELLS;
            if pc >= base && pc - base < size && (pc - base) % CELL == 8 {
                let id = ci * CHUNK_CELLS as usize + ((pc - base) / CELL) as usize;
                return (id < self.entries.len()).then_some(id);
            }
        }
        None
    }

    /// The TVector address already interned for `library/symbol`.
    pub fn tvector_of(&self, library: &str, symbol: &str) -> Option<u32> {
        self.by_key.get(&(library.into(), symbol.into())).and_then(|&i| self.entries.get(i)).map(|e| e.tvector)
    }
}
