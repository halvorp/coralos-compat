//! The Carbon Event Manager core over the engine's own window/input path (`CarbonLib`; CarbonEvents.h), and the Carbon window calls a program
//! needs to open a window. Events are the classic queue's events (`WaitNextEvent`'s: the GuestUi pump) converted to Carbon events and dispatched to
//! the program's handlers, which are PowerPC routines run through the callback machine ([`crate::callback`]); windows are the classic Window Manager's,
//! so a window a Carbon program opens is the same native window a classic one is.
//!
//! * **Targets** are opaque ids: the application target 3 and the event dispatcher target 4 (as Systemless numbers them; arbitrary), a window's target is
//!   `window + 1` (a window pointer is even). A handler installed on a window runs before the application's for that window's events (the event
//!   bubbles window -> application). `InstallEventHandler(target, UPP-or-TVector, n, {class, kind}[], userData, &ref)`.
//! * **Dispatch**: each matching handler is called `OSStatus h(EventHandlerCallRef next, EventRef event, void *userData)`; `eventNotHandledErr`
//!   (-9874) passes to the next, any other result ends it. `CallNextEventHandler(next, event)` runs the REST of the chain from inside a handler. When the chain is exhausted
//!   unhandled, a window with the STANDARD handler (`kWindowStandardHandlerAttribute` or `InstallStandardEventHandler`) gets its default: `kEventWindowClose` disposes the
//!   window, `kEventWindowUpdate` is `BeginUpdate`/`EndUpdate`. Nothing else has a default.
//! * **Conversions** (classic -> Carbon): mouseDown in content -> `mous`/1 (window = the one hit), in the close box -> `wind`/72 `kEventWindowClose`; mouseUp -> `mous`/2;
//!   keyDown/autoKey/keyUp -> `keyb`/1,2,3 for the front window; updateEvt -> `wind`/1; activateEvt -> `wind`/5 or 6; suspend/resume -> `appl`/2 or 1.
//!   NOT converted (dropped, logged): menu choices (`kEventCommandProcess` needs `SetMenuItemCommandID`, which is not provided), drags and grows (the native window manager
//!   does them), high-level Apple events.
//! * `ReceiveNextEvent` pulls from the same queue; `pullEvent` = false (peek) and a type filter are NOT honoured and refuse/are ignored by name (see the function).
//! * The class/kind/parameter four-character codes and numbers are CarbonEvents.h's as I know them (mous 1/2, keyb 1/2/3, wind 1/5/6/72/73, appl 1/2; '----' 'mloc' 'mbtn' 'kmod'
//!   'kcod' 'kchr' 'ccnt'); eventNotHandledErr -9874 and eventLoopTimedOutErr -9875 are confirmed by the Systemless importer's constants, -9870 eventParameterNotFoundErr is from the
//!   header. No Carbon header is in the reference tree to check the rest against.
//! * Checked against the Mac OS 9.2.1 CD's CarbonLib 1.4 (docs/consults/2026-10-24-macos921-oracle.md): its static EventTypeSpec tables carry exactly these classes and kinds, and the
//!   point type is `'QDpt'` (typeQDPoint), not `'QDPt'`.
//! * Window classes: `kDocumentWindowClass` (6) maps to the classic document procIDs (zoom+grow 8, grow 0, none 4); plain (13) 2; alert/modal (1..4) the dialog frames 1 and 5; floating (5) and
//!   utility (8) are presented as document windows without a grow box (the seam has no floating role yet -- logged on creation). Any other class is `paramErr`.
use crate::callback::{Continuation, Frame, Saved};
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::GuestUi;
use mac68k_engine::mixed::trap_opcode;
use mac68k_engine::toolbox::WindowSpec;
use mac68k_engine::pascal::Rect;
use retro_ppc::Cpu;

