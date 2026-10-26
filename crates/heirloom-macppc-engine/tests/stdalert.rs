//! StandardAlert asked of the host as the native kit alert: message, buttons from the parameter record in the Platinum order, kind from the alert type, the call parked until the
//! host answers, and the Dialogs.h item number returned.
use heirloom_ui_abi::{RecordingGuestUi, RequestKind, UiCall};
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const ERR: u32 = 0x3200;
const EXPL: u32 = 0x3300;
const PARAM: u32 = 0x3400;
const HIT: u32 = 0x3500;
const T_SAVE: u32 = 0x3600;

fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }
fn sa(m: &mut M, kind: u32, param: u32) -> GlueOutcome {
    m.bind("StandardAlert");
    let mut regs = [0u32; 32];
    regs[3] = kind; regs[4] = ERR; regs[5] = EXPL; regs[6] = param; regs[7] = HIT;
    let o = m.glue_call("StandardAlert", &mut regs);
    LAST.with(|l| l.set(regs[3]));
    o
}
thread_local! { static LAST: std::cell::Cell<u32> = const { std::cell::Cell::new(99) }; }
fn request(m: &M) -> heirloom_ui_abi::RequestModel { m.mac.ui.calls.iter().rev().find_map(|c| if let UiCall::Request(_, r) = c { Some(r.clone()) } else { None }).expect("a request") }

#[test]
fn a_standard_alert_is_asked_parked_and_answered_with_the_dialogs_h_item_number() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    pstr(&mut m, ERR, "Save changes?");
    pstr(&mut m, EXPL, "Your changes will be lost.");
    pstr(&mut m, T_SAVE, "Save");
    // AlertStdAlertParamRec (68k-aligned, the layout Warcraft III World Editor's own record has): defaultText "Save", cancelText standard (-1), otherText standard "Don't Save"; default button 1, cancel button 2
    for (o, v) in [(6u32, T_SAVE), (10, 0xFFFF_FFFF), (14, 0xFFFF_FFFF)] { m.mac.mem.write_u32(PARAM + o, v).unwrap(); }
    m.mac.mem.write_u16(PARAM + 18, 1).unwrap();
    m.mac.mem.write_u16(PARAM + 20, 2).unwrap();
    assert_eq!(sa(&mut m, 2, PARAM), GlueOutcome::Idle, "parked on the host's answer");
    let r = request(&m);
    assert_eq!((r.text.as_str(), r.kind), ("Save changes?\nYour changes will be lost.", RequestKind::Caution));
    assert_eq!(r.buttons, vec!["Don't Save", "Cancel", "Save"], "left to right: other, cancel, default");
    assert_eq!((r.default, r.cancel), (Some(2), Some(1)));
    // still parked until the host answers (the retry finds no answer)
    assert!(!m.mac.modal_ready());
    // the user picks "Don't Save" (button 0): the retried call stores item 3 and returns noErr
    let req = m.mac.tr.dlg.asked.as_ref().unwrap().req;
    m.mac.ui.answer_request(req, 0).unwrap();
    assert!(m.mac.modal_ready());
    assert_eq!(sa(&mut m, 2, PARAM), GlueOutcome::Return);
    assert_eq!(LAST.with(|l| l.get()), 0);
    assert_eq!(m.mac.mem.read_u16(HIT).unwrap(), 3, "kAlertStdAlertOtherButton");
    assert!(m.mac.tr.dlg.asked.is_none());
}

#[test]
fn null_param_is_one_ok_button_and_bad_input_is_refused_by_name() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    pstr(&mut m, ERR, "Disk full");
    assert_eq!(sa(&mut m, 0, 0), GlueOutcome::Idle);
    let r = request(&m);
    assert_eq!((r.buttons.clone(), r.kind, r.text.as_str()), (vec!["OK".to_string()], RequestKind::Stop, "Disk full"), "no explanation pointer: just the message");
    let req = m.mac.tr.dlg.asked.as_ref().unwrap().req;
    m.mac.ui.answer_request(req, 0).unwrap();
    assert!(m.mac.modal_ready(), "the run loop pumps the host before it retries");
    assert_eq!(sa(&mut m, 0, 0), GlueOutcome::Return);
    assert_eq!(m.mac.mem.read_u16(HIT).unwrap(), 1);
    // spec: an alert type outside 0..3 is a plain alert (no error); itemHit was set to -1 before the question
    // NEGATIVE controls: a host with no alert part (no DITL to fall back to)
    m.mac.ui.refuse_requests = true;
    assert!(matches!(sa(&mut m, 0, 0), GlueOutcome::Refused(w) if w.contains("host cannot ask")));
    m.mac.ui.refuse_requests = false;
    assert_eq!(sa(&mut m, 9, 0), GlueOutcome::Idle, "alert type 9 is a plain alert");
    assert_eq!(request(&m).kind, RequestKind::Unspecified);
    assert_eq!(m.mac.mem.read_u16(HIT).unwrap(), 0xFFFF, "*itemHit is -1 until the answer");
}

#[test]
fn the_spec_validation_vectors_return_param_err_before_anything_is_asked() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    pstr(&mut m, ERR, "x");
    let rec = |m: &mut M, dt: u32, ct: u32, ot: u32, db: u16, cb: u16| {
        m.mac.mem.write_bytes(PARAM, &[0u8; 24]).unwrap();
        for (o, v) in [(6u32, dt), (10, ct), (14, ot)] { m.mac.mem.write_u32(PARAM + o, v).unwrap(); }
        m.mac.mem.write_u16(PARAM + 18, db).unwrap();
        m.mac.mem.write_u16(PARAM + 20, cb).unwrap();
    };
    let n = |m: &M| m.mac.ui.calls.iter().filter(|c| matches!(c, UiCall::Request(..))).count();
    // spec test vectors: {defaultButton=0}: -50; {4}: -50; {1,1}: -50; {2, cancelText NULL, cancelButton 0}: -50 (default collapses to 0 = cancel 0)
    for (dt, ct, ot, db, cb) in [(u32::MAX, 0, 0, 0, 2), (u32::MAX, 0, 0, 4, 0), (u32::MAX, u32::MAX, 0, 1, 1), (u32::MAX, 0, 0, 2, 0)] {
        rec(&mut m, dt, ct, ot, db, cb);
        assert_eq!(sa(&mut m, 0, PARAM), GlueOutcome::Return);
        assert_eq!(LAST.with(|l| l.get()) as i32, -50, "({dt:#x},{ct:#x},{ot:#x},{db},{cb})");
        assert_eq!(m.mac.mem.read_u16(HIT).unwrap(), 0xFFFF);
    }
    assert_eq!(n(&m), 0, "nothing was asked");
    // {defaultButton=1, cancelButton=3, otherText=NULL}: cancel collapses to 0, the alert is asked (NEGATIVE control of the above)
    rec(&mut m, u32::MAX, 0, 0, 1, 3);
    assert_eq!(sa(&mut m, 0, PARAM), GlueOutcome::Idle);
    assert_eq!((request(&m).buttons, request(&m).cancel), (vec!["OK".to_string()], None));
}
