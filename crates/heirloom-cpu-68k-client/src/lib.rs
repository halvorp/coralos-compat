#![cfg_attr(not(test), no_std)]
// `bind-static` exists for HOST tests and host tools only: it links the static core so a test can bind its table. A CoralOS
// build (target_os = "none") opens cpu-68k.library instead, and linking the core there would be the CPU twice (doctrine
// LIBRARY-NOT-LINKED-TWICE) - refused at compile time, not by convention.
#[cfg(all(feature = "bind-static", target_os = "none"))]
compile_error!("heirloom-cpu-68k-client: `bind-static` is the host-test binding; a CoralOS engine binds the CpuVTable it opened from cpu-68k.library");
extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::cell::{Cell, UnsafeCell};
use core::ffi::c_void;
use core::fmt;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use heirloom_cpu_abi::{
    check, CpuBusCallbacks, CpuInstance, CpuStop, CpuVTable, OpenRefusal, ARCH_M68K, CPU_ABI_VERSION,
    STOP_HALT, STOP_STALL, STOP_TRAP,
};
use heirloom_mem_flat32::GuestMem;

// ── Public constants ────────────────────────────────────────────────────────

pub const REG_D0: u32 = 0;
pub const REG_A0: u32 = 8;
pub const REG_PC: u32 = 16;
const REG_SR: u32 = 17;

// ── CpuModel ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuModel {
    M68000 = 1,
    M68010 = 2,
    #[default]
    M68020 = 4,
}

// ── Host trait ──────────────────────────────────────────────────────────────

pub trait Host {
    fn mem(&mut self) -> &mut GuestMem;
    fn read(&mut self, addr: u32, len: u32) -> Result<u32, ()> {
        let mem = self.mem();
        match len {
            1 => mem.read_u8(addr).map(u32::from).map_err(|_| ()),
            2 => mem.read_u16(addr).map(u32::from).map_err(|_| ()),
            4 => mem.read_u32(addr).map_err(|_| ()),
            _ => Err(()),
        }
    }
    fn write(&mut self, addr: u32, len: u32, v: u32) -> Result<(), ()> {
        let mem = self.mem();
        match len {
            1 => mem.write_u8(addr, v as u8).map_err(|_| ()),
            2 => mem.write_u16(addr, v as u16).map_err(|_| ()),
            4 => mem.write_u32(addr, v).map_err(|_| ()),
            _ => Err(()),
        }
    }
    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool;
    fn aline(&mut self, _cpu: &mut Cpu, _opcode: u16, _pc: u32) -> bool {
        false
    }
}

// ── Cpu ─────────────────────────────────────────────────────────────────────

pub struct Cpu {
    halted: bool,
    blocked: bool,
    touched: Cell<bool>,
    attached: bool,
}

impl Cpu {
    pub fn detached() -> Cpu {
        Cpu { halted: false, blocked: false, touched: Cell::new(false), attached: false }
    }
    pub fn attached() -> Cpu {
        Cpu { halted: false, blocked: false, touched: Cell::new(false), attached: true }
    }
    pub fn touched_registers(&self) -> bool { self.touched.get() }
    pub fn is_halted(&self) -> bool { self.halted }
    pub fn is_blocked(&self) -> bool { self.blocked }

    fn read_reg(&self, r: u32) -> u32 {
        if self.attached {
            let table = table();
            let inst = INSTANCE.load(Ordering::Acquire);
            let mut out: u64 = 0;
            // The table trusts its caller here: instance is valid (created by table.create), out is a valid mutable pointer.
            let res = (table.get_reg)(inst, r, &mut out);
            if res != 0 { panic!("cpu-68k.library refused get_reg"); }
            out as u32
        } else {
            self.touched.set(true);
            0
        }
    }

    fn write_reg(&mut self, r: u32, v: u32) {
        if self.attached {
            let table = table();
            let inst = INSTANCE.load(Ordering::Acquire);
            // The table trusts its caller here: instance is valid, value is a u64 cast of v.
            let res = (table.set_reg)(inst, r, v as u64);
            if res != 0 { panic!("cpu-68k.library refused set_reg"); }
        } else {
            self.touched.set(true);
        }
    }

    pub fn dreg(&self, n: u32) -> u32 { self.read_reg(n) }
    pub fn areg(&self, n: u32) -> u32 { self.read_reg(REG_A0 + n) }
    pub fn pc(&self) -> u32 { self.read_reg(REG_PC) }
    pub fn sr(&self) -> u32 { self.read_reg(REG_SR) }

    pub fn set_d0(&mut self, v: u32) { self.set_dreg(0, v); }
    pub fn set_dreg(&mut self, n: u32, v: u32) { self.write_reg(n, v); }
    pub fn set_areg(&mut self, n: u32, v: u32) { self.write_reg(REG_A0 + n, v); }
    pub fn set_pc(&mut self, pc: u32) { self.write_reg(REG_PC, pc); }
    pub fn set_sr(&mut self, v: u32) { self.write_reg(REG_SR, v); }

