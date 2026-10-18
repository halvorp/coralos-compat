//! The SAME dialog program the 68k test runs (`mac68k_engine::fixture::dialog_script`), driven through the PowerPC glue the
//! way a PPC CPU calls the Toolbox: r3.. = the Pascal arguments, a `GlueOutcome` back. The guest-visible result must be the
//! one observation both CPUs share (`DIALOG_EXPECTED`): the personality is the Toolbox, not the processor.
use guestui_abi::{ControlValue, Intent, RecordingGuestUi, UiObjectRef};
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
    assert!(m.mac.modal_ready(), "typing in the edit field wakes it");
    let at = run_from(&mut m, at);
    assert_eq!(at, 6, "the second ModalDialog parks the caller: {:#?}", m.mac.log);
    m.mac.ui.push_intent(win, check, Intent::SetValue(ControlValue::Checked(true))).unwrap();
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

#[test]
fn control_manager_traps_through_the_glue_answer_like_the_68k_run() {
    // FindControl / TrackControl / GetControlValue / SetControlValue are the 68k handlers; a native click on a push button and
    // on a check box gives the part codes translate_controls.rs pins for 68k code (10 and 11), and the check box value stays
    // the program's to change.
    use mac68k_engine::pascal::Rect;
    let mut m = machine();
    let spec = mac68k_engine::toolbox::WindowSpec { storage: 0, rect: Rect { top: 100, left: 50, bottom: 220, right: 250 }, title: "w".into(), visible: true, proc_id: 0, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
    let win = m.mac.make_window(spec).unwrap();
    let btn = m.mac.create_control(win, Rect { top: 10, left: 20, bottom: 34, right: 100 }, "OK", true, 0, 0, 1, 0, 0).unwrap();
    let chk = m.mac.create_control(win, Rect { top: 50, left: 20, bottom: 66, right: 120 }, "Check", true, 0, 0, 1, 1, 0).unwrap();
    let g = |m: &mut M, name: &str, args: &[u32]| -> u32 {
        m.bind(name);
        let mut regs = [0u32; 32];
        regs[3..3 + args.len()].copy_from_slice(args);
        assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}");
        regs[3]
    };
    let nwin = m.mac.windows[&win].native.unwrap();
    let click = |m: &mut M, ctl: u32, intent: Intent| {
        let n = m.mac.tr.projected(Obj::Control(ctl)).unwrap().native;
        m.mac.ui.push_intent(nwin, n, intent).unwrap();
        m.mac.pump_ui();
    };
    for (ctl, intent, local, part) in [(btn, Intent::Activate, (22i16, 60i16), 10u32), (chk, Intent::SetValue(ControlValue::Checked(true)), (58, 70), 11)] {
        click(&mut m, ctl, intent);
        let pt = ((local.0 as u16 as u32) << 16) | local.1 as u16 as u32;
        let which = 0x3300;
        assert_eq!(g(&mut m, "FindControl", &[pt, win, which]) & 0xFFFF, part);
        assert_eq!(m.mac.mem.read_u32(which).unwrap(), ctl);
        assert_eq!(g(&mut m, "TrackControl", &[ctl, pt, 0]) & 0xFFFF, part);
    }
    assert_eq!(g(&mut m, "GetControlValue", &[chk]) & 0xFFFF, 0, "the click did not flip the check box");
    g(&mut m, "SetControlValue", &[chk, 1]);
    assert_eq!(g(&mut m, "GetControlValue", &[chk]) & 0xFFFF, 1);
    // negative control: a point outside every control finds nothing
    assert_eq!(g(&mut m, "FindControl", &[(200u32 << 16) | 200, win, 0x3300]) & 0xFFFF, 0);
    assert_eq!(m.mac.mem.read_u32(0x3300).unwrap(), 0);
    assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
}

/// The PowerPC CPU itself calls the Toolbox: a real fragment (`fixture::dialog_app`) runs `GetNewDialog` and `ModalDialog`
/// through retro-ppc and the glue. The caller PARKS in `ModalDialog` (the `wait_dialog` path of `MacPpc::step`), a native
/// intent wakes it, and the item hit lands in the program's own data section.
mod real_ppc {
    use super::*;
    use macppc_engine::cfm::Fragment;
    use macppc_engine::fixture::{dialog_app, DIALOG_HIT_AT};
    use macppc_engine::Step;
    use retro_pef::Pef;

    fn boot() -> (M, Fragment, u32) {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.attach_resources(&dialog_fork()).unwrap();
        let frag = m.instantiate("dialog", &Pef::parse(&dialog_app()).unwrap()).unwrap();
        m.start(&frag).unwrap();
        let r2 = m.cpu().unwrap().gpr[2];
        (m, frag, r2)
    }
    fn hit(m: &M, r2: u32) -> u16 { m.mac.mem.read_u16((r2 as i32 + DIALOG_HIT_AT + 2) as u32).unwrap() }

    #[test]
    fn a_ppc_program_parks_in_modal_dialog_and_wakes_on_a_native_hit() {
        let (mut m, _f, r2) = boot();
        let (step, stats) = m.step(1_000_000);
        assert_eq!(step, Step::Blocked, "{:#?}\nfault: {:?}", m.mac.log, m.fault());
        assert!(stats.insns > 10, "PowerPC instructions ran before the park: {}", stats.insns);
        assert!(m.mac.log.iter().any(|l| l.contains("ModalDialog: no item hit yet")), "{:#?}", m.mac.log);
        // negative control: with no hit, stepping again changes nothing and the program is still parked
        assert_eq!(m.step(1_000_000).0, Step::Blocked);
        assert_eq!(hit(&m, r2), 0);
        let (win, cancel) = item(&m, 2);
        m.mac.ui.push_intent(win, cancel, Intent::Activate).unwrap();
        let (step, _) = m.step(1_000_000);
        assert_eq!(step, Step::Exited, "{:#?}\nfault: {:?}", m.mac.log, m.fault());
        assert_eq!(hit(&m, r2), 2, "ModalDialog returned Cancel's item number to the PowerPC code");
        assert!(m.mac.refused.is_empty(), "{:?}", m.mac.refused);
    }

    #[test]
    fn typing_in_the_edit_field_wakes_it_with_the_edit_items_number() {
        let (mut m, _f, r2) = boot();
        assert_eq!(m.step(1_000_000).0, Step::Blocked);
        let (win, edit) = item(&m, 5);
        m.mac.ui.push_intent(win, edit, Intent::CommitText("typed".into())).unwrap();
        assert_eq!(m.step(1_000_000).0, Step::Exited, "{:#?}", m.mac.log);
        assert_eq!(hit(&m, r2), 5);
    }
}
