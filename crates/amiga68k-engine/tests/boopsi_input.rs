//! Pointer input into a guest BOOPSI gadget (tests/guest/boopsiinput.s): a native Pointer event is queued by `deliver`,
//! the guest's own WaitPort runs GM_HITTEST -> GM_GOACTIVE -> GM_HANDLEINPUT -> GM_GOINACTIVE on the GUEST dispatcher,
//! and a GMR_VERIFY answer arrives in the program as IDCMP_GADGETUP with the gadget as IAddress and the termination as Code.
//! Controls: a press outside the box and a press the dispatcher's GM_HITTEST rejects start nothing; the message does not
//! appear before the button-up.
use amiga68k_engine::{Amiga, Delivered, Step};
use guestui_abi::{GuestUiEvent, PointerPhase, RecordingGuestUi};
use retro_hunk::writer::{build, Hunk};

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
    let d = a.deliver(GuestUiEvent::Pointer { win, x, y, phase }).unwrap();
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
