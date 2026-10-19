//! `_UnloadSeg` (Inside Macintosh: Processes, "Segment Loader"): the jump-table entries of the unloaded segment go back to
//! their `_LoadSeg` stubs, the segment's handle is unlocked and purgeable, and a later call reloads it. Every behaviour has a
//! control that does NOT unload.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::Mac;
use retro_resfork::writer::ForkBuilder;

const ENTRY1: u32 = 32 + 8; // A5-relative address of jump-table entry 1 (segment 2's only entry)

fn words(w: &[u16]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }
fn seg(first: u16, body: &[u8]) -> Vec<u8> { let mut c = words(&[first, 1]); c.extend_from_slice(body); c }

/// CODE 1 = `main` (calls segment 2's routine, then `tail`, then ExitToShell); CODE 2's routine bumps the word at $3000.
fn app(tail: &[u16]) -> Vec<u8> {
    let mut code0 = Vec::new();
    for v in [48u32, 512, 16, 32] { code0.extend_from_slice(&v.to_be_bytes()); }
    for s in [1u16, 2] { code0.extend_from_slice(&words(&[0, 0x3F3C, s, 0xA9F0])); }
    let mut main = words(&[0x4EAD, (ENTRY1 + 2) as u16]); // jsr 42(a5)
    main.extend(words(tail));
    main.extend(words(&[0xA9F4]));
    let routine = words(&[0x5278, 0x3000, 0x4E75]); // addq.w #1,$3000.w ; rts
    ForkBuilder::new().add(b"CODE", 0, None, &code0).add(b"CODE", 1, None, &seg(0, &main)).add(b"CODE", 2, None, &seg(8, &routine)).build()
}
const UNLOAD: [u16; 3] = [0x486D, (ENTRY1 + 2) as u16, 0xA9F1]; // pea 42(a5) ; _UnloadSeg
const CALL_AGAIN: [u16; 2] = [0x4EAD, (ENTRY1 + 2) as u16];

fn run(file: &[u8]) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.launch(file).unwrap();
    let r = m.run_loop(100_000).unwrap();
    assert!(r.stats.halted, "{:#?} {:?}", m.log, m.refused);
    m
}
fn entry(m: &Mac<RecordingGuestUi>) -> [u16; 4] {
    let e = m.seg.a5 + ENTRY1;
    [0, 2, 4, 6].map(|k| m.mem.read_u16(e + k).unwrap())
}
fn handle_of_code2(m: &mut Mac<RecordingGuestUi>) -> u32 { m.get_resource(*b"CODE", 2, true).unwrap() }

#[test]
fn unload_seg_reverts_the_entries_to_load_seg_stubs_and_makes_the_handle_purgeable() {
    let mut m = run(&app(&UNLOAD));
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(entry(&m), [0, 0x3F3C, 2, 0xA9F0], "offset, move.w #2,-(sp), _LoadSeg");
    assert!(!m.seg.loaded.contains(&2) && m.seg.loaded.contains(&1));
    let h = handle_of_code2(&mut m);
    let b = m.mm.block(h).unwrap();
    assert!(!b.locked && b.purgeable, "{b:?}");
    assert!(m.log.iter().any(|l| l.contains("UnloadSeg 2: 1 entries back to their LoadSeg stubs")), "{:#?}", m.log);
    // NEGATIVE CONTROL: a program that never unloads keeps the patched jmp and a locked, non-purgeable segment.
    let mut c = run(&app(&[]));
    let e = c.seg.a5 + ENTRY1;
    assert_eq!((c.mem.read_u16(e).unwrap(), c.mem.read_u16(e + 2).unwrap()), (2, 0x4EF9));
    let h = handle_of_code2(&mut c);
    assert!(c.mm.block(h).unwrap().locked && !c.mm.block(h).unwrap().purgeable);
}

#[test]
fn a_segment_called_after_unload_seg_is_loaded_again_and_runs() {
    let mut tail = UNLOAD.to_vec();
    tail.extend(CALL_AGAIN);
    let m = run(&app(&tail));
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 2, "the routine ran before and after the unload");
    assert_eq!(m.log.iter().filter(|l| l.contains("LoadSeg 2:")).count(), 2, "{:#?}", m.log);
    assert_eq!(entry(&m)[1], 0x4EF9, "reloaded");
    // NEGATIVE CONTROL: without the unload the second call goes through the same patched entry, no second LoadSeg.
    let mut tail = CALL_AGAIN.to_vec();
    tail.push(0x4E71);
    let m = run(&app(&tail));
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 2);
    assert_eq!(m.log.iter().filter(|l| l.contains("LoadSeg 2:")).count(), 1);
}

#[test]
fn a_released_unloaded_segment_is_reloaded_from_the_resource_not_from_freed_memory() {
    // The program unloads, ReleaseResource's the CODE 2 handle it got from GetResource, then calls the routine again.
    let mut prog = UNLOAD.to_vec();
    // subq #4 result; GetResource('CODE',2); ReleaseResource(handle): the Pascal result slot is popped by the second call
    prog.extend([0x42A7, 0x2F3C, 0x434F, 0x4445, 0x3F3C, 2, 0xA9A0, 0xA9A3]); // clr.l -(sp) ; move.l #'CODE',-(sp) ; move.w #2,-(sp) ; _GetResource ; _ReleaseResource
    prog.extend(CALL_AGAIN);
    let m = run(&app(&prog));
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 2);
    assert_eq!(m.log.iter().filter(|l| l.contains("LoadSeg 2:")).count(), 2, "{:#?}", m.log);
}

#[test]
fn unload_seg_of_an_address_outside_the_jump_table_is_a_named_refusal() {
    let m = {
        let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
        m.launch(&app(&[0x486D, 0x0100, 0xA9F1])).unwrap(); // pea 256(a5): not an entry
        let _ = m.run_loop(100_000).unwrap();
        m
    };
    assert!(m.refused.iter().any(|r| r.contains("UnloadSeg:") && r.contains("is not a jump-table entry")), "{:?}", m.refused);
}

#[test]
fn unloading_an_already_unloaded_segment_is_a_no_op() {
    let mut tail = UNLOAD.to_vec();
    tail.extend(UNLOAD);
    let m = run(&app(&tail));
    assert!(m.refused.is_empty(), "{:?}", m.refused);
    assert!(m.log.iter().any(|l| l.contains("already unloaded (no-op)")), "{:#?}", m.log);
    assert_eq!(entry(&m), [0, 0x3F3C, 2, 0xA9F0]);
}
