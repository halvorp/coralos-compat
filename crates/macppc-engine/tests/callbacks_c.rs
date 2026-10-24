//! C library functions that call the program back: `qsort`/`bsearch` run the PROGRAM's comparison routine as PowerPC code; `exit` runs the
//! `atexit` handlers (last registered first) before the run ends. Real interpreted code on both sides: a main that calls the imports through its
//! TOC, comparison routines and handlers that are PowerPC code in the same fragment.
use guestui_abi::RecordingGuestUi;
use macppc_engine::asm::Asm;
use macppc_engine::{MacPpc, Step};
use retro_pef::loader::SymbolClass;
use retro_pef::writer::PefBuilder;
use retro_pef::Pef;

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const SUBF_R3_R4_R3: u32 = 0x7C64_1850; // subf r3,r4,r3: r3 = r3 - r4
const IMPORTS: [&str; 4] = ["qsort", "bsearch", "atexit", "exit"];
const TOC: i32 = 24; // data: main TVector [0,8), the second [8,16), the third [16,24), then one word per import
const ARR: i32 = TOC + 4 * 4; // the data area after the imports

fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }
/// `int cmp(const int*a, const int*b)`
fn int_cmp() -> Vec<u8> { let mut a = Asm::new(); a.lwz(3, 0, 3).lwz(4, 0, 4); let mut b = a.finish(); b.extend(words(&[SUBF_R3_R4_R3, 0x4E80_0020])); b }

/// A fragment: `main` (code), the routine at `second` (the second TVector's code), data = TVectors, TOC, then `data`.
fn build(main: &Asm, second: &[u8], data: &[u8]) -> (Vec<u8>, u32) { build3(main, second, &[], data) }
fn build3(main: &Asm, second: &[u8], third: &[u8], data: &[u8]) -> (Vec<u8>, u32) {
    let mut code = main.finish();
    let second_at = code.len() as u32;
    code.extend_from_slice(second);
    let third_at = code.len() as u32;
    code.extend_from_slice(third);
    let mut b = PefBuilder::new();
    b.code(&code);
    let mut init = vec![0u8; ARR as usize];
    init[8..12].copy_from_slice(&second_at.to_be_bytes());
    init[16..20].copy_from_slice(&third_at.to_be_bytes());
    init.extend_from_slice(data);
    let d = b.data(&init, init.len() as u32);
    let imports: Vec<(&str, SymbolClass, bool)> = IMPORTS.iter().map(|n| (*n, F, false)).collect();
    b.import_library("StdCLib", false, &imports);
    // three TVectors (main, second, third), then the four imports
    b.reloc(d as u16, &[0x4602, 0x4A00 | 3]);
    b.main(d as i32, 0);
    (b.build(), second_at)
}
fn boot(bytes: &[u8]) -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let frag = m.instantiate("test", &Pef::parse(bytes).unwrap()).unwrap();
    m.start(&frag).unwrap();
    m
}
fn prologue(a: &mut Asm) { a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1); }
fn epilogue(a: &mut Asm) { a.addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr(); }
fn arr(m: &M, base: u32, n: usize) -> Vec<i32> { (0..n).map(|i| m.mac.mem.read_u32(base + 4 * i as u32).unwrap() as i32).collect() }
fn data_base(m: &M) -> u32 { m.cpu().unwrap().gpr[2] }

#[test]
fn qsort_sorts_with_the_programs_comparison_routine_and_bsearch_finds_with_it() {
    let values = [5i32, 3, 9, 1, 7, 3, -4, 12];
    let n = values.len() as i32;
    let key_at = ARR + 4 * n;
    let res_at = key_at + 8;
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, ARR).li(4, n).li(5, 4).addi(6, 2, 8).call_toc(TOC); // qsort(arr, n, 4, cmp)
    a.addi(3, 2, key_at).addi(4, 2, ARR).li(5, n).li(6, 4).addi(7, 2, 8).call_toc(TOC + 4); // bsearch(&key, arr, n, 4, cmp)
    a.stw(3, res_at, 2);
    a.addi(3, 2, key_at + 4).addi(4, 2, ARR).li(5, n).li(6, 4).addi(7, 2, 8).call_toc(TOC + 4); // bsearch(&missing, ...)
    a.stw(3, res_at + 4, 2);
    epilogue(&mut a);
    let mut data = words(&values.map(|v| v as u32));
    data.extend(words(&[7, 8, 0, 0, 0, 0])); // key 7, missing key 8, result slots
    let (bytes, _) = build(&a, &int_cmp(), &data);
    let mut m = boot(&bytes);
    let base = data_base(&m);
    let (step, insns) = m.run_loop(1_000_000);
    assert_eq!(step, Step::Exited, "{:?} {:?}", m.fault(), m.mac.log);
    assert!(insns > 100, "the comparison routine really ran: {insns} instructions");
    assert_eq!(arr(&m, base + ARR as u32, values.len()), [-4, 1, 3, 3, 5, 7, 9, 12], "sorted ascending by the program's own comparison");
    let found = m.mac.mem.read_u32(base + res_at as u32).unwrap();
    assert_eq!(found, base + ARR as u32 + 4 * 5, "bsearch returns a pointer to the matching element (7 is index 5)");
    assert_eq!(m.mac.mem.read_u32(base + res_at as u32 + 4).unwrap(), 0, "NEGATIVE CONTROL: an absent key is NULL");
    assert_eq!(m.callback_depth(), 0);
}

