//! The Process Manager's PROCESS TABLE (Inside Macintosh: Processes): the applications one machine runs, cooperatively, the way
//! Mac OS 9 runs them - one address space, one heap zone, one File Manager and window list, but per process the things the real
//! Process Manager switches: the resource chain above the System file and CurResFile, `CurApName`/`CurApRefNum`, the
//! application's own file (the File Manager's `app`, which `processAppSpec` names), the Menu Manager's menu list, QuickDraw's
//! globals pointer and `thePort`, and the Apple Event handler table. [`Mac::switch_process`] saves the outgoing process's set
//! and restores the incoming one's; the ENGINE decides when (the PowerPC engine's scheduler, when a task of another process gets
//! the CPU). The 68k engine runs one process and never switches.
//!
//! Serial numbers are real ones: the first process is `{0, FIRST_PSN}`, each launch the next; `kCurrentProcess` `{0, 2}` only
//! ever means "the caller" and `kNoProcess` `{0, 0}` starts an iteration. The table is a growable map (no process limit); an
//! unknown serial number is `procNotFound` (-600).
use crate::lowmem::{CUR_AP_NAME, CUR_AP_REF_NUM};
use crate::machine::Mac;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_volume::NodeId;

/// The low word of the first process's serial number.
pub const FIRST_PSN: u32 = 0x2000;
/// `kCurrentProcess`.
pub const CURRENT_PROCESS: (u32, u32) = (0, 2);
pub const NO_PROCESS: (u32, u32) = (0, 0);

/// What the real Process Manager swaps when another process runs (see the module doc).
#[derive(Default)]
pub struct Context {
    res_files: Vec<crate::resmgr::ResFile>,
    cur_res: i16,
    cur_ap_name: [u8; 32],
    cur_ap_ref_num: u16,
    app: Option<NodeId>,
    menus: crate::menu::MenuMgr,
    qd_globals: u32,
    the_port: u32,
    ae: crate::appleevents::AppleEvents,
}

pub struct Process {
    pub name: Vec<u8>,
    pub file_type: u32,
    pub creator: u32,
    /// The application's file (None when started from memory).
    pub app: Option<NodeId>,
    pub launcher: (u32, u32),
    pub launch_tick: u32,
    /// The saved context while another process runs (None for the running one).
    saved: Option<Context>,
}

#[derive(Default)]
pub struct Processes {
    /// low word of the serial number -> process. Empty until the first process is registered (the 68k engine's one process then
    /// answers as `FIRST_PSN` through [`Mac::current_psn`]).
    pub table: BTreeMap<u32, Process>,
    pub current: u32,
    pub front: u32,
    next: u32,
}

impl Processes {
    pub fn get(&self, low: u32) -> Option<&Process> { self.table.get(&low) }
    pub fn len(&self) -> usize { self.table.len().max(1) }
    pub fn is_empty(&self) -> bool { false }
    /// The process whose application file is `node`.
    pub fn by_app(&self, node: NodeId) -> Option<u32> { self.table.iter().find(|(_, p)| p.app == Some(node)).map(|(&k, _)| k) }
    /// The process whose creator signature is `sig` (the lowest serial number when several run).
    pub fn by_creator(&self, sig: u32) -> Option<u32> { self.table.iter().find(|(_, p)| p.creator == sig).map(|(&k, _)| k) }
}

impl<U: GuestUi> Mac<U> {
    /// The running process's serial number low word.
    pub fn current_psn(&self) -> u32 { if self.procs.table.is_empty() { FIRST_PSN } else { self.procs.current } }
    pub fn front_psn(&self) -> u32 { if self.procs.table.is_empty() { FIRST_PSN } else { self.procs.front } }

    /// Resolve a guest serial number: kCurrentProcess is the caller; None for one that is not running.
    pub fn resolve_psn(&self, p: (u32, u32)) -> Option<u32> {
        if p == CURRENT_PROCESS { return Some(self.current_psn()) }
        if p.0 != 0 { return None }
        if self.procs.table.is_empty() { return (p.1 == FIRST_PSN).then_some(FIRST_PSN) }
        self.procs.table.contains_key(&p.1).then_some(p.1)
    }

    /// The process after `low` in serial-number order (GetNextProcess), None at the end.
    pub fn next_psn(&self, low: Option<u32>) -> Option<u32> {
        if self.procs.table.is_empty() { return low.is_none().then_some(FIRST_PSN) }
        match low { None => self.procs.table.keys().next().copied(), Some(l) => self.procs.table.range(l + 1..).next().map(|(&k, _)| k) }
    }

    /// Register the machine's first process (the application the host launched), running and in front: its name from CurApName,
    /// its file the File Manager's `app`. Idempotent.
    pub fn register_first_process(&mut self, file_type: u32, creator: u32) -> u32 {
        if !self.procs.table.is_empty() { return self.procs.current }
        let name = crate::pascal::read_pstring_bytes(&self.mem, CUR_AP_NAME).map(|b| b.to_vec()).unwrap_or_default();
        let tick = self.mem.read_u32(crate::lowmem::TICKS).unwrap_or(0);
        self.procs.table.insert(FIRST_PSN, Process { name, file_type, creator, app: self.fm.app, launcher: NO_PROCESS, launch_tick: tick, saved: None });
        self.procs.current = FIRST_PSN;
        self.procs.front = FIRST_PSN;
        self.procs.next = FIRST_PSN + 1;
        FIRST_PSN
    }

