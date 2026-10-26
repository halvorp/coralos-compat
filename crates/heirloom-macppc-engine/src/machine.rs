//! The machine: a [`Mac`] (guest RAM, heap, Memory/Resource/Window managers, the GuestUi) plus what the PowerPC side
//! adds: the glue table, the symbol bindings, CFM connections and the `qd` globals block.
use crate::error::CfmError;
use crate::glue::Glue;
use crate::symbols::{lookup, Binding};
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::Mac;

/// sizeof(QDGlobals) under mac68k alignment: privates[76] randSeed(4) screenBits(14) arrow(68) 5 patterns(40) thePort(4).
pub const QD_GLOBALS_SIZE: u32 = 206;
/// Where a strong data import with no provider points: outside guest RAM, so any access is a bus fault the CPU reports.
pub const UNBOUND_DATA: u32 = 0xBAD0_0000;

/// Finder's default partition for an application with no SIZE resource.
/// Documented Finder default; no citation invented.
pub const NO_SIZE_PARTITION: u32 = 4 << 20; // 4 MiB

/// Read the program's SIZE resource (ID -1 preferred, else 0) and return
/// max(preferredSize, minimumSize). None when no SIZE is present.
pub fn partition_bytes(program: &[u8]) -> Option<u32> {
    let opened = heirloom_fmt_resfork::open(program).ok()?;
    let rsrc = &program[opened.rsrc];
    // ResFork owns its bytes, so loading a program copies its resource fork once.
    let fork = heirloom_fmt_resfork::ResFork::parse(rsrc.to_vec()).ok()?;
    let idx = fork.find(*b"SIZE", -1).or_else(|| fork.find(*b"SIZE", 0))?;
    let data = fork.data(idx)?;
    if data.len() < 10 {
        return None;
    }
    // Inside Macintosh: Processes — u16 flags, u32 preferredSize, u32 minimumSize (BE)
    let preferred = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
    let minimum = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);
    Some(preferred.max(minimum))
}

/// Total guest RAM for a program: base + SIZE partition + 8 MiB system
/// heap/glue/stack, rounded up to 1 MiB.
pub fn ram_for_program(base: usize, program: &[u8]) -> usize {
    const SYSTEM_OVERHEAD: usize = 8 << 20; // 8 MiB
    const MIB: usize = 1 << 20;
    let partition = partition_bytes(program).unwrap_or(NO_SIZE_PARTITION) as usize;
    let raw = base
        .saturating_add(partition)
        .saturating_add(SYSTEM_OVERHEAD);
    // Guard the round-up so a saturated value cannot wrap.
    raw.checked_add(MIB - 1)
        .map(|x| x & !(MIB - 1))
        .unwrap_or(usize::MAX)
}

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
    /// Guest OpenGL: AGL/GL calls encoded for `heirloom-gl.engine` ([`crate::gl_c`]).
    pub gl: crate::gl_c::GlState,
    pub(crate) qa: crate::qdacc_c::QaState,
    pub(crate) isp: crate::isp_c::IspState,
    /// Everything the program wrote with `printf`/`puts`/`putchar`: there is no console device, so the bytes are kept here (and logged).
    pub console: Vec<u8>,
    /// PEF libraries supplied by name (the application's own `cfrg` import-library members, files beside it): loaded on first import.
    pub libs: BTreeMap<String, LibSlot>,
    /// Init routines of the libraries loaded so far, dependencies first: `start` runs them before the application's own.
    pub lib_inits: Vec<crate::cfm::Tv>,
    /// Guest address of every TOC/data word an import's value was relocated into -> (library, symbol, weak): names a call through a null weak import.
    pub import_slots: BTreeMap<u32, (String, String, bool)>,
}

