use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, Ordering};
use heirloom_cpu_68k::vtable;
use heirloom_cpu_abi::{
    check, CpuBusCallbacks, CpuStop, OpenRefusal, ARCH_M68K, STOP_HALT,
};

struct Harness {
    ram: Vec<u8>,
    faults: u32,
}

extern "C" fn read8(ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at >= h.ram.len() {
        h.faults += 1;
        return 1;
    }
    // SAFETY: Musashi supplies a writable result pointer for each read callback.
    unsafe { *out = u32::from(h.ram[at]) };
    0
}

extern "C" fn read16(ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at + 2 > h.ram.len() {
        h.faults += 1;
        return 1;
    }
    // SAFETY: Musashi supplies a writable result pointer for each read callback.
    unsafe { *out = u32::from(u16::from_be_bytes([h.ram[at], h.ram[at + 1]])) };
    0
}

extern "C" fn read32(ctx: *mut c_void, addr: u32, out: *mut u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at + 4 > h.ram.len() {
        h.faults += 1;
        return 1;
    }
    // SAFETY: Musashi supplies a writable result pointer for each read callback.
    unsafe {
        *out = u32::from_be_bytes([
            h.ram[at],
            h.ram[at + 1],
            h.ram[at + 2],
            h.ram[at + 3],
        ])
    };
    0
}

extern "C" fn write8(ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at >= h.ram.len() {
        h.faults += 1;
        return 1;
    }
    h.ram[at] = value as u8;
    0
}

extern "C" fn write16(ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at + 2 > h.ram.len() {
        h.faults += 1;
        return 1;
    }
    h.ram[at..at + 2].copy_from_slice(&(value as u16).to_be_bytes());
    0
}

extern "C" fn write32(ctx: *mut c_void, addr: u32, value: u32) -> i32 {
    // SAFETY: every callback receives the Harness pointer installed in the bus table.
    let h = unsafe { &mut *(ctx.cast::<Harness>()) };
    let at = addr as usize;
    if at + 4 > h.ram.len() {
        h.faults += 1;
        return 1;
    }
    h.ram[at..at + 4].copy_from_slice(&value.to_be_bytes());
    0
}

static TRAP_RETURN: AtomicI32 = AtomicI32::new(2);

extern "C" fn on_trap(_ctx: *mut c_void, _pc: u32, _opcode: u32) -> i32 {
    TRAP_RETURN.load(Ordering::Relaxed)
}

#[test]
fn vtable_executes_traps_and_enforces_the_single_instance() {
    let mut harness = Harness { ram: vec![0; 256], faults: 0 };
    harness.ram[4..8].copy_from_slice(&8u32.to_be_bytes());
    // moveq #5,d0 ; addq.l #2,d0 ; illegal ($4afc)
    harness.ram[8..14].copy_from_slice(&[0x70, 0x05, 0x54, 0x80, 0x4A, 0xFC]);
    let bus = CpuBusCallbacks {
        ctx: (&mut harness as *mut Harness).cast::<c_void>(),
        read8,
        read16,
        read32,
        write8,
        write16,
        write32,
        on_trap,
    };
    let table = vtable();

    assert!(check(table, 1, ARCH_M68K).is_ok());
    assert!(matches!(check(table, 3, ARCH_M68K), Err(OpenRefusal::VersionTooOld { found: 2, required: 3 })));

    let inst = (table.create)(4, &bus);
    assert!(!inst.is_null());
    (table.reset)(inst);
    let mut stop = CpuStop { kind: 0, pc: 0, detail: 0, bus_faults: 0, insns: 0 };
    assert_eq!((table.run)(inst, 100, &mut stop), 0);
    assert_eq!(stop.kind, STOP_HALT);
    let mut d0 = 0;
    assert_eq!((table.get_reg)(inst, 0, &mut d0), 0);
    assert_eq!(d0, 7);

    // move.l $f00000.l,d0 ; illegal: the unmapped read is reported to the bus.
    harness.ram[8..16].copy_from_slice(&[0x20, 0x39, 0x00, 0xF0, 0, 0, 0x4A, 0xFC]);
    (table.reset)(inst);
    assert_eq!((table.run)(inst, 100, &mut stop), 0);
    assert_eq!(stop.kind, STOP_HALT);
    assert_eq!(harness.faults, 1);

    assert!((table.create)(4, &bus).is_null());
    (table.destroy)(inst);
    let replacement = (table.create)(4, &bus);
    assert!(!replacement.is_null());
    (table.destroy)(replacement);
}