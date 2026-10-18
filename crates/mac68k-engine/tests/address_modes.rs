//! 24-bit versus 32-bit addressing (Inside Macintosh: Memory). A 68000-era application keeps flags in the high byte of
//! pointers and handles; the 24-bit bus ignores that byte. The machine boots 24-bit; 32-bit is a machine option. Every
//! behaviour has its 32-bit control.
use guestui_abi::RecordingGuestUi;
use mac68k_engine::addrmode::AddrMode;
use mac68k_engine::machine::CODE_BASE;
use mac68k_engine::Mac;

fn machine(mode: AddrMode) -> Mac<RecordingGuestUi> {
    let mut m = Mac::new(RecordingGuestUi::new()).unwrap();
    m.set_address_mode(mode).unwrap();
    m
}
fn l(m: &Mac<RecordingGuestUi>, a: u32) -> u32 { m.mem.read_u32(a).unwrap() }
fn words(w: &[u16]) -> Vec<u8> { w.iter().flat_map(|x| x.to_be_bytes()).collect() }

/// `move.l #(flag|target),-(sp) ; rts` jumps THROUGH a flagged address (WriteNow's segment-return trick: it sets the high
/// byte of a return address and `RTR`s to it); the target stores 42.
fn flagged_jump(m: &mut Mac<RecordingGuestUi>) -> u64 {
    let target = CODE_BASE + 8;
    let flagged = 0x8000_0000 | target;
    let prog = words(&[0x2F3C, (flagged >> 16) as u16, flagged as u16, 0x4E75, 0x303C, 42, 0x33C0, 0, 0x3000, 0xA9F4]);
    let r = m.run_program(&prog, 0, 1000).unwrap();
    u64::from(r.stats.bus_faults)
}

#[test]
fn a_24_bit_machine_runs_code_through_a_pointer_with_flags_in_its_high_byte() {
    let mut m = machine(AddrMode::Bit24);
    assert_eq!(flagged_jump(&mut m), 0);
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 42, "the jump landed on the real code");
    assert_eq!(m.mem.read_u8(mac68k_engine::lowmem::MMU_32_BIT).unwrap(), 0, "MMU32Bit says 24-bit");
}

#[test]
fn the_same_jump_on_a_32_bit_machine_runs_off_into_nothing() {
    let mut m = machine(AddrMode::Bit32);
    assert!(flagged_jump(&mut m) > 0, "bus faults: 0x8001000x is not RAM");
    assert_eq!(m.mem.read_u16(0x3000).unwrap(), 0, "the target never ran");
    assert_eq!(m.mem.read_u8(mac68k_engine::lowmem::MMU_32_BIT).unwrap(), 1);
}

/// NewHandle ; HLock ; read the master pointer ; HUnlock ; read again ; HGetState / StripAddress results.
fn lock_program(m: &mut Mac<RecordingGuestUi>) {
    let p = words(&[
        0x7010, 0xA122, 0x23C8, 0, 0x3000,       // moveq #16,d0 ; _NewHandle ; move.l a0,$3000
        0xA029, 0x23D0, 0, 0x3004,                // _HLock ; move.l (a0),$3004   -- the master pointer while locked
        0xA069, 0x23C0, 0, 0x300C,                // _HGetState ; move.l d0,$300C
        0x2050, 0x2008, 0xA055, 0x23C0, 0, 0x3010, // move.l (a0),a0 ; move.l a0,d0 ; _StripAddress ; move.l d0,$3010
        0x2079, 0, 0x3000, 0xA02A, 0x23D0, 0, 0x3008, // movea.l $3000,a0 ; _HUnlock ; move.l (a0),$3008
        0xA9F4,
    ]);
    m.run_program(&p, 0, 2000).unwrap();
}

#[test]
fn hlock_puts_the_lock_bit_in_the_master_pointers_high_byte_and_hunlock_takes_it_out() {
    let mut m = machine(AddrMode::Bit24);
    lock_program(&mut m);
    let (h, locked, unlocked) = (l(&m, 0x3000), l(&m, 0x3004), l(&m, 0x3008));
    assert_eq!(locked >> 24, 0x80, "locked: bit 7 of the master pointer's high byte");
    assert_eq!(unlocked >> 24, 0, "HUnlock clears it");
    assert_eq!(locked & 0x00FF_FFFF, unlocked, "the address bits are the block's");
    assert_eq!(l(&m, 0x300C) & 0xFF, 0x80, "HGetState reads the same bit");
    assert_eq!(l(&m, 0x3010), unlocked, "StripAddress removes the flag byte");
    assert_eq!(m.mm.block(h).map(|b| b.ptr), Some(unlocked), "the host copy of the block address is clean");
}

#[test]
fn on_a_32_bit_machine_master_pointers_are_bare_addresses_and_strip_address_is_the_identity() {
    let mut m = machine(AddrMode::Bit32);
    lock_program(&mut m);
    assert_eq!(l(&m, 0x3004) >> 24, 0, "no flag bits in a 32-bit master pointer");
    assert_eq!(l(&m, 0x300C) & 0xFF, 0x80, "HGetState still reports the lock (the host keeps the state)");
    // control for StripAddress: hand it a flagged value on each machine
    for (mode, want) in [(AddrMode::Bit24, 0x0012_3456u32), (AddrMode::Bit32, 0x8012_3456)] {
        let mut m = machine(mode);
        let p = words(&[0x203C, 0x8012, 0x3456, 0xA055, 0x23C0, 0, 0x3000, 0xA9F4]);
        m.run_program(&p, 0, 100).unwrap();
        assert_eq!(l(&m, 0x3000), want, "{mode:?}");
    }
}

#[test]
fn swap_mmu_mode_switches_the_bus_and_returns_the_previous_mode() {
    let mut m = machine(AddrMode::Bit24);
    // moveq #1,d0 ; _SwapMMUMode ; move.b d0,$3000 (previous) ; moveq #0,d0 ; _SwapMMUMode ; move.b d0,$3001
    let p = words(&[0x7001, 0xA05D, 0x13C0, 0, 0x3000, 0x7000, 0xA05D, 0x13C0, 0, 0x3001, 0xA9F4]);
    m.run_program(&p, 0, 100).unwrap();
    assert_eq!((m.mem.read_u8(0x3000).unwrap(), m.mem.read_u8(0x3001).unwrap()), (0, 1), "24 -> 32 reports 0, then 32 -> 24 reports 1");
    assert_eq!(m.address_mode(), AddrMode::Bit24);
    // a mode number that is neither is a NAMED refusal
    let mut m = machine(AddrMode::Bit24);
    m.run_program(&words(&[0x7007, 0xA05D, 0xA9F4]), 0, 100).unwrap();
    assert!(m.refused.iter().any(|r| r.contains("SwapMMUMode: mode 7")), "{:?}", m.refused);
    assert_eq!(m.address_mode(), AddrMode::Bit24);
}
