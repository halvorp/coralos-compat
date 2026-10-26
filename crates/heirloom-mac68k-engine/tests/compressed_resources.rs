//! Compressed resources (resource attribute bit 0, header $A89F6572): the Resource Manager does NOT expand them itself, it
//! runs the 'dcmp' resource's own 68k code in the guest. The test 'dcmp' here is hand-assembled; every behaviour has a control.
mod common;
use common::Prog;
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_mac68k_engine::resmgr::APP_REF_NUM;
use heirloom_mac68k_engine::Mac;
use heirloom_fmt_resfork::writer::ForkBuilder;
use heirloom_fmt_resfork::ResFork;

const TEXT: u32 = u32::from_be_bytes(*b"TEXT");
const OUT: u32 = 0x3000;

/// A version 8 'dcmp' (classic interface): `dcmp(src, dst, working: Ptr; len: LongInt)`, Pascal, pops 16. It adds `inc` to every
/// byte, and CLOBBERS D3, A2 and the CCR without saving them (the Resource Manager must hand the trap's registers back).
fn dcmp_classic(inc: u8) -> Vec<u8> {
    let w: &[u16] = &[
        0x6000, 0x000A, 0x6463, 0x6D70, 0x0000, 0x0000,          // bra.w +10 ; 'dcmp' ; 0
        0x4E56, 0x0000,                                            // link a6,#0
        0x48E7, 0xC0C0,                                            // movem.l d0-d1/a0-a1,-(sp)
        0x76FF, 0x347C, 0x1234,                                    // moveq #-1,d3 ; movea.w #$1234,a2   (clobbers)
        0x206E, 0x0014, 0x226E, 0x0010, 0x202E, 0x0008,            // a0=src a1=dst d0=len (fp+20, +16, +8)
        0x5380,                                                    // subq.l #1,d0
        0x1218, 0x0601, inc as u16, 0x12C1, 0x51C8, 0xFFF6,        // loop: move.b (a0)+,d1 ; addi.b #inc,d1 ; move.b d1,(a1)+ ; dbra d0,loop
        0x4CDF, 0x0303,                                            // movem.l (sp)+,d0-d1/a0-a1
        0x4E5E, 0x205F, 0xDEFC, 0x0010, 0x4ED0,                    // unlk a6 ; movea.l (sp)+,a0 ; adda.w #16,sp ; jmp (a0)
    ];
    w.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// A version 9 'dcmp' (table interface): [init, decompress, exit] offsets, then `FUNCTION dcmp(src, dst, header): OSErr`. It
/// copies `size` bytes (the header's longword at +8) adding `inc`, and returns `result` as its OSErr.
fn dcmp_table(inc: u8, result: u16) -> Vec<u8> {
    let w: &[u16] = &[
        0x0006, 0x0012, 0x000C,                                    // offsets
        0x205F, 0x588F, 0x4ED0,                                    // @6  init(x): movea.l (sp)+,a0 ; addq.l #4,sp ; jmp (a0)   (NOT called)
        0x205F, 0x588F, 0x4ED0,                                    // @C  exit(x)
        0x4E56, 0x0000, 0x48E7, 0xC0E0,                            // @12 link a6,#0 ; movem.l d0-d1/a0-a2,-(sp)
        0x206E, 0x0010, 0x226E, 0x000C, 0x246E, 0x0008,            // a0=src (fp+16) a1=dst (fp+12) a2=header (fp+8)
        0x202A, 0x0008, 0x5380,                                    // move.l 8(a2),d0 ; subq.l #1,d0
        0x1218, 0x0601, inc as u16, 0x12C1, 0x51C8, 0xFFF6,        // loop
        0x3D7C, result, 0x0014,                                    // move.w #result,20(a6)
        0x4CDF, 0x0703,                                            // movem.l (sp)+,d0-d1/a0-a2
        0x4E5E, 0x205F, 0xDEFC, 0x000C, 0x4ED0,                    // unlk a6 ; movea.l (sp)+,a0 ; adda.w #12,sp ; jmp (a0)
    ];
    w.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// A compressed-resource header (18 bytes) + body. Version 8: id at 14; version 9: id at 12.
fn compressed(version: u8, dcmp_id: i16, size: u32, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0xA8, 0x9F, 0x65, 0x72, 0x00, 0x12, version, 0x01];
    v.extend_from_slice(&size.to_be_bytes());
    if version == 8 { v.extend_from_slice(&[0, 0]); v.extend_from_slice(&dcmp_id.to_be_bytes()); v.extend_from_slice(&[0, 0]); }
    else { v.extend_from_slice(&dcmp_id.to_be_bytes()); v.extend_from_slice(&[0, 0, 0, 0]); }
    v.extend_from_slice(body);
    v
}

/// The bytes before `inc` is added: each one less than "hello".
const BODY: &[u8] = b"gdkkn";

fn fork_with(attrs: u8, header: Vec<u8>, dcmp: Option<(i16, Vec<u8>)>) -> ResFork {
    let mut b = ForkBuilder::new().add(b"TEXT", 128, None, &header).attrs(attrs);
    if let Some((id, code)) = dcmp { b = b.add(b"dcmp", id, None, &code); }
    ResFork::parse(b.build()).unwrap()
}

/// moveq #7,d3 ; movea.l #$5555,a2 ; GetResource('TEXT',128) -> OUT ; the same again -> OUT+4 ; d3 -> OUT+8 ; a2 -> OUT+12 ; ResError -> OUT+16.
fn program() -> Prog {
    let mut p = Prog::new();
    p.0.extend_from_slice(&[0x76, 0x07, 0x24, 0x7C, 0x00, 0x00, 0x55, 0x55]);
    p.clr_l().l(TEXT).w(128).trap(0xA9A0).pop_l(OUT);
    p.clr_l().l(TEXT).w(128).trap(0xA9A0).pop_l(OUT + 4);
    p.0.extend_from_slice(&[0x23, 0xC3, 0x00, 0x00, 0x30, 0x08, 0x23, 0xCA, 0x00, 0x00, 0x30, 0x0C]);
    p.clr_w().trap(0xA9AF).pop_w(OUT + 16);
    p.exit();
    p
}

fn run(fork: ResFork, system: Option<ResFork>) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    if let Some(s) = system { m.rm.add_system(s); }
    m.rm.open_fork(APP_REF_NUM, fork);
    m.start(&program().0, 0).unwrap();
    let r = m.run_loop(1_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?}\n{:#?}", r.stats, m.log);
    m
}
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }

