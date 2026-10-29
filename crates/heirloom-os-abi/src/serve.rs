//! The library side of the table: the machine session every `os-*.library` runs behind [`OsVTable`], generic over the family's
//! `GuestMachine`. The guest's UI is a heirloom-ui-abi `ProxyUi` whose transport is the engine's [`OsHost`], so every UI call
//! the guest makes leaves as a wire frame the moment it is made, and the session's Reports follow in the same stream.
//!
//! A family library is three lines: an [`OsFamily`] impl (its family number, how it binds its CPU, how it builds a machine) and
//! `static TABLE: OsVTable = vtable::<Family>();`.
use crate::*;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_common::{Delivery, GuestMachine, Refusal, Step};
use heirloom_ui_abi::wire::proxy::{ProxyUi, WireTransport};
use heirloom_ui_abi::wire::{decode, encode_report, Control, Frame, HostProfile, Report, WireStep};
use heirloom_ui_abi::UiObjectRef;

/// The engine's [`OsHost`] as a wire transport: frames go out through `emit`, canvases through `lease_canvas`. Nothing comes
/// back through it (the host's frames arrive through [`Session::apply`]), so `recv` is empty and `ask` has no answer.
pub struct HostTransport {
    host: OsHost,
}

impl HostTransport {
    /// The transport over an engine's `host` (a family's own tests make a session around a machine with it).
    pub fn new(host: OsHost) -> HostTransport {
        HostTransport { host }
    }
}

impl WireTransport for HostTransport {
    fn send(&mut self, frame: &[u8]) {
        (self.host.emit)(self.host.ctx, frame.as_ptr(), frame.len());
    }
    fn ask(&mut self, frame: &[u8]) -> Vec<u8> {
        self.send(frame);
        Vec::new()
    }
    fn recv(&mut self) -> Option<Vec<u8>> {
        None
    }
    fn lease_canvas(&mut self, win: UiObjectRef, width: i32, height: i32, pixels: &[u32]) -> u32 {
        (self.host.lease_canvas)(self.host.ctx, win.id, width, height, pixels.as_ptr(), pixels.len())
    }
}

/// The UI a family's machine draws on inside an os-*.library.
pub type LibUi = ProxyUi<HostTransport>;

/// One guest family behind the table.
pub trait OsFamily: 'static {
    const FAMILY: u32;
    type Machine: GuestMachine<Ui = LibUi> + 'static;
    /// Bind `cpu` as the CPU this library's machines run on (see BIND_*).
    fn bind_cpu(cpu: &'static CpuVTable) -> i32;
    /// Whether a CPU is bound: `create` refuses by name before it, so no guest can reach an unbound CPU.
    fn cpu_bound() -> bool;
    /// A machine of profile `profile` ("" = the family's default) drawing on `ui`.
    fn create(profile: &str, ui: LibUi) -> Result<Self::Machine, Refusal>;
    /// A guest-OS path alias from the host's manifest ([`Control::Assign`]). A family whose OS has none refuses it by name.
    fn assign(_m: &mut Self::Machine, name: &str, _target: &str, _add: bool) -> Result<(), String> {
        Err(format!("assign {name}: this guest OS has no assigns"))
    }
    /// The host's control metrics ([`Control::HostProfile`]) for the guest toolkit's layout. A family that lays nothing out for the
    /// host refuses it by name.
    fn host_profile(_m: &mut Self::Machine, _p: &HostProfile) -> Result<(), String> {
        Err(String::from("host profile: this guest OS lays out no window for host metrics"))
    }
}

/// A blob arriving in chunks (the staged volume snapshot, the program file).
#[derive(Default)]
struct Blob {
    bytes: Vec<u8>,
    total: Option<u32>,
}

impl Blob {
    /// Take one chunk; `Some(bytes)` once the blob is whole. A chunk of another total, or out of order, is a named refusal.
    fn take(&mut self, what: &str, offset: u32, total: u32, chunk: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if offset == 0 {
            self.bytes.clear();
            self.total = Some(total);
        }
        if self.total != Some(total) || offset as usize != self.bytes.len() {
            let have = self.bytes.len();
            *self = Blob::default();
            return Err(format!("{what}: chunk at {offset} of {total} does not follow the {have} bytes received"));
        }
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() == total as usize {
            self.total = None;
            return Ok(Some(core::mem::take(&mut self.bytes)));
        }
        Ok(None)
    }
}

/// One guest machine and the session state around it.
pub struct Session<M: GuestMachine<Ui = LibUi>> {
    machine: M,
    /// The family's own controls (OsFamily::assign / host_profile), captured when the session is made.
    hooks: FamilyHooks<M>,
    volume: Blob,
    program: Blob,
    staged_program: Option<Vec<u8>>,
    /// A named failure that ends the guest: the next Step reports it instead of running.
    fault: Option<String>,
    undelivered: Vec<String>,
    host_refused: Vec<String>,
}

