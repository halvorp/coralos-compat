//! Pointer input into a guest BOOPSI gadget (tests/guest/boopsiinput.s): a native Pointer event is queued by `deliver`,
//! the guest's own WaitPort runs GM_HITTEST -> GM_GOACTIVE -> GM_HANDLEINPUT -> GM_GOINACTIVE on the GUEST dispatcher,
//! and a GMR_VERIFY answer arrives in the program as IDCMP_GADGETUP with the gadget as IAddress and the termination as Code.
//! Controls: a press outside the box and a press the dispatcher's GM_HITTEST rejects start nothing; the message does not
//! appear before the button-up.
use heirloom_ami68k_engine::{Amiga, Delivered, Step};
use heirloom_ui_abi::{GuestUiEvent, PointerPhase, RecordingGuestUi};
use heirloom_fmt_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/boopsiinput.bin");

fn start() -> (Amiga<RecordingGuestUi>, u32) {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(2_000_000);
    assert_eq!(s, Step::Blocked, "{:?} {:?}", a.refused, &a.log[a.log.len().saturating_sub(6)..]);
    (a, loaded.bases[0])
}
fn c(a: &Amiga<RecordingGuestUi>, b: u32, i: u32) -> u32 { a.mem.read_u32(b + 8 + 4 * i).unwrap() }
fn point(a: &mut Amiga<RecordingGuestUi>, x: i32, y: i32, phase: PointerPhase) -> Delivered {
    let win = *a.windows.values().next().expect("an open window");
    let d = a.deliver(GuestUiEvent::primary(win, x, y, phase)).unwrap();
    let (s, _) = a.step(2_000_000);
    assert!(matches!(s, Step::Blocked | Step::Exited), "{s:?} {:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    d
}

#[test]
fn press_and_release_drive_the_guest_gadget_and_post_gadgetup() {
    let (mut a, b) = start();
    assert_eq!(point(&mut a, 15, 8, PointerPhase::Down), Delivered::Handled);
    assert_eq!((c(&a, b, 0), c(&a, b, 1), c(&a, b, 2)), (1, 1, 0), "hittest and goactive ran; no input yet");
    assert_eq!(c(&a, b, 4), 5, "GM_HITTEST got the mouse relative to the gadget (15 - 10)");
    assert_eq!(c(&a, b, 9), 0, "the program is still waiting: no IDCMP message before the button-up");
    point(&mut a, 16, 8, PointerPhase::Move);
    assert_eq!(c(&a, b, 2), 1, "a move goes to GM_HANDLEINPUT");
    assert_eq!(c(&a, b, 3), 0, "still active");
    point(&mut a, 16, 8, PointerPhase::Up);
    assert_eq!((c(&a, b, 2), c(&a, b, 3)), (2, 1), "the release is handled, then GM_GOINACTIVE");
    assert_eq!(c(&a, b, 9), 1, "the program received its message and ended");
    assert_eq!(c(&a, b, 5), 0x40, "IDCMP_GADGETUP");
    assert_eq!(c(&a, b, 6), 123, "Code = *gpi_Termination set by the guest dispatcher");
    assert_eq!(c(&a, b, 7), c(&a, b, 8), "IAddress = the gadget");
}

#[test]
fn a_press_outside_or_rejected_by_hittest_starts_nothing() {
    let (mut a, b) = start();
    point(&mut a, 50, 40, PointerPhase::Down);
    assert_eq!(c(&a, b, 0), 0, "outside the box: GM_HITTEST is not even asked");
    point(&mut a, 28, 8, PointerPhase::Down); // inside the box, gadget-relative x = 18 >= 15: the guest says no
    assert_eq!((c(&a, b, 0), c(&a, b, 1)), (1, 0), "asked, rejected, never activated");
    point(&mut a, 28, 8, PointerPhase::Up);
    assert_eq!((c(&a, b, 2), c(&a, b, 9)), (0, 0), "nothing active, nothing posted");
}

// ---- two tasks: the window owner is parked in WaitPort while a busy worker task runs ----

const TWO: &[u8] = include_bytes!("guest/boopsitwo.bin");

#[test]
fn a_click_reaches_the_gadget_of_a_parked_task_while_another_task_runs() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(TWO.to_vec(), vec![])], None)).unwrap();
    let b = loaded.bases[0];
    let (s, _) = a.step(300_000);
    assert_eq!(s, Step::BudgetSpent, "the worker never blocks: the step ends on its budget, not Blocked ({:?})", a.refused);
    assert_eq!(a.sched.tasks.len(), 2, "owner + worker");
    let owner = a.sched.tasks.values().find(|t| t.main).expect("main");
    assert!(owner.waiting.is_some(), "the window owner is parked in its WaitPort");
    let w0 = c(&a, b, 10);
    assert!(w0 > 0, "the worker ran while the owner was parked");
    let win = *a.windows.values().next().unwrap();
    for (x, y, ph) in [(15, 8, PointerPhase::Down), (16, 8, PointerPhase::Up)] {
        assert_eq!(a.deliver(GuestUiEvent::primary(win, x, y, ph)).unwrap(), Delivered::Handled);
        let (s, _) = a.step(300_000);
        assert!(matches!(s, Step::BudgetSpent | Step::Exited), "{s:?} {:?}", a.refused);
        assert!(a.refused.is_empty(), "{:?}", a.refused);
    }
    assert_eq!((c(&a, b, 0), c(&a, b, 1), c(&a, b, 2), c(&a, b, 3)), (1, 1, 1, 1), "hittest, goactive, handleinput(up), goinactive ran on the owner's gadget");
    assert_eq!((c(&a, b, 9), c(&a, b, 5), c(&a, b, 6)), (1, 0x40, 123), "the owner got IDCMP_GADGETUP and finished");
    assert!(c(&a, b, 10) > w0, "the worker kept running throughout");
}

#[test]
fn without_a_click_the_parked_owner_stays_parked() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(TWO.to_vec(), vec![])], None)).unwrap();
    let b = loaded.bases[0];
    a.step(300_000);
    a.step(300_000);
    assert_eq!((c(&a, b, 0), c(&a, b, 9)), (0, 0), "control: no pointer event, no gadget call, no message");
}
