//! The machine: a [`Mac`] (guest RAM, heap, Memory/Resource/Window managers, the GuestUi) plus what the PowerPC side
//! adds: the glue table, the symbol bindings, CFM connections and the `qd` globals block.
use crate::error::CfmError;
use crate::glue::Glue;
use crate::symbols::{lookup, Binding};
use alloc::collections::BTreeMap;
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
}

impl<U: GuestUi> MacPpc<U> {
    pub fn new(ui: U) -> Result<Self, CfmError> {
        let mac = Mac::new(ui).map_err(|e| CfmError::Memory(alloc::format!("{}", e.0)))?;
        Ok(MacPpc { mac, glue: Glue::default(), bindings: BTreeMap::new(), strict_imports: false, connections: Vec::new(), qd: None, rt: Default::default() })
    }

    /// The binding for `name`, computed once against the handlers `mac` has registered.
    pub fn bind(&mut self, name: &str) -> Binding {
        if let Some(b) = self.bindings.get(name) { return b.clone(); }
        let b = lookup(&self.mac, name);
        self.bindings.insert(name.into(), b.clone());
        b
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