#[test]
fn a_classic_dcmp_runs_in_the_guest_and_getresource_returns_the_expanded_bytes() {
    let m = run(fork_with(1, compressed(8, 0, 5, BODY), Some((0, dcmp_classic(1)))), None);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    let h = l(&m, OUT);
    assert_ne!(h, 0);
    assert_eq!(m.handle_bytes(h).unwrap(), b"hello", "the handle holds the DEcompressed bytes (the guest 'dcmp' added 1 to each)");
    assert_eq!(l(&m, OUT + 4), h, "the second GetResource finds the expanded handle in the cache: no second decompression");
    assert_eq!(m.log.iter().filter(|x| x.contains("decompress")).count(), 1, "{:#?}", m.log);
    assert_eq!(m.log.iter().filter(|x| x.contains("GetResource 'TEXT' 128")).count(), 2, "the trap is logged once per call: the aborted run leaves no trace");
    assert_eq!(l(&m, OUT + 8), 7, "D3 survives although the 'dcmp' clobbered it");
    assert_eq!(l(&m, OUT + 12), 0x5555, "A2 survives");
    assert_eq!(m.mem.read_u16(OUT + 16).unwrap(), 0, "ResError = noErr");
    assert_eq!(m.mm.live_ptrs(), 0, "the compressed copy and the working buffer are freed");
}

#[test]
fn control_without_the_compressed_bit_the_resource_is_handed_over_as_stored() {
    // The same bytes, bit 0 clear: nothing is decompressed (so it is the 'dcmp' that produced "hello", not the loader).
    let m = run(fork_with(0, compressed(8, 0, 5, BODY), Some((0, dcmp_classic(1)))), None);
    let h = l(&m, OUT);
    assert_eq!(m.handle_bytes(h).unwrap().len(), 18 + 5);
    assert!(!m.log.iter().any(|x| x.contains("decompress")));
    // ... and bit 0 set on data WITHOUT the signature is an ordinary resource too.
    let m = run(fork_with(1, b"plain".to_vec(), Some((0, dcmp_classic(1)))), None);
    assert_eq!(m.handle_bytes(l(&m, OUT)).unwrap(), b"plain");
}

