//! A GUEST-defined BOOPSI class (tests/guest/boopsi.s) end to end: MakeClass/AddClass/FindClass, NewObjectA of the guest
//! class (its dispatcher is 68k code run through guest_call; it does DoSuperMethod into the NATIVE gadgetclass through the
//! trap slot), SetAttrsA/GetAttr, the instance-data layout the dispatcher computes from cl_InstOffset, FreeClass refusing
//! while an object lives, and DisposeObject through rootclass. Controls: the negative results are in the same run.
use amiga68k_engine::{Amiga, Step};
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const BIN: &[u8] = include_bytes!("guest/boopsi.bin");
const RES: u32 = 4;

fn run(code: &[u8]) -> (Amiga<RecordingGuestUi>, u32) {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(code.to_vec(), vec![])], None)).unwrap();
    let (s, _) = a.step(2_000_000);
    assert_eq!(s, Step::Exited, "{:?} {:?}", a.refused, &a.log[a.log.len().saturating_sub(6)..]);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let base = loaded.bases[0] + 4;
    (a, base)
}
fn r(a: &Amiga<RecordingGuestUi>, base: u32, i: u32) -> u32 { a.mem.read_u32(base + RES + 4 * i).unwrap() }

#[test]
fn a_guest_class_is_made_listed_instantiated_and_freed() {
    let (a, b) = run(BIN);
    assert_eq!(r(&a, b, 25), 1, "the fixture ran to its end marker");
    let cl = r(&a, b, 0);
    assert_ne!(cl, 0, "MakeClass returned a class");
    assert_eq!(r(&a, b, 1), 0, "control: a private class is not found by name before AddClass");
    assert_eq!(r(&a, b, 2), cl, "FindClass finds it after AddClass");
    let obj = r(&a, b, 3);
    assert_ne!(obj, 0, "NewObjectA by public name ran the GUEST dispatcher's OM_NEW");
    assert_eq!(r(&a, b, 11), cl, "o_Class (at object - 4) is the guest class");
    assert_eq!(r(&a, b, 12), 1, "cl_ObjectCount");
    assert_eq!(r(&a, b, 13), 72, "cl_InstOffset = the gadgetclass instance (ExtGadget 56 + ICData 16)");
    assert_eq!(r(&a, b, 14), 8, "cl_InstSize");
    // gadgetclass's own OM_NEW ran through the trap slot (DoSuperMethod from guest code)
    assert_eq!(r(&a, b, 15), 0x8000, "Gadget.Flags = GFLG_EXTENDED");
    assert_eq!(r(&a, b, 16), 5, "Gadget.GadgetType = GTYP_CUSTOMGADGET");
    assert_eq!(r(&a, b, 17), cl, "Gadget.MutualExclude = &class->cl_Dispatcher");
    assert_eq!(r(&a, b, 18), 0x4D594D59, "the dispatcher wrote its instance data at obj + cl_InstOffset");
    assert_eq!((r(&a, b, 4), r(&a, b, 5)), (5, 1), "GetAttr of the guest attribute (OM_GET handled by the guest)");
    assert_eq!((r(&a, b, 6), r(&a, b, 7)), (10, 1), "GetAttr(GA_Left): the guest passes it to the native gadgetclass");
    assert_eq!((r(&a, b, 9), r(&a, b, 10)), (77, 9), "SetAttrsA reached the guest (MYATTR) and, via DoSuperMethod, gadgetclass (GA_Top)");
    assert_eq!(r(&a, b, 8), 1, "OM_SET: the native gadgetclass reports a visual change");
}

#[test]
fn free_class_refuses_while_objects_live_and_unknown_classes_make_null() {
    let (a, b) = run(BIN);
    assert_eq!(r(&a, b, 19), 0, "FreeClass with a live object is FALSE");
    assert_eq!(r(&a, b, 20), 0, "FreeClass still removed the class from the public list");
    assert_eq!(r(&a, b, 21), 0, "NewObjectA by the unlisted name is NULL");
    assert_eq!(r(&a, b, 22), 0, "DisposeObject (guest class -> gadgetclass -> rootclass) drops cl_ObjectCount");
    assert_eq!(r(&a, b, 23), 1, "FreeClass with no objects is TRUE");
    assert_eq!(r(&a, b, 24), 0, "NewObjectA of a class that does not exist is NULL");
    assert!(a.classes.objs.is_empty(), "host record: no live objects");
}

#[test]
fn the_check_can_fail() {
    // negative control: a fixture whose dispatcher never writes the instance data (patch the 'MYMY' immediate's store)
    let mut code = BIN.to_vec();
    let at = code.windows(4).position(|w| w == [0x4D, 0x59, 0x4D, 0x59]).expect("the marker immediate");
    code[at..at + 4].copy_from_slice(&[0, 0, 0, 0]);
    let (a, b) = run(&code);
    assert_ne!(r(&a, b, 18), 0x4D594D59, "with the marker changed the instance word differs");
}

