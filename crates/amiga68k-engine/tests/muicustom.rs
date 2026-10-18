//! A program's OWN subclass of a MUI class (tests/guest/muicustom.s): MUI_CreateCustomClass over Group.mui, NewObjectA of the subclass
//! (its 68k dispatcher runs OM_NEW, calls DoSuperMethod into the NATIVE Group.mui, and keeps instance data past the MUI half), GetAttr
//! answered by the program for its own attribute and by the native class for Group's, a MUIM_Notify whose destination is the subclass
//! instance (the notification becomes a guest call of the program's dispatcher), and DisposeObject through both halves.
use amiga68k_engine::{Amiga, Step};
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/muicustom.bin");
const RES: u32 = 4;

fn run() -> (Amiga<RecordingGuestUi>, u32) {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(BIN.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(2_000_000);
    assert_eq!(s, Step::Exited, "{:?} {:?}", a.refused, &a.log[a.log.len().saturating_sub(8)..]);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    (a, loaded.bases[0] + 4)
}
fn r(a: &Amiga<RecordingGuestUi>, base: u32, i: u32) -> u32 { a.mem.read_u32(base + RES + 4 * i).unwrap() }

#[test]
fn a_program_subclasses_a_mui_class_and_both_halves_answer() {
    let (a, b) = run();
    assert_eq!(r(&a, b, 14), 1, "the fixture ran to its end marker");
    let (mcc, cl, obj) = (r(&a, b, 0), r(&a, b, 1), r(&a, b, 2));
    assert!(mcc != 0 && cl != 0 && obj != 0, "{mcc:#x} {cl:#x} {obj:#x}");
    assert_eq!(r(&a, b, 3), cl, "o_Class of the instance is the program's class, so a guest DoMethod lands in its dispatcher");
    assert_eq!(r(&a, b, 4), 5, "GetAttr of the program's own attribute: answered by its dispatcher");
    assert_eq!(r(&a, b, 5), 1, "GetAttr(MUIA_Group_Horiz): the tag given at creation was stored by the NATIVE Group.mui half (DoSuperMethod)");
    assert_eq!(r(&a, b, 9), 0x4D594D59, "the program's instance data sits after the MUI half, in rootclass's block");
    assert_eq!((r(&a, b, 7), r(&a, b, 8)), (55, 1), "MUIM_Notify to the instance ran the program's dispatcher once, with the follow-up's argument");
    // Both objects were disposed: the MUI records and the BOOPSI blocks are gone, and so are the live-object counts.
    assert!(a.objects.map.is_empty() && a.classes.objs.is_empty(), "{} MUI records, {} BOOPSI objects left", a.objects.map.len(), a.classes.objs.len());
    assert_eq!(a.classes.recs[&cl].objects, 0);
}

#[test]
fn an_unknown_superclass_is_a_named_refusal_not_a_null_class() {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    // NEGATIVE CONTROL: the same program with a superclass this engine does not have. The fixture's superclass name is patched in place.
    let mut code = BIN.to_vec();
    let at = code.windows(9).position(|w| w == b"Group.mui").unwrap();
    code[at..at + 9].copy_from_slice(b"Clock.mcc");
    let _ = a.start_hunk(&build(&[Hunk::code(code, vec![])], None)).unwrap();
    let _ = a.step(2_000_000);
    assert!(a.refused.iter().any(|x| x.contains("MUI_CreateCustomClass: the superclass \"Clock.mcc\" is not implemented")), "{:?}", a.refused);
}