pub const APP_TARGET: u32 = 3;
pub const DISPATCHER_TARGET: u32 = 4;
const NO_ERR: i32 = 0;
const PARAM_ERR: i32 = -50;
pub const EVENT_NOT_HANDLED: i32 = -9874;
pub const EVENT_LOOP_TIMED_OUT: i32 = -9875;
pub const EVENT_PARAMETER_NOT_FOUND: i32 = -9870;
const COERCION_FAIL: i32 = -1700;
const ID_BASE: u32 = 0x0050_0000;
const STANDARD_HANDLER_ATTR: u32 = 1 << 25;
const fn fc(b: &[u8; 4]) -> u32 { u32::from_be_bytes(*b) }
const C_MOUSE: u32 = fc(b"mous"); const C_KEY: u32 = fc(b"keyb"); const C_WINDOW: u32 = fc(b"wind"); const C_APP: u32 = fc(b"appl");
const K_WINDOW_UPDATE: u32 = 1; const K_WINDOW_ACTIVATED: u32 = 5; const K_WINDOW_DEACTIVATED: u32 = 6; const K_WINDOW_CLOSE: u32 = 72; const K_WINDOW_CLOSED: u32 = 73;
const P_DIRECT: u32 = fc(b"----"); const P_MLOC: u32 = fc(b"mloc"); const P_MBTN: u32 = fc(b"mbtn"); const P_KMOD: u32 = fc(b"kmod"); const P_KCOD: u32 = fc(b"kcod");
const P_KCHR: u32 = fc(b"kchr"); const P_CCNT: u32 = fc(b"ccnt");
const T_WINDOW: u32 = fc(b"wind"); const T_QDPOINT: u32 = fc(b"QDpt"); const T_MBTN: u32 = fc(b"mbtn"); const T_UINT32: u32 = fc(b"magn"); const T_CHAR: u32 = fc(b"TEXT");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarbonFn {
    GetApplicationEventTarget, GetWindowEventTarget, GetEventDispatcherTarget, InstallEventHandler, RemoveEventHandler, InstallStandardEventHandler,
    GetEventClass, GetEventKind, GetEventParameter, SetEventParameter, CreateEvent, RetainEvent, ReleaseEvent, SendEventToEventTarget, CallNextEventHandler,
    ReceiveNextEvent, RunApplicationEventLoop, QuitApplicationEventLoop, GetMainEventQueue, GetCurrentEventQueue, GetCurrentEventTime,
    CreateNewWindow, SetWindowTitleWithCFString, GetWindowPort, SetPortWindowPort,
}
impl CarbonFn {
    pub fn by_name(name: &str) -> Option<CarbonFn> {
        use CarbonFn::*;
        Some(match name {
            "GetApplicationEventTarget" => GetApplicationEventTarget, "GetWindowEventTarget" => GetWindowEventTarget, "GetEventDispatcherTarget" => GetEventDispatcherTarget,
            "InstallEventHandler" => InstallEventHandler, "RemoveEventHandler" => RemoveEventHandler, "InstallStandardEventHandler" => InstallStandardEventHandler,
            "GetEventClass" => GetEventClass, "GetEventKind" => GetEventKind, "GetEventParameter" => GetEventParameter, "SetEventParameter" => SetEventParameter,
            "CreateEvent" => CreateEvent, "RetainEvent" => RetainEvent, "ReleaseEvent" => ReleaseEvent, "SendEventToEventTarget" => SendEventToEventTarget,
            "CallNextEventHandler" => CallNextEventHandler, "ReceiveNextEvent" => ReceiveNextEvent, "RunApplicationEventLoop" => RunApplicationEventLoop,
            "QuitApplicationEventLoop" => QuitApplicationEventLoop, "GetMainEventQueue" => GetMainEventQueue, "GetCurrentEventQueue" => GetCurrentEventQueue,
            "GetCurrentEventTime" => GetCurrentEventTime, "CreateNewWindow" => CreateNewWindow, "SetWindowTitleWithCFString" => SetWindowTitleWithCFString,
            "GetWindowPort" => GetWindowPort, "SetPortWindowPort" => SetPortWindowPort,
            _ => return None,
        })
    }
}

struct Handler { id: u32, target: u32, tv: u32, specs: Vec<(u32, u32)>, user: u32 }
#[derive(Clone)]
struct Event { class: u32, kind: u32, window: u32, params: BTreeMap<u32, (u32, Vec<u8>)>, refs: u32 }
/// A chain being walked: (tv, userData) of each handler, the next to call, and the window whose standard handler gives the default.
struct Chain { handlers: Vec<(u32, u32)>, next: usize, std_window: u32 }
#[derive(Default)]
pub(crate) struct CarbonState { handlers: Vec<Handler>, events: BTreeMap<u32, Event>, chains: BTreeMap<u32, Chain>, std_windows: BTreeSet<u32>, next: u32, quit: bool, scratch: u32 }