    pub fn halt(&mut self) { self.halted = true; }
    pub fn block(&mut self) { self.blocked = true; }
    pub fn cancel_stop(&mut self) { self.halted = false; self.blocked = false; }
}

// ── RunStats ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStats {
    pub insns: u64,
    pub bus_faults: u32,
    pub halted: bool,
    pub blocked: bool,
    pub stalled_at: Option<u32>,
}

// ── CpuContext ──────────────────────────────────────────────────────────────

pub struct CpuContext(Vec<u8>);

impl CpuContext {
    pub fn set_reg(&mut self, r: u32, v: u32) {
        let _guard = CpuLock::new();
        let table = table();
        let inst = INSTANCE.load(Ordering::Acquire);
        let len = self.0.len() as u32;
        // The table trusts its caller here: inst valid, self.0 is a valid buffer of `len` bytes.
        let res = (table.load_context)(inst, self.0.as_ptr(), len);
        if res != 0 { panic!("cpu-68k.library refused load_context"); }
        let res = (table.set_reg)(inst, r, v as u64);
        if res != 0 { panic!("cpu-68k.library refused set_reg"); }
        let res = (table.save_context)(inst, self.0.as_mut_ptr(), len);
        if res != 0 { panic!("cpu-68k.library refused save_context"); }
    }
}

// ── BindRefusal ─────────────────────────────────────────────────────────────

pub enum BindRefusal {
    Open(OpenRefusal),
    AlreadyBound,
}

impl fmt::Debug for BindRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindRefusal::Open(r) => match r {
                OpenRefusal::NullTable => write!(f, "Open(NullTable)"),
                OpenRefusal::VersionTooOld { found, required } =>
                    write!(f, "Open(VersionTooOld {{ found: {}, required: {} }})", found, required),
                OpenRefusal::TableTooSmall { found, required } =>
                    write!(f, "Open(TableTooSmall {{ found: {}, required: {} }})", found, required),
                OpenRefusal::WrongArch { found, required } =>
                    write!(f, "Open(WrongArch {{ found: {}, required: {} }})", found, required),
            },
            BindRefusal::AlreadyBound => write!(f, "AlreadyBound"),
        }
    }
}

// ── Global state ────────────────────────────────────────────────────────────

static VTABLE: AtomicPtr<CpuVTable> = AtomicPtr::new(ptr::null_mut());
static INSTANCE: AtomicPtr<CpuInstance> = AtomicPtr::new(ptr::null_mut());
static INSTANCE_MODEL: AtomicU32 = AtomicU32::new(0);
static CPU_LOCK: AtomicBool = AtomicBool::new(false);

