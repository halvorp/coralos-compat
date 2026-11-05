//! A program's OWN window definition, admitted by an explicit binding (coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING; the
//! design in compat docs/MACPPC-PLUMBING.md "Window definitions"). Carbon's `CreateCustomWindow` with a `kWindowDefProcPtr` spec hands
//! the Window Manager a WindowDefUPP. Its identity is what the code IS ([`heirloom_bind::WindowDefIdentity::FragmentCode`]: the length
//! and FNV-1a 64 digest of the code section that holds it, as the CFM placed it from its container, and the routine's offset there);
//! a binding ([`heirloom_bind::WindowDefBinding`], DATA the host loads) names what its window MEANS, and that meaning is the native
//! frame it is presented with. Without a binding the window is refused by name, the identity in the refusal.
//!
//! A bound definition still RUNS, through the ONE callback machine ([`crate::callback`], [`Continuation::WindowDef`]), in the Window Manager's
//! order (the Mac OS 8.5 one, which CarbonLib on Mac OS 9 is: the classic messages plus kWindowMsgGetFeatures / kWindowMsgGetRegion), as
//! `long def(short varCode, WindowRef w, short message, long param)`:
//! * `CreateCustomWindow`: the WindowRecord (invisible; `windowDefProc` = a handle whose block holds the UPP), then GetFeatures (7, param =
//!   OptionBits *), Initialize (3), CalculateShape (2), and - when the features say kWindowCanGetWindowRegion - GetRegion (8) for the
//!   structure (32) and the content (33) region INTO the record's own region handles: strucRgn and contRgn are what the DEFINITION writes
//!   (the engine never recomputes them for such a window);
//! * `ShowWindow` / `SelectWindow`: Draw (0, whole frame), then the call;
//! * `MoveWindow` / `SizeWindow`: the call, then CalculateShape and GetRegion again;
//! * `FindWindow`: visible windows front to back; a bound one whose structure region holds the point is asked HitTest (1, the global point
//!   packed v << 16 | h), mapped by the Window Manager's fixed table ([`hit_part`]); an unbound one answers by the engine's own hit test;
//! * `DisposeWindow`: CleanUp (4) while everything is alive, then the call.
//! The definition runs with the Window Manager port current, as the Window Manager calls one (what it draws there is not shown: the frame on
//! screen is the native one, and each Draw is counted as a presentation substitution). Not sent yet, named: wGrow, wDrawGIcon, DragHilite,
//! Modified, DrawInCurrentPort, StateChanged, MeasureTitle; varCode is 0 (a procedure definition has no procID). The message ORDER at
//! creation is the one this engine takes; it has not been checked against a Mac OS 9 oracle (named). A 68k definition procedure is refused
//! by name (trigger: the first program with one).
use crate::callback::{Continuation, Frame, Saved};
use crate::cpu::Cpu;
use crate::machine::MacPpc;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_bind::{WindowDefBinding, WindowDefIdentity};
use heirloom_mac68k_engine::mixed_mode::{resolve, Routine};
use heirloom_mac68k_engine::toolbox::WindowSpec;
use heirloom_ui_abi::GuestUi;

pub const W_DRAW: u32 = 0;
pub const W_HIT: u32 = 1;
pub const W_CALC_RGNS: u32 = 2;
pub const W_NEW: u32 = 3;
pub const W_DISPOSE: u32 = 4;
/// The Mac OS 8.5 Window Manager's messages (MacWindows.h): kWindowMsgGetFeatures (param = OptionBits *), kWindowMsgGetRegion (param =
/// GetWindowRegionRec * { RgnHandle winRgn; WindowRegionCode regionCode }).
pub const W_GET_FEATURES: u32 = 7;
pub const W_GET_REGION: u32 = 8;
/// kWindowCanGetWindowRegion: the definition answers kWindowMsgGetRegion (the Window Manager asks it for the structure and content
/// regions instead of reading what wCalcRgns left).
pub const CAN_GET_WINDOW_REGION: u32 = 1 << 4;
pub const REGION_STRUCTURE: u16 = 32;
pub const REGION_CONTENT: u16 = 33;

/// One message to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Msg {
    /// A message with parameter 0.
    Plain(u32),
    /// kWindowMsgGetFeatures into the window's scratch cell.
    Features,
    /// kWindowMsgGetRegion for this region code into the record's own region handle (sent only when the features say it can).
    Region(u16),
}
/// `windowDefProc` in a WindowRecord (after strucRgn 114, contRgn 118, updateRgn 122).
pub const WR_DEF_PROC: u32 = 126;