    /// A new process (not yet running): its context starts as a fresh one - the System file only in its resource chain plus
    /// `res_ref` (its own application resource file, already opened through the File Manager), CurApName = `name`.
    pub fn add_process(&mut self, name: &[u8], file_type: u32, creator: u32, app: Option<NodeId>, res_ref: Option<i16>) -> u32 {
        if self.procs.table.is_empty() { self.register_first_process(u32::from_be_bytes(*b"APPL"), u32::from_be_bytes(*b"????")); }
        let low = self.procs.next;
        self.procs.next += 1;
        let mut cur_ap_name = [0u8; 32];
        let n = name.len().min(31);
        cur_ap_name[0] = n as u8;
        cur_ap_name[1..=n].copy_from_slice(&name[..n]);
        // the new process's own resource file leaves the running chain: it belongs to the new process's chain only
        let res_files = match res_ref {
            Some(r) => match self.rm.files.iter().position(|f| f.ref_num == r) {
                Some(i) => { let f = self.rm.files.remove(i); if self.rm.cur == r { self.rm.cur = self.rm.files.last().map_or(0, |f| f.ref_num); } alloc::vec![f] }
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        let ctx = Context { res_files, cur_res: res_ref.unwrap_or(crate::resmgr::SYSTEM_REF_NUM), cur_ap_name, cur_ap_ref_num: res_ref.unwrap_or(0) as u16, app,
            menus: crate::menu::MenuMgr::default(), qd_globals: self.qd_globals, the_port: 0, ae: crate::appleevents::AppleEvents::default() };
        let tick = self.mem.read_u32(crate::lowmem::TICKS).unwrap_or(0);
        let launcher = (0, self.current_psn());
        self.procs.table.insert(low, Process { name: name.to_vec(), file_type, creator, app, launcher, launch_tick: tick, saved: Some(ctx) });
        low
    }

    fn take_context(&mut self) -> Context {
        let sys = self.rm.files.iter().position(|f| f.ref_num == crate::resmgr::SYSTEM_REF_NUM);
        let res_files: Vec<_> = self.rm.files.drain(..).collect();
        let (system, mine): (Vec<_>, Vec<_>) = res_files.into_iter().enumerate().partition(|(i, _)| Some(*i) == sys);
        self.rm.files = system.into_iter().map(|(_, f)| f).collect();
        let mut cur_ap_name = [0u8; 32];
        for (i, b) in cur_ap_name.iter_mut().enumerate() { *b = self.mem.read_u8(CUR_AP_NAME + i as u32).unwrap_or(0); }
        let the_port = if self.qd_globals != 0 { self.mem.read_u32(self.qd_globals).unwrap_or(0) } else { 0 };
        Context {
            res_files: mine.into_iter().map(|(_, f)| f).collect(),
            cur_res: self.rm.cur,
            cur_ap_name,
            cur_ap_ref_num: self.mem.read_u16(CUR_AP_REF_NUM).unwrap_or(0),
            app: self.fm.app,
            menus: core::mem::take(&mut self.menus),
            qd_globals: self.qd_globals,
            the_port,
            ae: core::mem::take(&mut self.ae),
        }
    }

    fn put_context(&mut self, c: Context) {
        self.rm.files.extend(c.res_files);
        self.rm.cur = c.cur_res;
        let _ = self.mem.write_bytes(CUR_AP_NAME, &c.cur_ap_name);
        let _ = self.mem.write_u16(CUR_AP_REF_NUM, c.cur_ap_ref_num);
        self.fm.app = c.app;
        self.menus = c.menus;
        self.qd_globals = c.qd_globals;
        if self.qd_globals != 0 && c.the_port != 0 { let _ = self.mem.write_u32(self.qd_globals, c.the_port); }
        self.ae = c.ae;
    }

    /// Make process `to` the running one: the running process's context is saved, `to`'s restored. No-op when it already runs.
    pub fn switch_process(&mut self, to: u32) -> Result<(), alloc::string::String> {
        let cur = self.current_psn();
        if to == cur { return Ok(()) }
        if !self.procs.table.get(&to).is_some_and(|p| p.saved.is_some()) { return Err(alloc::format!("switch to process {to:#x}: not a waiting process")) }
        let ctx = self.take_context();
        if let Some(p) = self.procs.table.get_mut(&cur) { p.saved = Some(ctx); }
        let next = self.procs.table.get_mut(&to).and_then(|p| p.saved.take()).expect("checked");
        self.put_context(next);
        self.procs.current = to;
        Ok(())
    }

    /// Process `low` is over (it is not the running one): its resource files are closed, it leaves the table. Returns its name.
    pub fn remove_process(&mut self, low: u32) -> Option<Vec<u8>> {
        let p = self.procs.table.remove(&low)?;
        if let Some(c) = p.saved {
            for f in &c.res_files { if f.ref_num != crate::resmgr::SYSTEM_REF_NUM { let _ = self.fm.close(f.ref_num); } }
            // the handles loaded from them stop being resources
            for f in c.res_files { self.rm.files.push(f); let r = self.rm.files.last().map_or(0, |f| f.ref_num); let cur = self.rm.cur; self.rm.close(r); self.rm.cur = cur; }
        }
        if self.procs.front == low { self.procs.front = self.procs.current; }
        Some(p.name)
    }
}
