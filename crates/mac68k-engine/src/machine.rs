//! The Mac machine: guest RAM + the trap table + a [`GuestUi`].
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::pascal::{self, Arg, Args};
use crate::traps::{describe, trap_key, trap_name};
use crate::{toolbox, Refusal};
use guestui_abi::{GuestUi, UiObjectRef};
use retro_guest_mem::heap::Heap;
use retro_guest_mem::{GuestMem, DEFAULT_SIZE};
use retro_m68k::{Cpu, Host};

/// Where the exit stub lives: one `_ExitToShell` word, the return address of the program's own RTS.
pub const EXIT_STUB: u32 = 0x500;
/// Where a flat code image is loaded (no resource fork / CODE loader yet).
pub const CODE_BASE: u32 = 0x1_0000;
pub const STACK_TOP: u32 = 0x20_0000;
/// The guest heap (the Memory Manager's application heap, in effect): everything above the stack.
pub const HEAP_START: u32 = STACK_TOP;
pub const HEAP_END: u32 = 0x100_0000;

/// What a trap handler is told: which trap, and its Pascal arguments in declaration order.
pub struct Call { pub opcode: u16, pub name: &'static str, pub args: Args }

/// A handler's success: the function result (ignored for procedures) and its log line.
pub struct Done { pub result: u32, pub log: String }

pub type Handler<U> = fn(&mut Mac<U>, &mut Cpu, &Call) -> Result<Done, Refusal>;

/// One implemented trap: its Pascal signature and handler.
pub struct TrapEntry<U: GuestUi> { pub name: &'static str, pub args: &'static [Arg], pub result: Option<Arg>, pub handler: Handler<U> }
impl<U: GuestUi> Clone for TrapEntry<U> { fn clone(&self) -> Self { *self } }
impl<U: GuestUi> Copy for TrapEntry<U> {}

/// A window the guest owns: its native window, and whether the engine allocated the WindowRecord (and so frees it).
pub struct MacWindow { pub native: UiObjectRef, pub owned: bool }

pub struct Mac<U: GuestUi> {
    pub mem: GuestMem,
    pub ui: U,
    /// Open guest windows: WindowPtr -> window.
    pub windows: BTreeMap<u32, MacWindow>,
    table: BTreeMap<u16, TrapEntry<U>>,
    /// One line per successful bridged call, in order.
    pub log: Vec<String>,
    /// Refusals (bad guest pointers, unimplemented traps ...), in order.
    pub refused: Vec<String>,
    /// Census mode: the first call to an unimplemented trap halts the guest (the run's last refusal names it).
    pub stop_on_unimplemented: bool,
    pub heap: Heap,
    /// Events delivered by the host and not yet taken by the guest (GetNextEvent / WaitNextEvent are the next step).
    pub pending: Vec<(u16, u32)>,
    pub(crate) ctx: Option<retro_m68k::CpuContext>,
}

/// The prefix every unimplemented-trap refusal starts with (a census runner keys on it).
pub const UNIMPLEMENTED: &str = "unimplemented trap";

impl<U: GuestUi> Mac<U> {
    pub fn new(ui: U) -> Result<Self, Refusal> {
        let mut m = Mac {
            mem: GuestMem::new(DEFAULT_SIZE), ui, windows: BTreeMap::new(), table: BTreeMap::new(), log: vec![], refused: vec![],
            stop_on_unimplemented: false, heap: Heap::new(HEAP_START, HEAP_END), pending: vec![], ctx: None,
        };
        toolbox::register(&mut m)?;
        m.mem.write_u16(EXIT_STUB, 0xA9F4).map_err(Refusal::mem("exit stub"))?; // _ExitToShell
        Ok(m)
    }

    /// Implement `opcode` (a Toolbox trap) with this Pascal signature. The name comes from the generated table; a
    /// trap that table does not know cannot be registered.
    pub fn register(&mut self, opcode: u16, args: &'static [Arg], result: Option<Arg>, handler: Handler<U>) -> Result<(), Refusal> {
        let name = trap_name(opcode).ok_or_else(|| Refusal(format!("register: {} has no name", describe(opcode))))?;
        self.table.insert(trap_key(opcode), TrapEntry { name, args, result, handler });
        Ok(())
    }

    /// Names of every implemented trap, in trap order.
    pub fn implemented(&self) -> Vec<&'static str> { self.table.values().map(|e| e.name).collect() }

    fn refuse(&mut self, name: &str, why: String) {
        self.log.push(format!("[toolbox] {name} REFUSED: {why}"));
        self.refused.push(format!("{name}: {why}"));
    }
}

impl<U: GuestUi> Host for Mac<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }

    /// The Mac engine has no `$4AFC` bridge slots: ILLEGAL stays the 68k exception.
    fn illegal(&mut self, _cpu: &mut Cpu, _pc: u32) -> bool { false }

    fn aline(&mut self, cpu: &mut Cpu, opcode: u16, _pc: u32) -> bool {
        let Some(entry) = self.table.get(&trap_key(opcode)).copied() else {
            // Unknown frame layout: the stack is left as the guest made it; D0 = 0 (noErr / NIL) and execution goes on.
            if self.stop_on_unimplemented { cpu.halt(); }
            let why = format!("{UNIMPLEMENTED} {}", describe(opcode));
            self.log.push(format!("[toolbox] REFUSED: {why}"));
            self.refused.push(why);
            cpu.set_d0(0);
            return true;
        };
        let sp = cpu.areg(7);
        let outcome = pascal::read_args(&self.mem, sp, entry.args).and_then(|args| {
            (entry.handler)(self, cpu, &Call { opcode, name: entry.name, args })
        });
        let result = match outcome {
            Ok(d) => { self.log.push(d.log); d.result }
            Err(Refusal(e)) => { self.refuse(entry.name, e); 0 }
        };
        // The convention is applied once, here: pop the arguments whatever the handler did, store the result.
        match pascal::complete(&mut self.mem, sp, entry.args, entry.result.map(|k| (k, result))) {
            Ok(new_sp) => cpu.set_areg(7, new_sp),
            Err(Refusal(e)) => { self.refuse(entry.name, e); cpu.halt(); }
        }
        true
    }
}
