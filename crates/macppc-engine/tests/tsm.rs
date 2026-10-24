//! Text Services Manager documents with no input method installed: the ID bookkeeping a caller checks, nothing pending to fix, and the input window said-by-name to be unused.
use guestui_abi::RecordingGuestUi;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3200;
fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
#[test]
fn documents_are_created_activated_fixed_and_deleted_and_a_stale_id_is_an_error() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(call(&mut m, "NewTSMDocument", &[1, 0x3300, OUT, 7]), 0);
    let id = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(id, 0);
    assert_eq!((call(&mut m, "ActivateTSMDocument", &[id]), call(&mut m, "FixTSMDocument", &[id]), call(&mut m, "DeactivateTSMDocument", &[id])), (0, 0, 0));
    assert_eq!(call(&mut m, "UseInputWindow", &[id, 1]), 0);
    assert!(m.mac.log.iter().any(|l| l.contains("[tsm] UseInputWindow") && l.contains("no input method is installed")), "{:?}", m.mac.log);
    assert_eq!(call(&mut m, "DeleteTSMDocument", &[id]), 0);
    // NEGATIVE CONTROLS: the deleted ID, and an ID never issued, are tsmInvalidDocIDErr; a NIL result pointer is paramErr
    for f in ["ActivateTSMDocument", "DeactivateTSMDocument", "FixTSMDocument", "DeleteTSMDocument"] { assert_eq!(call(&mut m, f, &[id]), -2505, "{f}"); }
    assert_eq!(call(&mut m, "UseInputWindow", &[0x1234, 0]), -2505);
    assert_eq!(call(&mut m, "NewTSMDocument", &[1, 0x3300, 0, 7]), -50);
}
