//! cpu-ppc.library's table as an engine drives it: raw entries, the heirloom-cpu-abi::ppc records, an engine-owned GuestMem.
extern crate std;

use crate::table::{plant_stale_code_mutant, vtable};
use core::ffi::c_void;
use heirloom_cpu_abi::ppc::*;
use heirloom_cpu_abi::CpuInstance;
use heirloom_mem_flat32::GuestMem;
use std::vec::Vec;

const CODE: u32 = 0x1000;
const LI_R3_5: u32 = 0x3860_0005;
const ADDI_R3_1: u32 = 0x3863_0001;
const B_SELF: u32 = 0x4800_0000;

/// What a test's callbacks see and do. `mem` is the ENGINE's RAM (the callback may grow it).
struct Eng {
    mem: GuestMem,
    seen: Vec<u32>,
    answer: i32,
    on_call: Option<fn(&mut Eng, &mut PpcContext, u32)>,
    inst: *mut CpuInstance,
}

extern "C" fn on_native(ctx: *mut c_void, cpu: *mut PpcContext, addr: u32) -> i32 {
    // SAFETY: ctx is the test's Eng, cpu the run's context, both live for the run.
    let (e, c) = unsafe { (&mut *(ctx as *mut Eng), &mut *cpu) };
    e.seen.push(addr);
    if let Some(f) = e.on_call { f(e, c, addr); }
    e.answer
}
extern "C" fn on_syscall(ctx: *mut c_void, cpu: *mut PpcContext, pc: u32) -> i32 { on_native(ctx, cpu, pc) }

struct Rig { eng: Box<Eng>, host: Box<PpcHostCallbacks>, inst: *mut CpuInstance }

impl Rig {
    fn new() -> Rig {
        let mut eng = Box::new(Eng { mem: GuestMem::new(0x10_0000), seen: Vec::new(), answer: HOST_HANDLED, on_call: None, inst: core::ptr::null_mut() });
        let host = Box::new(PpcHostCallbacks { size: core::mem::size_of::<PpcHostCallbacks>() as u32, flags: 0, ctx: (&mut *eng as *mut Eng).cast(), on_native, on_syscall });
        let cfg = PpcCreate { size: core::mem::size_of::<PpcCreate>() as u32, sub_arch: SUB_ARCH_G3, ram: eng.mem.flat_ram().unwrap(), host: &*host };
        let mut inst = core::ptr::null_mut();
        assert_eq!((vtable().create)(&cfg, &mut inst), 0);
        eng.inst = inst;
        Rig { eng, host, inst }
    }
    fn code(&mut self, at: u32, words: &[u32]) { for (i, w) in words.iter().enumerate() { self.eng.mem.write_u32(at + 4 * i as u32, *w).unwrap(); } }
    fn run(&mut self, ctx: &mut PpcContext, budget: u64) -> PpcExit {
        let mut x = PpcExit::default();
        assert_eq!((vtable().run)(self.inst, ctx, budget, &mut x), 0);
        x
    }
}
impl Drop for Rig { fn drop(&mut self) { let _ = &self.host; (vtable().destroy)(self.inst); } }

#[test]
fn the_table_header_names_the_flat_face_and_a_v2_shaped_table_is_refused() {
    // SAFETY: vtable() is a full static table.
    assert_eq!(unsafe { check_ppc(vtable(), 1) }, Ok(()));
    let mut t = PpcVTable { abi_family: 0, ..*vtable() };
    // SAFETY: a full table on the stack.
    assert_eq!(unsafe { check_ppc(&t, 1) }, Err(PpcOpenRefusal::WrongFamily { found: 0 }), "NEGATIVE CONTROL: not this family");
    t = PpcVTable { layout: LAYOUT + 1, ..*vtable() };
    // SAFETY: as above.
    assert!(matches!(unsafe { check_ppc(&t, 1) }, Err(PpcOpenRefusal::LayoutMismatch { .. })));
    // SAFETY: as above.
    assert!(matches!(unsafe { check_ppc(vtable(), 2) }, Err(PpcOpenRefusal::VersionTooOld { found: 1, required: 2 })));
}

