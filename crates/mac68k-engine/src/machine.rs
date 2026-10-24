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
use crate::{events, lowmem, memtraps, menu, qd, quickdraw, rects, resmgr, segment, toolbox, Refusal};
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
    /// A colour window (`NewCWindow` on a machine with Color QuickDraw): a CGrafPort over an 8-bit canvas.
    pub color: bool,
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
    /// The translation layer's state: projected controls, dialogs, pending gestures (translate/).
    pub tr: crate::translate::Translate,
    /// The address the program gave InitGraf (its `qd.thePort`); 0 before InitGraf.
    pub qd_globals: u32,
    /// QuickDraw's host-side state: window canvases, update regions, the font (qd/).
    pub qd: qd::Qd,
    /// Color QuickDraw's state: graphics devices, colour-table seeds, colour windows' pixel maps (cqd/).
    pub cqd: crate::cqd::Cqd,
    pub(crate) table: BTreeMap<u16, TrapEntry<U>>,
    /// One line per successful bridged call, in order.
    pub log: Vec<String>,
    /// Refusals (bad guest pointers, unimplemented traps ...), in order.
    pub refused: Vec<String>,
    /// Census mode: the first call to an unimplemented trap halts the guest (the run's last refusal names it).
    pub stop_on_unimplemented: bool,
    /// (trap word, address of the A-line word) for every call to an unimplemented trap, in order.
    pub unimplemented_at: Vec<(u16, u32)>,
    pub heap: Heap,
    /// The File Manager's volumes, open forks and working directories (fm/).
    pub fm: crate::fm::FileMgr,
    /// The desk scrap (scrap.rs).
    pub scrap: crate::scrap::Scrap,
    /// A guest routine a handler asked for; the A-line bridge enters it after the handler returns (guestcall.rs).
    pub(crate) pending_call: Option<crate::guestcall::GuestCall>,
    pub(crate) call_stack: Vec<crate::guestcall::Frame>,
    /// A resource decompression a handler queued; the A-line bridge runs it and retries the trap (compressed.rs).
    pub(crate) dcmp_pending: Option<crate::compressed::Decomp>,
    /// A dialog draw that has user-item procedures to run once the trap that asked has finished ([`crate::translate::useritems`]).
    pub(crate) pending_user_items: Option<crate::translate::useritems::Run>,
    /// Routine descriptors `NewRoutineDescriptor` / `NewFatRoutineDescriptor` allocated (`mixed_mode`): the blocks
    /// `DisposeRoutineDescriptor` may free, once. Ownership only: a descriptor the program built itself is callable too.
    pub routine_descriptors: alloc::collections::BTreeSet<u32>,
    /// True while a registered handler runs on the real CPU (a decompression can only be deferred then).
    pub(crate) in_trap: bool,
    /// Trap patches (`_SetTrapAddress`): trap key -> the guest routine that replaces it.
    pub patches: BTreeMap<u16, u32>,
    pub(crate) ctx: Option<retro_m68k::CpuContext>,
    /// The cursor the program set and its hide level (cursor.rs).
    pub cursor: crate::cursor::CursorState,
    pub clock: crate::clock::Clock,
    /// The Vertical Retrace Manager's queue (vbl.rs).
    pub vbl: crate::vbl::Vbl,
    /// The Apple Event Manager's handler tables (appleevents.rs).
    pub ae: crate::appleevents::AppleEvents,
    /// The address of the trap word the A-line bridge is handling (a handler that must resume AFTER its trap needs it).
    pub(crate) trap_pc: u32,
    /// Palettes associated with windows (palette.rs).
    pub pal: crate::palette::Palettes,
    /// The engine's own International Resource handles (intl.rs).
    pub intl: crate::intl::IntlHandles,
    /// The Sound Manager's channels beside the guest's records (sound/).
    pub snd: crate::sound::SoundState,
    /// Which classic Mac this is (profile.rs): the CPU model, the boot address mode, what Gestalt / SysEnvirons answer.
    pub profile: crate::profile::MachineProfile,
}

/// The prefix every unimplemented-trap refusal starts with (a census runner keys on it).
pub const UNIMPLEMENTED: &str = "unimplemented trap";