/// A code section as instantiated: where it lies, and what it is (length + digest of the bytes the container shipped).
#[derive(Debug, Clone)]
pub struct CodeSection { pub fragment: String, pub base: u32, pub len: u32, pub fnv1a64: u64 }

/// A window whose definition a binding admitted.
#[derive(Debug, Clone)]
pub struct BoundWindow {
    pub binding: String, pub tv: u32, pub def_handle: u32, pub frame_draws: u32, pub features: u32, pub(crate) scratch: u32,
    /// The procID's variation (0 for a procedure definition, which has no procID).
    pub var_code: u32,
    /// `def_handle` is the engine's (a procedure definition's), freed with the window; else it is the program's WDEF resource handle.
    pub owns_handle: bool,
}

#[derive(Default, Debug)]
pub struct WdefState {
    /// Window-definition bindings the host loaded (`MacPpc::add_window_def_binding`).
    pub bindings: Vec<WindowDefBinding>,
    /// Every instantiated code section (cfm.rs records them).
    pub code: Vec<CodeSection>,
    pub bound: BTreeMap<u32, BoundWindow>,
    /// A 4-byte scratch for `GetPort`.
    port_cell: u32,
}

/// What happens after the queued messages.
#[derive(Debug, Clone)]
pub(crate) enum Then {
    /// `CreateCustomWindow` made `win`: *out = win, noErr.
    Created { out: u32 },
    /// A Toolbox call to make (by name, its arguments) after the messages, then return to the caller.
    Call(&'static str, Vec<u32>),
    /// Nothing more: return to the caller (r3 untouched since the call was made).
    Return,
    /// `NewWindow` / `NewCWindow` / `GetNewWindow` / `GetNewCWindow` made `win` from a WDEF resource ([`crate::wdef_res`]): shown when the
    /// program asked for a visible window (its Draw was queued), r3 = the window.
    Opened { show: bool },
    /// `FindWindow(pt, &w)`: walking `order` front to back from `next`; `engine` = the engine's answer for the unbound windows.
    Find { pt: u32, out: u32, order: Vec<u32>, next: usize, engine: (u16, u32) },
}

#[derive(Debug, Clone)]
pub(crate) struct WdefRun { win: u32, port: u32, todo: VecDeque<Msg>, then: Then, msg: Msg }

/// The Window Manager's fixed map from a definition's wHit answer to a `FindWindow` part (Inside Macintosh / MacWindows.h: wNoHit 0,
/// wInContent 1, wInDrag 2, wInGrow 3, wInGoAway 4, wInZoomIn 5, wInZoomOut 6, wInCollapseBox 9, wInProxyIcon 10). `Ok(None)` = no hit.
pub fn hit_part(code: u32) -> Result<Option<u16>, String> {
    Ok(Some(match code {
        0 => return Ok(None),
        1 => 3, 2 => 4, 3 => 5, 4 => 6, 5 => 7, 6 => 8, 9 => 11, 10 => 12,
        c => return Err(format!("the window definition answered wHit with {c}, which the Window Manager's table does not map")),
    }))
}

impl<U: GuestUi> MacPpc<U> {
    /// Load one window-definition binding (the host reads `System/Heirloom/Bindings/macppc/*.sexp` and passes each).
    pub fn add_window_def_binding(&mut self, b: WindowDefBinding) { self.wdef.bindings.push(b); }

    /// What the definition behind `upp` IS: its TVector and its identity. A 68k definition, or code in no instantiated section, is refused.
    pub fn window_def_identity(&self, upp: u32) -> Result<(u32, WindowDefIdentity, String), String> {
        let tv = match resolve(&self.mac.mem, upp).map_err(|e| format!("{e}"))? {
            Routine::PowerPc(r) => r.proc_descriptor,
            Routine::M68k { code, .. } => return Err(format!("a 68k window definition procedure at {code:#x} (not run from the Window Manager yet)")),
        };
        self.tvector_identity(tv)
    }

    /// What the PowerPC routine behind TVector `tv` IS (see [`MacPpc::window_def_identity`]).
    pub(crate) fn tvector_identity(&self, tv: u32) -> Result<(u32, WindowDefIdentity, String), String> {
        let code = self.mac.mem.read_u32(tv).map_err(|e| format!("the definition's TVector at {tv:#x}: {e}"))?;
        let s = self.wdef.code.iter().find(|s| code >= s.base && code - s.base < s.len)
            .ok_or_else(|| format!("the definition's code {code:#x} lies in no instantiated code section"))?;
        Ok((tv, WindowDefIdentity::FragmentCode { code_length: s.len, code_fnv1a64: s.fnv1a64, entry_offset: code - s.base }, s.fragment.clone()))
    }

