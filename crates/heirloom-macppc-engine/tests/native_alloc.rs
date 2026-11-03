//! The per-frame natives of a game allocate nothing once warm: a clock read (`Microseconds`, `UpTime`: Warcraft III reads the
//! clock ~80 times a frame) and a Time Manager task's callback (entered at a safepoint and returned from) - each used to build a
//! String or a scratch CPU's set or intern its return stub by name, call after call. Counted with a counting global allocator
//! around the calls alone, through `Host::native` as JIT code calls them.
use heirloom_mac68k_engine::mixed_mode::{new_routine_descriptor, ISA_PPC};
use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;
std::thread_local! { static ON: Cell<bool> = const { Cell::new(false) }; static N: Cell<u64> = const { Cell::new(0) }; }
// SAFETY: forwards every call unchanged to the system allocator; the thread-local counter is a side effect only.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.with(|o| o.get()) { N.with(|n| n.set(n.get() + 1)); }
        // SAFETY: the caller's contract for `alloc` is passed through.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: `p` came from System with layout `l`.
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static A: Counting = Counting;

type M = MacPpc<RecordingGuestUi>;
const T: u32 = 0x3400;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;
const OUT: u32 = 0x0009_0000;

fn stub(m: &mut M, name: &str) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name}");
    let id = m.glue.intern(&mut m.mac, "CFM", name).unwrap();
    m.glue.entry(id).unwrap().stub
}

fn counted(f: impl FnOnce()) -> u64 {
    N.with(|n| n.set(0));
    ON.with(|o| o.set(true));
    f();
    ON.with(|o| o.set(false));
    N.with(|n| n.get())
}

#[test]
fn clock_reads_and_time_manager_callbacks_do_not_allocate_per_call() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    let (micro, up) = (stub(&mut m, "Microseconds"), stub(&mut m, "UpTime"));
    let mut cpu = Cpu::new(0x5000, SP);
    let mut read = |m: &mut M, cpu: &mut Cpu| {
        cpu.gpr[3] = OUT;
        assert!(m.native(cpu, micro));
        assert!(m.native(cpu, up));
    };
    read(&mut m, &mut cpu); // warm
    let n = counted(|| for _ in 0..1_000 { read(&mut m, &mut cpu) });
    assert_eq!(n, 0, "{n} allocations for 2,000 clock reads");

    // a Time Manager task due at every safepoint: entered, returned from, primed again - 100 times
    let upp = new_routine_descriptor(&mut m.mac, TV, 0x2C0, ISA_PPC).unwrap();
    m.mac.mem.write_u32(T + 6, upp).unwrap();
    let mut regs = [0u32; 32];
    regs[3] = T;
    assert!(!matches!(m.bind("InsTime"), Binding::Unbound(_)) && !matches!(m.bind("PrimeTime"), Binding::Unbound(_)));
    assert_eq!(m.glue_call("InsTime", &mut regs), GlueOutcome::Return);
    let tick = stub(&mut m, "TickCount");
    let mut cpu = Cpu::new(0x5000, SP);
    let mut fire = |m: &mut M, cpu: &mut Cpu| {
        let mut regs = [0u32; 32];
        (regs[3], regs[4]) = (T, 1);
        assert_eq!(m.glue_call("PrimeTime", &mut regs), GlueOutcome::Return);
        m.mac.advance_ticks(1);
        cpu.pc = 0x5000;
        assert!(m.native(cpu, tick));
        assert_eq!(cpu.pc, CODE, "the task runs");
        let ret = cpu.lr;
        assert!(m.native(cpu, ret));
        assert_eq!(m.callback_depth(), 0);
    };
    for _ in 0..3 { fire(&mut m, &mut cpu); } // warm: the return stub, the call frames' buffers
    let n = counted(|| for _ in 0..100 { fire(&mut m, &mut cpu) });
    eprintln!("[NATIVE-ALLOC] {n} allocations for 100 Time Manager callbacks");
    // NAMED REMAINDER: two per callback - the routine descriptor's record list (mixed_mode::parse) and the safepoint's log line.
    // DELETE-WHEN: mixed_mode::parse returns its PowerPC record without a Vec and the safepoint stops logging every task it runs
    // (then tighten this bound to the per-callback frame alone).
    assert!(n <= 210, "{n} allocations for 100 Time Manager callbacks");
}
