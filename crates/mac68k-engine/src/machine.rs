//! The Mac machine: guest RAM + the trap table + a [`GuestUi`].
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::events::EventMgr;
use crate::memmgr::MemMgr;
use crate::menu::MenuMgr;
use crate::osconv::{set_flags, PARAM_ERR};
pub use crate::osconv::Regs;
use crate::pascal::{self, Arg, Args, Rect};
use crate::resmgr::ResMgr;
use crate::segment::SegLoader;
use crate::traps::{describe, trap_key, trap_name};
use crate::{events, lowmem, memtraps, menu, quickdraw, resmgr, segment, toolbox, Refusal};
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
/// The heap ends where the (pretend) screen bitmap begins.
pub const HEAP_END: u32 = lowmem::SCREEN_BASE;

/// What a trap handler is told: which trap, and its Pascal arguments in declaration order.
pub struct Call { pub opcode: u16, /** address of the A-line word */ pub pc: u32, pub name: &'static str, pub args: Args }

/// A handler's success: the function result (ignored for procedures) and its log line.
pub struct Done { pub result: u32, pub log: String }

pub type Handler<U> = fn(&mut Mac<U>, &mut Cpu, &Call) -> Result<Done, Refusal>;
/// An OS-trap handler: registers in and out ([`Regs`]); the Ok text is the log line.
pub type OsHandler<U> = fn(&mut Mac<U>, &mut Cpu, &mut Regs) -> Result<String, Refusal>;

/// How a trap passes its arguments: the Pascal stack frame (Toolbox) or registers (Operating System traps).
pub enum Conv<U: GuestUi> {
    Pascal { args: &'static [Arg], result: Option<Arg>, handler: Handler<U> },
    Os(OsHandler<U>),
}
impl<U: GuestUi> Clone for Conv<U> { fn clone(&self) -> Self { *self } }
impl<U: GuestUi> Copy for Conv<U> {}

/// One implemented trap.
pub struct TrapEntry<U: GuestUi> { pub name: &'static str, pub conv: Conv<U> }
impl<U: GuestUi> Clone for TrapEntry<U> { fn clone(&self) -> Self { *self } }
impl<U: GuestUi> Copy for TrapEntry<U> {}

/// A window the guest owns: its native window, and whether the engine allocated the WindowRecord (and so frees it).
pub struct MacWindow {
    /// The native window; `None` until the window is shown (an invisible window opens nothing).
    pub native: Option<UiObjectRef>,
    pub owned: bool,
    /// Content rectangle in global coordinates.
    pub rect: Rect,
    pub title: String,
    pub go_away: bool,
    pub visible: bool,
    pub proc_id: u16,
}

pub struct Mac<U: GuestUi> {
    pub mem: GuestMem,
    pub ui: U,
    /// Open guest windows: WindowPtr -> window.
    pub windows: BTreeMap<u32, MacWindow>,
    /// Window z-order, front first.
    pub z: Vec<u32>,
    pub mm: MemMgr,
    pub rm: ResMgr,
    pub seg: SegLoader,
    pub menus: MenuMgr,
    pub ev: EventMgr,
    /// The address the program gave InitGraf (its `qd.thePort`); 0 before InitGraf.
    pub qd_globals: u32,
    table: BTreeMap<u16, TrapEntry<U>>,
    /// One line per successful bridged call, in order.
    pub log: Vec<String>,
    /// Refusals (bad guest pointers, unimplemented traps ...), in order.
    pub refused: Vec<String>,
    /// Census mode: the first call to an unimplemented trap halts the guest (the run's last refusal names it).
    pub stop_on_unimplemented: bool,
    pub heap: Heap,
    pub(crate) ctx: Option<retro_m68k::CpuContext>,
}

/// The prefix every unimplemented-trap refusal starts with (a census runner keys on it).
pub const UNIMPLEMENTED: &str = "unimplemented trap";

impl<U: GuestUi> Mac<U> {
    pub fn new(ui: U) -> Result<Self, Refusal> {
        let mut m = Mac {
            mem: GuestMem::new(DEFAULT_SIZE), ui, windows: BTreeMap::new(), z: vec![], mm: MemMgr::default(), rm: ResMgr::default(),
            seg: SegLoader::default(), menus: MenuMgr::default(), ev: EventMgr::default(), qd_globals: 0,
            table: BTreeMap::new(), log: vec![], refused: vec![],
            stop_on_unimplemented: false, heap: Heap::new(HEAP_START, HEAP_END), ctx: None,
        };
        lowmem::init(&mut m.mem)?;
        toolbox::register(&mut m)?;
        quickdraw::register(&mut m)?;
        memtraps::register(&mut m)?;
        resmgr::register(&mut m)?;
        segment::register(&mut m)?;
        menu::register(&mut m)?;
        events::register(&mut m)?;
        m.mem.write_u16(EXIT_STUB, 0xA9F4).map_err(Refusal::mem("exit stub"))?; // _ExitToShell
        Ok(m)
    }