    /// `CreateCustomWindow` with a `kWindowDefProcPtr` spec (`upp`): bound -> the window is made and the definition's wNew / wCalcRgns
    /// run (true: a callback was entered); unbound -> Err(the refusal, naming the identity).
    pub(crate) fn create_bound_window(&mut self, cpu: &mut Cpu, upp: u32, class: u32, attrs: u32, bounds: u32, out: u32) -> Result<(), String> {
        let (tv, id, frag) = self.window_def_identity(upp)?;
        let b = heirloom_bind::wdef::find(&self.wdef.bindings, &id).map_err(|e| format!("the definition in {frag}: {e}"))?.clone();
        let g = |m: &Self, i: u32| m.mac.mem.read_i16(bounds + 2 * i).map_err(|_| String::from("CreateCustomWindow: the bounds Rect leaves guest RAM"));
        let rect = heirloom_mac68k_engine::pascal::Rect { top: g(self, 0)?, left: g(self, 1)?, bottom: g(self, 2)?, right: g(self, 3)? };
        let spec = WindowSpec { storage: 0, rect, title: String::new(), visible: false, proc_id: 0, behind: u32::MAX, go_away: b.meaning.closable, ref_con: 0 };
        let win = self.mac.make_window_bound(spec, (b.meaning.role, b.meaning.resizable)).map_err(|e| format!("CreateCustomWindow: {}", e.0))?;
        // the parameter blocks the definition is handed: OptionBits at +0 (8 bytes, zero), a GetWindowRegionRec at +8
        let scratch = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 16, true).map_err(|e| format!("CreateCustomWindow: no memory (memory error {e})"))?;
        let def_handle = self.mac.mm.new_handle(&mut self.mac.mem, &mut self.mac.heap, 4, true).map_err(|e| format!("CreateCustomWindow: no memory for windowDefProc (memory error {e})"))?;
        let block = self.mac.mem.read_u32(def_handle).map_err(|e| format!("{e}"))?;
        self.put32(block, upp, "CreateCustomWindow")?;
        self.put32(win + WR_DEF_PROC, def_handle, "CreateCustomWindow")?;
        self.wc.class.insert(win, class);
        self.mac.log.push(format!("[WDEF-BOUND binding={} window={win:#x}] CreateCustomWindow class {class} attrs {attrs:#x}: the definition in {frag} {id} means {:?}; its own code runs for regions and hits, the frame is native", b.id, b.meaning));
        self.wdef.bound.insert(win, BoundWindow { binding: b.id.clone(), tv, def_handle, frame_draws: 0, features: 0, scratch, var_code: 0, owns_handle: true });
        self.begin_wdef(cpu, win, &[Msg::Features, Msg::Plain(W_NEW), Msg::Plain(W_CALC_RGNS), Msg::Region(REGION_STRUCTURE), Msg::Region(REGION_CONTENT)], Then::Created { out })
    }

    /// The Window Manager calls that send a bound window's definition a message: true = handled here (a callback was entered, or the
    /// call finished). `op` is the call's trap word, `name` its name.
    pub(crate) fn window_def_call(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool, String> {
        let w = cpu.gpr[3];
        match name {
            "FindWindow" if !self.wdef.bound.is_empty() => {
                let (pt, out) = (cpu.gpr[3], cpu.gpr[4]);
                let (v, h) = ((pt >> 16) as u16 as i16, pt as u16 as i16);
                let engine = self.mac.find_window(v, h);
                // the engine's own hit test answers for UNBOUND windows only: a bound window's extent is its definition's to say
                let engine = if self.wdef.bound.contains_key(&engine.1) { (heirloom_mac68k_engine::toolbox::IN_DESK, 0) } else { engine };
                let mbar = self.mac.mem.read_u16(heirloom_mac68k_engine::lowmem::MBAR_HEIGHT).unwrap_or(20) as i16;
                if v < mbar { return Ok(false); }
                let order: Vec<u32> = self.mac.z.iter().copied().filter(|z| self.mac.windows.get(z).is_some_and(|x| x.visible && x.native.is_some())).collect();
                self.find_next(cpu, Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::WindowDef(WdefRun { win: 0, port: 0, todo: VecDeque::new(), then: Then::Find { pt, out, order, next: 0, engine }, msg: Msg::Plain(W_HIT) }) })?;
                Ok(true)
            }
            "ShowWindow" | "SelectWindow" if self.wdef.bound.contains_key(&w) => { self.begin_wdef(cpu, w, &[Msg::Plain(W_DRAW)], Then::Call(if name == "ShowWindow" { "ShowWindow" } else { "SelectWindow" }, cpu.gpr[3..4].to_vec()))?; Ok(true) }
            "MoveWindow" | "SizeWindow" if self.wdef.bound.contains_key(&w) => {
                let args = cpu.gpr[3..7].to_vec();
                self.toolbox_call(name, &args);
                self.begin_wdef(cpu, w, &[Msg::Plain(W_CALC_RGNS), Msg::Region(REGION_STRUCTURE), Msg::Region(REGION_CONTENT)], Then::Return)?;
                Ok(true)
            }
            "NewWindow" | "NewCWindow" | "GetNewWindow" | "GetNewCWindow" => self.open_resource_window(cpu, name),
            "DisposeWindow" if self.wdef.bound.contains_key(&w) => { self.begin_wdef(cpu, w, &[Msg::Plain(W_DISPOSE)], Then::Call("DisposeWindow", alloc::vec![w]))?; Ok(true) }
            _ => Ok(false),
        }
    }

