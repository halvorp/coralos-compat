//! The Process Manager for PowerPC programs: a SECOND application in the same machine, the way Mac OS 9 runs several applications
//! cooperatively in one address space (Inside Macintosh: Processes). The process table and the per-process context the switch saves
//! and restores are the shared Mac semantics ([`heirloom_mac68k_engine::processes`]); this module is the PowerPC half: starting a
//! process's code, ending it, and which events each process sees.
//!
//! * `LaunchApplication(LaunchPBPtr)` is MEDIATED by this loader (doc 05 section 15: cross-application launches require approved loader
//!   mediation): the `launchAppSpec` must name an `'APPL'` file on a volume of THIS machine (the bundle's own volumes) whose `cfrg` has a
//!   PowerPC application member, and the host's [`LaunchPolicy`] must admit it (default: admit, each admitted launch logged; `Refuse`
//!   answers memFullErr as one-process Heirloom did). The new process gets its own resource fork opened through the File Manager (an
//!   ordinary FCB refnum, the bottom of ITS resource chain), its fragment instantiated by the same CFM loader, and a task of its own
//!   ([`crate::sched`], `Kind::Process`) with a fresh stack that runs its libraries' init routines and its own (each returning to
//!   `$ProcStart`) and then `main` with LR = the ExitToShell glue. `launchProcessSN`, `launchPreferredSize` / `MinimumSize` /
//!   `AvailableSize` (from its `SIZE` -1) are filled. `launchDontSwitch` leaves the front process alone; without `launchContinue` the
//!   launcher ends (as ExitToShell). An application that already runs is not started twice: its serial number comes back.
//!   The heap zone is the machine's one zone (the SIZE partition is reported, not reserved: logged).
//! * `ExitToShell` (or `main` returning) of a process other than the first ends only that process: its tasks go, its resource files
//!   close, Apple Event senders waiting on it get `connectionInvalid` (-609). The first process (the one the host launched) ending ends
//!   the machine, as before. Windows a process leaves open are not disposed (named in the log: no window carries its owner yet).
//! * Events: [`MacPpc::hle_deliver`] gives `WaitNextEvent` / `GetNextEvent` the caller's own pending high-level event first (ae_c.rs
//!   queues them per process); a BACKGROUND process (not the front one) gets no user events - only its high-level events or the null
//!   event, and a sleeping WaitNextEvent of a background process idles (another task runs).
use crate::machine::MacPpc;
use crate::sched::{Kind, TState, Task};
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_pef::Pef;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// The pseudo-symbol a launched process's init routines return to.
pub const PROC_START: &str = "$ProcStart";
const PARAM_ERR: i32 = -50;
const FNF_ERR: i32 = -43;
const MEM_FULL: i32 = -108;
pub const CONNECTION_INVALID: i32 = -609;
const LAUNCH_CONTINUE: u16 = 0x4000;
const LAUNCH_DONT_SWITCH: u16 = 0x0200;

/// Whether a guest LaunchApplication may start another application in this machine (the host's decision; see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaunchPolicy {
    /// Applications on this machine's own volumes start as processes of this machine (each launch logged).
    #[default]
    AdmitOwnVolumes,
    /// No second process: LaunchApplication answers memFullErr (named in the log).
    Refuse,
}

/// A launched process's start: its init routines still to run (TVector, r3), then main.
pub(crate) struct ProcStart { inits: VecDeque<(crate::cfm::Tv, u32)>, main: crate::cfm::StartState, main_tvector: u32 }

impl<U: GuestUi> MacPpc<U> {
    /// The first process is in the table (its creator from its file) and the scheduler knows it: done before anything multi-process.
    pub(crate) fn ensure_processes(&mut self) {
        if self.mac.procs.table.is_empty() {
            let (ty, cr) = self.mac.fm.app.map(|n| { let f = &self.mac.fm.tree.get(n).finfo; (u32::from_be_bytes(f.file_type), u32::from_be_bytes(f.creator)) })
                .unwrap_or((u32::from_be_bytes(*b"APPL"), u32::from_be_bytes(*b"????")));
            self.mac.register_first_process(ty, cr);
        }
        self.ensure_sched();
    }

