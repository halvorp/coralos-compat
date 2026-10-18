//! Glue: one 16-byte cell per imported function. `[0..8)` is the TVector `{code = stub, toc = 0}` an import relocates
//! to; `[8..12)` is the stub itself, a `blr`, and its address is what the CPU's hook recognises. Cells come in chunks
//! of [`CHUNK_CELLS`] taken from the guest heap, so the table grows with the number of distinct imports.
use crate::error::CfmError;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use mac68k_engine::Mac;

pub const CELL: u32 = 16;
pub const CHUNK_CELLS: u32 = 256;
/// `blr`: what the stub holds, so a CPU without the hook still returns to its caller (with r3 unchanged).
pub const BLR: u32 = 0x4E80_0020;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlueEntry { pub library: String, pub symbol: String, pub tvector: u32, pub stub: u32 }

#[derive(Default)]
pub struct Glue { chunks: Vec<u32>, entries: Vec<GlueEntry>, by_key: BTreeMap<(String, String), usize> }

/// The key a symbol's [`crate::symbols::Binding`] is stored under: the bare name for InterfaceLib (the Toolbox) and the
/// engine's own pseudo-library, `Library::name` for every other provided library, so two libraries exporting one name
/// keep separate bindings.
pub fn binding_key(library: &str, symbol: &str) -> String {
    if library == "InterfaceLib" || library == "CFM" { symbol.into() } else { alloc::format!("{library}::{symbol}") }
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
        self.by_key.insert((library.into(), symbol.into()), id);
        Ok(id)
    }

    /// The cell whose STUB is at `pc` (not its TVector), if any.
    pub fn at_stub(&self, pc: u32) -> Option<&GlueEntry> {
        for (ci, &base) in self.chunks.iter().enumerate() {
            let size = CELL * CHUNK_CELLS;
            if pc >= base && pc - base < size && (pc - base) % CELL == 8 {
                return self.entries.get(ci * CHUNK_CELLS as usize + ((pc - base) / CELL) as usize);
            }
        }
        None
    }

    /// The TVector address already interned for `library/symbol`.
    pub fn tvector_of(&self, library: &str, symbol: &str) -> Option<u32> {
        self.by_key.get(&(library.into(), symbol.into())).and_then(|&i| self.entries.get(i)).map(|e| e.tvector)
    }
}
