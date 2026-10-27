//! `CCoeControl` (coecntrl.h): the base of every on-screen control. Layout (ARM EABI, bases `CBase`, `MObjectProvider`): vptr 0,
//! `MObjectProvider` at 4 (vptr + reserved word, coemop.h:167; the import `_ZThn4_N24CEikFormattedCellListBox15MopSupplyObject...`
//! confirms 4), then the members declared at coecntrl.h:370-378.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::appstart::{anchor, SlotSpec};
use crate::callback::Continuation;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{ControlState, HostObj};
use crate::run::Outcome;

// CCoeControl's primary vtable, read off a real guest subclass's table (the inherited slots hold import stubs whose names fix the
// numbering): ~CCoeControl 0,1, Extension_ 2, OfferKeyEventL 3, MakeVisible 4, SetDimmed 5, SetContainerWindowL 6,
// ConstructFromResourceL 7, ActivateL 8, PrepareForFocusLoss 9, PrepareForFocusGain 10, SetAdjacent 11, SetNeighbor 12, HasBorder 13,
// MinimumSize 14, HandleResourceChange 15, GetColorUseListL 16, GetHelpContext 17, InputCapabilities 18, SetParent 19,
// SetLayoutManagerL 20, RequestRelayout 21, TextBaselineOffset 22, SetTextBaselineSpacing 23, HandlePointerEventL 24,
// HandlePointerBufferReadyL 25, FocusChanged 26, SizeChanged 27, PositionChanged 28, CountComponentControls 29,
// ComponentControl 30, HandleControlArrayEventL 31, Reserved_10..13 32-35, GetTextDrawer 36, Reserved_8/9 37-38, MopSupplyObject 39,
// MopNext 40, Draw 41, WriteInternalStateL 42, Reserved_2 43.
pub const SLOT_ACTIVATE_L: u32 = 8;
pub const SLOT_SIZE_CHANGED: u32 = 27;
pub const SLOT_POSITION_CHANGED: u32 = 28;
pub const SLOT_COUNT_COMPONENTS: u32 = 29;
pub const SLOT_COMPONENT_CONTROL: u32 = 30;
pub const SLOT_DRAW: u32 = 41;
pub const SLOT_FOCUS_CHANGED: u32 = 26;
const INH_FOCUS_CHANGED: &[&str] = &["_ZN11CCoeControl12FocusChangedE8TDrawNow"];
const INH_ACTIVATE: &[&str] = &["_ZN11CCoeControl9ActivateLEv"];
const INH_SIZE_CHANGED: &[&str] = &["_ZN11CCoeControl11SizeChangedEv"];
const INH_POSITION_CHANGED: &[&str] = &["_ZN11CCoeControl15PositionChangedEv"];
const INH_COUNT: &[&str] = &["_ZNK11CCoeControl21CountComponentControlsEv"];
const INH_COMPONENT: &[&str] = &["_ZNK11CCoeControl16ComponentControlEi"];
/// Bytes of the guest block standing for an `RWindow` (`iWin`): the guest passes it back to `RWindow` members and reads no field.
const RWINDOW_BYTES: u32 = 32;

pub const CTRL_MOBJECT_PROVIDER: u32 = 4;
pub const CTRL_I_COE_ENV: u32 = 12; // coecntrl.h:370 CCoeEnv* iCoeEnv
pub const CTRL_I_CONTEXT: u32 = 16; // :371 MCoeControlContext* iContext
pub const CTRL_I_POSITION: u32 = 20; // :372 TPoint iPosition (iX, iY)
pub const CTRL_I_SIZE: u32 = 28; // :373 TSize iSize (iWidth, iHeight)
pub const CTRL_I_FLAGS: u32 = 36; // :375 TInt iFlags
pub const CTRL_I_WIN: u32 = 40; // :376 RDrawableWindow* iWin
pub const CTRL_I_DATA: u32 = 44; // :377 RCoeDynamicDataStorage* iData
pub const CTRL_I_MOP_PARENT: u32 = 48; // :378 MObjectProvider* iMopParent
pub const CCOE_CONTROL_SIZE: u32 = 52;

