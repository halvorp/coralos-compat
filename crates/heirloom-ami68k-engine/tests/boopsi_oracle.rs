//! The SAME m68k binary (tests/oracle/boopsiprobe.s) was run on a real AROS m68k (Copperline, 2026-10-18) and its dump
//! committed (tests/oracle/boopsiprobe.aros: a guest-made subclass of gadgetclass, one object of it, and the built-in
//! gadgetclass / rootclass / imageclass IClass structs). This engine runs the binary and must agree on every field a
//! guest class reads: instance offsets and sizes, object size, flags, counts, the object's _Object header and the
//! ExtGadget fields gadgetclass's OM_NEW sets. Pointers are the machine's own: compared by relation, not value.
use heirloom_ami68k_engine::Amiga;
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_fmt_hunk::writer::{build, Hunk};

const PROBE: &[u8] = include_bytes!("oracle/boopsiprobe.bin");
const AROS: &[u8] = include_bytes!("oracle/boopsiprobe.aros");

fn ours(code: &[u8]) -> Vec<u8> {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(code.to_vec(), vec![])], None)).unwrap();
    let (_s, st) = a.step(500_000);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(st.insns > 100);
    (0..AROS.len() as u32).map(|i| a.mem.read_u8(loaded.bases[0] + 0xE8 + i).unwrap()).collect()
}
fn u32_at(b: &[u8], o: usize) -> u32 { u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) }
fn u16_at(b: &[u8], o: usize) -> u16 { u16::from_be_bytes(b[o..o + 2].try_into().unwrap()) }

/// (cl_InstOffset, cl_InstSize, cl_SubclassCount-is-nonzero, cl_ObjectCount, cl_Flags, cl_ObjectSize) of the class at `o`.
fn shape(b: &[u8], o: usize) -> (u16, u16, u32, u32, u32) { (u16_at(b, o + 32), u16_at(b, o + 34), u32_at(b, o + 44), u32_at(b, o + 48), u32_at(b, o + 52)) }

#[test]
fn class_and_object_layout_match_real_aros() {
    let o = ours(PROBE);
    assert_eq!(o.len(), AROS.len());
    for (name, at) in [("guest subclass", 0), ("gadgetclass", 156), ("rootclass", 216), ("imageclass", 276)] {
        assert_eq!(shape(&o, at), shape(AROS, at), "{name}: InstOffset/InstSize/ObjectCount/Flags/ObjectSize");
    }
    assert_eq!(shape(&o, 0), (72, 8, 1, 1, 92), "control: the AROS values themselves are the ones I expect");
    // the object: _Object header (12 bytes, o_Class last) then the 72-byte gadgetclass instance then the subclass's 8
    assert_eq!(&o[64..72], &AROS[64..72], "o_Node is zero");
    let (cl, ocl) = (0, u32_at(&o, 64 + 8));
    assert_ne!(ocl, 0);
    assert_eq!(u32_at(AROS, 64 + 8) != 0, true);
    let (acl_mutual, aocl) = (u32_at(AROS, 76 + 30), u32_at(AROS, 64 + 8));
    assert_eq!(acl_mutual, aocl, "AROS: MutualExclude == the class pointer == o_Class");
    assert_eq!(u32_at(&o, 76 + 30), ocl, "ours: the same relation");
    let _ = cl;
    for (name, off, len) in [("Flags", 12, 2), ("Activation", 14, 2), ("GadgetType", 16, 2), ("MoreFlags", 44, 4), ("NextGadget", 0, 4), ("Width/Height", 8, 4)] {
        assert_eq!(o[76 + off..76 + off + len], AROS[76 + off..76 + off + len], "ExtGadget.{name}");
    }
    assert_eq!(&o[76 + 72..76 + 80], &AROS[76 + 72..76 + 80], "the subclass's own 8 bytes start zeroed");
}

#[test]
fn the_comparison_detects_a_wrong_instance_size() {
    // negative control: MakeClass with instanceSize 12 (moveq #8,d0 -> moveq #12,d0) must change cl_InstSize and cl_ObjectSize
    let mut code = PROBE.to_vec();
    let at = code.windows(4).position(|w| w == [0x70, 0x08, 0x72, 0x00]).expect("moveq #8,d0; moveq #0,d1");
    code[at + 1] = 12;
    let o = ours(&code);
    assert_ne!(shape(&o, 0), shape(AROS, 0));
    assert_eq!(shape(&o, 0).1, 12);
}
