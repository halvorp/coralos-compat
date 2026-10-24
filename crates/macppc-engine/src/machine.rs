//! The machine: a [`Mac`] (guest RAM, heap, Memory/Resource/Window managers, the GuestUi) plus what the PowerPC side
//! adds: the glue table, the symbol bindings, CFM connections and the `qd` globals block.
use crate::error::CfmError;
use crate::glue::Glue;
use crate::symbols::{lookup, Binding};
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use mac68k_engine::Mac;

/// sizeof(QDGlobals) under mac68k alignment: privates[76] randSeed(4) screenBits(14) arrow(68) 5 patterns(40) thePort(4).
pub const QD_GLOBALS_SIZE: u32 = 206;
/// Where a strong data import with no provider points: outside guest RAM, so any access is a bus fault the CPU reports.
pub const UNBOUND_DATA: u32 = 0xBAD0_0000;

/// A shared library the program can import that is a PEF of its own (not one of the engine's native libraries).
pub enum LibSlot {
    /// Supplied, not yet instantiated (the PEF bytes).
    Available(Vec<u8>),
    /// Being instantiated now (an import cycle would find it here).
    Loading,
    /// Instantiated: exported name -> address.
    Loaded(BTreeMap<String, u32>),
    /// Could not be instantiated; every import from it names why.
    Failed(String),
}

pub struct MacPpc<U: GuestUi> {
    pub mac: Mac<U>,
    pub glue: Glue,
    /// symbol name -> how it is served (filled when an import or `FindSymbol` first names it).
    pub bindings: BTreeMap<String, Binding>,
    /// Strict mode: instantiating a fragment with an unresolved strong import fails instead of binding a refusing stub.
    pub strict_imports: bool,
    pub(crate) connections: Vec<String>,
    pub(crate) qd: Option<u32>,
    pub(crate) rt: crate::run::Runtime,
    /// `library/symbol` of every strong DATA import nobody provides: they all point at [`UNBOUND_DATA`], so a fault there
    /// names this list.
    pub unbound_data: Vec<String>,
    pub(crate) cdata: BTreeMap<String, u32>,
    /// Control / Window Manager bookkeeping with no home in the 68k records ([`crate::carbon_ctl`]).
    pub(crate) cx: crate::carbon_ctl::CxState,
    /// Text Services Manager documents ([`crate::tsm_c`]).
    pub(crate) tsm: crate::tsm_c::TsmState,
    pub(crate) rt_mn: crate::menu_c::MenuState,
    pub(crate) sc: crate::script_c::ScriptState,
    pub(crate) sys: crate::sysmisc_c::SysState,
    pub(crate) dsp: crate::dsp_c::DspState,
    pub(crate) tm: crate::tm_c::TmState,
    /// Everything the program wrote with `printf`/`puts`/`putchar`: there is no console device, so the bytes are kept here (and logged).
    pub console: Vec<u8>,
    /// PEF libraries supplied by name (the application's own `cfrg` import-library members, files beside it): loaded on first import.
    pub libs: BTreeMap<String, LibSlot>,
    /// Init routines of the libraries loaded so far, dependencies first: `start` runs them before the application's own.
    pub lib_inits: Vec<crate::cfm::Tv>,
    /// Guest address of every TOC/data word an import's value was relocated into -> (library, symbol, weak): names a call through a null weak import.
    pub import_slots: BTreeMap<u32, (String, String, bool)>,
}

/// Guest RAM of a PowerPC Macintosh: large PowerPC applications are tens of megabytes of code and data before their heap (Warcraft III: a 14 MB
/// application). The address space is zero-filled lazily by the host, so the size costs nothing until it is touched. `with_ram` picks another. 512 MB: the Frozen Throne
/// refuses to start below 320 MB of (virtual) memory, read from Gestalt `ram `.
pub const DEFAULT_PPC_RAM: usize = 512 << 20;

impl<U: GuestUi> MacPpc<U> {
    pub fn new(ui: U) -> Result<Self, CfmError> { Self::with_ram(ui, DEFAULT_PPC_RAM) }