#[test]
fn the_dcmp_resource_chosen_is_the_one_the_header_names_and_the_applications_own_wins_over_the_system_set() {
    // Header names id 5: the app has 5 (+2) and the system set has 5 (+1) and 6: the application's is the one that runs.
    let sys = ResFork::parse(ForkBuilder::new().add(b"dcmp", 5, None, &dcmp_classic(1)).add(b"dcmp", 6, None, &dcmp_classic(9)).build()).unwrap();
    let m = run(fork_with(1, compressed(8, 5, 5, BODY), Some((5, dcmp_classic(2)))), Some(sys));
    assert_eq!(m.handle_bytes(l(&m, OUT)).unwrap(), b"ifmmp", "+2 (the app's own dcmp 5)");
    // NEGATIVE CONTROL: no dcmp in the app: the system set supplies it (+1).
    let sys = ResFork::parse(ForkBuilder::new().add(b"dcmp", 5, None, &dcmp_classic(1)).build()).unwrap();
    let m = run(fork_with(1, compressed(8, 5, 5, BODY), None), Some(sys));
    assert_eq!(m.handle_bytes(l(&m, OUT)).unwrap(), b"hello");
}

#[test]
fn a_missing_dcmp_is_a_named_refusal_with_cant_decompress_and_no_host_side_fallback() {
    let m = run(fork_with(1, compressed(8, 0, 5, BODY), None), None);
    assert_eq!(l(&m, OUT), 0, "GetResource answers NIL");
    assert_eq!(m.mem.read_u16(OUT + 16).unwrap() as i16, -186, "ResError = CantDecompress");
    assert!(m.refused.iter().any(|r| r.contains("cannot decompress") && r.contains("'dcmp' 0")), "{:?}", m.refused);
    // An unknown header version, and a truncated header, are refused by name too.
    for (bad, why) in [(compressed(7, 0, 5, BODY), "header version 7"), (compressed(8, 0, 5, BODY)[..10].to_vec(), "shorter than its 18-byte header")] {
        let m = run(fork_with(1, bad, Some((0, dcmp_classic(1)))), None);
        assert_eq!(l(&m, OUT), 0);
        assert!(m.refused.iter().any(|r| r.contains(why)), "{why}: {:?}", m.refused);
    }
}

#[test]
fn a_table_interface_dcmp_gets_the_header_and_its_oserr_result_is_checked() {
    // Version 9: the decompress routine is the second offset of the table; FUNCTION dcmp(src, dst, header): OSErr.
    let m = run(fork_with(1, compressed(9, 3, 5, BODY), Some((3, dcmp_table(1, 0)))), None);
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(m.handle_bytes(l(&m, OUT)).unwrap(), b"hello");
    assert!(m.log.iter().any(|x| x.contains("table interface")));
    assert_eq!(l(&m, OUT + 8), 7, "registers back");
    // NEGATIVE CONTROL: the dcmp reports an error (-1): the engine names it, it does not pretend.
    let m = run(fork_with(1, compressed(9, 3, 5, BODY), Some((3, dcmp_table(1, 0xFFFF)))), None);
    assert!(m.refused.iter().any(|r| r.contains("returned error -1")), "{:?}", m.refused);
    // A dispatch table that points outside the resource is refused before anything is entered.
    let mut bad = dcmp_table(1, 0);
    bad[2] = 0x7F;
    let m = run(fork_with(1, compressed(9, 3, 5, BODY), Some((3, bad))), None);
    assert!(m.refused.iter().any(|r| r.contains("no usable dispatch table")), "{:?}", m.refused);
}

#[test]
fn a_dcmp_resource_cannot_itself_be_compressed() {
    let mut b = ForkBuilder::new().add(b"TEXT", 128, None, &compressed(8, 0, 5, BODY)).attrs(1);
    b = b.add(b"dcmp", 0, None, &compressed(8, 0, 5, &dcmp_classic(1))).attrs(1);
    let m = run(ResFork::parse(b.build()).unwrap(), None);
    assert_eq!(l(&m, OUT), 0);
    assert!(m.refused.iter().any(|r| r.contains("'dcmp' resource cannot itself be compressed")), "{:?}", m.refused);
}
