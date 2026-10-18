//! A guest BOOPSI gadget in a window (tests/guest/boopsigad.s): AddGList links it into Window.FirstGadget, RefreshGList
//! sends GM_RENDER to the GUEST dispatcher with the window's RastPort, and the dispatcher's RectFill lands in the canvas the
//! UI host receives. Controls: no RefreshGList = no pixels; RemoveGList unlinks; the pixels outside the box stay untouched.
use amiga68k_engine::{Amiga, Step};
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/boopsigad.bin");
const WHITE: u32 = retro_theme::amiga::system_palette()[2]; // pen 2 (shine)
const GREY: u32 = retro_theme::amiga::system_palette()[0]; // pen 0, the window background (Gute Form surface)

fn run(doref: u32) -> (Amiga<RecordingGuestUi>, u32) {
    let mut code = BIN.to_vec();
    code[4..8].copy_from_slice(&doref.to_be_bytes());
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(code, vec![])], None)).unwrap();
    let (s, _) = a.step(2_000_000);
    assert_eq!(s, Step::Blocked, "{:?} {:?}", a.refused, &a.log[a.log.len().saturating_sub(6)..]);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    (a, loaded.bases[0])
}
fn r(a: &Amiga<RecordingGuestUi>, base: u32, i: u32) -> u32 { a.mem.read_u32(base + 8 + 4 * i).unwrap() }
fn canvas(a: &Amiga<RecordingGuestUi>) -> (i32, i32, Vec<u32>) { a.ui.canvases[a.ui.canvases.keys().next().expect("a canvas")].clone() }
fn at(c: &(i32, i32, Vec<u32>), x: i32, y: i32) -> u32 { c.2[(y * c.0 + x) as usize] }

#[test]
fn refresh_glist_renders_a_guest_gadget_into_the_window() {
    let (a, b) = run(1);
    assert_eq!(r(&a, b, 6), 1, "the fixture ran to its end");
    let c = canvas(&a);
    assert_eq!(at(&c, 10, 5), WHITE, "top-left corner of the gadget box");
    assert_eq!(at(&c, 29, 14), WHITE, "bottom-right corner (GA_Left+GA_Width-1, GA_Top+GA_Height-1)");
    assert_eq!(at(&c, 20, 10), WHITE);
    for (x, y) in [(9, 5), (10, 4), (30, 14), (29, 15), (100, 50)] { assert_eq!(at(&c, x, y), GREY, "({x},{y}) is outside the gadget"); }
    let white = c.2.iter().filter(|&&p| p == WHITE).count();
    assert_eq!(white, 20 * 10, "exactly the 20x10 box");
}

#[test]
fn without_a_refresh_nothing_is_drawn() {
    let (a, b) = run(0);
    assert_eq!(r(&a, b, 6), 1);
    assert!(a.ui.canvases.is_empty(), "AddGList alone renders nothing: no content frame reached the host");
}

#[test]
fn the_gadget_list_is_linked_and_unlinked() {
    let (a, b) = run(1);
    let (gad, pos, first, rem, after) = (r(&a, b, 1), r(&a, b, 2), r(&a, b, 3), r(&a, b, 4), r(&a, b, 5));
    assert_ne!(gad, 0);
    assert_eq!(pos, 0, "AddGList at the end of an empty list reports position 0");
    assert_eq!(first, gad, "Window.FirstGadget is the new gadget");
    assert_eq!(rem, 0, "RemoveGList reports the old position");
    assert_eq!(after, 0, "Window.FirstGadget is NULL again");
    assert!(a.classes.class_of(gad).is_some(), "control: the gadget is still a live BOOPSI object (RemoveGList does not dispose)");
}