    /// A machine with `ram` bytes of guest RAM (at least the 68k engine's 16 MB): the extra memory joins the Memory Manager's heap.
    pub fn with_ram(ui: U, ram: usize) -> Result<Self, CfmError> {
        let mut mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        let base = mac.mem.len();
        if ram > base {
            mac.mem.grow_to(ram);
            mac.heap.add_region(base as u32, (ram - base) as u32);
            let _ = mac.mem.write_u32(mac68k_engine::lowmem::MEM_TOP, ram as u32); // MemTop: the top of the (larger) RAM
        }
        // PowerPC Mac OS is always 32-bit addressed: the engine's 24-bit default is the 68k personality's.
        mac.set_address_mode(mac68k_engine::addrmode::AddrMode::Bit32).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        Ok(MacPpc { mac, glue: Glue::default(), bindings: BTreeMap::new(), strict_imports: false, connections: Vec::new(), qd: None, rt: Default::default(), unbound_data: Vec::new(), cdata: BTreeMap::new(), cx: Default::default(), tsm: Default::default(), rt_mn: Default::default(), sc: Default::default(), sys: Default::default(), dsp: Default::default(), tm: Default::default(), console: Vec::new(), libs: BTreeMap::new(), lib_inits: Vec::new(), import_slots: BTreeMap::new() })
    }

    /// Make a PEF library available under `name` (the fragment name an import table uses). Instantiated, with its own imports and init routine, the first time something imports from it.
    pub fn provide_library(&mut self, name: &str, pef: &[u8]) { self.libs.insert(name.into(), LibSlot::Available(pef.to_vec())); }

    fn export_binding(&mut self, lib: &str, name: &str) -> Binding {
        if let Some(LibSlot::Available(_)) = self.libs.get(lib) {
            let Some(LibSlot::Available(bytes)) = self.libs.insert(lib.into(), LibSlot::Loading) else { unreachable!() };
            let loaded = retro_pef::Pef::parse(&bytes).map_err(|e| alloc::format!("{e}")).and_then(|pef| self.instantiate(lib, &pef).map_err(|e| alloc::format!("{e}")));
            match loaded {
                Ok(frag) => {
                    if let Some(init) = frag.init { self.lib_inits.push(init); }
                    self.libs.insert(lib.into(), LibSlot::Loaded(frag.exports.iter().cloned().collect()));
                }
                Err(why) => { self.libs.insert(lib.into(), LibSlot::Failed(why)); }
            }
        }
        match self.libs.get(lib) {
            Some(LibSlot::Loaded(ex)) => ex.get(name).map_or_else(|| Binding::Unbound(alloc::format!("{name} is not exported by the PEF library {lib}")), |&a| Binding::Export(a)),
            Some(LibSlot::Loading) => Binding::Unbound(alloc::format!("{lib} imports {name} from itself while it is being loaded (an import cycle)")),
            Some(LibSlot::Failed(why)) => Binding::Unbound(alloc::format!("the PEF library {lib} could not be loaded: {why}")),
            _ => Binding::Unbound(alloc::format!("library {lib} is not provided")),
        }
    }

    /// The binding for `name`, computed once against the handlers `mac` has registered.
    pub fn bind(&mut self, name: &str) -> Binding {
        if let Some(b) = self.bindings.get(name) { return b.clone(); }
        let b = lookup(&self.mac, name);
        self.bindings.insert(name.into(), b.clone());
        b
    }

