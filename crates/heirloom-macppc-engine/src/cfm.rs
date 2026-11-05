//! Code Fragment Manager instantiation: place a PEF fragment's sections in guest memory, bind its imports, apply its
//! relocations, find its entry TVectors.
//!
//! Imports become the value the relocation engine adds into the importing fragment's TOC / data: for a function, the
//! address of a glue TVector ([`crate::glue`]); for the `qd` data symbol, the globals block; for a weak symbol nobody
//! provides, zero (the fragment tests for it); for a strong symbol nobody provides, a glue TVector that REFUSES BY
//! NAME when called (or [`CfmError::UnresolvedImport`] in strict mode) and, for data, an address outside guest RAM.
use crate::error::CfmError;
use crate::machine::{MacPpc, UNBOUND_DATA};
use crate::symbols::Binding;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_fmt_pef::loader::{ExportTarget, SymbolClass};
use heirloom_fmt_pef::{Pef, PefError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportStatus {
    /// Served by a handler; the string says which.
    Bound(String),
    /// Strong import with no provider: calling it refuses by name (NEXT-NEEDED).
    Unbound(String),
    /// Weak import with no provider: relocated to zero.
    WeakAbsent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub library: String,
    pub symbol: String,
    pub class: SymbolClass,
    pub weak: bool,
    pub status: ImportStatus,
    /// The value relocations added: a TVector / data address, or 0.
    pub value: u32,
}

/// Where a library loaded at run time came from (its CFragInitBlock's locator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibLocator {
    /// kDataForkCFragLocator: the guest FSSpec it was loaded through (copied into the block), and the span of the data fork.
    Disk { spec: u32, offset: u32, length: u32 },
    /// kMemoryCFragLocator: where the container was in memory.
    Memory { address: u32, length: u32 },
}

/// A TVector read from the instantiated fragment: where the code is and the TOC to load into r2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tv { pub address: u32, pub code: u32, pub toc: u32 }

#[derive(Debug, Clone)]
pub struct Fragment {
    pub name: String,
    /// Guest address of each instantiated section (None for loader/debug/... sections).
    pub section_bases: Vec<Option<u32>>,
    pub imports: Vec<ResolvedImport>,
    pub main: Option<Tv>,
    pub init: Option<Tv>,
    pub term: Option<Tv>,
    /// Exported symbol name -> guest address (relocated).
    pub exports: Vec<(String, u32)>,
}

/// What the CPU needs to start `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartState {
    pub pc: u32,
    pub r1: u32,
    pub r2: u32,
    /// LR: returning from `main` lands on the `ExitToShell` glue stub.
    pub lr: u32,
}

pub const STACK_SIZE: u32 = 256 * 1024;
/// Sections may ask for at most 4 KiB alignment (a page); more is refused by name, never silently reduced.
pub const MAX_ALIGNMENT_EXPONENT: u8 = 12;