/// What a finished dispatch does: re-enter the event loop that started it, or hand its result to the caller of `SendEventToEventTarget`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Then { Reenter, Return }
#[derive(Debug, Clone)]
pub(crate) struct Dispatch { chain: u32, event: u32, then: Then }

impl<U: GuestUi> MacPpc<U> {
    fn cb_id(&mut self) -> u32 { self.rt.carbon.next += 1; ID_BASE + 4 * self.rt.carbon.next }

    fn cb_scratch(&mut self) -> Result<u32, String> {
        if self.rt.carbon.scratch == 0 { self.rt.carbon.scratch = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 64, true).map_err(|e| format!("Carbon: no memory (memory error {e})"))?; }
        Ok(self.rt.carbon.scratch)
    }

    fn new_event(&mut self, class: u32, kind: u32, window: u32) -> u32 {
        let id = self.cb_id();
        self.rt.carbon.events.insert(id, Event { class, kind, window, params: BTreeMap::new(), refs: 1 });
        id
    }
    fn put_param(&mut self, ev: u32, name: u32, ty: u32, data: Vec<u8>) { if let Some(e) = self.rt.carbon.events.get_mut(&ev) { e.params.insert(name, (ty, data)); } }

    /// The handlers of `targets`, in order, that listen for (class, kind).
    fn matching(&self, targets: &[u32], class: u32, kind: u32) -> Vec<(u32, u32)> {
        targets.iter().flat_map(|t| self.rt.carbon.handlers.iter().filter(move |h| h.target == *t && h.specs.contains(&(class, kind)))).map(|h| (h.tv, h.user)).collect()
    }
    fn chain_for(&self, ev: u32, target: u32) -> Vec<u32> {
        let e = &self.rt.carbon.events[&ev];
        let win = if target & 1 == 1 && target != APP_TARGET { target - 1 } else { e.window };
        let mut t = Vec::new();
        if target != APP_TARGET && win != 0 { t.push(win + 1); }
        t.push(APP_TARGET);
        t
    }

    /// A classic event (the EventRecord at `rec`) as a Carbon event, if it has a conversion.
    fn convert(&mut self, rec: u32) -> Result<Option<u32>, String> {
        let m = |e: retro_guest_mem::MemError| format!("the EventRecord at {rec:#x}: {e}");
        let (what, message, v, h, mods) = (self.mac.mem.read_u16(rec).map_err(m)?, self.mac.mem.read_u32(rec + 2).map_err(m)?, self.mac.mem.read_i16(rec + 10).map_err(m)?, self.mac.mem.read_i16(rec + 12).map_err(m)?, self.mac.mem.read_u16(rec + 14).map_err(m)?);
        let front = self.toolbox_call("FrontWindow", &[]);
        let point = |v: i16, h: i16| { let mut b = v.to_be_bytes().to_vec(); b.extend(h.to_be_bytes()); b };
        let ev = match what {
            1 | 2 => {
                let mut window = front;
                if what == 1 {
                    let sc = self.cb_scratch()?;
                    let part = self.toolbox_call("FindWindow", &[(v as u16 as u32) << 16 | h as u16 as u32, sc]);
                    window = self.mac.mem.read_u32(sc).unwrap_or(0);
                    if part == 6 { // inGoAway
                        let e = self.new_event(C_WINDOW, K_WINDOW_CLOSE, window);
                        self.put_param(e, P_DIRECT, T_WINDOW, window.to_be_bytes().to_vec());
                        return Ok(Some(e));
                    }
                    if part != 3 { self.mac.log.push(format!("[carbon] mouseDown in part {part} dropped (the native window manager handles it)")); return Ok(None); }
                }
                let e = self.new_event(C_MOUSE, what as u32, window);
                self.put_param(e, P_MLOC, T_QDPOINT, point(v, h));
                self.put_param(e, P_MBTN, T_MBTN, 1u16.to_be_bytes().to_vec());
                self.put_param(e, P_KMOD, T_UINT32, u32::from(mods).to_be_bytes().to_vec());
                self.put_param(e, P_CCNT, T_UINT32, 1u32.to_be_bytes().to_vec());
                self.put_param(e, P_DIRECT, T_WINDOW, window.to_be_bytes().to_vec());
                e
            }
            3 | 4 | 5 => {
                let kind = match what { 3 => 1, 5 => 2, _ => 3 };
                let e = self.new_event(C_KEY, kind, front);
                self.put_param(e, P_KCOD, T_UINT32, ((message >> 8) & 0xFF).to_be_bytes().to_vec());
                self.put_param(e, P_KCHR, T_CHAR, alloc::vec![message as u8]);
                self.put_param(e, P_KMOD, T_UINT32, u32::from(mods).to_be_bytes().to_vec());
                e
            }
            6 => { let e = self.new_event(C_WINDOW, K_WINDOW_UPDATE, message); self.put_param(e, P_DIRECT, T_WINDOW, message.to_be_bytes().to_vec()); e }
            8 => {
                let kind = if mods & 1 != 0 { K_WINDOW_ACTIVATED } else { K_WINDOW_DEACTIVATED };
                let e = self.new_event(C_WINDOW, kind, message);
                self.put_param(e, P_DIRECT, T_WINDOW, message.to_be_bytes().to_vec());
                e
            }
            15 if message >> 24 == 1 => self.new_event(C_APP, if message & 1 != 0 { 1 } else { 2 }, 0),
            0 => return Ok(None),
            other => { self.mac.log.push(format!("[carbon] classic event {other} has no Carbon conversion: dropped")); return Ok(None); }
        };
        Ok(Some(ev))
    }

    /// Call a Pascal Toolbox trap by name with value arguments; its result (0 when the call cannot be made).
    fn toolbox_call(&mut self, name: &str, args: &[u32]) -> u32 {
        match trap_opcode(name) { Some(op) => self.mac.pascal_call(op, args).map_or(0, |r| r.value), None => 0 }
    }

    /// Begin walking the handlers for `ev` (a Carbon event) on `targets`; the interrupted call resumes at `pc` when the chain is done.
    fn start_dispatch(&mut self, cpu: &mut Cpu, ev: u32, targets: &[u32], then: Then, pc: u32) -> Result<(), String> {
        let (class, kind, window) = { let e = &self.rt.carbon.events[&ev]; (e.class, e.kind, e.window) };
        let handlers = self.matching(targets, class, kind);
        let std_window = if self.rt.carbon.std_windows.contains(&window) { window } else { 0 };
        let chain = self.cb_id();
        self.rt.carbon.chains.insert(chain, Chain { handlers, next: 0, std_window });
        let frame = Frame { saved: Saved::of(cpu, pc), cont: Continuation::Carbon(Dispatch { chain, event: ev, then }) };
        self.dispatch_step(cpu, frame, None)
    }

    pub(crate) fn dispatch_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let r = cpu.gpr[3] as i32;
        self.dispatch_step(cpu, frame, Some(r))
    }

    fn dispatch_step(&mut self, cpu: &mut Cpu, frame: Frame, result: Option<i32>) -> Result<(), String> {
        let Continuation::Carbon(d) = &frame.cont else { return Err(String::from("a Carbon dispatch step on another continuation")) };
        let (chain, ev) = (d.chain, d.event);
        if let Some(r) = result { if r != EVENT_NOT_HANDLED { return self.dispatch_done(cpu, frame, r); } }
        let (call, std_window) = match self.rt.carbon.chains.get_mut(&chain) {
            Some(c) => { let call = c.handlers.get(c.next).copied(); if call.is_some() { c.next += 1; } (call, c.std_window) }
            None => return Err(String::from("Carbon dispatch: the handler chain is gone")),
        };
        match call {
            Some((tv, user)) => {
                let sp = frame.saved.gpr[1];
                self.enter_tvector(cpu, tv, &[chain, ev, user], sp).map_err(|e| format!("Carbon event handler: {e}"))?;
                self.rt.callbacks.push(frame);
                Ok(())
            }
            None => {
                let (class, kind) = { let e = &self.rt.carbon.events[&ev]; (e.class, e.kind) };
                let handled = std_window != 0 && match (class, kind) {
                    (C_WINDOW, K_WINDOW_CLOSE) => {
                        let closed = self.new_event(C_WINDOW, K_WINDOW_CLOSED, std_window);
                        self.put_param(closed, P_DIRECT, T_WINDOW, std_window.to_be_bytes().to_vec());
                        self.toolbox_call("DisposeWindow", &[std_window]);
                        self.rt.carbon.std_windows.remove(&std_window);
                        self.rt.carbon.events.remove(&closed);
                        self.mac.log.push(format!("[carbon] standard handler: kEventWindowClose disposed window {std_window:#x}"));
                        true
                    }
                    (C_WINDOW, K_WINDOW_UPDATE) => { self.toolbox_call("BeginUpdate", &[std_window]); self.toolbox_call("EndUpdate", &[std_window]); true }
                    _ => false,
                };
                self.dispatch_done(cpu, frame, if handled { NO_ERR } else { EVENT_NOT_HANDLED })
            }
        }
    }

    fn dispatch_done(&mut self, cpu: &mut Cpu, frame: Frame, r: i32) -> Result<(), String> {
        let Continuation::Carbon(d) = &frame.cont else { return Err(String::from("Carbon dispatch done on another continuation")) };
        self.rt.carbon.chains.remove(&d.chain);
        if let Then::Reenter = d.then { self.rt.carbon.events.remove(&d.event); }
        frame.saved.restore(cpu);
        if let Then::Return = d.then { cpu.gpr[3] = r as u32; }
        Ok(())
    }

    /// Park the call (retry at `addr` when an event may have arrived) or run another task meanwhile.
    fn park(&mut self, cpu: &mut Cpu, addr: u32, deadline: Option<u32>) {
        self.rt.wait_mask = Some(0xFFFF);
        self.rt.wait_deadline = deadline;
        if !self.idle_yield(cpu, addr) { cpu.block(); cpu.set_pc(addr); }
    }

    /// Pull one classic event as a Carbon event (None: nothing pending).
    fn pull_event(&mut self) -> Result<Option<u32>, String> {
        let rec = self.cb_scratch()? + 16;
        let Some(op) = trap_opcode("WaitNextEvent") else { return Err(String::from("WaitNextEvent has no trap word")) };
        let r = self.mac.pascal_call(op, &[0xFFFF, rec, 0, 0]).map_err(|e| format!("event pull: {e}"))?;
        if r.value == 0 { return Ok(None); }
        self.convert(rec)
    }

    /// Serve `f`; `addr` is the glue stub the CPU called (a parked call retries there).
    pub(crate) fn carbon_call(&mut self, f: CarbonFn, cpu: &mut Cpu, addr: u32) -> Result<(), String> {
        use CarbonFn::*;
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let ret = cpu.lr & !3;
        let r: i32 = match f {
            GetApplicationEventTarget => APP_TARGET as i32,
            GetEventDispatcherTarget => DISPATCHER_TARGET as i32,
            GetWindowEventTarget => (a0 + 1) as i32,
            GetMainEventQueue | GetCurrentEventQueue => 1,
            GetCurrentEventTime => { cpu.fpr[1] = (f64::from(self.mac.ev.ticks) / 60.0).to_bits(); return Ok(()); }
            InstallEventHandler => {
                let (target, handler, n, list, user, out) = (a0, a1, a2, a3, a(self, 4)?, a(self, 5)?);
                let ok_target = target == APP_TARGET || target == DISPATCHER_TARGET || (target & 1 == 1 && self.mac.windows.contains_key(&(target - 1)));
                if !ok_target { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()); }
                let mut specs = Vec::new();
                for i in 0..n { specs.push((self.mac.mem.read_u32(list + 8 * i).map_err(|_| String::from("InstallEventHandler: the EventTypeSpec list leaves guest RAM"))?, self.mac.mem.read_u32(list + 8 * i + 4).map_err(|_| String::from("InstallEventHandler: the EventTypeSpec list leaves guest RAM"))?)); }
                let tv = self.proc_tvector(handler)?;
                let id = self.cb_id();
                self.rt.carbon.handlers.push(Handler { id, target: if target == DISPATCHER_TARGET { APP_TARGET } else { target }, tv, specs, user });
                self.put32(out, id, "InstallEventHandler")?;
                0
            }
            RemoveEventHandler => { let n = self.rt.carbon.handlers.len(); self.rt.carbon.handlers.retain(|h| h.id != a0); if self.rt.carbon.handlers.len() < n { 0 } else { PARAM_ERR } }
            InstallStandardEventHandler => {
                if a0 == APP_TARGET || a0 == DISPATCHER_TARGET { 0 }
                else if a0 & 1 == 1 && self.mac.windows.contains_key(&(a0 - 1)) { self.rt.carbon.std_windows.insert(a0 - 1); 0 }
                else { PARAM_ERR }
            }
            GetEventClass => match self.rt.carbon.events.get(&a0) { Some(e) => { cpu.gpr[3] = e.class; return Ok(()); } None => return Err(format!("GetEventClass: {a0:#x} is not an EventRef this engine made")) },
            GetEventKind => match self.rt.carbon.events.get(&a0) { Some(e) => { cpu.gpr[3] = e.kind; return Ok(()); } None => return Err(format!("GetEventKind: {a0:#x} is not an EventRef this engine made")) },
            RetainEvent => { if let Some(e) = self.rt.carbon.events.get_mut(&a0) { e.refs += 1; } cpu.gpr[3] = a0; return Ok(()); }
            ReleaseEvent => { if let Some(e) = self.rt.carbon.events.get_mut(&a0) { e.refs -= 1; if e.refs == 0 { self.rt.carbon.events.remove(&a0); } } return Ok(()); }
            CreateEvent => {
                // (allocator, class, kind, double when [r6,r7], flags, &event)
                let (class, kind, out) = (a1, a2, a(self, 5)?);
                let id = self.new_event(class, kind, 0);
                self.put32(out, id, "CreateEvent")?;
                0
            }
            SetEventParameter => { // (event, name, type, size, data)
                let (size, data) = (a3, a(self, 4)?);
                if !self.rt.carbon.events.contains_key(&a0) { PARAM_ERR } else {
                    let bytes = self.get_bytes(data, size, "SetEventParameter")?;
                    self.put_param(a0, a1, a2, bytes);
                    0
                }
            }
            GetEventParameter => { // (event, name, desiredType, &actualType, bufferSize, &actualSize, &data)
                let (actual_type_p, buf, actual_size_p, data_p) = (a3, a(self, 4)?, a(self, 5)?, a(self, 6)?);
                match self.rt.carbon.events.get(&a0).and_then(|e| e.params.get(&a1)).cloned() {
                    None => EVENT_PARAMETER_NOT_FOUND,
                    Some((ty, bytes)) => {
                        if a2 != 0 && a2 != ty { COERCION_FAIL } else {
                            self.put32(actual_type_p, ty, "GetEventParameter")?;
                            self.put32(actual_size_p, bytes.len() as u32, "GetEventParameter")?;
                            if data_p != 0 { if buf < bytes.len() as u32 { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()); } self.put_bytes(data_p, &bytes, "GetEventParameter")?; }
                            0
                        }
                    }
                }
            }
            SendEventToEventTarget => {
                if !self.rt.carbon.events.contains_key(&a0) { PARAM_ERR } else {
                    let targets = self.chain_for(a0, a1);
                    return self.start_dispatch(cpu, a0, &targets, Then::Return, ret);
                }
            }
            CallNextEventHandler => {
                let Some(c) = self.rt.carbon.chains.get(&a0) else { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()) };
                if !self.rt.carbon.events.contains_key(&a1) { PARAM_ERR } else {
                    // the rest of the chain, as a nested dispatch whose result goes back to the calling handler
                    let (handlers, next, std_window) = (c.handlers.clone(), c.next, c.std_window);
                    let chain = self.cb_id();
                    self.rt.carbon.chains.insert(chain, Chain { handlers, next, std_window });
                    let frame = Frame { saved: Saved::of(cpu, ret), cont: Continuation::Carbon(Dispatch { chain, event: a1, then: Then::Return }) };
                    return self.dispatch_step(cpu, frame, None);
                }
            }
            QuitApplicationEventLoop => { self.rt.carbon.quit = true; return Ok(()); }
            RunApplicationEventLoop => {
                if self.rt.carbon.quit { self.rt.carbon.quit = false; return Ok(()); }
                match self.pull_event()? {
                    Some(ev) => {
                        let targets = self.chain_for(ev, DISPATCHER_TARGET);
                        return self.start_dispatch(cpu, ev, &targets, Then::Reenter, addr);
                    }
                    None => { self.park(cpu, addr, None); return Ok(()); }
                }
            }
            ReceiveNextEvent => { // (numTypes, typeList, double timeout [r5,r6], pullEvent, &event)
                // r3 numTypes, r4 list, the double in f1 (it reserves r5/r6), so pullEvent is r7 and the out pointer r8
                let (pull_event, out_event) = (cpu.gpr[7], cpu.gpr[8]);
                if pull_event == 0 { return Err(String::from("ReceiveNextEvent: pullEvent = false (peek) is not supported: the classic queue cannot hand an event back")); }
                let timeout = f64::from_bits(cpu.fpr[1]);
                let timed_out = core::mem::take(&mut self.rt.wait_timed_out);
                match self.pull_event()? {
                    Some(ev) => { self.put32(out_event, ev, "ReceiveNextEvent")?; 0 }
                    None if timed_out || timeout == 0.0 => EVENT_LOOP_TIMED_OUT,
                    None => {
                        let deadline = (timeout > 0.0).then(|| self.mac.ev.ticks.wrapping_add(libm::ceil(timeout * 60.0).max(1.0) as u32));
                        self.park(cpu, addr, deadline);
                        return Ok(());
                    }
                }
            }
            CreateNewWindow => { // (windowClass, attributes, const Rect* bounds, WindowRef* out)
                let (class, attrs, bounds, out) = (a0, a1, a2, a3);
                let g = |m: &Self, i: u32| m.mac.mem.read_i16(bounds + 2 * i).map_err(|_| String::from("CreateNewWindow: the bounds Rect leaves guest RAM"));
                let rect = Rect { top: g(self, 0)?, left: g(self, 1)?, bottom: g(self, 2)?, right: g(self, 3)? };
                let resizable = attrs & (1 << 4) != 0;
                let zoom = attrs & 3 == 3;
                let (proc_id, note) = match class {
                    6 => (if resizable && zoom { 8u16 } else if resizable { 0 } else { 4 }, ""),
                    13 => (2, ""),
                    1 | 3 => (1, ""),
                    2 | 4 => (5, ""),
                    5 | 8 => (4, " (floating/utility windows are presented as document windows: the seam has no floating role yet)"),
                    _ => { cpu.gpr[3] = PARAM_ERR as u32; return Ok(()); }
                };
                let spec = WindowSpec { storage: 0, rect, title: String::new(), visible: false, proc_id, behind: u32::MAX, go_away: attrs & 1 != 0, ref_con: 0 };
                let win = self.mac.make_window(spec).map_err(|e| format!("CreateNewWindow: {}", e.0))?;
                if attrs & STANDARD_HANDLER_ATTR != 0 { self.rt.carbon.std_windows.insert(win); }
                self.put32(out, win, "CreateNewWindow")?;
                self.mac.log.push(format!("[carbon] CreateNewWindow class {class} attrs {attrs:#x} -> window {win:#x} (procID {proc_id}){note}"));
                0
            }
            SetWindowTitleWithCFString => {
                let Some(bytes) = self.cf_bytes(a1).map(<[u8]>::to_vec) else { return Err(format!("SetWindowTitleWithCFString: {a1:#x} is not a CFString this engine made")) };
                let n = bytes.len().min(255);
                let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, n as u32 + 2, true).map_err(|e| format!("SetWindowTitleWithCFString: no memory (memory error {e})"))?;
                let mut ps = alloc::vec![n as u8]; ps.extend_from_slice(&bytes[..n]);
                self.put_bytes(p, &ps, "SetWindowTitleWithCFString")?;
                self.toolbox_call("SetWTitle", &[a0, p]);
                let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, p);
                0
            }
            GetWindowPort => { cpu.gpr[3] = a0; return Ok(()); }
            SetPortWindowPort => { self.toolbox_call("SetPort", &[a0]); return Ok(()); }
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }
}
