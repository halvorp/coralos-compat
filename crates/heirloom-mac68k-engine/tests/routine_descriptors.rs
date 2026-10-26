//! Mixed Mode routine descriptors (`mixed_mode`): the constructors write a REAL descriptor (never the procedure's own pointer),
//! the one parser refuses every malformed or unrunnable form by name before anything could be called, a descriptor the program
//! built itself resolves as well as an allocated one, and only the constructors' blocks may be disposed, once.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::mixed_mode::*;
use heirloom_mac68k_engine::Mac;

type M = Mac<RecordingGuestUi>;

fn mac() -> M { Mac::new(RecordingGuestUi::new()).unwrap() }

const TVECTOR: u32 = 0x0012_3450; // a PowerPC procedure's TVector (any guest address: nothing is called here)
const M68K_CODE: u32 = 0x0001_2000;
const PROC_INFO: u32 = 0x0000_03C0; // kPascalStackBased, two 2-byte arguments (a user item's shape); opaque to the parser

#[test]
fn new_routine_descriptor_writes_the_documented_bytes_not_the_procedure_pointer() {
    let mut m = mac();
    let upp = new_routine_descriptor(&mut m, TVECTOR, PROC_INFO, ISA_PPC).unwrap();
    assert_ne!(upp, TVECTOR, "a UPP is a descriptor, not the TVector (the identity shortcut is the defect this rules out)");
    let w16 = |o: u32| m.mem.read_u16(upp + o).unwrap();
    let w32 = |o: u32| m.mem.read_u32(upp + o).unwrap();
    let b = |o: u32| m.mem.read_u8(upp + o).unwrap();
    assert_eq!((w16(0), b(2), b(3), w16(10)), (0xAAFE, 7, 0, 0), "magic, version 7, no indexable selectors, ONE record (count is the last index)");
    let r = HEADER_LEN;
    assert_eq!((w32(r), b(r + 5), w16(r + 6), w32(r + 8), w32(r + 16)), (PROC_INFO, ISA_PPC, USE_NATIVE_ISA, TVECTOR, 0));
    let d = parse(&m.mem, upp).unwrap();
    assert_eq!(ppc_record(&d).unwrap().proc_descriptor, TVECTOR);
}

#[test]
fn a_fat_descriptor_carries_both_records_and_a_powerpc_caller_runs_the_powerpc_one() {
    let mut m = mac();
    let upp = new_fat_routine_descriptor(&mut m, M68K_CODE, TVECTOR, PROC_INFO).unwrap();
    let d = parse(&m.mem, upp).unwrap();
    assert_eq!(m.mem.read_u16(upp + 10).unwrap(), 1, "routineCount is the last index: two records");
    assert_eq!(d.records.iter().map(|r| (r.isa, r.proc_descriptor)).collect::<Vec<_>>(), [(ISA_M68K, M68K_CODE), (ISA_PPC, TVECTOR)]);
    assert_eq!(ppc_record(&d).unwrap().proc_descriptor, TVECTOR);
    // NEGATIVE CONTROL: a 68k-only descriptor is a named refusal, never its 68k code treated as a TVector
    let only68k = new_routine_descriptor(&mut m, M68K_CODE, PROC_INFO, ISA_M68K).unwrap();
    let e = ppc_record(&parse(&m.mem, only68k).unwrap()).unwrap_err();
    assert!(matches!(&e, MixedModeError::NoRunnableRecord { why, .. } if why.contains("68k code only")), "{e}");
}

#[test]
fn a_descriptor_the_program_built_itself_resolves_but_is_not_the_constructors_to_free() {
    let mut m = mac();
    let at = 0x0002_0000; // a static descriptor in the program's own data
    for (o, v) in [(0u32, 0xAAFEu32), (2, 0x0700), (10, 0)] { m.mem.write_u16(at + o, v as u16).unwrap(); }
    m.mem.write_u32(at + HEADER_LEN, PROC_INFO).unwrap();
    m.mem.write_u8(at + HEADER_LEN + 5, ISA_PPC).unwrap();
    m.mem.write_u32(at + HEADER_LEN + 8, TVECTOR).unwrap();
    assert_eq!(ppc_record(&parse(&m.mem, at).unwrap()).unwrap().proc_descriptor, TVECTOR, "the ledger is ownership, not a whitelist");
    assert_eq!(dispose_routine_descriptor(&mut m, at), Err(MixedModeError::NotOwned(at)));
}

#[test]
fn every_malformed_or_unrunnable_descriptor_is_refused_by_name() {
    let mut m = mac();
    let upp = new_routine_descriptor(&mut m, TVECTOR, PROC_INFO, ISA_PPC).unwrap();
    // not a descriptor at all: a raw 68k procedure pointer
    assert_eq!(parse(&m.mem, M68K_CODE), Err(MixedModeError::NotADescriptor(M68K_CODE)));
    // each mutation of the real block is refused; the block is restored after each
    let mutate = |m: &mut M, off: u32, v: u16| { let old = m.mem.read_u16(upp + off).unwrap(); m.mem.write_u16(upp + off, v).unwrap(); old };
    let old = mutate(&mut m, 2, 0x0600);
    assert_eq!(parse(&m.mem, upp), Err(MixedModeError::Version { at: upp, version: 6 }));
    mutate(&mut m, 2, old);
    let old = mutate(&mut m, 10, 0xFFFF);
    assert!(matches!(parse(&m.mem, upp), Err(MixedModeError::TooManyRecords { count: 0x1_0000, .. })));
    mutate(&mut m, 10, old);
    let old = mutate(&mut m, HEADER_LEN + 6, PROC_DESCRIPTOR_IS_RELATIVE);
    assert!(matches!(ppc_record(&parse(&m.mem, upp).unwrap()), Err(MixedModeError::NoRunnableRecord { .. })), "a relative record");
    mutate(&mut m, HEADER_LEN + 6, old);
    let nil = new_routine_descriptor(&mut m, 0, PROC_INFO, ISA_PPC).unwrap();
    assert!(matches!(ppc_record(&parse(&m.mem, nil).unwrap()), Err(MixedModeError::NoRunnableRecord { why, .. }) if why.contains("NIL")));
    // CONTROL: the restored block resolves again
    assert_eq!(ppc_record(&parse(&m.mem, upp).unwrap()).unwrap().proc_descriptor, TVECTOR);
}

#[test]
fn only_the_constructors_blocks_are_disposed_and_only_once() {
    let mut m = mac();
    let upp = new_routine_descriptor(&mut m, TVECTOR, PROC_INFO, ISA_PPC).unwrap();
    assert_eq!(dispose_routine_descriptor(&mut m, 0), Ok(()), "NIL is a no-op");
    assert_eq!(dispose_routine_descriptor(&mut m, upp), Ok(()));
    assert_eq!(dispose_routine_descriptor(&mut m, upp), Err(MixedModeError::NotOwned(upp)), "a second dispose is refused, the heap untouched");
    // the freed block can be reused by the next descriptor
    let again = new_routine_descriptor(&mut m, TVECTOR, PROC_INFO, ISA_PPC).unwrap();
    assert!(m.routine_descriptors.contains(&again));
}
