//! The SAME dialog program the 68k test runs (`mac68k_engine::fixture::dialog_script`), driven through the PowerPC glue the
//! way a PPC CPU calls the Toolbox: r3.. = the Pascal arguments, a `GlueOutcome` back. The guest-visible result must be the
//! one observation both CPUs share (`DIALOG_EXPECTED`): the personality is the Toolbox, not the processor.
use guestui_abi::{ControlValue, GuestUiEvent, Intent, Modifiers, RecordingGuestUi, UiObjectRef};
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use mac68k_engine::fixture::{cells, dialog_fork, dialog_observation, dialog_script, ScriptArg, DIALOG_EXPECTED};
use mac68k_engine::pascal::Arg;
use mac68k_engine::translate::Obj;

type M = MacPpc<RecordingGuestUi>;

fn machine() -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.rm.open_fork(mac68k_engine::resmgr::APP_REF_NUM, retro_resfork::ResFork::parse(dialog_fork()).unwrap());
    m.mac.mem.write_bytes(cells::NEW_TEXT, &[7, b'C', b'h', b'a', b'n', b'g', b'e', b'd']).unwrap();
    m
}

/// Call script entry `i` as a PowerPC caller would; `Ok(true)` = done, `Ok(false)` = the call parked the CPU (Idle).
fn call(m: &mut M, c: &mac68k_engine::fixture::ScriptCall) -> bool {
    let op = mac68k_engine::mixed::trap_opcode(c.name).unwrap();
    let (layout, result) = m.mac.pascal_signature(op).unwrap();
    m.bind(c.name);
    let mut regs = [0u32; 32];
    for (i, (k, a)) in layout.iter().zip(&c.args).enumerate() {
        regs[3 + i] = match a {
            ScriptArg::Lit(v) => *v,
            ScriptArg::Mem(at) => if *k == Arg::L { m.mac.mem.read_u32(*at).unwrap() } else { u32::from(m.mac.mem.read_u16(*at).unwrap()) },
        };
    }
    match m.glue_call(c.name, &mut regs) {
        GlueOutcome::Return => {}
        GlueOutcome::Idle => return false,
        other => panic!("{}: {other:?}", c.name),
    }
    if let (Some(kind), Some(at)) = (result, c.save) {
        match kind {
            Arg::L => m.mac.mem.write_u32(at, regs[3]).unwrap(),
            Arg::W => m.mac.mem.write_u16(at, regs[3] as u16).unwrap(),
            Arg::B => m.mac.mem.write_u16(at, ((regs[3] & 0xFF) << 8) as u16).unwrap(), // a 68k pop leaves a Boolean in the high byte
        }
    }
    true
}

fn run_from(m: &mut M, from: usize) -> usize {
    let script = dialog_script();
    let mut at = script.len();
    for (i, c) in script.iter().enumerate().skip(from) { if !call(m, c) { at = i; break; } }
    mac68k_engine::qd::flush(&mut m.mac); // what the end of every `MacPpc::step` does
    at
}

fn item(m: &M, n: usize) -> (UiObjectRef, UiObjectRef) {
    let win = *m.mac.tr.dlg.dialogs.keys().next().unwrap();
    let h = m.mac.tr.dlg.dialogs[&win].list[n - 1].handle;
    let p = m.mac.tr.projected(Obj::Control(h)).or_else(|| m.mac.tr.projected(Obj::Text(h))).unwrap();
    (m.mac.windows[&win].native.unwrap(), p.native)
}

#[test]
fn the_modal_dialog_through_the_powerpc_glue_leaves_the_same_guest_state_as_the_68k_run() {
    let mut m = machine();
    let at = run_from(&mut m, 0);
    assert_eq!(at, 1, "the first ModalDialog parks the caller: {:#?}", m.mac.log);
    assert!(!m.mac.modal_ready(), "nothing has been hit yet: the caller stays parked");
    let (win, edit) = item(&m, 5);
    let (_, check) = item(&m, 3);
    m.mac.ui.push_intent(win, edit, Intent::CommitText("World".into())).unwrap();
    m.mac.ui.push_intent(win, check, Intent::SetValue(ControlValue::Checked(true))).unwrap();
    assert!(m.mac.modal_ready(), "the hit wakes it");
    let at = run_from(&mut m, at);
    assert_eq!(at, 6, "the second ModalDialog parks the caller: {:#?}", m.mac.log);
    m.mac.ui.events.push_back(GuestUiEvent::Key { win, code: 36, text: "\r".into(), modifiers: Modifiers::default(), down: true });
    assert!(m.mac.modal_ready());
    assert_eq!(run_from(&mut m, at), dialog_script().len());
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    let got = dialog_observation(&m.mac);
    assert_eq!(got, DIALOG_EXPECTED, "\n{}", got.join("\n"));
}

#[test]
fn negative_control_a_wrong_hit_changes_the_observation() {
    // the comparison is not vacuous: hitting Cancel (item 2) instead of the check box gives a different first line
    let mut m = machine();
    let at = run_from(&mut m, 0);
    let (win, cancel) = item(&m, 2);
    m.mac.ui.push_intent(win, cancel, Intent::Activate).unwrap();
    assert!(m.mac.modal_ready());
    run_from(&mut m, at);
    assert_ne!(dialog_observation(&m.mac)[0], DIALOG_EXPECTED[0]);
    assert!(dialog_observation(&m.mac)[0].starts_with("hit1=2 "));
}
