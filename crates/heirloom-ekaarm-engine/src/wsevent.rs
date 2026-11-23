//! The window server's event stream for this process: the producer of key events (consult-2026-10-26-symbian-runapplication.md,
//! slice 4). `CCoeEnv` IS an active object (coemain.h: `CCoeEnv : public CActive`, priority `EActivePriorityWsEvents` = 100, COEMAIN.CPP:160):
//! the engine-built environment is added to the scheduler's queue and holds the request for the next window server event
//! (`RWsSession::EventReady`, here `request_issue`). A host key event queues window server events and completes that request; the
//! scheduler then runs `CCoeEnv::RunL` (below, the native the environment's vtable slot 4 holds), which is COEMAIN.CPP:229-250: take
//! the event, ask for the next notification, and hand the event to `iAppUi->HandleWsEventL(event, window)` with the root window group's
//! handle, which is the environment's own address (`iRootWin.Construct((TUint32)this, ...)`, COEMAIN.CPP:669).
//! A `TWsEvent` is iType 0, iHandle 4, iTime 8 (a TInt64), then `iEventData` at 16 (w32std.h:1196-1202); a `TKeyEvent` is iCode, iScanCode,
//! iModifiers, iRepeats (w32std.h:995-1021).
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::Modifiers;
use crate::appstart::{anchor, SlotSpec};
use crate::callback::Continuation;
use crate::cone::COEENV_I_APPUI;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::sched::{ACTIVE_I_FLAGS, ACTIVE_I_PRIORITY, ACTIVE_I_STATUS, FLAG_ACTIVE};

/// The mangled name of the engine's `CCoeEnv::RunL`.
pub const ENV_RUN_L: &str = "_ZN8CCoeEnv4RunLEv";
/// `EActivePriorityWsEvents` (coemain.h:49).
const ACTIVE_PRIORITY_WS_EVENTS: u32 = 100;
/// `TEventCode` (w32std.h:277-297): EEventNull 0, EEventKey 1, EEventKeyUp 2, EEventKeyDown 3, EEventFocusLost 5, EEventFocusGained 6.
pub const EEVENT_KEY: u32 = 1;
pub const EEVENT_KEY_UP: u32 = 2;
pub const EEVENT_KEY_DOWN: u32 = 3;
pub const EEVENT_FOCUS_LOST: u32 = 5;
pub const EEVENT_FOCUS_GAINED: u32 = 6;
/// `EEventPointer` (w32std.h:287) and `TPointerEvent::TType` values.
pub const EEVENT_POINTER: u32 = 10;
pub const EPOINTER_BUTTON1_DOWN: u32 = 0;
pub const EPOINTER_BUTTON1_UP: u32 = 1;
pub const EPOINTER_DRAG: u32 = 6;
const TWS_EVENT_BYTES: u32 = 48;
const TWS_I_HANDLE: u32 = 4;
const TWS_I_EVENT_DATA: u32 = 16;
/// `KErrCancel` / `KErrServerTerminated` (e32err.h): the environment's `RunL` returns without handling an event.
const KERR_CANCEL: i32 = -3;
const KERR_SERVER_TERMINATED: i32 = -15;
/// `CActive`'s virtual slots (`schedrun.rs`): `DoCancel` 3, `RunL` 4, `RunError` 5.
const ENV_VTABLE: &[(usize, &str)] = &[
    (0, "_ZN9CEikonEnvD1Ev"), (1, "_ZN9CEikonEnvD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZN8CCoeEnv8DoCancelEv"), (4, ENV_RUN_L), (5, "_ZN8CCoeEnv8RunErrorEi"),
];

/// One window server event as the host produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsEvent {
    Key { ty: u32, code: u32, scan: u32, modifiers: u32 },
    /// Touch / mouse pointer event (`EEventPointer`, `TPointerEvent`).
    Pointer { ty: u32, x: i32, y: i32 },
    /// Window focus gained/lost (`EEventFocusGained` / `EEventFocusLost`).
    Focus(bool),
    /// Initial or deferred Window Server redraw (`EEventRedraw`) for a window-owning control.
    Redraw(u32),
    /// A menu or CBA command ID (`CAknView::HandleCommandL` / `CEikAppUi::HandleCommandL`).
    Command(u32),
    /// A list box observer event (`MEikListBoxObserver::HandleListBoxEventL(lb, event)`).
    ListEvent { lb: u32, event: u32 },
}

/// `EEikCmdExit` (eikcmd.hrh): standard exit command ID.
pub const EEIK_CMD_EXIT: u32 = 1000;