/// The phases of `ActivateL` over a control's components: ask how many, ask for each, activate each (virtual `ActivateL`).
#[derive(Debug, Clone, Copy)]
pub enum CompStep {
    Counted,
    Fetched { index: u32, count: u32 },
    Activated { index: u32, count: u32 },
}

impl Process {
    /// `CCoeControl::CCoeControl()` / `CCoeControl(CCoeEnv*)`: everything after the vptrs cleared, `iCoeEnv` set (a null argument is
    /// `CCoeEnv::Static()`), so a control that reads `iCoeEnv`, `iPosition`, `iSize`, `iWin` inline finds the constructed state.
    fn control_construct(&mut self, this: u32, env: Option<u32>) -> Result<(), String> {
        let env = match env { Some(e) if e != 0 => e, _ => self.coe_env()? };
        for off in (CTRL_MOBJECT_PROVIDER + 4..CCOE_CONTROL_SIZE).step_by(4) {
            self.mem.vm.write_u32(this.wrapping_add(off), 0).map_err(|_| format!("the control at {this:#x} is unwritable"))?;
        }
        self.mem.vm.write_u32(this.wrapping_add(CTRL_I_COE_ENV), env).map_err(|_| format!("the control at {this:#x} is unwritable"))
    }
}

fn vcall(p: &Process, obj: u32, class: &'static str, method: &'static str, slot: u32, inherited: &'static [&'static str]) -> Result<u32, String> {
    anchor(p, obj, &SlotSpec { class, method, slot, inherited })
}

impl Process {
    pub(crate) fn control_state(&mut self, addr: u32) -> Result<&mut ControlState, String> {
        if !matches!(self.obj_get(addr), Some(HostObj::Control(_))) {
            self.obj_set(addr, HostObj::Control(ControlState::default()))?;
        }
        match self.obj_get(addr) { Some(HostObj::Control(c)) => Ok(c), _ => Err(format!("{addr:#x}: host state is not a control's")) }
    }

    /// `CCoeControl::CreateWindowL()` and friends: the control gets a window of its own, `iWin`.
    fn control_create_window(&mut self, this: u32) -> Result<(), String> {
        let win = self.alloc_zeroed(RWINDOW_BYTES).ok_or_else(|| String::from("no guest heap for the window"))?;
        self.mem.vm.write_u32(this.wrapping_add(CTRL_I_WIN), win).map_err(|_| format!("the control at {this:#x} is unwritable"))
    }

    /// `SetExtent`: store position and size; `SizeChanged()` when the size changed, then `PositionChanged()` when the position did.
    fn control_set_extent(&mut self, this: u32, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
        let rd = |p: &Process, o: u32| p.mem.vm.read_u32(this.wrapping_add(o)).map(|v| v as i32).map_err(|_| format!("the control at {this:#x} is unreadable"));
        let old = [rd(self, CTRL_I_POSITION)?, rd(self, CTRL_I_POSITION + 4)?, rd(self, CTRL_I_SIZE)?, rd(self, CTRL_I_SIZE + 4)?];
        for (o, v) in [(CTRL_I_POSITION, x), (CTRL_I_POSITION + 4, y), (CTRL_I_SIZE, w), (CTRL_I_SIZE + 4, h)] {
            self.mem.vm.write_u32(this.wrapping_add(o), v as u32).map_err(|_| format!("the control at {this:#x} is unwritable"))?;
        }
        let mut calls: Vec<(u32, [u32; 4])> = Vec::new();
        if (old[2], old[3]) != (w, h) {
            calls.push((vcall(self, this, "CCoeControl", "SizeChanged", SLOT_SIZE_CHANGED, INH_SIZE_CHANGED)?, [this, 0, 0, 0]));
        }
        if (old[0], old[1]) != (x, y) {
            calls.push((vcall(self, this, "CCoeControl", "PositionChanged", SLOT_POSITION_CHANGED, INH_POSITION_CHANGED)?, [this, 0, 0, 0]));
        }
        self.call_chain(calls, this)
    }

