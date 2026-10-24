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
}

impl<U: GuestUi> MacPpc<U> {
    pub fn new(ui: U) -> Result<Self, CfmError> {
        let mut mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        // PowerPC Mac OS is always 32-bit addressed: the engine's 24-bit default is the 68k personality's.
        mac.set_address_mode(mac68k_engine::addrmode::AddrMode::Bit32).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        Ok(MacPpc { mac, glue: Glue::default(), bindings: BTreeMap::new(), strict_imports: false, connections: Vec::new(), qd: None, rt: Default::default(), unbound_data: Vec::new(), cdata: BTreeMap::new() })
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
        match lib {
            crate::stdclib::NAME => match crate::stdclib::lookup(name) {
                Some(b) => { self.bindings.insert(crate::glue::binding_key(lib, name), b.clone()); b }
                None => Binding::Unbound(alloc::format!("{name} is not in the part of StdCLib this engine provides")),
            },
            l if crate::natives::provided(l) => self.bind(name),
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

    /// The QuickDraw globals block `qd` imports resolve to (allocated on first use, zeroed).
    pub fn qd_block(&mut self) -> Result<u32, CfmError> {
        if let Some(q) = self.qd { return Ok(q); }
        let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, QD_GLOBALS_SIZE, true)
            .map_err(|_| CfmError::NoMemory { what: "the QuickDraw globals (qd)", bytes: QD_GLOBALS_SIZE })?;
        self.qd = Some(p);
        Ok(p)
    }
}