    /// Open the application's resource fork (from a MacBinary / AppleSingle / AppleDouble file, or a raw fork) as the
    /// current resource file, so `GetResource`, menus, windows and dialogs find the application's resources. A file with
    /// no resource fork is an error the caller may ignore (a bare PEF has none).
    pub fn attach_resources(&mut self, file: &[u8]) -> Result<usize, String> {
        let opened = retro_resfork::open(file).map_err(|e| e.to_string())?;
        let fork = retro_resfork::ResFork::parse(file[opened.rsrc.clone()].to_vec()).map_err(|e| e.to_string())?;
        let n = fork.len();
        self.mac.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, fork);
        Ok(n)
    }

    /// The binding for `name` as library `lib` exports it: StdCLib only what [`crate::stdclib`] provides, the Toolbox libraries (`glue::TOOLBOX_LIBRARIES`: InterfaceLib, CarbonLib, ...) the
    /// Toolbox, anything else unbound.
    pub fn bind_from(&mut self, lib: &str, name: &str) -> Binding {
        if lib == "CarbonLib" { self.carbon_implicit_graf(); }
        match lib {
            crate::stdclib::NAME => match crate::stdclib::lookup(name) {
                Some(b) => { self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of StdCLib this engine provides"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
            crate::mp::NAME => match crate::stdclib::mp_lookup(name) {
                Some(b) => { self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of MPLibrary this engine provides"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
            crate::threads::NAME => match crate::stdclib::threads_lookup(name) {
                Some(b) => { self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of ThreadsLib this engine provides"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
            crate::dsp_c::NAME => match crate::dsp_c::DspFn::by_name(name) {
                Some(f) => { let b = Binding::Native(crate::symbols::Native::Dsp(f)); self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of DrawSprocketLib this engine provides"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
            crate::stdclib::MATH_NAME => match crate::stdclib::math_lookup(name) {
                Some(b) => { self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of MathLib this engine provides"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
            // CarbonLib exports the Toolbox AND the Carbon-only calls (CoreFoundation strings, the Carbon Event Manager, CreateNewWindow ...)
            "CarbonLib" if crate::stdclib::carbon_lookup(name).is_some() => {
                let b = crate::stdclib::carbon_lookup(name).expect("checked");
                self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                b
            }
            l if crate::natives::provided(l) => self.bind(name),
            l if self.libs.contains_key(l) => self.export_binding(l, name),
            // Universal Procedure Pointer constructors are library glue everywhere (CarbonLib exports one per callback type, including the networking and multimedia ones
            // whose OTHER calls this engine lacks): building a routine descriptor needs nothing from the absent library, and an unguarded call must not jump to 0.
            l if (name.starts_with("New") && name.ends_with("UPP") && name.len() > 6) || (name.starts_with("Dispose") && name.ends_with("UPP") && name.len() > 9) => {
                let b = Binding::Native(if name.starts_with("New") { crate::symbols::Native::NewUpp } else { crate::symbols::Native::DisposeUpp });
                self.bindings.insert(crate::glue::binding_key(l, name), b.clone());
                b
            }
            l => {
                // Recorded under the glue key like every other binding: a CALL of the symbol must refuse by NAME (`NEXT-NEEDED lib/sym: ...`),
                // not stop on "no binding was recorded for this glue stub".
                let b = Binding::Unbound(alloc::format!("library {l} is not provided by the engine"));
                self.bindings.insert(crate::glue::binding_key(l, name), b.clone());
                b
            }
        }
    }

    /// The guest address a DATA binding resolves to: the `qd` block, or a zeroed block per C runtime variable (allocated on
    /// first use, so every import and every `FindSymbol` of it agree).
    pub fn data_block(&mut self, name: &str, b: &Binding) -> Result<u32, CfmError> {
        match b {
            Binding::Qd => self.qd_block(),
            Binding::CData(n) => {
                if let Some(&p) = self.cdata.get(name) { return Ok(p); }
                let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, *n, true).map_err(|_| CfmError::NoMemory { what: "a C runtime variable", bytes: *n })?;
                self.cdata.insert(name.into(), p);
                Ok(p)
            }
            _ => Err(CfmError::Memory(alloc::format!("{name} is not a data symbol"))),
        }
    }

    /// A Carbon program never calls `InitGraf` (Carbon initialises QuickDraw for the process): the first CarbonLib import makes the `qd` block and runs what `InitGraf(&qd.thePort)` would,
    /// so `SetPort`, `NewGWorld` and the drawing calls have a `thePort` slot. A failure here (no memory) leaves QuickDraw uninitialised and the first QuickDraw call refuses by name.
    fn carbon_implicit_graf(&mut self) {
        if self.mac.qd_globals != 0 { return; }
        let Ok(q) = self.qd_block() else { return };
        let p = q + QD_GLOBALS_SIZE - 4;
        if mac68k_engine::quickdraw::fill_globals(&mut self.mac, p).is_err() { return; }
        self.mac.qd_globals = p;
        let _ = self.mac.init_devices();
    }

    /// The QuickDraw globals block `qd` imports resolve to (allocated on first use, zeroed).
    pub fn qd_block(&mut self) -> Result<u32, CfmError> {
        if let Some(q) = self.qd { return Ok(q); }
        let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, QD_GLOBALS_SIZE, true)
            .map_err(|_| CfmError::NoMemory { what: "the QuickDraw globals (qd)", bytes: QD_GLOBALS_SIZE })?;
        self.qd = Some(p);
        Ok(p)
    }
}
