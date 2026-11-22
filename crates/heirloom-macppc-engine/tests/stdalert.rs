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

#[test]
fn classic_alerts_ask_host_from_alrt_ditl_and_modal_dialog_ignores_ppc_filterproc() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // Build a minimal resource fork containing ALRT 128 -> DITL 128 (static text + Cancel + OK) and DLOG 129 -> DITL 128.
    let mut ditl = Vec::new();
    ditl.extend_from_slice(&2u16.to_be_bytes()); // count - 1 = 2 (3 items)
    let mut add_item = |rect: [i16; 4], ty: u8, text: &[u8]| {
        ditl.extend_from_slice(&0u32.to_be_bytes()); // placeholder handle
        for v in rect { ditl.extend_from_slice(&v.to_be_bytes()); }
        ditl.push(ty);
        ditl.push(text.len() as u8);
        ditl.extend_from_slice(text);
        if text.len() % 2 == 1 { ditl.push(0); }
    };
    add_item([60, 140, 80, 210], 4, b"OK");      // item 1: button
    add_item([60, 20, 80, 100], 4, b"Cancel");   // item 2: button
    add_item([10, 20, 40, 210], 8, b"Proceed?"); // item 3: static text
    let mut alrt = Vec::new();
    for v in [40i16, 40, 140, 280, 128, 0] { alrt.extend_from_slice(&v.to_be_bytes()); }
    let mut dlog = Vec::new();
    for v in [40i16, 40, 140, 280, 1] { dlog.extend_from_slice(&v.to_be_bytes()); }
    dlog.push(1); dlog.push(0); // visible=true, pad
    dlog.push(0); dlog.push(0); // goAway=false, pad
    dlog.extend_from_slice(&0u32.to_be_bytes()); // refCon
    dlog.extend_from_slice(&128i16.to_be_bytes()); // ditl id
    dlog.push(0); // empty title
    let fork = heirloom_fmt_resfork::writer::ForkBuilder::new()
        .add(b"DITL", 128, None, &ditl)
        .add(b"ALRT", 128, None, &alrt)
        .add(b"DLOG", 129, None, &dlog)
        .build();
    m.mac.rm.open_fork(heirloom_mac68k_engine::resmgr::APP_REF_NUM, heirloom_fmt_resfork::ResFork::parse(fork).unwrap());

    // StopAlert(128, non_nil_filter) asks host with RequestKind::Stop and returns chosen item in r3.
    m.bind("StopAlert");
    let mut regs = [0u32; 32];
    regs[3] = 128; regs[4] = 0x123456;
    assert_eq!(m.glue_call("StopAlert", &mut regs), GlueOutcome::Idle);
    let r = request(&m);
    assert_eq!((r.kind, r.text.as_str()), (RequestKind::Stop, "Proceed?"));
    assert_eq!(r.buttons, vec!["Cancel", "OK"]);
    let req = m.mac.tr.dlg.asked.as_ref().unwrap().req;
    m.mac.ui.answer_request(req, 0).unwrap(); // "Cancel" -> DITL item 2
    assert!(m.mac.modal_ready());
    assert_eq!(m.glue_call("StopAlert", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3] as i32, 2);

    // NEGATIVE CONTROL: missing ALRT resource returns -1 without parking.
    regs[3] = 999; regs[4] = 0;
    assert_eq!(m.glue_call("StopAlert", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3] as i32, -1);

    // ModalDialog with a non-NIL filterProc on PPC must park cleanly (NOT NeedsCpuRegisters).
    m.bind("GetNewDialog");
    m.bind("ModalDialog");
    let mut dregs = [0u32; 32];
    dregs[3] = 129; dregs[4] = 0; dregs[5] = 0xFFFF_FFFF;
    assert_eq!(m.glue_call("GetNewDialog", &mut dregs), GlueOutcome::Return);
    assert_ne!(dregs[3], 0);
    let mut mregs = [0u32; 32];
    mregs[3] = 0x002A_ADE8; // non-NIL filterProc
    mregs[4] = HIT;
    assert_eq!(m.glue_call("ModalDialog", &mut mregs), GlueOutcome::Idle);
    assert!(m.mac.log.iter().any(|l| l.contains("filterProc 0x2aade8")));
}

#[test]
fn script_keys_lowmem_and_desk_manager_calls_succeed_on_ppc() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // GetScriptVariable(smRoman=0, smScriptKeys=32) -> 0
    m.bind("GetScriptVariable");
    let mut regs = [0u32; 32];
    regs[3] = 0; regs[4] = 32;
    assert_eq!(m.glue_call("GetScriptVariable", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3], 0);

    // New low-memory globals: LMGetKbdType (2), LMGetFSFCBLen (94), LMGetHWCfgFlags / LMSetHWCfgFlags
    for (name, expect) in [("LMGetKbdType", 2u32), ("LMGetFSFCBLen", 94u32)] {
        m.bind(name);
        let mut r = [0u32; 32];
        assert_eq!(m.glue_call(name, &mut r), GlueOutcome::Return);
        assert_eq!(r[3], expect, "{name}");
    }
    m.bind("LMSetHWCfgFlags");
    m.bind("LMGetHWCfgFlags");
    let mut r = [0u32; 32];
    r[3] = 0x0200;
    assert_eq!(m.glue_call("LMSetHWCfgFlags", &mut r), GlueOutcome::Return);
    assert_eq!(m.glue_call("LMGetHWCfgFlags", &mut r), GlueOutcome::Return);
    assert_eq!(r[3], 0x0200);

    // Desk Manager: SystemEdit -> FALSE (0), OpenDeskAcc -> 0, CloseDeskAcc & SystemClick -> Return
    for name in ["SystemEdit", "OpenDeskAcc", "CloseDeskAcc", "SystemClick"] {
        m.bind(name);
        let mut dr = [0u32; 32];
        assert_eq!(m.glue_call(name, &mut dr), GlueOutcome::Return, "{name}");
        if matches!(name, "SystemEdit" | "OpenDeskAcc") {
            assert_eq!(dr[3], 0, "{name}");
        }
    }
}
