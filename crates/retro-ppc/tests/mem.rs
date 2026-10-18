//! Loads, stores, update/indexed/byte-reversed forms, multiple and string moves, reservations, dcbz, bus faults.
//! Big-endian layout checked against hand-written byte patterns; each refusal has a passing control.
mod common;
use common::*;
use retro_ppc::{Trap, XER_SO};

const D: u32 = 0x4000; // data area

fn data() -> impl Fn(&mut retro_ppc::Cpu, &mut Vm) {
    |c, v| {
        v.mem.write_bytes(D, &[0x11, 0x22, 0x33, 0x44, 0x80, 0x66, 0xF7, 0x88, 0x99, 0xAA, 0xBB, 0xCC]).unwrap();
        c.gpr[3] = D;
    }
}

#[test]
fn loads_are_big_endian_with_correct_extension() {
    let (c, _, st) = prog(&["lwz 4,0(3)", "lbz 5,4(3)", "lhz 6,4(3)", "lha 7,6(3)", "lha 8,0(3)", "lbz 9,3(3)"], data());
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(c.gpr[4], 0x1122_3344);
    assert_eq!((c.gpr[5], c.gpr[6]), (0x80, 0x8066));
    assert_eq!(c.gpr[7], 0xFFFF_F788, "lha sign-extends 0xF788");
    assert_eq!(c.gpr[8], 0x1122, "CONTROL: lha of a positive halfword");
    assert_eq!(c.gpr[9], 0x44);
}

#[test]
fn unaligned_plain_loads_work_as_on_the_6xx() {
    let (c, _, st) = prog(&["lwz 4,1(3)", "lhz 5,3(3)"], data());
    assert!(st.trap.is_none());
    assert_eq!((c.gpr[4], c.gpr[5]), (0x2233_4480, 0x4480));
}

#[test]
fn stores_write_big_endian_and_leave_neighbours() {
    let (_, v, st) = prog(&["stw 4,0(3)", "sth 5,4(3)", "stb 6,6(3)"], |c, v| {
        data()(c, v);
        c.gpr[4] = 0xDEAD_BEEF; c.gpr[5] = 0xFFFF_1234; c.gpr[6] = 0xFFFF_FF5A;
    });
    assert!(st.trap.is_none());
    assert_eq!(v.mem.bytes(v.mem.span(D, 8).unwrap()).unwrap(), &[0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x5A, 0x88]);
    assert_eq!(v.mem.read_u8(D + 8).unwrap(), 0x99, "byte after the stores untouched");
}

#[test]
fn update_forms_write_back_the_effective_address() {
    let (c, _, st) = prog(&["lwzu 4,4(3)", "stwu 4,8(3)", "lbzu 5,1(3)", "lhzu 6,1(3)"], data());
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(c.gpr[4], 0x8066_F788);
    assert_eq!(c.gpr[3], D + 4 + 8 + 1 + 1, "RA advanced by each update");
}

#[test]
fn update_form_with_ra_equal_rt_or_ra_zero_is_refused() {
    let (_, _, st) = prog(&["lwzu 3,4(3)"], data());
    assert!(matches!(st.trap, Some(Trap::InvalidForm { pc: CODE, .. })), "{st:?}");
    let (_, _, st) = prog(&["lwzu 4,4(0)"], data());
    assert!(matches!(st.trap, Some(Trap::InvalidForm { .. })), "{st:?}");
    // CONTROL: a store-update with RA == RS is legal (the stored value is the old RA).
    let (_, v, st) = prog(&["stwu 3,0(3)"], data());
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(v.mem.read_u32(D).unwrap(), D);
}

#[test]
fn ra_zero_means_zero_not_r0() {
    let (c, _, st) = prog(&["lwz 4,0x100(0)", "lwzx 5,0,6"], |c, v| {
        v.mem.write_u32(0x100, 0xCAFE_F00D).unwrap();
        c.gpr[0] = D; // must NOT be used as the base
        c.gpr[6] = 0x100;
    });
    assert!(st.trap.is_none());
    assert_eq!((c.gpr[4], c.gpr[5]), (0xCAFE_F00D, 0xCAFE_F00D));
}