    /// The first (host-launched) process's serial number low word.
    pub(crate) fn first_psn(&self) -> u32 { self.mac.procs.table.keys().next().copied().unwrap_or(heirloom_mac68k_engine::processes::FIRST_PSN) }

    /// `LaunchApplication(pb)`: sets r3 (and may end the launcher). See the module doc.
    pub(crate) fn launch_application(&mut self, pb: u32, cpu: &mut Cpu) -> Result<(), String> {
        let r = self.launch(pb);
        let r = match r { Ok(v) => v, Err(e) => { cpu.gpr[3] = e as u32; return Ok(()) } };
        let (flags, psn) = r;
        cpu.gpr[3] = 0;
        if flags & LAUNCH_CONTINUE == 0 {
            self.mac.log.push(format!("[process] LaunchApplication without launchContinue: the launcher ends, process {psn:#x} goes on"));
            if !self.process_exit(cpu)? {
                self.mac.log.push(String::from("[process] the launcher is the first process: the machine ends with it"));
                cpu.halt();
            }
        }
        Ok(())
    }

    fn launch(&mut self, pb: u32) -> Result<(u16, u32), i32> {
        let m = &self.mac.mem;
        if pb == 0 || m.read_u16(pb + 6).unwrap_or(0) != 0x4C43 { return Err(PARAM_ERR) } // launchBlockID 'LC'
        let (flags, spec) = (m.read_u16(pb + 14).unwrap_or(0), m.read_u32(pb + 16).unwrap_or(0));
        let node = heirloom_mac68k_engine::fm::fsspec::spec_loc(&self.mac, spec).ok().and_then(|l| l.node);
        let Some(node) = node else { self.mac.log.push(String::from("[process] LaunchApplication: launchAppSpec names no file -> fnfErr")); return Err(FNF_ERR) };
        let shown: String = self.mac.fm.full_path(node).iter().map(|&b| char::from(b)).collect();
        if self.launch_policy == LaunchPolicy::Refuse {
            self.mac.log.push(format!("[process] LaunchApplication \"{shown}\" launchControlFlags {flags:#06x}: the host's launch policy refuses a second process -> memFullErr"));
            return Err(MEM_FULL);
        }
        let finfo = self.mac.fm.tree.get(node).finfo.clone();
        if &finfo.file_type != b"APPL" {
            self.mac.log.push(format!("[process] LaunchApplication \"{shown}\": type '{}' is not an application -> paramErr", String::from_utf8_lossy(&finfo.file_type)));
            return Err(PARAM_ERR);
        }
        self.ensure_processes();
        if let Some(psn) = self.mac.procs.by_app(node) {
            if flags & LAUNCH_DONT_SWITCH == 0 { self.mac.procs.front = psn; }
            self.mac.log.push(format!("[process] LaunchApplication \"{shown}\": already running as {{0,{psn:#x}}}"));
            self.write_launch_results(pb, psn, None);
            return Ok((flags, psn));
        }
        // the file's forks, as the CFM reads them
        let len = self.mac.fm.tree.file_len(node).unwrap_or(0) as usize;
        let mut data = alloc::vec![0u8; len];
        if self.mac.fm.tree.read_at(node, 0, &mut data) != Some(len) { return Err(FNF_ERR) }
        let rsrc = self.mac.fm.tree.get(node).rsrc.clone();
        let (frag_name, bytes) = match crate::source::fragment_bytes(&rsrc, Some(&data)) {
            Ok(v) => v,
            Err(e) => { self.mac.log.push(format!("[process] LaunchApplication \"{shown}\": not a PowerPC application ({e}) -> paramErr")); return Err(PARAM_ERR) }
        };
        let pef = Pef::parse(&bytes).map_err(|e| { self.mac.log.push(format!("[process] LaunchApplication \"{shown}\": {e}")); PARAM_ERR })?;
        let span = crate::source::fragment_span(&rsrc, Some(&data)).ok();
        let size = size_resource(&rsrc);
        // its resource fork, opened through the File Manager: the bottom of the new process's resource chain
        let loc = heirloom_mac68k_engine::fm::fsspec::spec_loc(&self.mac, spec).map_err(|e| i32::from(e))?;
        let res_ref = self.mac.open_res_file(&loc, 1).ok();
        let name: Vec<u8> = self.mac.fm.tree.name(node).chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
        let psn = self.mac.add_process(&name, u32::from_be_bytes(finfo.file_type), u32::from_be_bytes(finfo.creator), Some(node), res_ref);
        // its private libraries and the shared libraries in its own folder, then its fragment
        for (lib, lib_bytes) in crate::source::library_fragments(&rsrc, Some(&data)) { if !self.libs.contains_key(lib.as_str()) { self.provide_library(&lib, &lib_bytes); } }
        self.provide_libraries_in(self.mac.fm.tree.parent(node));
        let before = self.rt.pending_inits.len();
        let frag = match self.instantiate(&frag_name, &pef) {
            Ok(f) => f,
            Err(e) => {
                self.mac.log.push(format!("[process] LaunchApplication \"{shown}\": its fragment does not load ({e}) -> memFullErr"));
                self.mac.remove_process(psn);
                return Err(MEM_FULL);
            }
        };
        // the libraries it loaded run their init routines on ITS task, before its own
        let lib_inits: Vec<(crate::cfm::Tv, String, u32)> = self.rt.pending_inits.drain(before..).collect();
        let mut inits: VecDeque<(crate::cfm::Tv, u32)> = lib_inits.into_iter().map(|(tv, _, block)| (tv, block)).collect();
        if let Some(init) = frag.init { inits.push_back((init, self.init_block_for(node, span).unwrap_or(0))); }
        let Some(main) = frag.main else { self.mac.remove_process(psn); return Err(PARAM_ERR) };
        let stack = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, crate::cfm::STACK_SIZE, true).map_err(|_| MEM_FULL)?;
        let r1 = (stack + crate::cfm::STACK_SIZE - 64) & !0xF;
        let _ = self.mac.mem.write_u32(r1, 0);
        let exit = self.prepare_exit_stub().map_err(|_| MEM_FULL)?;
        let start = crate::cfm::StartState { pc: main.code, r1, r2: main.toc, lr: exit };
        let mut c = Cpu::new(main.code, r1);
        c.gpr[2] = main.toc;
        c.gpr[12] = main.address;
        c.lr = exit;
        if let Some((first, r3)) = inits.pop_front() {
            let id = self.glue.intern(&mut self.mac, "CFM", PROC_START).map_err(|_| MEM_FULL)?;
            self.bindings.insert(PROC_START.into(), crate::symbols::Binding::Native(crate::symbols::Native::InitDone));
            c.pc = first.code;
            c.gpr[2] = first.toc;
            c.gpr[12] = first.address;
            c.gpr[3] = r3;
            c.lr = self.glue.entry(id).map_or(0, |e| e.stub);
        }
        self.rt.proc_starts.insert(psn, ProcStart { inits, main: start, main_tvector: main.address });
        let id = self.rt.sched.next_id;
        self.rt.sched.next_id += 1;
        self.rt.sched.tasks.insert(id, Task { cpu: Some(c), callbacks: Vec::new(), state: TState::Ready, kind: Kind::Process, stack: Some(stack), result_ptr: 0, wait: None, woken: None, terminator: None, switch_in: None, switch_out: None, notify: None, bound: 0, psn });
        if flags & LAUNCH_DONT_SWITCH == 0 { self.mac.procs.front = psn; }
        self.write_launch_results(pb, psn, size);
        self.mac.log.push(format!("[process] LaunchApplication admitted: \"{shown}\" (fragment \"{frag_name}\") is process {{0,{psn:#x}}}, task {id}, launchControlFlags {flags:#06x}{}; its SIZE partition is reported, the heap zone is shared",
            if flags & LAUNCH_DONT_SWITCH != 0 { " (stays in the background)" } else { " (brought to the front)" }));
        Ok((flags, psn))
    }

    fn write_launch_results(&mut self, pb: u32, psn: u32, size: Option<(u32, u32)>) {
        let (pref, min) = size.unwrap_or((0, 0));
        let avail = self.mac.heap.available(false);
        for (off, v) in [(20, 0), (24, psn), (28, pref), (32, min), (36, avail)] { let _ = self.mac.mem.write_u32(pb + off, v); }
    }

    /// The ExitToShell glue stub (a launched process's main returns into it).
    fn prepare_exit_stub(&mut self) -> Result<u32, crate::error::CfmError> {
        self.bindings.entry("ExitToShell".into()).or_insert(crate::symbols::Binding::Native(crate::symbols::Native::Exit));
        let id = self.glue.intern(&mut self.mac, "InterfaceLib", "ExitToShell")?;
        Ok(self.glue.entry(id).map_or(0, |e| e.stub))
    }

    /// `$ProcStart` reached: a launched process's init routine returned (r3 = its OSErr). The next one runs, then main.
    pub(crate) fn proc_start_returned(&mut self, cpu: &mut Cpu, addr: u32) -> Result<(), String> {
        let psn = self.mac.current_psn();
        let status = cpu.gpr[3] as u16 as i16;
        if status != 0 {
            self.mac.log.push(format!("[process] process {{0,{psn:#x}}}: an init routine failed with OSErr {status}; the process ends before main"));
            self.rt.proc_starts.remove(&psn);
            if !self.process_exit(cpu)? { cpu.halt(); }
            return Ok(());
        }
        let Some(ps) = self.rt.proc_starts.get_mut(&psn) else { return Err(format!("$ProcStart reached by process {psn:#x}, which is not starting")) };
        if let Some((next, r3)) = ps.inits.pop_front() {
            cpu.gpr[2] = next.toc;
            cpu.gpr[12] = next.address;
            cpu.gpr[3] = r3;
            cpu.lr = addr;
            cpu.set_pc(next.code);
            return Ok(());
        }
        let ps = self.rt.proc_starts.remove(&psn).expect("checked");
        cpu.gpr[1] = ps.main.r1;
        cpu.gpr[2] = ps.main.r2;
        cpu.gpr[12] = ps.main_tvector;
        cpu.lr = ps.main.lr;
        cpu.set_pc(ps.main.pc);
        Ok(())
    }

    /// ExitToShell (or main returning) in the current task: a process other than the first ends alone and another task runs (true);
    /// false when it is the first process (the caller ends the machine).
    pub(crate) fn process_exit(&mut self, cpu: &mut Cpu) -> Result<bool, String> {
        if self.mac.procs.table.len() < 2 && self.rt.sched.tasks.values().all(|t| t.kind != Kind::Process) { return Ok(false) }
        let cur = self.rt.sched.current;
        let psn = self.rt.sched.tasks.get(&cur).map_or(self.mac.current_psn(), |t| t.psn);
        if psn == self.first_psn() {
            let others: Vec<String> = self.mac.procs.table.iter().filter(|(&k, _)| k != psn).map(|(k, p)| format!("{{0,{k:#x}}} \"{}\"", String::from_utf8_lossy(&p.name))).collect();
            if !others.is_empty() { self.mac.log.push(format!("[process] the first process ends: the machine ends with {}", others.join(", "))); }
            return Ok(false);
        }
        // its other tasks (threads) go; Apple Event senders waiting on it are told the connection is gone
        let mine: Vec<u32> = self.rt.sched.tasks.iter().filter(|(&id, t)| id != cur && t.psn == psn).map(|(&id, _)| id).collect();
        for id in mine { if let Some(t) = self.rt.sched.tasks.remove(&id) { if let Some(s) = t.stack { let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, s); } } }
        let in_flight: Vec<String> = self.callback_kinds();
        if !in_flight.is_empty() { self.mac.log.push(format!("[process] process {{0,{psn:#x}}} ends inside {} callback(s): {}", in_flight.len(), in_flight.join("; "))); }
        self.ae_process_gone(psn);
        if self.mac.procs.front == psn { self.mac.procs.front = self.first_psn(); }
        let Some(to) = self.runnable_after(cur, true) else {
            return Err(format!("process {{0,{psn:#x}}} ended and every remaining task waits with no deadline (a deadlock)"));
        };
        self.switch_to(cpu, to, None, 0)?;
        let name = self.mac.remove_process(psn).unwrap_or_default();
        self.mac.log.push(format!("[process] process {{0,{psn:#x}}} \"{}\" ended (ExitToShell); its resource files are closed; windows it left open stay (no window carries its owner yet)", String::from_utf8_lossy(&name)));
        Ok(true)
    }

    /// At an event call: should the current task give its turn to a ready task of ANOTHER process first? Once per call: the retried
    /// call (when the task is scheduled again) goes on.
    pub(crate) fn process_turn(&mut self) -> bool {
        let cur = self.rt.sched.current;
        if self.rt.turn_given.remove(&cur) { return false }
        let Some(psn) = self.rt.sched.tasks.get(&cur).map(|t| t.psn) else { return false };
        if !self.rt.sched.tasks.values().any(|t| t.state == TState::Ready && t.psn != psn) { return false }
        self.rt.turn_given.insert(cur);
        true
    }

    /// Is the running process in the background (another process is in front)?
    pub(crate) fn in_background(&self) -> bool { self.mac.procs.table.len() > 1 && self.mac.current_psn() != self.mac.procs.front }

    /// WaitNextEvent / GetNextEvent / EventAvail: the caller's own pending high-level event, written to `rec` (taken off the queue unless
    /// `peek`). True when one was delivered.
    pub(crate) fn hle_deliver(&mut self, mask: u16, rec: u32, peek: bool) -> bool {
        const HIGH_LEVEL_EVENT_MASK: u16 = 0x0400;
        if mask & HIGH_LEVEL_EVENT_MASK == 0 { return false }
        let psn = self.mac.current_psn();
        let Some(q) = self.rt.ae.queues.get_mut(&psn) else { return false };
        let Some(h) = (if peek { q.front().cloned() } else { q.pop_front() }) else { return false };
        let ticks = self.mac.ev.ticks;
        let m = &mut self.mac.mem;
        let w = m.write_u16(rec, 23).and_then(|_| m.write_u32(rec + 2, h.class)).and_then(|_| m.write_u32(rec + 6, ticks))
            .and_then(|_| m.write_u32(rec + 10, h.id)).and_then(|_| m.write_u16(rec + 14, 0));
        if w.is_err() { return false }
        self.mac.log.push(format!("[ae] {} the high-level event '{}'/'{}' to process {{0,{psn:#x}}}", if peek { "EventAvail shows" } else { "delivered" },
            heirloom_mac68k_engine::aedesc::tag(h.class), heirloom_mac68k_engine::aedesc::tag(h.id)));
        if !peek { self.rt.ae.delivered.insert(psn, h); }
        true
    }

    /// The null event (a background process gets no user events).
    pub(crate) fn null_event(&mut self, rec: u32) {
        let ticks = self.mac.ev.ticks;
        let m = &mut self.mac.mem;
        let _ = m.write_u16(rec, 0).and_then(|_| m.write_u32(rec + 2, 0)).and_then(|_| m.write_u32(rec + 6, ticks)).and_then(|_| m.write_u32(rec + 10, 0)).and_then(|_| m.write_u16(rec + 14, 0));
    }

    /// The `CFragInitBlock` for an application file `node` whose fragment lies at `span` of its data fork (a launched process's init routine).
    fn init_block_for(&mut self, node: heirloom_volume::NodeId, span: Option<(u32, u32)>) -> Result<u32, crate::error::CfmError> {
        let saved = (self.mac.fm.app, self.app_span);
        self.mac.fm.app = Some(node);
        self.app_span = span;
        let r = self.app_init_block();
        (self.mac.fm.app, self.app_span) = saved;
        r
    }
}

/// `SIZE` -1 (else 0): (preferred, minimum) partition bytes.
fn size_resource(rsrc: &[u8]) -> Option<(u32, u32)> {
    let fork = heirloom_fmt_resfork::ResFork::parse(rsrc.to_vec()).ok()?;
    let i = fork.find(*b"SIZE", -1).or_else(|| fork.find(*b"SIZE", 0))?;
    let d = fork.data(i)?;
    if d.len() < 10 { return None }
    Some((u32::from_be_bytes([d[2], d[3], d[4], d[5]]), u32::from_be_bytes([d[6], d[7], d[8], d[9]])))
}
