//! `Alert`, `StopAlert`, `NoteAlert`, `CautionAlert` on the Dialog Manager table: an `ALRT` resource names a rectangle and
//! a `DITL`; the alert is the SAME toolkit object a dialog is (the item table in [`super::dialogs`] gives its controls
//! their roles), shown modally. The call returns the number of the item the user hit, as `ModalDialog` does, and then the
//! alert window is disposed (a program never sees an alert's window).
//!
//! The stage count and sound of an `ALRT`, and the Stop / Note / Caution icon a real alert draws in its top-left corner, are
//! not modelled (the icon belongs to the System file, not to the program's `DITL`): the dialog shows only its items. A
//! missing `ALRT` makes the call return -1, as the Dialog Manager does. A non-NIL `filterProc` is a guest callback the trap
//! bridge cannot run from inside a trap: named as a refusal, the standard filtering applies.
use super::dialogs::Def;
use crate::machine::{Call, Done, Mac};
use crate::pascal::{Arg, Rect};
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

/// Bytes of the Pascal frame after the trap: alertID (2), filterProc (4); the INTEGER result slot lies just above.
const ARGS: u32 = 6;

fn alert<U: GuestUi>(m: &mut Mac<U>, cpu: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (id, filter) = (c.args.get(0) as u16 as i16, c.args.get(1));
    if filter != 0 { m.note_refusal(c.name, format!("filterProc {filter:#x} is a guest callback and is not called; the standard filtering applies")); }
    let Ok(h) = m.get_resource(*b"ALRT", id, false) else {
        return Ok(Done { result: 0xFFFF, log: format!("[dialog] {} {id}: no ALRT resource -> -1", c.name) });
    };
    let b = m.handle_bytes(h).unwrap_or_default();
    if b.len() < 12 { return Err(Refusal(format!("{} {id}: ALRT resource is {} bytes, too short for its fields", c.name, b.len()))); }
    let be = |i: usize| i16::from_be_bytes([b[i], b[i + 1]]);
    let def = Def { what: format!("{} {id}", c.name), rect: Rect { top: be(0), left: be(2), bottom: be(4), right: be(6) }, title: String::new(), proc_id: 1, go_away: false, ref_con: 0, ditl: be(8), show: true };
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
