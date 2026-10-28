//! `Alert`, `StopAlert`, `NoteAlert`, `CautionAlert` on the Dialog Manager table: an `ALRT` resource names a rectangle and
//! a `DITL`; the alert is the SAME toolkit object a dialog is (the item table in [`super::dialogs`] gives its controls
//! their roles), shown modally. The call returns the number of the item the user hit, as `ModalDialog` does, and then the
//! alert window is disposed (a program never sees an alert's window).
//!
//! **Asked, not drawn.** An alert whose `DITL` is a message and buttons (static text and push buttons, icon and picture items
//! ignored: the host draws the icon the TRAP names) is ASKED of the host (`GuestUi::request`): the static text is the message
//! (`ParamText` applied, the lines joined by `\n`), the buttons are listed left to right as the alert lays them out, the default
//! is item 1 (what Return hits) and the cancel the button titled "Cancel". `Alert` asks with no kind, `StopAlert` / `NoteAlert` /
//! `CautionAlert` with the kind their name says ([`RequestKind`]); the call returns the item number of the button the user chose
//! ([`Mac::complete_alert`]). A host that refuses requests (`UiRefusal`), and a `DITL` the request cannot carry (an edit field, a
//! check box, a radio button, a user item), take the old path: the DITL becomes a native dialog window, shown modally.
//!
//! The stage count and sound of an `ALRT` are not modelled. A missing `ALRT` makes the call return -1, as the Dialog Manager
//! does. A non-NIL `filterProc` is a guest callback the trap bridge cannot run from inside a trap: named as a refusal, the
//! standard filtering applies.
use super::dialogs::{parse_ditl, Def};
use crate::machine::{Call, Done, Mac};
use crate::pascal::{Arg, Rect};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::{GuestUi, RequestKind, RequestModel, UiObjectRef, UiRefusal};
use crate::cpu::Cpu;

/// Bytes of the Pascal frame after the trap: alertID (2), filterProc (4); the INTEGER result slot lies just above.
const ARGS: u32 = 6;

/// An alert the host was asked: where the call's result goes, the button item numbers (index = the request's button) and, once the
/// user has chosen, the item number to return.
#[derive(Debug, Clone)]
pub struct Asked { pub req: UiObjectRef, pub items: Vec<i16>, pub slot: u32, pub answer: Option<i16> }

/// The kind each alert trap names (`Alert` names none).
fn kind_of(trap: u16) -> RequestKind {
    match trap { 0xA986 => RequestKind::Stop, 0xA987 => RequestKind::Note, 0xA988 => RequestKind::Caution, _ => RequestKind::Unspecified }
}

impl<U: GuestUi> Mac<U> {
    /// The request an `ALRT`'s `DITL` stands for, or None when it holds an item a request cannot carry. The buttons come in on-screen
    /// order (left to right; item number breaks a tie), each with its DITL item number.
    fn alert_request(&mut self, ditl: i16, kind: RequestKind) -> Option<(RequestModel, Vec<i16>)> {
        let h = self.get_resource(*b"DITL", ditl, false).ok()?;
        let items = parse_ditl(&self.handle_bytes(h)?).ok()?;
        let (mut text, mut buttons) = (Vec::new(), Vec::new());
        for (i, it) in items.iter().enumerate() {
            let n = i as i16 + 1;
            let label: String = it.data.iter().map(|&c| char::from(c)).collect();
            match it.ty & 0x7F {
                4 => buttons.push((it.rect.left, n, label)),
                8 => text.push((it.rect.top, self.substitute_params(&label))),
                32 | 64 => {} // an icon or a picture: the host draws the kind's icon
                _ => return None, // an edit field, a check box, a radio button, a user item: a dialog's business
            }
        }
        if buttons.is_empty() || buttons.len() > 4 { return None; }
        text.sort_by_key(|t| t.0);
        buttons.sort_by_key(|b| (b.0, b.1));
        let cancel = buttons.iter().position(|b| b.2.eq_ignore_ascii_case("cancel")).map(|i| i as u32);
        let default = buttons.iter().position(|b| b.1 == 1).map(|i| i as u32);
        let model = RequestModel {
            title: String::new(),
            text: text.into_iter().map(|t| t.1).collect::<Vec<_>>().join("\n"),
            buttons: buttons.iter().map(|b| b.2.clone()).collect(),
            default,
            cancel,
            kind,
        };
        Some((model, buttons.iter().map(|b| b.1).collect()))
    }