impl<U: GuestUi> Mac<U> {
    pub fn new(ui: U) -> Result<Self, Refusal> {
        let mut m = Mac {
            mem: GuestMem::new(DEFAULT_SIZE), ui, windows: BTreeMap::new(), z: vec![], mm: MemMgr::default(), rm: ResMgr::default(),
            seg: SegLoader::default(), menus: MenuMgr::default(), ev: EventMgr::default(), tr: Default::default(), qd_globals: 0, qd: qd::Qd::default(), cqd: Default::default(),
            table: BTreeMap::new(), log: vec![], refused: vec![],
            stop_on_unimplemented: false, unimplemented_at: vec![], heap: Heap::new(HEAP_START, HEAP_END), ctx: None,
            fm: crate::fm::FileMgr::new(), scrap: Default::default(), pending_call: None, call_stack: vec![], dcmp_pending: None, pending_user_items: None, routine_descriptors: Default::default(), in_trap: false, patches: BTreeMap::new(), cursor: Default::default(), clock: Default::default(), vbl: Default::default(), ae: Default::default(), trap_pc: 0, pal: Default::default(), intl: Default::default(), snd: Default::default(), profile: crate::profile::DEFAULT,
        };
        m.fm.install_system_folder(); // a real Mac has its System Folder and the folders in it from the start
        lowmem::init(&mut m.mem)?;
        lowmem::set_rom85(&mut m.mem, m.profile.rom85)?;
        m.set_address_mode(crate::addrmode::AddrMode::Bit24)?; // a classic 68k Mac boots in 24-bit mode
        crate::addrmode::register(&mut m)?;
        m.install_trampoline()?;
        toolbox::register(&mut m)?;
        quickdraw::register(&mut m)?;
        rects::register(&mut m)?;
        qd::register(&mut m)?;
        crate::cqd::register(&mut m)?;
        memtraps::register(&mut m)?;
        resmgr::register(&mut m)?;
        segment::register(&mut m)?;
        crate::fm::traps::register(&mut m)?;
        crate::fm::fsspec::register(&mut m)?;
        crate::fm::alias::register(&mut m)?;
        crate::fm::resfiles::register(&mut m)?;
        crate::scrap::register(&mut m)?;
        crate::sysenv::register(&mut m)?;
        crate::gestalt::register(&mut m)?;
        crate::pack2::register(&mut m)?;
        crate::pack7::register(&mut m)?;
        crate::vbl::register(&mut m)?;
        crate::palette::register(&mut m)?;
        crate::appleevents::register(&mut m)?;
        crate::fixmath::register(&mut m)?;
        crate::mouse::register(&mut m)?;
        crate::toolutil::register(&mut m)?;
        crate::package::register(&mut m)?;
        crate::intl::register(&mut m)?;
        crate::strcmp::register(&mut m)?;
        crate::procmgr::register(&mut m)?;
        crate::cursor::register(&mut m)?;
        crate::clock::register(&mut m)?;
        crate::winmgr::register(&mut m)?;
        crate::textedit::register(&mut m)?;
        crate::script::register(&mut m)?;
        crate::sound::register(&mut m)?;
        crate::fontnames::register(&mut m)?;
        menu::register(&mut m)?;
        crate::menu_items::register(&mut m)?;
        events::register(&mut m)?;
        crate::traptable::register(&mut m)?;
        crate::translate::register(&mut m)?;
        m.mem.write_u16(EXIT_STUB, 0xA9F4).map_err(Refusal::mem("exit stub"))?; // _ExitToShell
        Ok(m)
    }

    /// A machine of the profile called `name` (profile.rs), for a host that chooses it before anything exists: [`Mac::new`] then
    /// [`Mac::set_machine`]. An unknown name is a named refusal.
    pub fn with_profile(ui: U, name: &str) -> Result<Self, Refusal> {
        let mut m = Self::new(ui)?;
        m.set_machine(name)?;
        Ok(m)
    }

    /// Become the machine called `name` (profile.rs; a bundle's `(guest (machine "mac-plus"))`): the CPU model the NEXT
    /// [`Mac::launch`] / [`Mac::start`] runs on, the address mode, and the Gestalt / SysEnvirons answers. An unknown name is a
    /// named refusal and changes nothing.
    pub fn set_machine(&mut self, name: &str) -> Result<(), Refusal> {
        let p = crate::profile::lookup(name).ok_or_else(|| Refusal(format!("unknown machine profile \"{name}\" (known: {})", crate::profile::PROFILES.iter().map(|p| p.name).collect::<Vec<_>>().join(", "))))?;
        self.set_address_mode(p.addr)?;
        lowmem::set_rom85(&mut self.mem, p.rom85)?;
        self.profile = p;
        Ok(())
    }

    /// Implement `opcode` (a Toolbox trap) with this Pascal signature. The name comes from the generated table; a
    /// trap that table does not know cannot be registered.
    pub fn register(&mut self, opcode: u16, args: &'static [Arg], result: Option<Arg>, handler: Handler<U>) -> Result<(), Refusal> {
        self.insert(opcode, Conv::Pascal { args, result, handler })
    }