/// The family-specific controls a session forwards (see [`OsFamily`]).
pub struct FamilyHooks<M> {
    pub assign: fn(&mut M, &str, &str, bool) -> Result<(), String>,
    pub host_profile: fn(&mut M, &HostProfile) -> Result<(), String>,
}

impl<M> FamilyHooks<M> {
    /// The hooks of family `F`.
    pub fn of<F: OsFamily<Machine = M>>() -> Self {
        FamilyHooks { assign: F::assign, host_profile: F::host_profile }
    }
}

impl<M: GuestMachine<Ui = LibUi>> Session<M> {
    pub fn new(machine: M, hooks: FamilyHooks<M>) -> Self {
        Session { machine, hooks, volume: Blob::default(), program: Blob::default(), staged_program: None, fault: None, undelivered: Vec::new(), host_refused: Vec::new() }
    }

    pub fn machine(&mut self) -> &mut M {
        &mut self.machine
    }

    fn report(&mut self, r: &Report) {
        let mut frame = Vec::new();
        if encode_report(r, &mut frame).is_ok() {
            self.machine.ui_mut().transport_mut().send(&frame);
        }
    }

    fn fail(&mut self, why: String) {
        if self.fault.is_none() {
            self.fault = Some(why);
        }
    }

    /// Apply concatenated frames in order. A frame that does not decode, or that an engine never takes (a UI call, a lease, a
    /// report), stops the batch: the frames before it were applied.
    pub fn apply(&mut self, mut frames: &[u8]) -> Result<(), String> {
        while !frames.is_empty() {
            let (frame, used) = decode(frames).map_err(|e| format!("frame refused: {}", e.name()))?;
            frames = &frames[used..];
            match frame {
                Frame::Control(c) => self.control(c),
                Frame::Event(ev) => match self.machine.deliver(ev) {
                    Ok(Delivery::Queued) | Ok(Delivery::Handled) | Ok(Delivery::Ignored) => {}
                    Ok(other) => self.undelivered.push(format!("{other:?}")),
                    Err(r) => self.fail(r.0),
                },
                Frame::Refused { token, why } => self.host_refused.push(format!("the host refused call {}: {why:?}", token.0)),
                Frame::Call(_) | Frame::Lease { .. } | Frame::Report(_) => return Err(String::from("frame refused: an engine takes no UI call, lease or report")),
            }
        }
        Ok(())
    }

    fn control(&mut self, c: Control) {
        match c {
            // The engine creates the machine and maps the canvas region itself; a library that sees either has nothing to do.
            Control::Machine { .. } | Control::Canvases { .. } => {}
            Control::Volume { offset, total, chunk } => match self.volume.take("volume", offset, total, &chunk) {
                Ok(Some(bytes)) => match heirloom_volume::snapshot::decode(&bytes) {
                    Ok(tree) => {
                        if let Err(e) = heirloom_volume::snapshot::merge(self.machine.volume(), &tree) {
                            self.fail(format!("volume: the staged tree does not fit the machine's ({e:?})"));
                        }
                    }
                    Err(e) => self.fail(format!("volume: snapshot refused ({e:?})")),
                },
                Ok(None) => {}
                Err(e) => self.fail(e),
            },
            Control::Program { offset, total, chunk } => match self.program.take("program", offset, total, &chunk) {
                Ok(Some(bytes)) => self.staged_program = Some(bytes),
                Ok(None) => {}
                Err(e) => self.fail(e),
            },
            Control::StartFile { name } => {
                let r = match (self.staged_program.take(), &self.fault) {
                    (_, Some(f)) => Err(f.clone()),
                    (None, None) => Err(String::from("start_file: no program was sent")),
                    (Some(file), None) => self.machine.start_file(&file, &name).map_err(|e| e.0),
                };
                self.report(&Report::Started(r));
            }
            Control::SetWallClock { local_unix_secs } => {
                let r = self.machine.set_wall_clock(local_unix_secs).map_err(|e| e.0);
                self.report(&Report::WallClock(r));
            }
            Control::StopOnUnimplemented(on) => self.machine.set_stop_on_unimplemented(on),
            // A refused family control ends the guest by name before it starts: a guest run without its assigns or its layout
            // metrics would fail later and further from the cause.
            Control::Assign { name, target, add } => {
                if let Err(e) = (self.hooks.assign)(&mut self.machine, &name, &target, add) {
                    self.fail(e);
                }
            }
            Control::HostProfile(p) => {
                if let Err(e) = (self.hooks.host_profile)(&mut self.machine, &p) {
                    self.fail(e);
                }
            }
            Control::Advance { virtual_us } => {
                if let Err(r) = self.machine.advance_clock_to(virtual_us) {
                    self.fail(r.0);
                }
            }
            Control::Step { budget } => {
                let (step, insns, bus_faults) = match self.fault.clone() {
                    Some(f) => (WireStep::Fault(f), 0, 0),
                    None => {
                        let (step, stats) = self.machine.step(budget);
                        let step = match step {
                            Step::Exited => WireStep::Exited,
                            Step::Fault(f) => WireStep::Fault(f),
                            Step::BudgetSpent => WireStep::BudgetSpent,
                            Step::Blocked => WireStep::Blocked,
                        };
                        (step, stats.insns, stats.bus_faults)
                    }
                };
                // The bridge log is the engine's own; a long-lived session must not keep it. Refusals go to the host.
                self.machine.drain_log();
                let mut refused = core::mem::take(&mut self.host_refused);
                refused.extend(self.machine.drain_refused());
                let undelivered = core::mem::take(&mut self.undelivered);
                let deadline_us = self.machine.next_timer_deadline();
                self.report(&Report::Stepped { step, insns, bus_faults, deadline_us, refused, undelivered });
            }
        }
    }
}