fn mem_err(what: &'static str) -> impl Fn(heirloom_mem_flat32::MemError) -> CfmError { move |e| CfmError::Memory(format!("{what}: {e}")) }

impl<U: GuestUi> MacPpc<U> {
    /// Instantiate `pef` (named `name` in reports).
    pub fn instantiate(&mut self, name: &str, pef: &Pef) -> Result<Fragment, CfmError> {
        let loader = pef.loader()?;
        // 1. sections
        let mut bases: Vec<Option<u32>> = alloc::vec![None; pef.sections.len()];
        for (i, s) in pef.sections.iter().enumerate() {
            if !s.kind.instantiated() { continue; }
            if s.alignment > MAX_ALIGNMENT_EXPONENT { return Err(CfmError::AlignmentTooLarge { section: i, exponent: s.alignment }); }
            let align = 1u32 << s.alignment;
            // guest memory first: totalSize is guest-controlled, so the host-side image is only built once it fits
            let bytes = s.total_size.max(1).saturating_add(align);
            let raw = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, bytes, true).map_err(|_| CfmError::NoMemory { what: "a PEF section", bytes })?;
            let base = raw.next_multiple_of(align);
            let image = pef.section_image(i)?;
            self.mac.mem.write_bytes(base, &image).map_err(mem_err("placing a section"))?;
            bases[i] = Some(base);
        }
        // 1b. a fragment that is not a supplied library (the APPLICATION) is known by its name from now on, its exports at their placed
        // addresses: a library its imports load may import from it in turn (CodeWarrior's MSL imports `operator new` from "application",
        // Photoshop 7), as the CFM resolves an import of the application's own fragment name. A supplied library keeps its Loading slot (a
        // cycle between two libraries is named).
        if !self.libs.contains_key(name) {
            let early: alloc::collections::BTreeMap<String, u32> = loader.exports.iter().filter_map(|e| match e.target {
                ExportTarget::Section { index, offset } => bases.get(usize::from(index)).copied().flatten().map(|b| (e.name.clone(), b.wrapping_add(offset))),
                ExportTarget::Absolute(v) => Some((e.name.clone(), v)),
                ExportTarget::Reexport { .. } => None,
            }).collect();
            if !early.is_empty() {
                self.mac.log.push(format!("[cfm] \"{name}\": its {} exports are importable by the libraries it loads", early.len()));
                self.libs.insert(name.into(), crate::machine::LibSlot::Loaded(early, None));
            }
        }
        // 2. imports
        let mut resolved = Vec::with_capacity(loader.imports.len());
        for imp in &loader.imports {
            let lib = &loader.libraries[imp.library];
            // the value a function import relocates to: a TVector for TVector class, the code address (the stub) for Code class
            let fn_value = |glue: &crate::glue::Glue, id: usize| glue.entry(id).map_or(0, |e| if imp.class == SymbolClass::Code { e.stub } else { e.tvector });
            let b = self.bind_from(&lib.name, &imp.name);
            if let Binding::Export(addr) = b {
                resolved.push(ResolvedImport { library: lib.name.clone(), symbol: imp.name.clone(), class: imp.class, weak: imp.weak || lib.weak, status: ImportStatus::Bound(b.describe()), value: addr });
                continue;
            }
            let weak = imp.weak || lib.weak;
            // Why this import cannot be served as declared, if it cannot.
            let why = match (&b, imp.class) {
                (Binding::Unbound(w), _) => Some(w.clone()),
                (_, SymbolClass::Toc) => Some("TOC-class imports are not supported".into()),
                (Binding::Qd, c) if c != SymbolClass::Data => Some(format!("{} is imported as {} but the engine provides it as data", imp.name, c.name())),
                (b, SymbolClass::Data) if !b.is_data() => Some(format!("{} is imported as data but the engine provides it as a function", imp.name)),
                _ => None,
            };
            let (status, value) = match why {
                Some(why) if weak => (ImportStatus::WeakAbsent(why), 0),
                Some(why) if self.strict_imports => return Err(CfmError::UnresolvedImport { library: lib.name.clone(), symbol: imp.name.clone(), why }),
                Some(why) if imp.class == SymbolClass::Data || !matches!(b, Binding::Unbound(_)) => {
                    if imp.class == SymbolClass::Data { self.unbound_data.push(format!("{}/{}", lib.name, imp.name)); }
                    (ImportStatus::Unbound(why), UNBOUND_DATA)
                }
                Some(why) => {
                    // a function nobody provides: calling it refuses by name
                    let id = self.glue.intern(&mut self.mac, &lib.name, &imp.name)?;
                    (ImportStatus::Unbound(why), fn_value(&self.glue, id))
                }
                None if b.is_data() => (ImportStatus::Bound(b.describe()), self.data_block(&imp.name, &b)?),
                None => {
                    let id = self.glue.intern(&mut self.mac, &lib.name, &imp.name)?;
                    // a deferrable GL call imported as a TVector calls the queue routine (gl_c/defer.rs)
                    if let (Binding::Native(crate::symbols::Native::Gl(f)), SymbolClass::TVector) = (&b, imp.class) { self.gl_defer_bind(id, *f).map_err(CfmError::Memory)?; }
                    (ImportStatus::Bound(b.describe()), fn_value(&self.glue, id))
                }
            };
            resolved.push(ResolvedImport { library: lib.name.clone(), symbol: imp.name.clone(), class: imp.class, weak, status, value });
        }
        // 3. relocations. First a dry run with a distinct sentinel per import: it tells which guest word each import's value is added into (the TOC slot), which is how a call
        // through an ABSENT weak import (value 0) is traced back to its name when it faults (`MacPpc::null_call_import`).
        let n_imports = resolved.len() as u32;
        let sentinels: Vec<u32> = (0..n_imports).map(|i| 0xF000_0000 + i).collect();
        let mut slots = Vec::new();
        loader.relocate(&bases, &sentinels, &mut |sec, off, addend| {
            if addend >= 0xF000_0000 && addend - 0xF000_0000 < n_imports { if let Some(b) = bases.get(usize::from(sec)).copied().flatten() { slots.push((b.wrapping_add(off), (addend - 0xF000_0000) as usize)); } }
            Ok(())
        })?;
        for (at, i) in slots { self.import_slots.insert(at, (resolved[i].library.clone(), resolved[i].symbol.clone(), resolved[i].weak)); }
        let values: Vec<u32> = resolved.iter().map(|r| r.value).collect();
        let mem = &mut self.mac.mem;
        loader.relocate(&bases, &values, &mut |sec, off, addend| {
            let s = &pef.sections[usize::from(sec)];
            let base = bases[usize::from(sec)].ok_or(PefError::BadReloc("a relocation header names a section that is not instantiated"))?;
            if u64::from(off) + 4 > u64::from(s.total_size) { return Err(PefError::BadReloc("a relocated word lies past the end of its section")); }
            let at = base + off;
            let cur = mem.read_u32(at).map_err(|_| PefError::BadReloc("a relocated word lies outside guest memory"))?;
            mem.write_u32(at, cur.wrapping_add(addend)).map_err(|_| PefError::BadReloc("a relocated word lies outside guest memory"))
        })?;
        // 4. entry points and exports
        let tv = |e: Option<heirloom_fmt_pef::EntryPoint>| -> Result<Option<Tv>, CfmError> {
            let Some(e) = e else { return Ok(None) };
            let base = bases.get(e.section as usize).copied().flatten().ok_or(CfmError::Pef(PefError::BadLoader("an entry point names a section that is not instantiated")))?;
            let address = base.checked_add(e.offset).ok_or(CfmError::Pef(PefError::BadLoader("an entry offset overflows")))?;
            Ok(Some(Tv { address, code: self.mac.mem.read_u32(address).map_err(mem_err("reading an entry TVector"))?, toc: self.mac.mem.read_u32(address + 4).map_err(mem_err("reading an entry TVector"))? }))
        };
        let (main, init, term) = (tv(loader.main)?, tv(loader.init)?, tv(loader.term)?);
        let mut exports = Vec::new();
        for e in &loader.exports {
            let v = match e.target {
                ExportTarget::Section { index, offset } => bases.get(usize::from(index)).copied().flatten().map(|b| b.wrapping_add(offset)),
                ExportTarget::Absolute(v) => Some(v),
                ExportTarget::Reexport { import } => values.get(import as usize).copied(),
            };
            if let Some(v) = v { exports.push((e.name.clone(), v)); }
        }
        // 5. the guest-PC profile's map (profile.rs): the sections as placed, the function starts found in the code
        let secs: Vec<(usize, bool, u32, u32)> = pef.sections.iter().enumerate()
            .filter_map(|(i, s)| bases[i].map(|b| (i, matches!(s.kind, heirloom_fmt_pef::SectionKind::Code | heirloom_fmt_pef::SectionKind::ExecutableData), b, s.total_size)))
            .collect();
        let entries: Vec<u32> = [main, init, term].iter().flatten().map(|t| t.code).chain(exports.iter().map(|e| e.1)).collect();
        // what each code section IS (window-definition bindings are keyed by it, wdef_c.rs): its length and the digest of the bytes the
        // container shipped (before relocation, so the same build has the same digest wherever it is placed)
        for &(i, is_code, base, size) in &secs {
            if is_code { self.wdef.code.push(crate::wdef_c::CodeSection { fragment: name.into(), base, len: size, fnv1a64: heirloom_bind::wdef::fnv1a64(&pef.section_image(i)?) }); }
        }
        let mem = &self.mac.mem;
        self.code_map.add(name, &secs, &|a| mem.read_u32(a).ok(), &entries);
        Ok(Fragment { name: name.into(), section_bases: bases, imports: resolved, main, init, term, exports })
    }

    /// Registers for starting `frag`'s main: a fresh stack, r2 = the main TVector's TOC, LR = the ExitToShell stub.
    pub fn prepare_main(&mut self, frag: &Fragment) -> Result<StartState, CfmError> {
        let main = frag.main.ok_or(CfmError::NoMain)?;
        let stack = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, STACK_SIZE, true).map_err(|_| CfmError::NoMemory { what: "the PowerPC stack", bytes: STACK_SIZE })?;
        // PowerPC ABI: r1 is 16-byte aligned and points at a back-chain word (0 ends the chain).
        let r1 = (stack + STACK_SIZE - 64) & !0xF;
        self.mac.mem.write_u32(r1, 0).map_err(mem_err("the stack back chain"))?;
        self.bindings.entry("ExitToShell".into()).or_insert(Binding::Native(crate::symbols::Native::Exit));
        // the application's A5 world (the Process Manager gives every process one; a PowerPC application's is minimal): InitGraf stores the
        // QuickDraw globals' address at 0(A5), and 68k code it calls through Mixed Mode runs with it
        if self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::CURRENT_A5).unwrap_or(0) == 0 {
            let a5 = self.mac.new_a5_world().map_err(|_| CfmError::NoMemory { what: "the A5 world", bytes: 64 })?;
            self.mac.mem.write_u32(heirloom_mac68k_engine::lowmem::CURRENT_A5, a5).map_err(mem_err("CurrentA5"))?;
        }
        let id = self.glue.intern(&mut self.mac, "InterfaceLib", "ExitToShell")?;
        let lr = self.glue.entry(id).map_or(0, |e| e.stub);
        Ok(StartState { pc: main.code, r1, r2: main.toc, lr })
    }

    /// The `CFragInitBlock` the application's init routine is entered with (CodeFragments.h, the System 7 layout): contextID 0,
    /// closureID 4, connectionID 8, fragLocator 12 (`where` 12; the data-fork locator's fileSpec 16, offset 20, length 24), libName 28,
    /// reservedA 32; the FSSpec (70 bytes) and the name follow it in the same block. The locator is kDataForkCFragLocator (1): the
    /// application's own file ([`heirloom_mac68k_engine::fm::fsspec::write_app_spec`]) and the fragment's span in its data fork
    /// ([`MacPpc::app_span`], set by the launch from the `cfrg`). 0 when no application file was installed (a fragment started
    /// from memory by a host): no locator would be true then, and the routine gets NULL.
    /// The `CFragInitBlock` of a library loaded while the program runs: the locator its loader recorded ([`MacPpc::lib_locator`]: the data
    /// fork of a file, or memory), else kMemoryCFragLocator with no address (a library supplied as bytes); libName is the library's name.
    pub(crate) fn lib_init_block(&mut self, lib: &str) -> Result<u32, CfmError> {
        const BLOCK: u32 = 36;
        const FSSPEC: u32 = 70;
        let name: Vec<u8> = lib.chars().take(63).map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
        let size = BLOCK + FSSPEC + 1 + name.len() as u32;
        let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, size, true).map_err(|_| CfmError::NoMemory { what: "a CFragInitBlock", bytes: size })?;
        let (spec, lib_name) = (p + BLOCK, p + BLOCK + FSSPEC);
        let (where_, a, b, c) = match self.lib_locator.remove(lib) {
            Some(LibLocator::Disk { spec: src, offset, length }) => {
                let bytes = self.mac.mem.span(src, FSSPEC).and_then(|s| self.mac.mem.bytes(s).map(<[u8]>::to_vec)).map_err(mem_err("the library's FSSpec"))?;
                self.mac.mem.write_bytes(spec, &bytes).map_err(mem_err("the CFragInitBlock's FSSpec"))?;
                (1, spec, offset, length)
            }
            Some(LibLocator::Memory { address, length }) => (0, address, length, 0),
            None => (0, 0, 0, 0),
        };
        let m = &mut self.mac.mem;
        for (at, v) in [(0, 1), (4, 1), (8, 1), (12, where_), (16, a), (20, b), (24, c), (28, lib_name)] {
            m.write_u32(p + at, v).map_err(mem_err("the CFragInitBlock"))?;
        }
        m.write_u8(lib_name, name.len() as u8).and_then(|_| m.write_bytes(lib_name + 1, &name)).map_err(mem_err("the CFragInitBlock's name"))?;
        Ok(p)
    }

    pub fn app_init_block(&mut self) -> Result<u32, CfmError> {
        const BLOCK: u32 = 36;
        const FSSPEC: u32 = 70;
        let Some((offset, length)) = self.app_span else { return Ok(0) };
        if self.mac.fm.app.is_none() { return Ok(0) }
        let name = self.mac.fm.app.map(|n| String::from(self.mac.fm.tree.name(n))).unwrap_or_default();
        let name: Vec<u8> = name.chars().take(63).map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
        let size = BLOCK + FSSPEC + 1 + name.len() as u32;
        let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, size, true).map_err(|_| CfmError::NoMemory { what: "the CFragInitBlock", bytes: size })?;
        let (spec, lib_name) = (p + BLOCK, p + BLOCK + FSSPEC);
        if heirloom_mac68k_engine::fm::fsspec::write_app_spec(&mut self.mac, spec) != Ok(true) { return Ok(0) }
        let m = &mut self.mac.mem;
        for (at, v) in [(0, 1), (4, 1), (8, 1), (12, 1), (16, spec), (20, offset), (24, length), (28, lib_name)] {
            m.write_u32(p + at, v).map_err(mem_err("the CFragInitBlock"))?;
        }
        m.write_u8(lib_name, name.len() as u8).and_then(|_| m.write_bytes(lib_name + 1, &name)).map_err(mem_err("the CFragInitBlock's name"))?;
        Ok(p)
    }
}