#[derive(Debug, Default)]
pub struct WsState {
    /// Events the window server holds for the client: the queue behind `RWsSession::EventReady` / `GetEvent`.
    pub queue: VecDeque<WsEvent>,
    /// The guest `TWsEvent` the environment's `RunL` fills for each event it hands out.
    pub block: Option<u32>,
}

impl Process {
    /// Make the environment the window-event active object: the CActive part (priority 100) is set, it joins the scheduler's queue,
    /// its vtable carries `RunL`, and the request for the first event is outstanding (`CCoeEnv::ConstructL`'s `RequestEventNotification`).
    pub(crate) fn env_as_active_object(&mut self, env: u32) -> Result<(), String> {
        let vptr = self.build_vtable("CEikonEnv", &crate::enginestub::slots_with_gaps(6, ENV_VTABLE))?;
        self.wr(env, vptr)?;
        self.wr(env + ACTIVE_I_PRIORITY, ACTIVE_PRIORITY_WS_EVENTS)?;
        self.sched_add(env)?;
        self.ws_request_event(env)
    }

    /// `CCoeEnv::RequestEventNotification()`: an event request on the environment's status. An event already waiting completes it at once.
    fn ws_request_event(&mut self, env: u32) -> Result<(), String> {
        self.request_issue(env + ACTIVE_I_STATUS)?;
        let flags = self.rd(env + ACTIVE_I_FLAGS)?;
        self.wr(env + ACTIVE_I_FLAGS, flags | FLAG_ACTIVE)?;
        if !self.nat.ws.queue.is_empty() {
            self.request_complete(env + ACTIVE_I_STATUS, 0)?;
        }
        Ok(())
    }

    fn ws_enqueue(&mut self, env: u32, ev: WsEvent) -> Result<(), String> {
        self.nat.ws.queue.push_back(ev);
        if self.nat.requests.is_pending(env + ACTIVE_I_STATUS) {
            self.request_complete(env + ACTIVE_I_STATUS, 0)?;
        }
        Ok(())
    }