#[test]
fn create_refuses_every_malformed_request_by_returning_minus_one() {
    let r = Rig::new();
    let ram = r.eng.mem.flat_ram().unwrap();
    let good = PpcCreate { size: core::mem::size_of::<PpcCreate>() as u32, sub_arch: SUB_ARCH_G3, ram, host: &*r.host };
    let mut out = core::ptr::null_mut();
    assert_eq!((vtable().create)(core::ptr::null(), &mut out), -1);
    for bad in [
        PpcCreate { size: 3, ..good }, PpcCreate { sub_arch: 4, ..good }, PpcCreate { ram: core::ptr::null(), ..good },
        PpcCreate { host: core::ptr::null(), ..good },
    ] {
        assert_eq!((vtable().create)(&bad, &mut out), -1);
        assert!(out.is_null(), "*out untouched on a refusal");
    }
    let bad_host = PpcHostCallbacks { size: 1, ..*r.host };
    assert_eq!((vtable().create)(&PpcCreate { host: &bad_host, ..good }, &mut out), -1);
    assert_eq!((vtable().create)(&good, &mut out), 0, "NEGATIVE CONTROL: the good request is accepted");
    (vtable().destroy)(out);
}


#[test]
fn a_run_executes_on_the_callers_context_and_two_contexts_keep_their_own_registers() {
    let mut r = Rig::new();
    r.code(CODE, &[LI_R3_5, ADDI_R3_1, B_SELF]);
    r.code(0x2000, &[ADDI_R3_1, ADDI_R3_1, B_SELF]);
    let mut a = PpcContext::new(CODE, 0x8_0000);
    let x = r.run(&mut a, 3);
    assert_eq!((x.code, x.retired, a.regs.gpr[3], a.regs.pc), (EXIT_BUDGET, 3, 6, CODE + 8));
    let mut b = PpcContext::new(0x2000, 0x9_0000);
    b.regs.gpr[3] = 100;
    b.regs.fpr[7] = 0x4009_21FB_5444_2D18;
    b.regs.lr = 0x1234;
    let _ = r.run(&mut b, 2);
    let _ = r.run(&mut a, 1);
    assert_eq!((a.regs.gpr[3], a.regs.pc, a.regs.fpr[7], a.regs.lr), (6, CODE + 8, 0, 0), "NEGATIVE CONTROL: A is untouched by B's run");
    assert_eq!((b.regs.gpr[3], b.regs.fpr[7], b.regs.lr, b.regs.gpr[1]), (102, 0x4009_21FB_5444_2D18, 0x1234, 0x9_0000));
}

fn native_rig(answer: i32, f: Option<fn(&mut Eng, &mut PpcContext, u32)>) -> (Rig, PpcContext) {
    let mut r = Rig::new();
    r.code(0x2000, &[B_SELF]);
    r.code(0x3000, &[B_SELF]);
    assert_eq!((vtable().bind_native)(r.inst, CODE), 0);
    r.eng.answer = answer;
    r.eng.on_call = f;
    let mut c = PpcContext::new(CODE, 0x8_0000);
    c.regs.lr = 0x2000;
    (r, c)
}

#[test]
fn every_native_outcome_continues_as_the_contract_says() {
    let (mut r, mut c) = native_rig(HOST_HANDLED, Some(|_, c, _| c.regs.gpr[3] = 42));
    let x = r.run(&mut c, 5);
    assert_eq!((x.code, c.regs.gpr[3], c.regs.pc, r.eng.seen.as_slice()), (EXIT_BUDGET, 42, 0x2000, &[CODE][..]), "HANDLED returns through LR");
    let (mut r, mut c) = native_rig(HOST_REDIRECTED, Some(|_, c, _| c.regs.pc = 0x3000));
    let _ = r.run(&mut c, 5);
    assert_eq!(c.regs.pc, 0x3000, "REDIRECTED continues at the published pc");
    let (mut r, mut c) = native_rig(HOST_HALT, None);
    assert_eq!(r.run(&mut c, 5).code, EXIT_HALT);
    assert_eq!(c.state, CTX_HALTED);
    assert_eq!((r.run(&mut c, 5).code, r.eng.seen.len()), (EXIT_HALT, 1), "a halted context runs nothing more");
    let (mut r, mut c) = native_rig(HOST_BLOCK, None);
    let x = r.run(&mut c, 5);
    assert_eq!((x.code, c.regs.pc), (EXIT_BLOCK, CODE), "BLOCK leaves pc ON the native");
    let _ = r.run(&mut c, 5);
    assert_eq!(r.eng.seen.len(), 2, "the next run calls it again");
    let (mut r, mut c) = native_rig(HOST_UNHANDLED, None);
    assert_eq!(r.run(&mut c, 5).decode_trap(), Some(Trap::NativeUnhandled { addr: CODE }));
    let (mut r, mut c) = native_rig(99, None);
    let x = r.run(&mut c, 5);
    assert_eq!((x.code, x.word, x.pc, c.state), (EXIT_HOST_PROTOCOL, 99, CODE, CTX_RUNNABLE), "an unknown code is a protocol fault, not a guess");
}