    fn current_port(&mut self) -> u32 {
        if self.wdef.port_cell == 0 { self.wdef.port_cell = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 4, true).unwrap_or(0); }
        let cell = self.wdef.port_cell;
        self.toolbox_call("GetPort", &[cell]);
        self.mac.mem.read_u32(cell).unwrap_or(0)
    }

    /// Save the interrupted call and send `win`'s definition `msgs` in order; `then` afterwards.
    pub(crate) fn begin_wdef(&mut self, cpu: &mut Cpu, win: u32, msgs: &[Msg], then: Then) -> Result<(), String> {
        let port = self.current_port();
        let frame = Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::WindowDef(WdefRun { win, port, todo: msgs.iter().copied().collect(), then, msg: Msg::Plain(0) }) };
        self.next_wdef(cpu, frame)
    }

    /// Enter the definition with `msg` (param `param`). The current port is the Window Manager port, as the classic Window Manager
    /// calls a definition (what it draws there is not shown: the frame on screen is the native one).
    fn send(&mut self, cpu: &mut Cpu, win: u32, msg: u32, param: u32, sp: u32) -> Result<(), String> {
        let (tv, var_code) = self.wdef.bound.get(&win).map(|b| (b.tv, b.var_code)).ok_or_else(|| format!("window {win:#x} has no bound definition"))?;
        let wmgr = self.mac.mem.read_u32(heirloom_mac68k_engine::lowmem::WMGR_PORT).unwrap_or(0);
        if wmgr != 0 { self.toolbox_call("SetPort", &[wmgr]); }
        if msg == W_DRAW { if let Some(b) = self.wdef.bound.get_mut(&win) { b.frame_draws += 1; } }
        self.mac.log.push(format!("[wdef] window {win:#x}: message {msg} param {param:#x}"));
        self.enter_tvector(cpu, tv, &[var_code, win, msg, param], sp).map_err(|e| format!("window {win:#x}'s definition (message {msg}): {e}"))
    }

    /// The message's number and parameter; None = not sent (a GetRegion the definition's features do not offer).
    fn message(&mut self, win: u32, m: Msg) -> Result<Option<(u32, u32)>, String> {
        let b = self.wdef.bound.get(&win).cloned().ok_or_else(|| format!("window {win:#x} has no bound definition"))?;
        Ok(match m {
            Msg::Plain(n) => Some((n, 0)),
            Msg::Features => { self.put32(b.scratch, 0, "wdef")?; self.put32(b.scratch + 4, 0, "wdef")?; Some((W_GET_FEATURES, b.scratch)) }
            Msg::Region(_) if b.features & CAN_GET_WINDOW_REGION == 0 => None,
            Msg::Region(code) => {
                let off = if code == REGION_STRUCTURE { heirloom_mac68k_engine::winregions::WR_STRUC_RGN } else { heirloom_mac68k_engine::winregions::WR_CONT_RGN };
                let h = self.mac.mem.read_u32(win + off).map_err(|e| format!("{e}"))?;
                self.put32(b.scratch + 8, h, "wdef")?;
                self.mac.mem.write_u16(b.scratch + 12, code).map_err(|e| format!("{e}"))?;
                Some((W_GET_REGION, b.scratch + 8))
            }
        })
    }

    fn next_wdef(&mut self, cpu: &mut Cpu, mut frame: Frame) -> Result<(), String> {
        let Continuation::WindowDef(run) = &mut frame.cont else { return Err(String::from("a window-definition step on another continuation")) };
        if let Then::Find { .. } = run.then { return self.find_next(cpu, frame); }
        while let Some(m) = run.todo.pop_front() {
            let win = run.win;
            let Some((msg, param)) = self.message(win, m)? else { continue };
            let Continuation::WindowDef(run) = &mut frame.cont else { unreachable!() };
            run.msg = m;
            let sp = frame.saved.gpr[1];
            self.send(cpu, win, msg, param, sp)?;
            self.rt.callbacks.push(frame);
            return Ok(());
        }
        let Continuation::WindowDef(run) = frame.cont.clone() else { unreachable!() };
        if run.port != 0 { self.toolbox_call("SetPort", &[run.port]); }
        frame.saved.restore(cpu);
        match run.then {
            Then::Created { out } => {
                self.note_regions(run.win);
                self.put32(out, run.win, "CreateCustomWindow")?;
                cpu.gpr[3] = 0;
            }
            Then::Call(name, args) => {
                if name == "DisposeWindow" {
                    if let Some(b) = self.wdef.bound.remove(&run.win) {
                        let _ = self.put32(run.win + WR_DEF_PROC, 0, "DisposeWindow");
                        if b.owns_handle { let _ = self.mac.mm.dispose_handle(&mut self.mac.mem, &mut self.mac.heap, b.def_handle); }
                        let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, b.scratch);
                        self.mac.log.push(format!("[WDEF-BOUND binding={} window={:#x}] disposed: {} frame draw(s) by the definition, presented as the native frame instead", b.binding, run.win, b.frame_draws));
                    }
                }
                cpu.gpr[3] = self.toolbox_call(name, &args);
            }
            Then::Return => { self.note_regions(run.win); }
            Then::Opened { show } => {
                self.note_regions(run.win);
                if show { self.toolbox_call("ShowWindow", &[run.win]); }
                cpu.gpr[3] = run.win;
            }
            Then::Find { .. } => unreachable!(),
        }
        Ok(())
    }

    /// Log the regions the definition left (diagnostics: bounds of strucRgn / contRgn).
    fn note_regions(&mut self, win: u32) {
        let b = |m: &Self, off: u32| -> String {
            let h = m.mac.mem.read_u32(win + off).unwrap_or(0);
            match m.mac.read_region(h) { Ok(r) => format!("{:?}", r.bounds()), Err(e) => e.0 }
        };
        let (s, c) = (b(self, heirloom_mac68k_engine::winregions::WR_STRUC_RGN), b(self, heirloom_mac68k_engine::winregions::WR_CONT_RGN));
        self.mac.log.push(format!("[wdef] window {win:#x}: the definition's regions: strucRgn {s}, contRgn {c}"));
    }

    /// FindWindow's walk: the next window front to back; a bound one is asked wHit.
    fn find_next(&mut self, cpu: &mut Cpu, mut frame: Frame) -> Result<(), String> {
        let Continuation::WindowDef(run) = &mut frame.cont else { return Err(String::from("a FindWindow step on another continuation")) };
        let Then::Find { pt, out, order, next, engine } = &mut run.then else { return Err(String::from("not a FindWindow walk")) };
        while let Some(&w) = order.get(*next) {
            *next += 1;
            if self.wdef.bound.contains_key(&w) {
                // the Window Manager asks a definition about a point only inside the window's structure region (the definition's own)
                let h = self.mac.mem.read_u32(w + heirloom_mac68k_engine::winregions::WR_STRUC_RGN).unwrap_or(0);
                let (y, x) = ((*pt >> 16) as u16 as i16, *pt as u16 as i16);
                if !self.mac.read_region(h).is_ok_and(|g| g.contains(i32::from(x), i32::from(y))) { continue; }
                run.win = w;
                run.msg = Msg::Plain(W_HIT);
                let (pt, sp) = (*pt, frame.saved.gpr[1]);
                if run.port == 0 { run.port = self.current_port(); }
                self.send(cpu, w, W_HIT, pt, sp)?;
                self.rt.callbacks.push(frame);
                return Ok(());
            }
            if engine.1 == w { break; }
        }
        // no bound window hit before the engine's answer (or none at all): the engine's answer stands
        let (part, w, out, port) = (engine.0, if order.contains(&engine.1) || engine.1 == 0 { engine.1 } else { 0 }, *out, run.port);
        if port != 0 { self.toolbox_call("SetPort", &[port]); }
        frame.saved.restore(cpu);
        self.put32(out, w, "FindWindow")?;
        cpu.gpr[3] = u32::from(part);
        self.mac.log.push(format!("[toolbox] FindWindow -> part {part} window {w:#x} (front to back, bound definitions asked first)"));
        Ok(())
    }

    /// A definition returned (from `callback_returned`).
    pub(crate) fn wdef_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let r = cpu.gpr[3];
        let Continuation::WindowDef(run) = &frame.cont else { return Err(String::from("a window-definition return on another continuation")) };
        if let Then::Find { out, .. } = run.then {
            if let Some(part) = hit_part(r)? {
                let (w, port) = (run.win, run.port);
                if port != 0 { self.toolbox_call("SetPort", &[port]); }
                frame.saved.restore(cpu);
                self.put32(out, w, "FindWindow")?;
                cpu.gpr[3] = u32::from(part);
                self.mac.log.push(format!("[toolbox] FindWindow -> part {part} window {w:#x} (its definition's wHit {r})"));
                return Ok(());
            }
            return self.find_next(cpu, frame);
        }
        self.mac.log.push(format!("[wdef] window {:#x}: {:?} returned {r:#x}", run.win, run.msg));
        if run.msg == Msg::Features {
            let win = run.win;
            let scratch = self.wdef.bound.get(&win).map_or(0, |b| b.scratch);
            let f = if r & 0xFF != 0 { self.mac.mem.read_u32(scratch).unwrap_or(0) } else { 0 }; // a Boolean: the definition answered
            if let Some(b) = self.wdef.bound.get_mut(&win) { b.features = f; }
            self.mac.log.push(format!("[wdef] window {win:#x}: features {f:#x}{}", if f & CAN_GET_WINDOW_REGION != 0 { " (it answers GetRegion)" } else { "" }));
        }
        self.next_wdef(cpu, frame)
    }
}