    /// Implement `opcode` (a Toolbox trap) with this Pascal signature. The name comes from the generated table; a
    /// trap that table does not know cannot be registered.
    pub fn register(&mut self, opcode: u16, args: &'static [Arg], result: Option<Arg>, handler: Handler<U>) -> Result<(), Refusal> {
        self.insert(opcode, Conv::Pascal { args, result, handler })
    }

    /// Implement `opcode` (an Operating System trap) with the register convention.
    pub fn register_os(&mut self, opcode: u16, handler: OsHandler<U>) -> Result<(), Refusal> {
        if opcode & 0x0800 != 0 { return Err(Refusal(format!("register_os: {} is a Toolbox trap", describe(opcode)))); }
        self.insert(opcode, Conv::Os(handler))
    }

    fn insert(&mut self, opcode: u16, conv: Conv<U>) -> Result<(), Refusal> {
        let name = trap_name(opcode).ok_or_else(|| Refusal(format!("register: {} has no name", describe(opcode))))?;
        if self.table.insert(trap_key(opcode), TrapEntry { name, conv }).is_some() {
            return Err(Refusal(format!("register: {} is registered twice", describe(opcode))));
        }
        Ok(())
    }

    /// Names of every implemented trap, in trap order.
    pub fn implemented(&self) -> Vec<&'static str> { self.table.values().map(|e| e.name).collect() }

    pub(crate) fn note_refusal(&mut self, name: &str, why: String) {
        self.log.push(format!("[toolbox] {name} REFUSED: {why}"));
        self.refused.push(format!("{name}: {why}"));
    }
}

impl<U: GuestUi> Host for Mac<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }

    /// The Mac engine has no `$4AFC` bridge slots: ILLEGAL stays the 68k exception.
    fn illegal(&mut self, _cpu: &mut Cpu, _pc: u32) -> bool { false }

    fn aline(&mut self, cpu: &mut Cpu, opcode: u16, pc: u32) -> bool {
        let Some(entry) = self.table.get(&trap_key(opcode)).copied() else {
            // Unknown frame layout: the stack is left as the guest made it; D0 = 0 (noErr / NIL) and execution goes on.
            if self.stop_on_unimplemented { cpu.halt(); }
            let why = format!("{UNIMPLEMENTED} {}", describe(opcode));
            self.log.push(format!("[toolbox] REFUSED: {why}"));
            self.refused.push(why);
            cpu.set_d0(0);
            return true;
        };
        let name = trap_name(opcode).unwrap_or(entry.name);
        match entry.conv {
            Conv::Pascal { args: layout, result: rkind, handler } => {
                let sp = cpu.areg(7);
                let outcome = pascal::read_args(&self.mem, sp, layout).and_then(|args| handler(self, cpu, &Call { opcode, pc, name, args }));
                let result = match outcome {
                    Ok(d) => { self.log.push(d.log); d.result }
                    Err(Refusal(e)) => { self.note_refusal(name, e); 0 }
                };
                // The convention is applied once, here: pop the arguments whatever the handler did, store the result.
                match pascal::complete(&mut self.mem, sp, layout, rkind.map(|k| (k, result))) {
                    Ok(new_sp) => cpu.set_areg(7, new_sp),
                    Err(Refusal(e)) => { self.note_refusal(name, e); cpu.halt(); }
                }
            }
            Conv::Os(handler) => {
                let mut regs = Regs::read(cpu, opcode);
                match handler(self, cpu, &mut regs) {
                    Ok(log) => self.log.push(log),
                    Err(Refusal(e)) => { self.note_refusal(name, e); regs.err(PARAM_ERR); }
                }
                cpu.set_d0(regs.d0);
                cpu.set_areg(0, regs.a0);
                set_flags(cpu, regs.d0);
            }
        }
        true
    }
}