impl<U: GuestUi> MacPpc<U> {
    /// Private helper: own the fallible growth path. Takes an already-
    /// constructed `Mac` so callers that need the base (new, for_program)
    /// can construct `Mac::new(ui)` exactly once.
    fn from_mac(mut mac: Mac<U>, ram: usize) -> Result<Self, CfmError> {
        let base = mac.mem.len();
        if ram > base {
            mac.mem.try_grow_to(ram).map_err(|_| CfmError::NoMemory {
                what: "guest RAM",
                bytes: ram.min(u32::MAX as usize) as u32,
            })?;
            mac.heap.add_region(base as u32, (ram - base) as u32);
            let _ = mac.mem.write_u32(
                heirloom_mac68k_engine::lowmem::MEM_TOP,
                ram.min(u32::MAX as usize) as u32,
            ); // MemTop: the top of the (larger) RAM
        }
        let caps = heirloom_gl_abi::caps::derive(
            heirloom_gl_abi::ENTRIES,
            heirloom_gl_abi::caps::ATTESTED_THROUGH,
        );
        match crate::gl_extension::install_opengl_engine(&mut mac.fm, &caps) {
            Ok(true) => {
                let (maj, min) = caps.version.map_or((0, 0), |v| v.pair()); // Ok(true) only when a version is claimed
                mac.log.push(alloc::format!("[gl] OpenGLEngine in Extensions: vers {}.{}", maj, min));
            }
            Ok(false) => {
                mac.log.push(alloc::format!("[gl] no OpenGLEngine in Extensions: no GL version is attested"));
            }
            Err(e) => {
                mac.log.push(alloc::format!("[gl] OpenGLEngine install error: {}", e));
            }
        }
        // PowerPC Mac OS is always 32-bit addressed: the engine's 24-bit default is the 68k personality's.
        mac.set_address_mode(heirloom_mac68k_engine::addrmode::AddrMode::Bit32).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        Ok(MacPpc { mac, glue: Glue::default(), bindings: BTreeMap::new(), strict_imports: false, connections: Vec::new(), qd: None, rt: Default::default(), unbound_data: Vec::new(), cdata: BTreeMap::new(), cx: Default::default(), tsm: Default::default(), rt_mn: Default::default(), sc: Default::default(), sys: Default::default(), dsp: Default::default(), tm: Default::default(), gl: Default::default(), qa: Default::default(), isp: Default::default(), console: Vec::new(), libs: BTreeMap::new(), lib_inits: Vec::new(), import_slots: BTreeMap::new() })
    }

    /// A machine with `ram` bytes of guest RAM (at least the 68k engine's 16 MB): the extra memory joins the Memory Manager's heap.
    pub fn with_ram(ui: U, ram: usize) -> Result<Self, CfmError> {
        let mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        Self::from_mac(mac, ram)
    }

    pub fn new(ui: U) -> Result<Self, CfmError> {
        // Construct once to read the base, then build with the computed ram.
        let mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        let base = mac.mem.len();
        let ram = ram_for_program(base, &[]);
        Self::from_mac(mac, ram)
    }

    pub fn for_program(ui: U, program: &[u8]) -> Result<Self, CfmError> {
        // Construct once to read the base, then build with the computed ram.
        let mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        let base = mac.mem.len();
        let ram = ram_for_program(base, program);
        Self::from_mac(mac, ram)
    }

    /// Make a PEF library available under `name` (the fragment name an import table uses). Instantiated, with its own imports and init routine, the first time something imports from it.
    pub fn provide_library(&mut self, name: &str, pef: &[u8]) { self.libs.insert(name.into(), LibSlot::Available(pef.to_vec())); }

    fn export_binding(&mut self, lib: &str, name: &str) -> Binding {
        if let Some(LibSlot::Available(_)) = self.libs.get(lib) {
            let Some(LibSlot::Available(bytes)) = self.libs.insert(lib.into(), LibSlot::Loading) else { unreachable!() };
            let loaded = heirloom_fmt_pef::Pef::parse(&bytes).map_err(|e| alloc::format!("{e}")).and_then(|pef| self.instantiate(lib, &pef).map_err(|e| alloc::format!("{e}")));
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
        let opened = heirloom_fmt_resfork::open(file).map_err(|e| e.to_string())?;
        let fork = heirloom_fmt_resfork::ResFork::parse(file[opened.rsrc.clone()].to_vec()).map_err(|e| e.to_string())?;
        let n = fork.len();
        self.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, fork);
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
            crate::isp_c::NAME => match crate::isp_c::IspFn::by_name(name) {
                Some(f) => { let b = Binding::Native(crate::symbols::Native::Isp(f)); self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the part of InputSprocketLib this engine provides"));
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
            l if crate::gl_c::provided(l) => match crate::gl_c::GlFn::by_name(name) {
                Some(f) => { let b = Binding::Native(crate::symbols::Native::Gl(f)); self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => {
                    let b = Binding::Unbound(alloc::format!("{name} is not in the contract table of {l} (heirloom-gl-abi)"));
                    self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
                    b
                }
            },
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
                self.bindings.insert(crate::glue::binding_key(lib, name), b.clone());
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
        if heirloom_mac68k_engine::quickdraw::fill_globals(&mut self.mac, p).is_err() { return; }
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