    /// Try to ASK the host instead of drawing the DITL. True = the call is parked on the answer.
    fn ask_alert(&mut self, cpu: &mut Cpu, trap: u16, ditl: i16) -> Result<bool, Refusal> {
        let Some((model, items)) = self.alert_request(ditl, kind_of(trap)) else { return Ok(false) };
        match self.ui.request(None, model) {
            Ok(req) => {
                let slot = cpu.areg(7).wrapping_add(ARGS);
                self.tr.dlg.asked = Some(Asked { req, items, slot, answer: None });
                cpu.block();
                Ok(true)
            }
            // a host with no alert part (or one the kit cannot show): the dialog path, named
            Err(UiRefusal::Unsupported(why)) => { self.log.push(format!("[dialog] alert not asked: {why}; shown as a dialog")); Ok(false) }
            Err(e) => Err(Refusal::ui(e)),
        }
    }

    /// The host answered request `req` with `button`: the call's result becomes that button's item number.
    pub(crate) fn alert_answered(&mut self, req: UiObjectRef, button: u32) -> Result<(), Refusal> {
        let Some(a) = self.tr.dlg.asked.as_mut().filter(|a| a.req == req && a.answer.is_none()) else { return Err(Refusal(format!("deliver: RequestAnswered {{ req: {req:?}, button: {button} }}: not an open request of this Mac"))) };
        let Some(&item) = a.items.get(button as usize) else { return Err(Refusal(format!("deliver: RequestAnswered: button {button} of an alert with {} buttons", a.items.len()))) };
        a.answer = Some(item);
        Ok(())
    }

    /// Finish a blocked, ASKED alert if the host has answered: the item number goes to the call's result. False while it must stay blocked.
    pub(crate) fn complete_alert(&mut self) -> bool {
        self.pump_ui();
        let Some(a) = self.tr.dlg.asked.as_ref().filter(|a| a.answer.is_some()) else { return false };
        let (slot, item) = (a.slot, a.answer.unwrap_or(0));
        self.tr.dlg.asked = None;
        if let Err(Refusal(why)) = self.mem.write_u16(slot, item as u16).map_err(Refusal::mem("itemHit")) { self.note_refusal("Alert", why); }
        true
    }
}

fn alert<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (id, filter) = (c.args.get(0) as u16 as i16, c.args.get(1));
    if filter != 0 { m.note_refusal(c.name, format!("filterProc {filter:#x} is a guest callback and is not called; the standard filtering applies")); }
    let Ok(h) = m.get_resource(*b"ALRT", id, false) else {
        return Ok(Done { result: 0xFFFF, log: format!("[dialog] {} {id}: no ALRT resource -> -1", c.name) });
    };
    let b = m.handle_bytes(h).unwrap_or_default();
    if b.len() < 12 { return Err(Refusal(format!("{} {id}: ALRT resource is {} bytes, too short for its fields", c.name, b.len()))); }
    let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
    if m.ask_alert(cpu, c.opcode, be(8))? {
        return Ok(Done { result: 0, log: format!("[dialog] {} {id} asked of the host; blocks on its answer", c.name) });
    }
    let def = Def { what: format!("{} {id}", c.name), rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) }, title: String::new(), proc_id: 1, go_away: false, ref_con: 0, ditl: be(8), show: true, color: false, items: None };
    let win = m.build_dialog(def, 0, 0xFFFF_FFFF)?;
    // The modal call blocks; the item number is written into the result slot when the hit arrives.
    m.tr.dlg.alert = Some(win);
    m.tr.dlg.modal = Some(cpu.areg(7).wrapping_add(ARGS));
    cpu.block();
    Ok(Done { result: 0, log: format!("[dialog] {} {id} -> {win:#x} blocks (stages/sound/icon not modelled)", c.name) })
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    for op in [0xA985, 0xA986, 0xA987, 0xA988] { m.register(op, &[Arg::W, Arg::L], Some(Arg::W), alert::<U>)?; }
    Ok(())
}