    /// Implement `opcode` with the REGISTER convention: every Operating System trap, and the handful of Toolbox-range
    /// utilities (HandToHand, PtrToHand ...) that also pass their arguments in A0/A1/D0.
    pub fn register_os(&mut self, opcode: u16, handler: OsHandler<U>) -> Result<(), Refusal> {
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

    /// What a Pascal handler's outcome means, whichever CPU called it: log the line or record the refusal; the function
    /// result (0 after a refusal). Shared by the 68k A-line bridge and the Mixed Mode seam ([`crate::mixed`]).
    pub(crate) fn finish_pascal(&mut self, name: &str, outcome: Result<Done, Refusal>) -> u32 {
        match outcome {
            Ok(d) => { self.log.push(d.log); d.result }
            Err(Refusal(e)) => { self.note_refusal(name, e); 0 }
        }
    }

    /// The same for an OS-trap handler: log, or record the refusal and return paramErr in D0.
    pub(crate) fn finish_os(&mut self, name: &str, outcome: Result<String, Refusal>, regs: &mut Regs) {
        match outcome {
            Ok(log) => self.log.push(log),
            Err(Refusal(e)) => { self.note_refusal(name, e); regs.err(PARAM_ERR); }
        }
    }

    pub(crate) fn note_refusal(&mut self, name: &str, why: String) {
        self.log.push(format!("[toolbox] {name} REFUSED: {why}"));
        self.refused.push(format!("{name}: {why}"));
    }
}

impl<U: GuestUi> Host for Mac<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }

    /// The only `$4AFC` is the completion trampoline ([`crate::guestcall::TRAMPOLINE`]); any other ILLEGAL stays the 68k exception.
    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool { self.mem.strip(pc) == crate::guestcall::TRAMPOLINE && self.leave_guest(cpu) } // a 24-bit program returns through addresses it has flagged in the high byte (WriteNow's segment glue): the bus ignores it, so must this

    fn aline(&mut self, cpu: &mut Cpu, opcode: u16, pc: u32) -> bool {
        let pc = self.mem.strip(pc);
        let via_stub = crate::traptable::in_stub(pc);
        self.trap_pc = pc;
        // A vertical-retrace task that came due runs first; this very trap word then runs again (vbl.rs).
        if !via_stub && self.enter_vbl(cpu, pc) { return true; }
        // A patched trap (`_SetTrapAddress`) runs the guest routine first; the routine chains to the original through the
        // stub `GetTrapAddress` gave it, and that stub's own trap word (`via_stub`) is not patched again.
        if !via_stub {
            if let Some(&target) = self.patches.get(&trap_key(opcode)) {
                self.log.push(format!("[toolbox] {} -> patched routine {target:#x}", trap_name(opcode).unwrap_or("trap")));
                self.enter_guest(cpu, pc, crate::guestcall::GuestCall { target, d0: None, a0: None, d1: Some(u32::from(opcode)), restore: None });
                return true;
            }
        }
        let Some(entry) = self.table.get(&trap_key(opcode)).copied() else {
            // Unknown frame layout: the stack is left as the guest made it; D0 = 0 (noErr / NIL) and execution goes on.
            self.unimplemented_at.push((opcode, pc));
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
                // Reached through a stub (`JSR` to a GetTrapAddress answer): the return address is on top of the arguments.
                let stub_ret = if via_stub { self.mem.read_u32(cpu.areg(7)).ok() } else { None };
                let sp = cpu.areg(7).wrapping_add(if stub_ret.is_some() { 4 } else { 0 });
                // Entered through a stub, the trap's own address is the original call site: just before the return address.
                let site = stub_ret.map_or(pc, |r| r.wrapping_sub(2));
                let pc_before = cpu.pc();
                self.dcmp_pending = None;
                self.in_trap = true;
                let outcome = pascal::read_args(&self.mem, sp, layout).and_then(|args| handler(self, cpu, &Call { opcode, pc: site, name, args }));
                self.in_trap = false;
                // A resource the handler needed is compressed: expand it in the guest, then run this very trap again.
                if let Some(d) = self.dcmp_pending.take() { self.enter_decompress(cpu, pc, d); return true; }
                let redirected = cpu.pc() != pc_before; // a handler that moves the PC (LoadSeg) takes the return address with it
                let result = self.finish_pascal(name, outcome);
                // The convention is applied once, here: pop the arguments whatever the handler did, store the result.
                match pascal::complete(&mut self.mem, sp, layout, rkind.map(|k| (k, result))) {
                    Ok(mut new_sp) => {
                        if let (Some(ret), false) = (stub_ret, redirected) { new_sp = new_sp.wrapping_sub(4); let _ = self.mem.write_u32(new_sp, ret); }
                        cpu.set_areg(7, new_sp)
                    }
                    Err(Refusal(e)) => { self.note_refusal(name, e); cpu.halt(); }
                }
                if let Some(run) = self.pending_user_items.take() { self.enter_user_items(cpu, run); }
            }
            Conv::Os(handler) => {
                let mut regs = Regs::read(cpu, opcode);
                // A0/A1 carry pointers and handles whose high byte a 24-bit application may use for flags: the bus ignores it.
                regs.a0 = self.mem.strip(regs.a0);
                regs.a1 = self.mem.strip(regs.a1);
                self.dcmp_pending = None;
                self.in_trap = true;
                let outcome = handler(self, cpu, &mut regs);
                self.in_trap = false;
                if let Some(d) = self.dcmp_pending.take() { self.enter_decompress(cpu, pc, d); return true; }
                self.finish_os(name, outcome, &mut regs);
                cpu.set_d0(regs.d0);
                cpu.set_areg(0, regs.a0);
                set_flags(cpu, regs.d0);
                // A handler may ask for guest code to run before the program resumes (a completion routine).
                if let Some(mut call) = self.pending_call.take() {
                    if call.restore.is_some() { call.restore = Some((regs.d0, regs.a0)); }
                    self.enter_guest(cpu, pc, call);
                }
            }
        }
        true
    }
}
