//! The client as the macppc engine uses it, on the static core's table (feature bind-static) and a real engine-owned GuestMem.
use super::*;
use heirloom_mem_flat32::GuestMem;

const CODE: u32 = 0x1000;
const LI_R3_5: u32 = 0x3860_0005;
const ADDI_R3_1: u32 = 0x3863_0001;
const B_SELF: u32 = 0x4800_0000;

struct H { mem: GuestMem, calls: Vec<u32>, act: fn(&mut H, &mut Cpu, u32) -> bool }
impl Host for H {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool { self.calls.push(addr); (self.act)(self, cpu, addr) }
}
fn host(act: fn(&mut H, &mut Cpu, u32) -> bool) -> H {
    let mut mem = GuestMem::new(0x10_0000);
    for (i, w) in [LI_R3_5, ADDI_R3_1, B_SELF].iter().enumerate() { mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    mem.write_u32(0x2000, B_SELF).unwrap();
    H { mem, calls: Vec::new(), act }
}

#[test]
fn bind_takes_the_same_table_twice_and_refuses_a_foreign_family() {
    let t = heirloom_cpu_ppc::table::vtable();
    assert_eq!(bind(t), Ok(()));
    assert_eq!(bind(t), Ok(()), "the same table again is fine");
    let foreign: &'static PpcVTable = Box::leak(Box::new(PpcVTable { abi_family: 0, ..*t }));
    assert_eq!(bind(foreign), Err(BindRefusal::Open(PpcOpenRefusal::WrongFamily { found: 0 })), "NEGATIVE CONTROL");
}

#[test]
fn registers_round_trip_through_a_run() {
    let mut h = host(|_, _, _| true);
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    let st = Exec::default().resume(&mut h, &mut cpu, 3);
    assert_eq!((st.insns, cpu.gpr[3], cpu.pc, cpu.gpr[1]), (3, 6, CODE + 8, 0x8_0000));
}

#[test]
fn a_native_runs_the_hosts_handler_with_the_engines_cpu() {
    let mut h = host(|_, c, _| { c.gpr[3] = 42; true });
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    cpu.bind_native(CODE);
    cpu.lr = 0x2000;
    let mut e = Exec::default();
    let _ = e.resume(&mut h, &mut cpu, 4);
    assert_eq!((cpu.gpr[3], cpu.pc), (42, 0x2000));
    let mut h = host(|_, _, _| false);
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    cpu.bind_native(CODE);
    assert_eq!(Exec::default().resume(&mut h, &mut cpu, 4).trap, Some(Trap::NativeUnhandled { addr: CODE }), "NEGATIVE CONTROL: declined");
}

#[test]
fn halt_and_block_inside_a_native_behave_as_with_the_core() {
    let mut h = host(|_, c, _| { c.halt(); true });
    let mut cpu = Cpu::new(CODE, 0);
    cpu.bind_native(CODE);
    let mut e = Exec::default();
    assert!(e.resume(&mut h, &mut cpu, 4).halted);
    assert_eq!(e.resume(&mut h, &mut cpu, 4).insns, 0, "a halted guest runs nothing more");
    let mut h = host(|_, c, a| { c.block(); c.set_pc(a); true });
    let mut cpu = Cpu::new(CODE, 0);
    cpu.bind_native(CODE);
    let mut e = Exec::default();
    let st = e.resume(&mut h, &mut cpu, 4);
    assert_eq!((st.blocked, cpu.pc), (true, CODE));
    let _ = e.resume(&mut h, &mut cpu, 4);
    assert_eq!(h.calls.len(), 2, "the parked call is entered again");
}

#[test]
fn a_native_bound_inside_a_native_is_honoured_in_the_same_run() {
    let mut h = host(|_, c, a| { if a == CODE { c.bind_native(0x2000); } true });
    let mut cpu = Cpu::new(CODE, 0);
    cpu.bind_native(CODE);
    cpu.lr = 0x2000;
    let _ = Exec::default().resume(&mut h, &mut cpu, 2);
    assert_eq!(h.calls, [CODE, 0x2000]);
}

#[test]
fn two_threads_alternate_on_one_exec_and_keep_their_own_registers() {
    let mut h = host(|_, _, _| true);
    let mut e = Exec::default();
    let mut a = Cpu::new(CODE, 0x8_0000);
    let mut b = Cpu::new(CODE + 4, 0x9_0000);
    b.gpr[3] = 100;
    b.fpr[2] = 0x3FF0_0000_0000_0000;
    b.lr = 0x77;
    let _ = e.resume(&mut h, &mut a, 1);
    let _ = e.resume(&mut h, &mut b, 1);
    let _ = e.resume(&mut h, &mut a, 1);
    assert_eq!((a.gpr[3], a.fpr[2], a.lr, a.pc), (6, 0, 0, CODE + 8), "NEGATIVE CONTROL for cached registers");
    assert_eq!((b.gpr[3], b.fpr[2], b.lr, b.gpr[1]), (101, 0x3FF0_0000_0000_0000, 0x77, 0x9_0000));
}

/// The same loop on the static core and through the table: equal registers and counts.
#[test]
fn the_library_path_gives_the_cores_result() {
    let prog = [0x3880_0064u32, 0x7C89_03A6, 0x3863_0003, 0x4200_FFFC, B_SELF]; // li r4,100; mtctr r4; addi r3,r3,3; bdnz -4; b .
    let mut core_mem = GuestMem::new(0x10_0000);
    for (i, w) in prog.iter().enumerate() { core_mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    struct C(GuestMem);
    impl heirloom_cpu_ppc::Host for C { fn mem(&mut self) -> &mut GuestMem { &mut self.0 } }
    let mut c = C(core_mem);
    let mut core_cpu = heirloom_cpu_ppc::Cpu::new(CODE, 0);
    let cs = heirloom_cpu_ppc::Exec::default().resume(&mut c, &mut core_cpu, 250);
    let mut h = host(|_, _, _| true);
    for (i, w) in prog.iter().enumerate() { h.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    let mut cpu = Cpu::new(CODE, 0);
    let ls = Exec::default().resume(&mut h, &mut cpu, 250);
    assert_eq!((cpu.gpr[3], cpu.ctr, cpu.pc, ls.insns), (core_cpu.gpr[3], core_cpu.ctr, core_cpu.pc, cs.insns));
    assert_eq!(cpu.gpr[3], 300);
}

#[test]
fn a_native_reads_the_arguments_the_guest_just_computed() {
    let mut h = host(|_, c, _| { c.gpr[3] *= 10; true });
    let mut cpu = Cpu::new(CODE, 0);
    cpu.bind_native(CODE + 4); // after li r3,5
    cpu.lr = 0x2000;
    let _ = Exec::default().resume(&mut h, &mut cpu, 3);
    assert_eq!(cpu.gpr[3], 50, "the handler saw r3 = 5 from the guest's own li");
}

#[test]
fn a_run_that_ran_nothing_is_a_named_refusal_never_a_guest_fault() {
    let owner = host(|_, _, _| true);
    // SAFETY: `owner.mem` is a live owned GuestMem; its descriptor outlives `view` (both live to the end of the test).
    let view = unsafe { GuestMem::view(owner.mem.flat_ram().unwrap()) };
    let mut h = H { mem: view, calls: Vec::new(), act: |_, _, _| true };
    let mut cpu = Cpu::new(CODE, 0);
    let st = Exec::default().resume(&mut h, &mut cpu, 3);
    assert_eq!((st.insns, st.trap), (0, Some(Trap::Refused { why: REFUSED_NO_FLAT_RAM })), "a view owns no descriptor to hand the library");
    // the library's own refusals and an undefined exit code name themselves through the one codec
    for (exit, why) in [
        (PpcExit::refused(heirloom_cpu_abi::ppc::REFUSE_RAM), heirloom_cpu_abi::ppc::REFUSED_RAM),
        (PpcExit::host_protocol(CODE, 99, 0), heirloom_cpu_abi::ppc::REFUSED_HOST_PROTOCOL),
        (PpcExit { code: 77, ..PpcExit::budget(0, 0) }, heirloom_cpu_abi::ppc::REFUSED_EXIT_CODE),
    ] {
        assert_eq!(refusal_of(&exit), why);
        assert_eq!(PpcExit::trap(&Trap::Refused { why }, 0).decode_trap(), Some(Trap::Refused { why }), "{why} crosses by id");
    }
}

extern "C" fn refuse_reserve(_: *mut c_void, _: usize, _: *mut *mut u8) -> i32 { -1 }
extern "C" fn refuse_range(_: *mut c_void, _: *mut u8, _: usize) -> i32 { -1 }

/// Tier::Jit crosses the table only with executable memory the library accepted; otherwise the run is tier 1 and named.
#[test]
fn tier_jit_needs_an_accepted_code_mem_and_otherwise_runs_tier_1_by_name() {
    let mut h = host(|_, _, _| true);
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    let mut e = Exec::new(Tier::Jit);
    let _ = e.resume(&mut h, &mut cpu, 3);
    assert!(e.lockstep_refused(), "no provider: tier 1, named");
    assert_eq!(cpu.gpr[3], 6);
    // a malformed record is refused by the library, and the run stays tier 1 by name
    let bad = PpcCodeMem { size: 1, flags: 0, ctx: ptr::null_mut(), reserve: refuse_reserve, publish: refuse_range, reset: refuse_range };
    let mut e = Exec::new(Tier::Jit);
    e.set_code_mem(bad);
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    let _ = e.resume(&mut h, &mut cpu, 3);
    assert!(e.lockstep_refused());
    // an accepted record selects TIER_JIT (this one refuses to reserve, so the library's JIT itself falls back to tier 1)
    let ok = PpcCodeMem { size: core::mem::size_of::<PpcCodeMem>() as u32, ..bad };
    let mut e = Exec::new(Tier::Jit);
    e.set_code_mem(ok);
    let mut cpu = Cpu::new(CODE, 0x8_0000);
    let st = e.resume(&mut h, &mut cpu, 3);
    assert!(!e.lockstep_refused(), "NEGATIVE CONTROL: an accepted provider is not a refusal");
    assert_eq!((st.insns, cpu.gpr[3]), (3, 6));
    assert_eq!(e.tier_set, Some(TIER_JIT));
}

/// Counts the allocations of the thread that switched it on.
struct Counting;
std::thread_local! { static ON: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; static N: core::cell::Cell<u64> = const { core::cell::Cell::new(0) }; }
// SAFETY: forwards every call unchanged to the system allocator; the thread-local counter is a side effect only.
unsafe impl core::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: core::alloc::Layout) -> *mut u8 {
        if ON.with(|o| o.get()) { N.with(|n| n.set(n.get() + 1)); }
        // SAFETY: the caller's contract for `alloc` is passed through.
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: core::alloc::Layout) {
        // SAFETY: `p` came from System with layout `l`.
        unsafe { std::alloc::System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

/// h86: two guest threads' CPUs carrying the same 1,300 bound natives (each its own copy of the set, so their generations differ)
/// run in turn on one Exec: switching between them allocates nothing once both are bound (the mirror was cloned whole per
/// switch: ~120 node allocations each - RED before the in-place sync), and a native bound on one thread only still reaches the
/// library when that thread runs and is unbound when the other does.
#[test]
fn switching_between_threads_with_equal_native_sets_allocates_nothing() {
    let mut h = host(|_, _, _| true);
    let mut a = Cpu::new(CODE, 0x8_0000);
    for k in 0..1_300u32 { a.bind_native(0x10_0000 + 16 * k); }
    let mut b = a.clone();
    b.bind_native(0x9_0000);
    b.unbind_native(0x9_0000); // b's set equals a's, under its own generation
    let mut e = Exec::default();
    for _ in 0..2 { let _ = e.resume(&mut h, &mut a, 2); let _ = e.resume(&mut h, &mut b, 2); }
    (a.pc, b.pc) = (CODE, CODE);
    ON.with(|o| o.set(true));
    for _ in 0..50 {
        a.pc = CODE;
        let _ = e.resume(&mut h, &mut a, 2);
        b.pc = CODE;
        let _ = e.resume(&mut h, &mut b, 2);
    }
    ON.with(|o| o.set(false));
    assert_eq!(N.with(|n| n.get()), 0, "switching threads allocated");
    // a native only b has: b's run calls it, a's does not
    b.bind_native(CODE);
    b.pc = CODE;
    b.lr = 0x2000;
    let calls = h.calls.len();
    let _ = e.resume(&mut h, &mut b, 4);
    assert_eq!(h.calls.len(), calls + 1, "b's own native ran");
    a.pc = CODE;
    let _ = e.resume(&mut h, &mut a, 2);
    assert_eq!(h.calls.len(), calls + 1, "a, without it, executed the code instead");
}