    /// `CCoeControl::ActivateL()`: the control is activated, then each component control is (the virtual `ActivateL` of each).
    fn control_activate(&mut self, this: u32) -> Result<(), String> {
        self.control_state(this)?.activated = true;
        let t = vcall(self, this, "CCoeControl", "CountComponentControls", SLOT_COUNT_COMPONENTS, INH_COUNT)?;
        self.call_guest(t, &[this], Continuation::Control { this, step: CompStep::Counted })
    }
}

/// A component-activation call returned with `r0`.
pub(crate) fn comp_step(p: &mut Process, this: u32, step: CompStep, r0: u32) -> Result<(), Outcome> {
    let fault = |m: String| Outcome::Fault(m);
    let next = |p: &mut Process, index: u32, count: u32| -> Result<(), Outcome> {
        if index >= count {
            return Ok(());
        }
        let t = vcall(p, this, "CCoeControl", "ComponentControl", SLOT_COMPONENT_CONTROL, INH_COMPONENT).map_err(fault)?;
        p.call_guest(t, &[this, index], Continuation::Control { this, step: CompStep::Fetched { index, count } }).map_err(fault)
    };
    match step {
        CompStep::Counted => next(p, 0, r0),
        CompStep::Fetched { index, count } => {
            if r0 == 0 {
                return Err(Outcome::Refused(format!("NEXT-NEEDED cone ActivateL: ComponentControl({index}) of {this:#x} returned NULL")));
            }
            let t = vcall(p, r0, "CCoeControl", "ActivateL", SLOT_ACTIVATE_L, INH_ACTIVATE).map_err(fault)?;
            p.call_guest(t, &[r0], Continuation::Control { this, step: CompStep::Activated { index, count } }).map_err(fault)
        }
        CompStep::Activated { index, count } => next(p, index + 1, count),
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let done = |p: &mut Process, r: Result<(), String>| match r {
        Ok(()) => { p.cpu.regs[0] = this; NativeOutcome::Return }
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
    };
    Some(match name {
        "_ZN11CCoeControlC1Ev" | "_ZN11CCoeControlC2Ev" => { let r = p.control_construct(this, None); done(p, r) }
        "_ZN11CCoeControlC1EP7CCoeEnv" | "_ZN11CCoeControlC2EP7CCoeEnv" => { let r = p.control_construct(this, Some(r1)); done(p, r) }
        // void CCoeControl::SetMopParent(MObjectProvider*) (coecntrl.h): iMopParent.
        "_ZN11CCoeControl12SetMopParentEP15MObjectProvider" => {
            match p.mem.vm.write_u32(this.wrapping_add(CTRL_I_MOP_PARENT), r1) {
                Ok(()) => NativeOutcome::Return,
                Err(_) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: the control at {this:#x} is unwritable")),
            }
        }
        "_ZN11CCoeControl13CreateWindowLEv" | "_ZN11CCoeControl13CreateWindowLEPKS_" => { let r = p.control_create_window(this); done(p, r) }
        // RWindow& CCoeControl::Window() const: iWin.
        "_ZNK11CCoeControl6WindowEv" => match p.mem.vm.read_u32(this.wrapping_add(CTRL_I_WIN)) {
            Ok(w) if w != 0 => { p.cpu.regs[0] = w; NativeOutcome::Return }
            _ => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: the control at {this:#x} owns no window (E32USER-CBase panic on a device)")),
        },
        // void CCoeControl::SetRect(const TRect&): the rectangle is (left, top, right, bottom).
        "_ZN11CCoeControl7SetRectERK5TRect" => {
            let mut v = [0i32; 4];
            for (i, slot) in v.iter_mut().enumerate() {
                match p.mem.vm.read_u32(r1 + i as u32 * 4) { Ok(w) => *slot = w as i32, Err(_) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: the TRect at {r1:#x} is unreadable"))) }
            }
            let r = p.control_set_extent(this, v[0], v[1], v[2] - v[0], v[3] - v[1]);
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) }
        }
        "_ZN11CCoeControl9ActivateLEv" => match p.control_activate(this) { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) },
        "_ZN11CCoeControl11SetObserverEP19MCoeControlObserver" => { let r = p.control_state(this).map(|c| c.observer = r1); done(p, r) }
        "_ZNK11CCoeControl8ObserverEv" => match p.control_state(this) { Ok(c) => { let o = c.observer; p.cpu.regs[0] = o; NativeOutcome::Return } Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) },
        // SetFocusing(TBool): a non-focusing control never takes focus. SetBlank(): nothing is drawn behind it. EnableDragEvents():
        // drag events are delivered; the host's pointer events carry no drag distinction yet, so only the flag is kept. DELETE WHEN
        // pointer input reaches the guest (slice 4: then deliver EDrag to a control with the flag set).
        "_ZN11CCoeControl11SetFocusingEi" => { let r = p.control_state(this).map(|c| c.non_focusing = r1 == 0); done(p, r) }
        "_ZN11CCoeControl8SetBlankEv" => { let r = p.control_state(this).map(|c| c.blank = true); done(p, r) }
        "_ZN11CCoeControl16EnableDragEventsEv" => { let r = p.control_state(this).map(|c| c.drag_events = true); done(p, r) }
        // TBool IsVisible() const / IsFocused() const
        "_ZNK11CCoeControl9IsVisibleEv" => match p.control_state(this) { Ok(c) => { let v = !c.hidden; p.cpu.regs[0] = v as u32; NativeOutcome::Return } Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) },
        "_ZNK11CCoeControl9IsFocusedEv" => match p.control_state(this) { Ok(c) => { let v = c.focused; p.cpu.regs[0] = v as u32; NativeOutcome::Return } Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) },
        // void SetFocus(TBool aFocus, TDrawNow aDrawNow): changes the focus flag and, when it changed, calls the virtual FocusChanged(aDrawNow).
        "_ZN11CCoeControl8SetFocusEi8TDrawNow" => {
            let changed = match p.control_state(this) { Ok(c) => { let ch = c.focused != (r1 != 0); c.focused = r1 != 0; ch } Err(m) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}"))) };
            let mut calls = Vec::new();
            if changed {
                match vcall(p, this, "CCoeControl", "FocusChanged", SLOT_FOCUS_CHANGED, INH_FOCUS_CHANGED) { Ok(t) => calls.push((t, [this, r2, 0, 0])), Err(m) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}"))) }
            }
            match p.call_chain(calls, 0) { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) }
        }
        // TRect Rect() const: (position, position + size), through the hidden pointer in r0 (this in r1).
        "_ZNK11CCoeControl4RectEv" => {
            let rd = |p: &Process, o: u32| p.mem.vm.read_u32(r1.wrapping_add(o)).map(|v| v as i32);
            let (Ok(x), Ok(y), Ok(w), Ok(h)) = (rd(p, CTRL_I_POSITION), rd(p, CTRL_I_POSITION + 4), rd(p, CTRL_I_SIZE), rd(p, CTRL_I_SIZE + 4)) else { return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: the control at {r1:#x} is unreadable"))) };
            let ok = [x, y, x + w, y + h].iter().enumerate().all(|(i, v)| p.mem.vm.write_u32(this + i as u32 * 4, *v as u32).is_ok());
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: the result at {this:#x} is unwritable")) }
        }
        // A control with no components (the base implementations of the two virtuals).
        "_ZNK11CCoeControl21CountComponentControlsEv" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        _ => return None,
    })
}