struct ActiveCell(UnsafeCell<Option<(*mut (dyn Host + 'static), *mut Cpu)>>);

// SAFETY: ACTIVE is only read or written while CPU_LOCK is held by the running
// CPU operation; its raw pointers remain valid for that operation's lifetime.
unsafe impl Sync for ActiveCell {}

static ACTIVE: ActiveCell = ActiveCell(UnsafeCell::new(None));

// ── Bus callbacks ───────────────────────────────────────────────────────────

#[repr(transparent)]
struct BusCallbacksCell(CpuBusCallbacks);

// SAFETY: The `ctx` pointer is never read; routing goes through ACTIVE which is
// guarded by CPU_LOCK. The fn pointers are pure and only access ACTIVE.
unsafe impl Sync for BusCallbacksCell {}

extern "C" fn bus_read8(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    bus_read(addr, 1, out)
}
extern "C" fn bus_read16(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    bus_read(addr, 2, out)
}
extern "C" fn bus_read32(_ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    bus_read(addr, 4, out)
}

fn bus_read(addr: u32, len: u32, out: *mut u32) -> i32 {
    // SAFETY: ACTIVE is only written under CPU_LOCK; read_volatile prevents compiler reordering.
    let active = unsafe { ptr::read_volatile(ACTIVE.0.get()) };
    match active {
        Some((host_ptr, _)) => {
            // SAFETY: ACTIVE names the host of the run in progress, set by `activate` for exactly this call's duration.
            let host = unsafe { &mut *host_ptr };
            match host.read(addr, len) {
                // SAFETY: the table passes a writable result pointer for every read callback (the ABI's contract).
                Ok(v) => { unsafe { *out = v; } 0 }
                Err(()) => 1,
            }
        }
        None => 1,
    }
}

extern "C" fn bus_write8(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    bus_write(addr, 1, value)
}
extern "C" fn bus_write16(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    bus_write(addr, 2, value)
}
extern "C" fn bus_write32(_ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    bus_write(addr, 4, value)
}

fn bus_write(addr: u32, len: u32, value: u32) -> i32 {
    // SAFETY: ACTIVE is written only by `activate` under CPU_LOCK, and callbacks run only inside that window.
    let active = unsafe { ptr::read_volatile(ACTIVE.0.get()) };
    match active {
        Some((host_ptr, _)) => {
            // SAFETY: ACTIVE names the host of the run in progress, set by `activate` for exactly this call's duration.
            let host = unsafe { &mut *host_ptr };
            match host.write(addr, len, value) {
                Ok(()) => 0,
                Err(()) => 1,
            }
        }
        None => 1,
    }
}

extern "C" fn bus_on_trap(_ctx: *mut c_void, pc: u32, opcode_or_trap: u32) -> i32 {
    // SAFETY: ACTIVE is written only by `activate` under CPU_LOCK, and callbacks run only inside that window.
    let active = unsafe { ptr::read_volatile(ACTIVE.0.get()) };
    match active {
        Some((host_ptr, cpu_ptr)) => {
            // SAFETY: ACTIVE names the host and Cpu of the run in progress, both live for exactly this call's duration.
            let host = unsafe { &mut *host_ptr };
            let cpu = unsafe { &mut *cpu_ptr };
            let handled = if opcode_or_trap == 0x4AFC {
                host.illegal(cpu, pc)
            } else {
                host.aline(cpu, opcode_or_trap as u16, pc)
            };
            if cpu.is_halted() { 2 }
            else if cpu.is_blocked() { 3 }
            else if handled { 0 }
            else { 1 }
        }
        None => 1,
    }
}

static BUS: BusCallbacksCell = BusCallbacksCell(CpuBusCallbacks {
    ctx: ptr::null_mut(),
    read8: bus_read8,
    read16: bus_read16,
    read32: bus_read32,
    write8: bus_write8,
    write16: bus_write16,
    write32: bus_write32,
    on_trap: bus_on_trap,
});

// ── Spin lock ───────────────────────────────────────────────────────────────

struct CpuLock;

impl CpuLock {
    fn new() -> Self {
        while CPU_LOCK
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            core::hint::spin_loop();
        }
        CpuLock
    }
}

impl Drop for CpuLock {
    fn drop(&mut self) {
        CPU_LOCK.store(false, Ordering::Release);
    }
}

// ── ACTIVE guard ────────────────────────────────────────────────────────────

struct ActiveGuard;

fn activate<H: Host + 'static>(host: &mut H, cpu: &mut Cpu) -> ActiveGuard {
    let host_ptr = host as *mut H as *mut (dyn Host + 'static);
    let cpu_ptr = cpu as *mut Cpu;
    // SAFETY: ACTIVE is only accessed under CPU_LOCK; host and cpu are valid for the guard's lifetime.
    unsafe {
        let p = ACTIVE.0.get();
        ptr::write(p, Some((host_ptr, cpu_ptr)));
        ptr::read_volatile(p);
    }
    ActiveGuard
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        // SAFETY: ACTIVE was set by activate; clearing it under CPU_LOCK.
        unsafe {
            let p = ACTIVE.0.get();
            ptr::write(p, None);
            ptr::read_volatile(p);
        }
    }
}

// ── Table helper ────────────────────────────────────────────────────────────

fn table() -> &'static CpuVTable {
    let p = VTABLE.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: p was stored by bind() from a &'static CpuVTable.
        return unsafe { &*p };
    }
    #[cfg(feature = "bind-static")]
    {
        let static_table = heirloom_cpu_68k::vtable();
        let _ = bind(static_table);
        let p = VTABLE.load(Ordering::Acquire);
        if !p.is_null() {
            // SAFETY: the pointer was stored by `bind` from a `&'static CpuVTable`.
            return unsafe { &*p };
        }
    }
    panic!("heirloom-cpu-68k-client: cpu-68k.library is not bound (call bind with the library's CpuVTable first)");
}

// ── Instance management ─────────────────────────────────────────────────────

fn ensure_instance(model: CpuModel, table: &CpuVTable) {
    let current_model = INSTANCE_MODEL.load(Ordering::Acquire);
    let inst = INSTANCE.load(Ordering::Acquire);
    if !inst.is_null() && current_model == model as u32 {
        return;
    }
    if !inst.is_null() {
        // The table trusts its caller here: inst was created by table.create.
        (table.destroy)(inst);
        INSTANCE.store(ptr::null_mut(), Ordering::Release);
    }
    // The table trusts its caller here: &BUS.0 is a valid *const CpuBusCallbacks.
    let new_inst = (table.create)(model as u32, &BUS.0 as *const CpuBusCallbacks);
    if new_inst.is_null() {
        panic!("cpu-68k.library refused create");
    }
    INSTANCE.store(new_inst, Ordering::Release);
    INSTANCE_MODEL.store(model as u32, Ordering::Release);
}