#[test]
fn indexed_and_byte_reversed_forms() {
    let (c, v, st) = prog(&["lwzx 4,3,5", "lbzx 6,3,5", "lhzx 7,3,5", "lhax 8,3,9", "lwbrx 10,3,5", "lhbrx 11,3,5",
        "stwbrx 12,3,13", "sthbrx 14,3,15", "lwzux 16,3,5"], |c, v| {
        data()(c, v);
        c.gpr[5] = 0; c.gpr[9] = 6; c.gpr[12] = 0x0102_0304; c.gpr[13] = 0x20; c.gpr[14] = 0xAABB; c.gpr[15] = 0x30;
    });
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!((c.gpr[4], c.gpr[6], c.gpr[7], c.gpr[8]), (0x1122_3344, 0x11, 0x1122, 0xFFFF_F788));
    assert_eq!((c.gpr[10], c.gpr[11]), (0x4433_2211, 0x2211));
    assert_eq!(v.mem.read_u32(D + 0x20).unwrap(), 0x0403_0201, "stwbrx stores byte-reversed");
    assert_eq!(v.mem.read_u16(D + 0x30).unwrap(), 0xBBAA);
    assert_eq!(c.gpr[3], D, "lwzux with rB=0 updates RA to the same address");
}

#[test]
fn load_multiple_and_store_multiple() {
    let (c, v, st) = prog(&["lmw 29,0(3)", "stmw 29,0x40(3)"], data());
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!((c.gpr[29], c.gpr[30], c.gpr[31]), (0x1122_3344, 0x8066_F788, 0x99AA_BBCC));
    assert_eq!(v.mem.read_u32(D + 0x44).unwrap(), 0x8066_F788);
    // negative controls: misaligned lmw/stmw are alignment traps; RA inside the loaded range is invalid
    let (_, _, st) = prog(&["lmw 29,1(3)"], data());
    assert!(matches!(st.trap, Some(Trap::Alignment { addr, .. }) if addr == D + 1), "{st:?}");
    let (_, _, st) = prog(&["stmw 29,2(3)"], data());
    assert!(matches!(st.trap, Some(Trap::Alignment { .. })), "{st:?}");
    let (_, _, st) = prog(&["lmw 3,0(3)"], data());
    assert!(matches!(st.trap, Some(Trap::InvalidForm { .. })), "{st:?}");
}

#[test]
fn string_instructions_pack_big_endian_and_zero_fill() {
    let (c, v, st) = prog(&["lswi 5,3,7", "stswi 5,3,7"], |c, v| { data()(c, v); });
    assert!(st.trap.is_none());
    assert_eq!((c.gpr[5], c.gpr[6]), (0x1122_3344, 0x8066_F700), "7 bytes: the last register's tail is zeroed");
    assert_eq!(v.mem.read_u8(D + 6).unwrap(), 0xF7);
    // lswi with NB=0 loads 32 bytes into 8 registers
    let (c, _, _) = prog(&["lswi 5,3,0"], |c, v| { data()(c, v); });
    assert_eq!(c.gpr[5], 0x1122_3344);
    assert_eq!(c.gpr[12], 0, "8 registers r5..r12 loaded (past the 12 data bytes: zeros)");
    // register numbering wraps r31 -> r0
    let (c, _, _) = prog(&["lswi 31,3,8"], |c, v| { data()(c, v); });
    assert_eq!((c.gpr[31], c.gpr[0]), (0x1122_3344, 0x8066_F788));
    // lswx / stswx (absent from LLVM's tables: raw X-form words 31/533 and 31/661) take the byte count from XER
    let x = |xo: u32, rt: u32, ra: u32, rb: u32| 0x7C00_0000 | rt << 21 | ra << 16 | rb << 11 | xo << 1;
    let (c, v, st) = run_words(&[x(533, 5, 3, 4), x(661, 5, 3, 9)], |c, v| { data()(c, v); c.xer = 5; c.gpr[4] = 0; c.gpr[9] = 0x20; });
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!((c.gpr[5], c.gpr[6]), (0x1122_3344, 0x8000_0000));
    assert_eq!(v.mem.read_u8(D + 0x20).unwrap(), 0x11);
    assert_eq!(v.mem.read_u8(D + 0x24).unwrap(), 0x80);
    assert_eq!(v.mem.read_u8(D + 0x25).unwrap(), 0, "CONTROL: only five bytes stored");
}