#[cfg(test)]
mod tests {
    //! The bound-definition lifecycle through the callback machine. The definition is SIMULATED at each entry (the test reads the message
    //! from r5, answers in r3 and returns through `$CallbackReturn`), so what is checked is the engine's half: identity, message order and
    //! parameters, what it does with each answer, the regions it leaves to the definition, the refusals.
    use super::*;
    use heirloom_bind::{WindowDefBinding, WindowDefIdentity, WindowMeaning};
    use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
    use heirloom_ui_abi::{RecordingGuestUi, WindowRole};
    type M = MacPpc<RecordingGuestUi>;
    const SP: u32 = 0x0007_0000;
    const R: u32 = 0x4000;
    const SEC: u32 = 0x0003_0000; // a "code section" of 0x100 bytes
    const CODE: u32 = SEC + 0x40; // the definition's entry, offset 0x40
    const TV: u32 = 0x0003_8000;
    const DIGEST: u64 = 0x1234_5678_9abc_def0;

    fn setup(bound: bool) -> (M, u32) {
        let mut m: M = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.implicit_graf();
        m.toolbox_call("InitWindows", &[]);
        for (i, v) in [40i16, 40, 140, 240].iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, *v as u16).unwrap(); }
        m.mac.mem.write_u32(TV, CODE).unwrap();
        m.mac.mem.write_u32(TV + 4, 0x0003_9000).unwrap();
        m.wdef.code.push(CodeSection { fragment: "testlib".into(), base: SEC, len: 0x100, fnv1a64: DIGEST });
        if bound {
            m.add_window_def_binding(WindowDefBinding { id: "testlib-1".into(), identity: WindowDefIdentity::FragmentCode { code_length: 0x100, code_fnv1a64: DIGEST, entry_offset: 0x40 },
                meaning: WindowMeaning { role: WindowRole::Dialog, titled: false, movable: false, closable: false, resizable: false } });
        }
        let upp = new_routine_descriptor(&mut m.mac, TV, 0x3BF0, ISA_PPC).unwrap();
        (m, upp)
    }

    /// While the CPU is inside the definition: answer each message with `answer(msg, param, m)` until the call is over; the messages seen.
    fn drive(m: &mut M, cpu: &mut Cpu, answer: &mut dyn FnMut(u32, u32, &mut M) -> u32) -> Vec<(u32, u32)> {
        let mut seen = Vec::new();
        while cpu.pc == CODE {
            let (msg, param) = (cpu.gpr[5], cpu.gpr[6]);
            assert_eq!((cpu.gpr[3], cpu.gpr[2], cpu.gpr[12]), (0, 0x0003_9000, TV), "varCode 0, the definition's TOC and TVector");
            seen.push((msg, param));
            cpu.gpr[3] = answer(msg, param, m);
            m.callback_returned(cpu).unwrap();
        }
        seen
    }

    fn create(m: &mut M, upp: u32) -> Cpu {
        m.mac.mem.write_u32(R + 40, 0).unwrap(); m.mac.mem.write_u32(R + 44, upp).unwrap(); m.mac.mem.write_u32(R + 48, 0xDEAD).unwrap();
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.lr = 0x0005_1230;
        for (i, a) in [R + 40, 4, 0, R, R + 48].iter().enumerate() { cpu.gpr[3 + i] = *a; }
        m.carbon_call(crate::carbon::CarbonFn::CreateCustomWindow, &mut cpu, 0).unwrap();
        cpu
    }

    #[test]
    fn a_bound_definition_gets_its_messages_in_order_and_its_regions_are_its_own() {
        let (mut m, upp) = setup(true);
        let mut cpu = create(&mut m, upp);
        // the definition: features = CanGetWindowRegion; GetRegion writes a 200x100 rectangle (structure one pixel larger) into the record's handle
        let mut answer = |msg: u32, param: u32, m: &mut M| -> u32 {
            match msg {
                W_GET_FEATURES => { m.mac.mem.write_u32(param, CAN_GET_WINDOW_REGION).unwrap(); 1 }
                W_GET_REGION => {
                    let (h, code) = (m.mac.mem.read_u32(param).unwrap(), m.mac.mem.read_u16(param + 4).unwrap());
                    let g = if code == REGION_STRUCTURE { heirloom_raster::Region::from_rect(heirloom_raster::Rect::new(39, 39, 241, 141)) } else { heirloom_raster::Region::from_rect(heirloom_raster::Rect::new(40, 40, 240, 140)) };
                    m.mac.write_region(h, &g).unwrap();
                    0
                }
                W_HIT => 2, // wInDrag
                _ => 0,
            }
        };
        let seen = drive(&mut m, &mut cpu, &mut answer);
        assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), vec![W_GET_FEATURES, W_NEW, W_CALC_RGNS, W_GET_REGION, W_GET_REGION], "GetFeatures, Initialize, CalculateShape, GetRegion x2");
        let win = m.mac.mem.read_u32(R + 48).unwrap();
        assert_eq!((cpu.pc, cpu.gpr[3]), (0x0005_1230, 0), "CreateCustomWindow returns noErr to its caller");
        assert!(m.mac.windows.contains_key(&win) && m.wdef.bound.contains_key(&win));
        let def = m.mac.mem.read_u32(win + WR_DEF_PROC).unwrap();
        assert_eq!(m.mac.mem.read_u32(m.mac.mem.read_u32(def).unwrap()).unwrap(), upp, "windowDefProc is a handle holding the UPP");
        let struc = m.mac.read_region(m.mac.mem.read_u32(win + heirloom_mac68k_engine::winregions::WR_STRUC_RGN).unwrap()).unwrap();
        assert_eq!(struc.bounds(), heirloom_raster::Rect::new(39, 39, 241, 141), "strucRgn is what the definition wrote");
        // the engine never rewrites them: a MoveWindow re-asks the definition (CalculateShape + GetRegion), it does not recompute them itself
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.lr = 0x0005_2000;
        cpu.gpr[3..7].copy_from_slice(&[win, 60, 60, 0]);
        assert!(m.window_def_call(&mut cpu, "MoveWindow").unwrap());
        let seen = drive(&mut m, &mut cpu, &mut answer);
        assert_eq!(seen.iter().map(|s| s.0).collect::<Vec<_>>(), vec![W_CALC_RGNS, W_GET_REGION, W_GET_REGION]);
        // ShowWindow: wDraw first (the frame drawing runs; the frame shown is the native one)
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.gpr[3] = win;
        assert!(m.window_def_call(&mut cpu, "ShowWindow").unwrap());
        assert_eq!(drive(&mut m, &mut cpu, &mut answer), vec![(W_DRAW, 0)]);
        assert!(m.mac.windows[&win].native.is_some(), "then the window is shown");
        // FindWindow inside the definition's structure region: wHit with the packed point, answer mapped (wInDrag 2 -> inDrag 4)
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.lr = 0x0005_3000;
        cpu.gpr[3..5].copy_from_slice(&[(100 << 16) | 100, R + 60]);
        assert!(m.window_def_call(&mut cpu, "FindWindow").unwrap());
        assert_eq!(drive(&mut m, &mut cpu, &mut answer), vec![(W_HIT, (100 << 16) | 100)]);
        assert_eq!((cpu.gpr[3], m.mac.mem.read_u32(R + 60).unwrap(), cpu.pc), (4, win, 0x0005_3000));
        // a point outside its structure region is never asked about
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.gpr[3..5].copy_from_slice(&[(400 << 16) | 400, R + 60]);
        assert!(m.window_def_call(&mut cpu, "FindWindow").unwrap());
        assert!(drive(&mut m, &mut cpu, &mut answer).is_empty());
        // DisposeWindow: wDispose while it lives, then the disposal; the handle is freed
        let mut cpu = Cpu::new(0x0005_0000, SP);
        cpu.gpr[3] = win;
        assert!(m.window_def_call(&mut cpu, "DisposeWindow").unwrap());
        assert_eq!(drive(&mut m, &mut cpu, &mut answer), vec![(W_DISPOSE, 0)]);
        assert!(!m.mac.windows.contains_key(&win) && !m.wdef.bound.contains_key(&win));
    }

    #[test]
    fn an_unbound_definition_is_refused_with_its_identity_and_an_unknown_hit_is_a_fault() {
        // NEGATIVE CONTROL: the same code with no binding loaded
        let (mut m, upp) = setup(false);
        let before = m.mac.windows.len();
        let cpu = create(&mut m, upp);
        assert_eq!((cpu.gpr[3] as i32, m.mac.mem.read_u32(R + 48).unwrap()), (-50, 0));
        assert_eq!(m.mac.windows.len(), before, "nothing allocated");
        assert!(m.mac.refused.iter().any(|r| r.contains("[WDEF-UNSUPPORTED]") && r.contains("(fragment-code (length 256)") && r.contains("(entry-offset 64)")), "{:?}", m.mac.refused);
        // the Window Manager's hit table: only its codes map; anything else is a named fault, never a guess
        assert_eq!(hit_part(0).unwrap(), None);
        assert_eq!(hit_part(1).unwrap(), Some(3));
        assert_eq!(hit_part(10).unwrap(), Some(12));
        assert!(hit_part(7).is_err() && hit_part(99).is_err());
    }

    #[test]
    fn content_theme_parts_become_the_kits_bevel_and_a_bound_windows_variant_is_zero() {
        let (mut m, _) = setup(false);
        assert_eq!(m.create_window(6, 0, R, R + 16, None).unwrap(), Some(0));
        let w = m.mac.mem.read_u32(R + 16).unwrap();
        m.toolbox_call("ShowWindow", &[w]);
        m.toolbox_call("SetPort", &[w]);
        for (i, v) in [10i16, 10, 50, 90].iter().enumerate() { m.mac.mem.write_u16(R + 32 + 2 * i as u32, *v as u16).unwrap(); }
        let mut cpu = Cpu::new(0x0005_0000, SP);
        let theme = |m: &mut M, cpu: &mut Cpu, n: &str, a: &[u32]| { for (i, x) in a.iter().enumerate() { cpu.gpr[3 + i] = *x; } m.cx_call(crate::carbon_ctl::CxFn::by_name(n).unwrap(), cpu).unwrap(); cpu.gpr[3] as i32 };
        assert_eq!(theme(&mut m, &mut cpu, "DrawThemePrimaryGroup", &[R + 32, 0]), 0);
        let ops = m.mac.qd.plane.ops(w);
        assert!(ops.iter().any(|o| matches!(o, heirloom_ui_abi::DrawOp::Bevel { raised: false, rect } if rect.width == 80 && rect.height == 40)), "{ops:?}");
        // a focus ring is the kit's to show: nothing recorded; NEGATIVE CONTROL: an empty rect is paramErr
        let n = m.mac.qd.plane.ops(w).len();
        assert_eq!(theme(&mut m, &mut cpu, "DrawThemeFocusRect", &[R + 32, 1]), 0);
        assert_eq!(m.mac.qd.plane.ops(w).len(), n);
        m.mac.mem.write_u16(R + 36, 10).unwrap();
        assert_eq!(theme(&mut m, &mut cpu, "DrawThemeEditTextFrame", &[R + 32, 0]), -50);
        // GetWVariant: a system definition's variation from its procID; a bound definition's window (no procID) 0
        assert_eq!(m.toolbox_call("GetWVariant", &[w]), u32::from(m.mac.windows[&w].proc_id & 15));
    }
}
