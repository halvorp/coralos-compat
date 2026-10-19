//! muimaster.library/MUI_RequestA on the requester seam: Zune's gadget-string and ReturnID arithmetic, the text engine's codes stripped,
//! the Application's title for a NULL title, the reference window passed through. Driven by tests/guest/c/mui_request.c.
use amiga68k_engine::{requester, Amiga, Step};
use guestui_abi::{RecordingGuestUi, RequestModel, UiCall};
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/mui_request.bin");
const RES: u32 = 0x1F000;
type M = Amiga<RecordingGuestUi>;

fn res(a: &M, i: u32) -> u32 { a.mem.read_u32(RES + 4 * i).unwrap() }
fn run(answers: &[u32]) -> M {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    a.ui.request_answers.extend(answers.iter().copied());
    a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    for _ in 0..200 {
        match a.step(2_000_000).0 {
            Step::Exited => break,
            Step::Blocked => { if !requester::pump(&mut a).unwrap() { break; } }
            Step::BudgetSpent => {}
            Step::Fault(e) => panic!("{e}\n{:#?}", a.log),
        }
    }
    a
}
fn requests(a: &M) -> Vec<(Option<guestui_abi::UiObjectRef>, RequestModel)> {
    a.ui.calls.iter().filter_map(|c| if let UiCall::Request(w, m) = c { Some((*w, m.clone())) } else { None }).collect()
}

#[test]
fn zunes_return_ids_leftmost_middle_rightmost_and_a_lone_button() {
    // buttons 0,1,2 of three; the lone button; NULL-title OK (button 0 of 2); with a window Cancel (button 1 of 2).
    let a = run(&[0, 1, 2, 0, 0, 1, 0, 1, 0]);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!((res(&a, 0), res(&a, 1), res(&a, 2)), (1, 2, 0), "ReturnID j+2, the last 1, result = ID - 1: 1, 2 and 0");
    assert_eq!(res(&a, 3), 1, "a LONE button returns 1 (ReturnID 2), unlike EasyRequest's 0");
    assert_eq!((res(&a, 4), res(&a, 5)), (1, 0), "two buttons: the first 1, the last 0");
    // NEGATIVE CONTROL: without an Application the answer is Intuition's numbering, and the lone button is 0.
    assert_eq!((res(&a, 6), res(&a, 7), res(&a, 8)), (1, 2, 0), "app NULL: EasyRequestArgs numbering");
    assert_eq!(res(&a, 10), 1, "the program ran to its end");
}

#[test]
fn the_alert_shows_plain_text_with_the_default_button_and_the_applications_title() {
    let a = run(&[0; 9]);
    let q = requests(&a);
    assert_eq!(q.len(), 9);
    let m = &q[0].1;
    assert_eq!(m.title, "Quit");
    assert_eq!(m.text, "Save notes (1234 bytes)?", "RawDoFmt filled the format, then the ESC c / ESC b / ESC n sequences were stripped");
    assert_eq!(m.buttons, ["Save", "Discard", "Cancel"], "'*' and the '_' hotkey marks removed, split at '|'");
    assert_eq!((m.default, m.cancel), (Some(0), Some(2)), "'*' marks the default button; the last is the cancel");
    assert_eq!(q[3].1.default, None, "NEGATIVE CONTROL: no '*', no default");
    assert_eq!(q[4].1.title, "Notes", "a NULL title is MUIA_Application_Title");
    assert_eq!(q[6].1.title, "Easy");
    assert!(q[0].0.is_none() && q[4].0.is_none(), "no reference window");
    let win = q[5].0.expect("the reference window passes through as the alert's parent");
    let main = a.objects.map.values().find_map(|o| match &o.ext { amiga68k_engine::boopsi::Ext::Win { native: Some(n), .. } => Some(*n), _ => None }).expect("the Main window is open");
    assert_eq!(win, main, "it is the native window of the MUI window the program passed");
}