#[test]
fn a_descending_comparison_reverses_the_order_and_zero_or_one_element_arrays_make_no_calls() {
    // cmp returns b - a
    let mut c = Asm::new();
    c.lwz(3, 0, 3).lwz(4, 0, 4);
    let mut cmp = c.finish();
    cmp.extend(words(&[0x7C63_2050, 0x4E80_0020])); // subf r3,r3,r4: r3 = r4 - r3
    let values = [2u32, 8, 4, 6];
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, ARR).li(4, 4).li(5, 4).addi(6, 2, 8).call_toc(TOC);
    a.addi(3, 2, ARR).li(4, 1).li(5, 4).addi(6, 2, 8).call_toc(TOC); // one element: nothing to do
    a.addi(3, 2, ARR).li(4, 0).li(5, 4).addi(6, 2, 8).call_toc(TOC); // none
    epilogue(&mut a);
    let (bytes, _) = build(&a, &cmp, &words(&values));
    let mut m = boot(&bytes);
    let base = data_base(&m);
    assert_eq!(m.run_loop(1_000_000).0, Step::Exited, "{:?}", m.fault());
    assert_eq!(arr(&m, base + ARR as u32, 4), [8, 6, 4, 2]);
}

#[test]
fn qsort_with_a_null_comparison_or_a_zero_size_stops_naming_qsort() {
    for (size, cmp, want) in [(4, 0, "qsort: the comparison function pointer is NULL"), (0, 8, "qsort: element size 0")] {
        let mut a = Asm::new();
        prologue(&mut a);
        a.addi(3, 2, ARR).li(4, 3).li(5, size);
        if cmp == 0 { a.li(6, 0); } else { a.addi(6, 2, 8); }
        a.call_toc(TOC);
        epilogue(&mut a);
        let (bytes, _) = build(&a, &int_cmp(), &words(&[3, 2, 1]));
        let mut m = boot(&bytes);
        let (step, _) = m.run_loop(100_000);
        assert!(matches!(&step, Step::Fault(w) if w.starts_with(want)), "{step:?}");
    }
}

#[test]
fn exit_runs_the_atexit_handlers_last_registered_first_and_without_any_it_just_ends() {
    // two different handlers share a counter and each records the counter value it saw in its own slot
    let (cnt, slot_a, slot_b) = (ARR, ARR + 4, ARR + 8);
    let handler = |slot: i32| { let mut h = Asm::new(); h.lwz(9, cnt, 2).addi(9, 9, 1).stw(9, cnt, 2).stw(9, slot, 2).blr(); h.finish() };
    let mut a = Asm::new();
    prologue(&mut a);
    a.addi(3, 2, 8).call_toc(TOC + 8);  // atexit(A)
    a.addi(3, 2, 16).call_toc(TOC + 8); // atexit(B)
    a.li(3, 7).call_toc(TOC + 12);      // exit(7)
    a.li(3, 99).stw(3, cnt, 2);         // never reached
    epilogue(&mut a);
    let (bytes, _) = build3(&a, &handler(slot_a), &handler(slot_b), &words(&[0, 0, 0]));
    let mut m = boot(&bytes);
    let base = data_base(&m);
    let (step, _) = m.run_loop(100_000);
    assert_eq!(step, Step::Exited, "{:?} {:?}", m.fault(), m.mac.log);
    assert_eq!(arr(&m, base + cnt as u32, 3), [2, 2, 1], "B (registered last) ran first and saw 1; A saw 2; nothing after exit() ran");
    assert_eq!(m.mac.log.iter().filter(|l| l.contains("atexit handler")).count(), 2);
    assert_eq!(m.callback_depth(), 0);
    // NEGATIVE CONTROL: with no handler registered exit ends the run at once
    let mut a = Asm::new();
    prologue(&mut a);
    a.li(3, 7).call_toc(TOC + 12);
    a.li(3, 99).stw(3, cnt, 2);
    epilogue(&mut a);
    let (bytes, _) = build3(&a, &handler(slot_a), &handler(slot_b), &words(&[0, 0, 0]));
    let mut m = boot(&bytes);
    let base = data_base(&m);
    assert_eq!(m.run_loop(100_000).0, Step::Exited);
    assert_eq!(arr(&m, base + cnt as u32, 3), [0, 0, 0]);
    assert!(!m.mac.log.iter().any(|l| l.contains("atexit handler")));
}