extern "C" fn bind_cpu_impl<F: OsFamily>(cpu: *const CpuVTable) -> i32 {
    // SAFETY: the engine passes the table its cpu-*.library's coral_lib_open returned (or null); its header is readable.
    if unsafe { heirloom_cpu_abi::check_ptr(cpu, heirloom_cpu_abi::CPU_ABI_VERSION, cpu_arch_of(F::FAMILY)) }.is_err() {
        return BIND_REFUSED;
    }
    // SAFETY: checked above; a library's table is a static that lives as long as the library stays open (the engine's life).
    F::bind_cpu(unsafe { &*cpu })
}

/// The CPU architecture a family's machines run on.
const fn cpu_arch_of(family: u32) -> u32 {
    match family {
        FAMILY_MACPPC => heirloom_cpu_abi::ARCH_PPC,
        FAMILY_EKAARM => heirloom_cpu_abi::ARCH_ARM,
        _ => heirloom_cpu_abi::ARCH_M68K,
    }
}

fn write_why(why: &str, out: *mut u8, cap: usize, len: *mut usize) {
    let n = why.len().min(cap);
    if !out.is_null() && n > 0 {
        // SAFETY: the caller lends `cap` writable bytes at `out`.
        unsafe { core::ptr::copy_nonoverlapping(why.as_ptr(), out, n) };
    }
    if !len.is_null() {
        // SAFETY: the caller lends one writable usize at `len`.
        unsafe { *len = n };
    }
}

extern "C" fn create_impl<F: OsFamily>(profile: *const u8, len: usize, host: *const OsHost, why: *mut u8, why_cap: usize, why_len: *mut usize) -> *mut OsInstance {
    write_why("", why, why_cap, why_len);
    if host.is_null() || (profile.is_null() && len != 0) {
        write_why("create: a null host or profile", why, why_cap, why_len);
        return core::ptr::null_mut();
    }
    // SAFETY: the caller lends `len` readable bytes at `profile` (checked non-null when len > 0).
    let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(profile, len) } };
    let Ok(profile) = core::str::from_utf8(bytes) else {
        write_why("create: the machine profile is not UTF-8", why, why_cap, why_len);
        return core::ptr::null_mut();
    };
    if !F::cpu_bound() {
        write_why("create: no CPU is bound (bind_cpu with the cpu-*.library table first)", why, why_cap, why_len);
        return core::ptr::null_mut();
    }
    // SAFETY: `host` is non-null and points to an OsHost the caller filled; it is copied here.
    let ui = ProxyUi::new(HostTransport { host: unsafe { *host } });
    match F::create(profile, ui) {
        Ok(machine) => Box::into_raw(Box::new(Session::new(machine, FamilyHooks::of::<F>()))) as *mut OsInstance,
        Err(r) => {
            write_why(&r.0, why, why_cap, why_len);
            core::ptr::null_mut()
        }
    }
}

extern "C" fn destroy_impl<F: OsFamily>(inst: *mut OsInstance) {
    if !inst.is_null() {
        // SAFETY: a non-null instance is one create_impl::<F> boxed; the engine destroys it once.
        drop(unsafe { Box::from_raw(inst as *mut Session<F::Machine>) });
    }
}

extern "C" fn apply_impl<F: OsFamily>(inst: *mut OsInstance, frames: *const u8, len: usize) -> i32 {
    if inst.is_null() || (frames.is_null() && len != 0) {
        return APPLY_BAD_ARGS;
    }
    // SAFETY: a non-null instance is one create_impl::<F> boxed and not yet destroyed; the engine calls it from one thread.
    let session = unsafe { &mut *(inst as *mut Session<F::Machine>) };
    // SAFETY: the caller lends `len` readable bytes at `frames`.
    let bytes = if len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(frames, len) } };
    match session.apply(bytes) {
        Ok(()) => APPLY_OK,
        Err(_) => APPLY_BAD_FRAME,
    }
}

/// The table for family `F`: `static TABLE: OsVTable = heirloom_os_abi::serve::vtable::<MyFamily>();`.
pub const fn vtable<F: OsFamily>() -> OsVTable {
    OsVTable {
        version: OS_ABI_VERSION,
        size: core::mem::size_of::<OsVTable>() as u32,
        family: F::FAMILY,
        bind_cpu: bind_cpu_impl::<F>,
        create: create_impl::<F>,
        destroy: destroy_impl::<F>,
        apply: apply_impl::<F>,
    }
}