#[test]
fn reservation_succeeds_only_for_the_reserved_address() {
    let (c, v, st) = prog(&["lwarx 4,0,3", "addi 4,4,1", "stwcx. 4,0,3"], data());
    assert!(st.trap.is_none(), "{st:?}");
    assert_eq!(c.cr_field(0), 2, "success: EQ");
    assert_eq!(v.mem.read_u32(D).unwrap(), 0x1122_3345);
    // CONTROL 1: no reservation -> store refused, memory unchanged, EQ clear
    let (c, v, _) = prog(&["stwcx. 4,0,3"], |c, v| { data()(c, v); c.gpr[4] = 7; });
    assert_eq!(c.cr_field(0), 0);
    assert_eq!(v.mem.read_u32(D).unwrap(), 0x1122_3344);
    // CONTROL 2: reservation for another address
    let (c, v, _) = prog(&["lwarx 4,0,3", "stwcx. 5,0,6"], |c, v| { data()(c, v); c.gpr[5] = 9; c.gpr[6] = D + 4; });
    assert_eq!(c.cr_field(0), 0);
    assert_eq!(v.mem.read_u32(D + 4).unwrap(), 0x8066_F788);
    // The reservation is consumed: a second stwcx. fails even for the same address.
    let (c, _, _) = prog(&["lwarx 4,0,3", "stwcx. 4,0,3", "stwcx. 4,0,3"], data());
    assert_eq!(c.cr_field(0), 0);
    // CR0[SO] is copied in
    let (c, _, _) = prog(&["lwarx 4,0,3", "stwcx. 4,0,3"], |c, v| { data()(c, v); c.xer = XER_SO });
    assert_eq!(c.cr_field(0), 2 | 1);
}

#[test]
fn misaligned_reservation_ops_are_refused() {
    let (_, _, st) = prog(&["lwarx 4,3,5"], |c, v| { data()(c, v); c.gpr[5] = 2 });
    assert!(matches!(st.trap, Some(Trap::Alignment { what: "lwarx", .. })), "{st:?}");
    let (_, _, st) = prog(&["stwcx. 4,3,5"], |c, v| { data()(c, v); c.gpr[5] = 1 });
    assert!(matches!(st.trap, Some(Trap::Alignment { what: "stwcx.", .. })), "{st:?}");
    // CONTROL: the aligned address works
    let (_, _, st) = prog(&["lwarx 4,3,5"], |c, v| { data()(c, v); c.gpr[5] = 4 });
    assert!(st.trap.is_none());
}

#[test]
fn dcbz_clears_the_aligned_32_byte_line() {
    let (_, v, st) = prog(&["dcbz 0,4"], |c, v| {
        v.mem.write_bytes(0x5000, &[0xFF; 96]).unwrap();
        c.gpr[4] = 0x5000 + 37; // inside the line 0x5020..0x5040
    });
    assert!(st.trap.is_none());
    assert_eq!(v.mem.read_u8(0x501F).unwrap(), 0xFF, "byte before the line");
    assert_eq!(v.mem.read_u8(0x5020).unwrap(), 0);
    assert_eq!(v.mem.read_u8(0x503F).unwrap(), 0);
    assert_eq!(v.mem.read_u8(0x5040).unwrap(), 0xFF, "byte after the line");
}

#[test]
fn accesses_outside_ram_are_bus_faults_at_the_instruction() {
    let (c, _, st) = prog(&["li 5,1", "lwz 4,0(3)"], |c, _| c.gpr[3] = 0x20_0000);
    assert!(matches!(st.trap, Some(Trap::BusFault { pc, addr: 0x20_0000, write: false }) if pc == CODE + 4), "{st:?}");
    assert_eq!(c.pc, CODE + 4, "pc stays at the faulting load");
    assert_eq!(c.gpr[5], 1);
    let (_, _, st) = prog(&["stw 4,0(3)"], |c, _| c.gpr[3] = 0xFFFF_FFFE);
    assert!(matches!(st.trap, Some(Trap::BusFault { write: true, .. })), "wrapping addresses are refused: {st:?}");
    // an instruction fetch outside RAM
    let (_, _, st) = run_words(&[asm("blr")], |c, _| c.lr = 0x30_0000);
    assert!(matches!(st.trap, Some(Trap::BusFault { pc: 0x30_0000, write: false, .. })), "{st:?}");
}