#[test]
fn a_bind_made_inside_a_callback_is_honoured_in_the_same_run() {
    let (mut r, mut c) = native_rig(HOST_HANDLED, Some(|e, _, addr| if addr == CODE { assert_eq!((vtable().bind_native)(e.inst, 0x2000), 0); }));
    let _ = r.run(&mut c, 3);
    assert_eq!(r.eng.seen[..2], [CODE, 0x2000], "the native bound during the first callback is reached and called (then again: it returns to LR = itself)");
}

#[test]
fn ram_grown_inside_a_callback_is_seen_by_the_rest_of_the_run() {
    let (mut r, mut c) = native_rig(HOST_REDIRECTED, Some(|e, c, _| {
        e.mem.try_grow_to(0x20_0000).unwrap();
        e.mem.write_u32(0x18_0000, LI_R3_5).unwrap();
        e.mem.write_u32(0x18_0004, B_SELF).unwrap();
        c.regs.pc = 0x18_0000;
    }));
    let x = r.run(&mut c, 4);
    assert_eq!((x.code, c.regs.gpr[3], c.regs.pc), (EXIT_BUDGET, 5, 0x18_0004), "code past the OLD end of RAM ran (the view was remade)");
}

/// A loop block at CODE: addi r3,r3,1 ; b CODE. The engine rewrites its first word between runs.
fn smc(mutant: bool) -> u32 {
    let mut r = Rig::new();
    r.code(CODE, &[ADDI_R3_1, 0x4BFF_FFFC]);
    if mutant { plant_stale_code_mutant(r.inst); }
    let mut c = PpcContext::new(CODE, 0x8_0000);
    let _ = r.run(&mut c, 10);
    assert_eq!(c.regs.gpr[3], 5);
    r.eng.mem.write_u32(CODE, 0x3863_0064).unwrap(); // addi r3,r3,100
    let _ = r.run(&mut c, 2);
    c.regs.gpr[3]
}

#[test]
fn an_engine_write_into_decoded_code_is_seen_through_the_shared_watch() {
    assert_eq!(smc(false), 105, "the rewritten instruction ran");
    assert_eq!(smc(true), 6, "NEGATIVE CONTROL: with the stale-code mutant the old decoded block runs");
}

#[test]
fn a_trap_crosses_as_its_record_and_the_profile_counts() {
    let mut r = Rig::new();
    r.code(CODE, &[LI_R3_5, 0]);
    let mut c = PpcContext::new(CODE, 0x8_0000);
    let x = r.run(&mut c, 10);
    assert!(matches!(x.decode_trap(), Some(Trap::Unimplemented { pc, word: 0, .. }) if pc == CODE + 4));
    let mut p = PpcProf::default();
    assert_eq!((vtable().read_prof)(r.inst, &mut p), 0);
    assert_eq!((p.insns, p.tier), (1, TIER_BLOCKS));
    assert_eq!((vtable().set_tier)(r.inst, 7), -1);
    assert_eq!((vtable().set_tier)(r.inst, TIER_INTERP), 0);
    let mut c = PpcContext::new(CODE, 0x8_0000);
    let _ = r.run(&mut c, 1);
    assert_eq!(c.regs.gpr[3], 5, "tier 0 gives the same registers");
}

#[test]
fn a_null_or_malformed_argument_is_refused_by_name() {
    let mut r = Rig::new();
    let mut x = PpcExit::default();
    let mut c = PpcContext::new(CODE, 0);
    assert_eq!((vtable().run)(core::ptr::null_mut(), &mut c, 1, &mut x), 0);
    assert_eq!((x.code, x.word), (EXIT_REFUSED, REFUSE_INSTANCE));
    c.size = 1;
    assert_eq!((r.run(&mut c, 1).code, r.run(&mut c, 1).word), (EXIT_REFUSED, REFUSE_CONTEXT));
    assert_eq!((vtable().run)(r.inst, &mut c, 1, core::ptr::null_mut()), -1);
    assert_eq!((vtable().bind_native)(core::ptr::null_mut(), 0), -1);
}