    /// Deliver a key event. `scan` carries the **host** key code (CoralOS Unicode
    /// scalar or C0 key-wire byte). Internally translated to Symbian TStdScanCode /
    /// TKeyCode via `keymap::host_key`; the translated scan is what reaches the
    /// WsEvent queue.
    pub fn deliver_key(&mut self, scan: u32, text: &str, modifiers: Modifiers, down: bool) -> Result<(), String> {
        let env = self.host.env.ok_or_else(|| String::from("the application has no environment yet: nothing can receive a key"))?;
        let mods = key_modifiers(modifiers)?;
        let (std_scan, key_code) = crate::keymap::host_key(scan, text)?;
        if down {
            self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY_DOWN, code: 0, scan: std_scan, modifiers: mods })?;
            if let Some(code) = key_code {
                self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY, code, scan: std_scan, modifiers: mods })?;
            }
        } else {
            self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY_UP, code: 0, scan: std_scan, modifiers: mods })?;
        }
        Ok(())
    }

    /// Deliver a menu or CBA command ID to the active view (or AppUi).
    pub fn deliver_command(&mut self, cmd: u32) -> Result<(), String> {
        let env = self.host.env.ok_or_else(|| String::from("the application has no environment yet: nothing can receive a command"))?;
        self.ws_enqueue(env, WsEvent::Command(cmd))
    }

    /// Deliver a touch/mouse pointer event into the Symbian Window Server event queue (`EEventPointer`).
    pub fn deliver_pointer(&mut self, x: i32, y: i32, phase: heirloom_ui_abi::PointerPhase) -> Result<(), String> {
        let env = self.host.env.ok_or_else(|| String::from("the application has no environment yet: nothing can receive pointer input"))?;
        let ty = match phase {
            heirloom_ui_abi::PointerPhase::Down => EPOINTER_BUTTON1_DOWN,
            heirloom_ui_abi::PointerPhase::Move => EPOINTER_DRAG,
            heirloom_ui_abi::PointerPhase::Up => EPOINTER_BUTTON1_UP,
        };
        self.ws_enqueue(env, WsEvent::Pointer { ty, x, y })
    }

    /// Queue an initial `EEventFocusGained` window server event (Symbian sends this when a launched app enters the foreground).
    pub fn enqueue_focus_gained(&mut self) -> Result<(), String> {
        let Some(env) = self.host.env else { return Ok(()) };
        self.ws_enqueue(env, WsEvent::Focus(true))
    }

    /// Queue a Window Server redraw (`EEventRedraw`) for an activated window-owning control.
    pub fn enqueue_redraw(&mut self, ctrl: u32) -> Result<(), String> {
        let Some(env) = self.host.env else { return Ok(()) };
        self.ws_enqueue(env, WsEvent::Redraw(ctrl))
    }

    /// Deliver a native control intent (list row selection, scroll, or activation) to a projected `CEikListBox`.
    pub fn deliver_intent(
        &mut self,
        control: heirloom_ui_abi::UiObjectRef,
        token: heirloom_ui_abi::CallbackToken,
        generation: u32,
        intent: heirloom_ui_abi::Intent,
    ) -> Result<(), String> {
        let env = self.host.env.ok_or_else(|| String::from("the application has no environment yet: nothing can receive a control intent"))?;
        let lb = token.0 as u32;
        let matches_live = matches!(
            self.obj_get(lb),
            Some(crate::objects::HostObj::List(l)) if l.control == Some(control) && control.generation == generation
        );
        if !matches_live {
            return Err(format!("control {control:?} (token {lb:#x}, gen {generation}) is not a live list box"));
        }
        let view = self.rd(lb + crate::listbox::LB_I_VIEW)?;
        if view == 0 {
            return Err(format!("list box {lb:#x} has no view"));
        }
        let count = self.list_rows(lb)?.len() as u32;
        match intent {
            heirloom_ui_abi::Intent::SelectRow(row) => {
                if count > 0 {
                    let clamped = row.min(count - 1);
                    self.wr(view + crate::listbox::VIEW_I_CURRENT, clamped)?;
                    self.list_scroll_to_current(lb, view, clamped as i32)?;
                    self.list_sync(lb)?;
                }
                Ok(())
            }
            heirloom_ui_abi::Intent::ScrollTo(top) => {
                let max_top = count.saturating_sub(1);
                self.wr(view + crate::listbox::VIEW_I_TOP, top.min(max_top))?;
                self.list_sync(lb)?;
                Ok(())
            }
            heirloom_ui_abi::Intent::Activate => {
                if count > 0 {
                    // Deliver S60 center-select (EKeyOK / EStdKeyDevice3) through the control stack:
                    // containers handling EKeyOK directly run their handler, and containers that
                    // delegate to iListBox->OfferKeyEventL notify MEikListBoxObserver::HandleListBoxEventL.
                    const EKEY_OK: u32 = 0xf845;
                    const ESTD_KEY_DEVICE3: u32 = 0xa7;
                    self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY_DOWN, code: 0, scan: ESTD_KEY_DEVICE3, modifiers: 0 })?;
                    self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY, code: EKEY_OK, scan: ESTD_KEY_DEVICE3, modifiers: 0 })?;
                    self.ws_enqueue(env, WsEvent::Key { ty: EEVENT_KEY_UP, code: 0, scan: ESTD_KEY_DEVICE3, modifiers: 0 })?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// `CCoeEnv::RunL()` (COEMAIN.CPP:229-250).
    fn env_run_l(&mut self, env: u32) -> Result<(), String> {
        match self.rd(env + ACTIVE_I_STATUS)? as i32 {
            0 => {}
            KERR_CANCEL | KERR_SERVER_TERMINATED => return Ok(()),
            other => return Err(format!("CCoeEnv::RunL with status {other} (ECoePanicWservBlindCommandFailed on a device)")),
        }
        let ev = self.nat.ws.queue.pop_front().ok_or_else(|| String::from("CCoeEnv::RunL with no window server event (GetEvent would block)"))?;
        self.ws_request_event(env)?;
        let appui = self.rd(env + COEENV_I_APPUI)?;
        if appui == 0 {
            return Err(String::from("CCoeEnv::RunL: the environment has no AppUi to handle the event"));
        }
        match ev {
            WsEvent::Key { ty, code, scan, modifiers } => {
                let block = match self.nat.ws.block {
                    Some(b) => b,
                    None => {
                        let b = self.alloc_zeroed(TWS_EVENT_BYTES).ok_or_else(|| String::from("no guest heap for the window server event"))?;
                        self.nat.ws.block = Some(b);
                        b
                    }
                };
                self.wr(block, ty)?;
                self.wr(block + TWS_I_HANDLE, env)?;
                for (i, v) in [code, scan, modifiers, 0].into_iter().enumerate() {
                    self.wr(block + TWS_I_EVENT_DATA + 4 * i as u32, v)?;
                }
                let spec = SlotSpec { class: "CCoeAppUi", method: "HandleWsEventL", slot: crate::keyroute::SLOT_HANDLE_WS_EVENT_L, inherited: crate::keyroute::INHERITED_HANDLE_WS_EVENT_L };
                let target = anchor(self, appui, &spec)?;
                self.call_guest(target, &[appui, block, env], Continuation::Resume)
            }
            WsEvent::Pointer { ty, x, y } => {
                let block = match self.nat.ws.block {
                    Some(b) => b,
                    None => {
                        let b = self.alloc_zeroed(TWS_EVENT_BYTES).ok_or_else(|| String::from("no guest heap for the window server event"))?;
                        self.nat.ws.block = Some(b);
                        b
                    }
                };
                self.wr(block, EEVENT_POINTER)?;
                self.wr(block + TWS_I_HANDLE, env)?;
                // TPointerEvent (w32std.h): iType (+0), iModifiers (+4), iPosition (+8,+12), iParentPosition (+16,+20)
                let pe = block + TWS_I_EVENT_DATA;
                self.wr(pe, ty)?;
                self.wr(pe + 4, 0)?;
                self.wr(pe + 8, x as u32)?;
                self.wr(pe + 12, y as u32)?;
                self.wr(pe + 16, x as u32)?;
                self.wr(pe + 20, y as u32)?;
                let targets = self.stack_key_targets(appui).unwrap_or_default();
                let target_ctrl = targets.into_iter().find(|&c| {
                    self.rd(c).ok()
                        .and_then(|vt| self.rd(vt.wrapping_add(crate::control::SLOT_HANDLE_POINTER_EVENT_L * 4)).ok())
                        .is_some_and(|fn_ptr| self.is_guest_code(fn_ptr))
                });
                if let Some(ctrl) = target_ctrl {
                    let vt = self.rd(ctrl)?;
                    let fn_ptr = self.rd(vt.wrapping_add(crate::control::SLOT_HANDLE_POINTER_EVENT_L * 4))?;
                    self.call_guest(fn_ptr, &[ctrl, pe], Continuation::Resume)
                } else {
                    Ok(())
                }
            }
            WsEvent::Focus(gained) => {
                let block = match self.nat.ws.block {
                    Some(b) => b,
                    None => {
                        let b = self.alloc_zeroed(TWS_EVENT_BYTES).ok_or_else(|| String::from("no guest heap for the window server event"))?;
                        self.nat.ws.block = Some(b);
                        b
                    }
                };
                self.wr(block, if gained { EEVENT_FOCUS_GAINED } else { EEVENT_FOCUS_LOST })?;
                self.wr(block + TWS_I_HANDLE, env)?;
                let spec = SlotSpec { class: "CCoeAppUi", method: "HandleWsEventL", slot: crate::keyroute::SLOT_HANDLE_WS_EVENT_L, inherited: crate::keyroute::INHERITED_HANDLE_WS_EVENT_L };
                let target = anchor(self, appui, &spec)?;
                self.call_guest(target, &[appui, block, env], Continuation::Resume)
            }
            WsEvent::Redraw(ctrl) => {
                self.draw_guest_control(ctrl);
                Ok(())
            }
            WsEvent::Command(cmd) => crate::viewappui::dispatch_command(self, appui, cmd),
            WsEvent::ListEvent { lb, event } => {
                let obs = self.rd(lb + crate::listbox::LB_I_OBSERVER)?;
                if obs == 0 {
                    return Ok(());
                }
                let spec = SlotSpec { class: "MEikListBoxObserver", method: "HandleListBoxEventL", slot: crate::listkey::SLOT_HANDLE_LIST_BOX_EVENT_L, inherited: &[] };
                let target = anchor(self, obs, &spec)?;
                self.log.push(format!("[list] {lb:#x} HandleListBoxEventL(obs={obs:#x}, ev={event})"));
                self.call_guest(target, &[obs, lb, event], Continuation::Resume)
            }
        }
    }
}

/// The host's modifier bits as `TEventModifier` (e32keys.h:64-78). META has no Symbian modifier: refused, not dropped.
fn key_modifiers(m: Modifiers) -> Result<u32, String> {
    if m.has(Modifiers::META) {
        return Err(String::from("the Meta modifier has no Symbian TEventModifier"));
    }
    let mut out = 0;
    if m.has(Modifiers::SHIFT) { out |= 0x0000_0400; } // EModifierShift
    if m.has(Modifiers::CTRL) { out |= 0x0000_0080; } // EModifierCtrl
    if m.has(Modifiers::ALT) { out |= 0x0000_0010; } // EModifierAlt
    Ok(out)
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if name != ENV_RUN_L { return None; }
    let env = p.cpu.regs[0];
    Some(match p.env_run_l(env) {
        Ok(()) => NativeOutcome::Return,
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
    })
}