// ── Public API ──────────────────────────────────────────────────────────────

pub fn bind(table: &'static CpuVTable) -> Result<(), BindRefusal> {
    if let Err(r) = check(table, CPU_ABI_VERSION, ARCH_M68K) {
        return Err(BindRefusal::Open(r));
    }
    let new_ptr = table as *const CpuVTable as *mut CpuVTable;
    match VTABLE.compare_exchange(ptr::null_mut(), new_ptr, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => Ok(()),
        Err(current) => {
            if current == new_ptr { Ok(()) } else { Err(BindRefusal::AlreadyBound) }
        }
    }
}

pub fn is_bound() -> bool {
    !VTABLE.load(Ordering::Acquire).is_null()
}

pub fn start<H: Host + 'static>(host: &mut H, entry: u32, sp: u32) -> CpuContext {
    start_with(host, entry, sp, &[])
}

pub fn start_with<H: Host + 'static>(
    host: &mut H,
    entry: u32,
    sp: u32,
    regs: &[(u32, u32)],
) -> CpuContext {
    start_model(host, entry, sp, regs, CpuModel::M68020)
}

pub fn start_model<H: Host + 'static>(
    host: &mut H,
    entry: u32,
    sp: u32,
    regs: &[(u32, u32)],
    model: CpuModel,
) -> CpuContext {
    let _lock = CpuLock::new();
    let table = table();
    ensure_instance(model, table);
    let inst = INSTANCE.load(Ordering::Acquire);

    let saved4 = host.mem().read_u32(4).unwrap_or(0);
    host.mem().write_u32(0, sp).unwrap();
    host.mem().write_u32(4, entry).unwrap();

    {
        let mut cpu = Cpu::attached();
        let _active = activate(host, &mut cpu);
        // The table trusts its caller here: inst is valid.
        (table.reset)(inst);
    }

    host.mem().write_u32(4, saved4).unwrap();

    for &(r, v) in regs {
        // The table trusts its caller here: inst is valid.
        let res = (table.set_reg)(inst, r, v as u64);
        if res != 0 { panic!("cpu-68k.library refused set_reg"); }
    }

    let size = (table.context_size)(inst);
    let mut buf = vec![0u8; size as usize];
    // The table trusts its caller here: buf is a valid buffer of `size` bytes.
    let res = (table.save_context)(inst, buf.as_mut_ptr(), size);
    if res != 0 { panic!("cpu-68k.library refused save_context"); }

    CpuContext(buf)
}

pub fn resume<H: Host + 'static>(
    host: &mut H,
    ctx: &mut CpuContext,
    max_insns: u64,
) -> RunStats {
    let _lock = CpuLock::new();
    let table = table();
    let inst = INSTANCE.load(Ordering::Acquire);
    let len = ctx.0.len() as u32;

    // The table trusts its caller here: ctx.0 is a valid buffer of `len` bytes.
    let res = (table.load_context)(inst, ctx.0.as_ptr(), len);
    if res != 0 { panic!("cpu-68k.library refused load_context"); }

    let mut stop = CpuStop { kind: 0, pc: 0, detail: 0, bus_faults: 0, insns: 0 };
    {
        let mut cpu = Cpu::attached();
        let _active = activate(host, &mut cpu);
        // The table trusts its caller here: inst is valid, stop is a valid pointer.
        let res = (table.run)(inst, max_insns, &mut stop);
        if res != 0 { panic!("cpu-68k.library refused run"); }
    }

    // The table trusts its caller here: ctx.0 is a valid mutable buffer of `len` bytes.
    let res = (table.save_context)(inst, ctx.0.as_mut_ptr(), len);
    if res != 0 { panic!("cpu-68k.library refused save_context"); }

    RunStats {
        insns: stop.insns,
        bus_faults: stop.bus_faults,
        halted: stop.kind == STOP_HALT,
        blocked: stop.kind == STOP_TRAP && stop.detail == 1,
        stalled_at: if stop.kind == STOP_STALL { Some(stop.pc) } else { None },
    }
}

pub fn run<H: Host + 'static>(host: &mut H, entry: u32, sp: u32, max_insns: u64) -> RunStats {
    let mut ctx = start(host, entry, sp);
    resume(host, &mut ctx, max_insns)
}

pub fn recent_pcs() -> Vec<u32> {
    let table = table();
    let inst = INSTANCE.load(Ordering::Acquire);
    let mut buf = vec![0u32; 16384];
    let count = (table.recent_pcs)(inst, buf.as_mut_ptr(), 16384);
    buf.truncate(count as usize);
    buf
}

#[cfg(test)]
mod tests;