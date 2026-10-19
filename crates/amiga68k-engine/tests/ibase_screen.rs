//! IntuitionBase.ActiveScreen / FirstScreen point at the one screen: a program that seeds its preferences from `IntuitionBase->ActiveScreen`
//! between LockIBase and UnlockIBase (Jano's Editor takes its main screen from it, and without a screen ended "Main interface failed to open!").
use amiga68k_engine::{screen::SCREEN, Amiga};
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

#[test]
fn the_intuition_base_names_the_screen() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.start_hunk(&build(&[Hunk::code(vec![0x4E, 0x75], vec![])], None)).unwrap(); // rts
    assert_eq!(a.mem.read_u32(0x2000 + 56).unwrap(), SCREEN, "ActiveScreen");
    assert_eq!(a.mem.read_u32(0x2000 + 60).unwrap(), SCREEN, "FirstScreen");
    // NEGATIVE CONTROL: there is no active WINDOW before the program opens one (ActiveWindow is the field before).
    assert_eq!(a.mem.read_u32(0x2000 + 52).unwrap(), 0);
}
