//! The tiered runner (`Exec`): tier 0 through it is exactly `resume`, the profile counters and line, the census.
mod common;
use common::*;
use retro_ppc::{resume, Exec, Tier};

const BUF: u32 = 0x4000;

fn crc_vm(bytes: &[u8]) -> (retro_ppc::Cpu, Vm) {
    let mut v = vm();
    for (i, w) in programs::CRC32.iter().enumerate() { v.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    v.mem.write_u32(CODE + 4 * programs::CRC32.len() as u32, asm("blr")).unwrap();
    v.mem.write_bytes(BUF, bytes).unwrap();
    let mut c = retro_ppc::Cpu::new(CODE, STACK);
    c.lr = EXIT;
    c.bind_native(EXIT);
    c.gpr[3] = BUF;
    c.gpr[4] = bytes.len() as u32;
    (c, v)
}

#[test]
fn tier_0_through_exec_is_resume_and_counts_what_it_retired() {
    let (mut a, mut va) = crc_vm(b"123456789");
    let ra = resume(&mut va, &mut a, 1_000_000);
    let (mut b, mut vb) = crc_vm(b"123456789");
    let mut e = Exec::new(Tier::Interp);
    let mut total = 0;
    loop { let st = e.resume(&mut vb, &mut b, 37); total += st.insns; if st.halted { break; } }
    assert_eq!((a.gpr, a.pc, a.tb, a.cr, a.xer), (b.gpr, b.pc, b.tb, b.cr, b.xer), "same state, in 37-instruction slices");
    assert_eq!(b.gpr[5], 0xCBF4_3926);
    assert_eq!(e.prof.insns, ra.insns);
    assert_eq!(e.prof.insns, total);
    assert_eq!(e.prof.natives, 1, "the EXIT hook");
}

#[test]
fn the_profile_line_has_the_agreed_shape() {
    let p = retro_ppc::Prof { insns: 3_000_000, natives: 12, ..Default::default() };
    assert_eq!(p.line(1_500_000, Tier::Interp), "[PPC-PROF insns=3000000 native_calls=12 elapsed_us=1500000 mips=2.00 tier=0]");
    assert_eq!(p.line(0, Tier::Interp), "[PPC-PROF insns=3000000 native_calls=12 elapsed_us=0 mips=0.00 tier=0]", "no division by zero");
}

#[test]
fn the_census_counts_every_instruction_and_block_of_a_known_program() {
    let (mut c, mut v) = crc_vm(b"abc");
    let mut e = Exec::new(Tier::Interp);
    assert!(e.census().is_none(), "NEGATIVE CONTROL: no census unless asked for");
    e.enable_census();
    let st = e.resume(&mut v, &mut c, 1_000_000);
    assert!(st.halted);
    let k = e.census().unwrap();
    assert_eq!(k.total() + k.natives, st.insns, "every retired instruction is in the census, the native hook apart");
    let ops: std::collections::BTreeMap<String, u64> = k.top_ops(100).into_iter().collect();
    assert_eq!(ops["lbz"], 3, "one load per byte");
    assert_eq!(ops["andi."], 24, "one per bit");
    assert_eq!(ops["bclr"], 1);
    let (_, entries, insns) = k.block_summary();
    assert_eq!(insns, k.total(), "blocks partition the instructions");
    assert!(entries > 24, "a block ends at every taken branch: {entries}");
}

#[test]
fn native_timing_attributes_calls_to_their_address() {
    fn clock() -> u64 { 7 }
    let (mut c, mut v) = crc_vm(b"");
    let mut e = Exec::new(Tier::Interp);
    assert!(e.native_profile().is_none());
    e.profile_natives(clock);
    e.resume(&mut v, &mut c, 1_000);
    let p = e.native_profile().unwrap();
    assert_eq!(p.by_addr.get(&EXIT), Some(&(1, 0)), "one call; the constant clock measured 0");
    assert_eq!(p.by_addr.len(), 1);
}
