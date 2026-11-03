//! Tier 2's dispatcher and NATIVE BLOCKS against tier 0: a guest loop calling natives that do everything a native may do -
//! write guest RAM and the loop's own code, write the time base, bind and unbind a native over guest code that was already
//! compiled, grow guest RAM, redirect the return, block the CPU, trap - ends in tier 0's state in every slicing, with the
//! natives really run from JIT code; and the planted dispatcher mutants go RED (a native counted twice, a dispatch table
//! kept across a flush, no resync after a native changed the natives set or RAM). Also how control leaves a SUPERBLOCK: a
//! side exit stores the registers dirty at its branch (mutant 16 RED); and a chained jump skips its target's page-generation
//! check only from the same page at the same generation (mutant 17 RED). Host only (HostCodeMem).
#![cfg(all(target_arch = "x86_64", target_os = "linux"))]
use heirloom_cpu_ppc::{CodeMem, Cpu, Exec, Host, HostCodeMem, RunStats, Tier};
use heirloom_mem_flat32::GuestMem;

const CODE: u32 = 0x1000;
const STACK: u32 = 0x8000;
const BUF: u32 = 0x4000;
const EXIT: u32 = 0xF000;
const N_ADD: u32 = 0xF100;
const N_TB: u32 = 0xF200;
const N_BIND: u32 = 0xF300;
/// Guest code (`addi r5,r5,100; blr`) that N_BIND binds a native over and unbinds again.
const TOGGLE: u32 = 0xF400;
const N_GROW: u32 = 0xF500;
const N_REDIR: u32 = 0xF600;
const N_BLOCK: u32 = 0xF700;
const N_TRAP: u32 = 0xF800;
/// Rewrites instruction 8 of the same-page program (its block B) when r31 is 50.
const N_PATCH: u32 = 0xF900;
const RAM: usize = 1 << 20;
/// The loop's first body instruction, which N_ADD rewrites now and then (a native writing code that JIT code runs).
const PATCH: u32 = CODE + 4;

struct M { mem: GuestMem, calls: Vec<u32>, adds: u32 }
impl Host for M {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }
    fn native(&mut self, cpu: &mut Cpu, addr: u32) -> bool {
        self.calls.push(addr);
        match addr {
            EXIT => cpu.halt(),
            N_ADD => {
                self.adds += 1;
                self.mem.write_u32(BUF + 4 * (self.adds % 64), cpu.gpr[3] ^ cpu.gpr[5]).unwrap();
                if self.adds % 37 == 0 {
                    let w = self.mem.read_u32(PATCH).unwrap();
                    self.mem.write_u32(PATCH, w ^ 0x6).unwrap(); // addi r3,r3,3 <-> addi r3,r3,5
                }
            }
            N_TB => cpu.tb = cpu.tb.wrapping_mul(3).wrapping_add(7),
            // called every trip (so it runs from JIT code), toggles every 16th: the natives set changes under a native block
            N_BIND => if cpu.gpr[31] % 16 == 0 && !cpu.unbind_native(TOGGLE) { cpu.bind_native(TOGGLE) },
            TOGGLE => cpu.gpr[5] = cpu.gpr[5].wrapping_add(1000),
            N_GROW => if self.mem.len() < 4 * RAM { let n = self.mem.len() * 2; self.mem.grow_to(n) },
            N_REDIR => cpu.set_pc(cpu.lr.wrapping_add(4)),
            N_BLOCK => cpu.block(),
            N_TRAP => return cpu.gpr[31] != 75,
            N_PATCH => if cpu.gpr[31] == 50 { self.mem.write_u32(CODE + 4 * 8, d(14, 6, 6, 2)).unwrap() },
            _ => {}
        }
        true
    }
    fn syscall(&mut self, _: &mut Cpu) -> bool { true }
}

fn d(op: u32, rt: u32, ra: u32, imm: u32) -> u32 { op << 26 | rt << 21 | ra << 16 | (imm & 0xFFFF) }
fn bc(bo: u32, bi: u32, rel: i32) -> u32 { 16 << 26 | bo << 21 | bi << 16 | (rel as u32 & 0xFFFC) }
fn mtspr(spr: u32, rs: u32) -> u32 { 31 << 26 | rs << 21 | (spr & 31) << 16 | (spr >> 5) << 11 | 467 << 1 }
const BCTRL: u32 = 0x4E80_0421;
const BLR: u32 = 0x4E80_0020;

/// `call r` = mtctr r; bctrl. `every` = only when r31 % every == 0 (andi. r8,r31,every-1; bne past the call).
fn call(p: &mut Vec<u32>, r: u32, every: u32) {
    if every > 1 { p.push(d(28, 31, 8, every - 1)); p.push(bc(4, 2, 12)); }
    p.push(mtspr(9, r));
    p.push(BCTRL);
}

/// 200 trips of a loop calling the natives (the trap variant calls N_TRAP every trip, which traps at trip 75).
fn program(trap: bool) -> Vec<u32> {
    let mut p = vec![d(14, 31, 0, 200)];
    let top = p.len();
    p.push(d(14, 3, 3, 3)); // PATCH: addi r3,r3,3
    call(&mut p, 6, 1); // N_ADD
    call(&mut p, 7, 1); // N_TB
    call(&mut p, 9, 1); // N_BIND
    call(&mut p, 10, 1); // TOGGLE: guest code or the native
    call(&mut p, 11, 64); // N_GROW
    call(&mut p, 12, 1); // N_REDIR: returns past the next instruction
    p.push(d(14, 5, 5, 7)); // skipped by N_REDIR
    call(&mut p, 13, 32); // N_BLOCK
    if trap { call(&mut p, 14, 1); }
    p.push(d(13, 31, 31, 0xFFFF)); // addic. r31,r31,-1
    let back = -(4 * (p.len() - top) as i32);
    p.push(bc(4, 2, back)); // bne top
    p.push(d(15, 24, 0, EXIT >> 16));
    p.push(d(24, 24, 24, EXIT & 0xFFFF));
    p.push(mtspr(8, 24));
    p.push(BLR);
    p
}

fn load(prog: &[u32]) -> (Cpu, M) {
    let mut m = M { mem: GuestMem::new(RAM), calls: vec![], adds: 0 };
    for (i, w) in prog.iter().enumerate() { m.mem.write_u32(CODE + 4 * i as u32, *w).unwrap(); }
    m.mem.write_u32(TOGGLE, d(14, 5, 5, 100)).unwrap();
    m.mem.write_u32(TOGGLE + 4, BLR).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    for a in [EXIT, N_ADD, N_TB, N_BIND, N_GROW, N_REDIR, N_BLOCK, N_TRAP, N_PATCH] { c.bind_native(a); }
    for (r, a) in [(6, N_ADD), (7, N_TB), (9, N_BIND), (10, TOGGLE), (11, N_GROW), (12, N_REDIR), (13, N_BLOCK), (14, N_TRAP), (15, N_PATCH)] { c.gpr[r] = a; }
    (c, m)
}

#[derive(Debug, PartialEq)]
struct End { gpr: [u32; 32], cr: u32, lr: u32, ctr: u32, xer: u32, pc: u32, tb: u64, ram: Vec<u8>, last: RunStats, total: u64, calls: Vec<u32> }

fn run(tier: Tier, mutant: u8, prog: &[u32], slice: u64) -> (End, Exec) { run_with(tier, mutant, prog, slice, Box::new(HostCodeMem::new())) }

fn run_with(tier: Tier, mutant: u8, prog: &[u32], slice: u64, mem: Box<dyn CodeMem>) -> (End, Exec) {
    let (mut c, mut m) = load(prog);
    let mut e = Exec::new(tier);
    e.set_code_mem(mem);
    e.set_jit_publish_after(0);
    if mutant != 0 { e.plant_jit_mutant(mutant); }
    let mut total = 0;
    let last = loop {
        let st = e.resume(&mut m, &mut c, slice);
        total += st.insns;
        if st.halted || st.trap.is_some() || total >= 1_000_000 || e.divergence().is_some() { break st; }
    };
    let ram = m.mem.bytes(m.mem.span(0, m.mem.len() as u32).unwrap()).unwrap().to_vec();
    (End { gpr: c.gpr, cr: c.cr, lr: c.lr, ctr: c.ctr, xer: c.xer, pc: c.pc, tb: c.tb, ram, last: RunStats { insns: 0, ..last }, total, calls: m.calls }, e)
}

#[test]
fn natives_run_from_jit_code_and_end_in_tier_0s_state_in_every_slicing() {
    for trap in [false, true] {
        let prog = program(trap);
        let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
        assert!(if trap { want.last.trap.is_some() } else { want.last.halted }, "the reference run ends as designed: {:?}", want.last);
        let mut from_jit = 0;
        for slice in [1, 7, 37, 1_000, 1_000_000] {
            let (got, e) = run(Tier::Jit, 0, &prog, slice);
            assert!(got == want, "trap={trap} slice {slice}: gpr {:x?}/{:x?} tb {}/{} pc {:#x}/{:#x} last {:?}/{:?} total {}/{} ram equal {} calls equal {} {:?}",
                got.gpr, want.gpr, got.tb, want.tb, got.pc, want.pc, got.last, want.last, got.total, want.total, got.ram == want.ram, got.calls == want.calls, e.jit_stats());
            from_jit += e.jit_stats().natives;
            assert!(e.jit_stats().flushes > 0, "binding a native over compiled code dropped the code");
        }
        // lockstep never runs a native in JIT code (it cannot be rolled back), but must agree too
        let (got, e) = run(Tier::JitLockstep, 0, &prog, 50);
        assert!(e.divergence().is_none() && got == want, "lockstep: {:?}", e.divergence());
        assert_eq!(e.jit_stats().natives, 0, "lockstep ran a native in JIT code");
        assert!(from_jit > 1000, "the natives really ran from JIT code: {from_jit}");
    }
}

#[test]
fn negative_control_a_native_counted_twice_or_no_resync_goes_red() {
    let prog = program(false);
    let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
    let (got, _) = run(Tier::Jit, 13, &prog, 1_000);
    assert!(got != want, "mutant 13 (a native block retires two instructions) stayed GREEN");
    // 15: JIT code goes on after a native bound a native over compiled guest code (or grew RAM)
    let (got, _) = run(Tier::Jit, 15, &prog, 1_000_000);
    assert!(got != want, "mutant 15 (no resync after a native changed the natives set / RAM) stayed GREEN");
}

/// Negative control for the dispatch table, in a child process (the mutant leaves table entries pointing into code that a
/// flush gave back and rewrote: the dispatcher may jump into anything). Green without the mutant, RED with it.
#[test]
fn negative_control_a_dispatch_table_kept_across_a_flush_goes_red() {
    let child = |mutant: u8| std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "table_child", "--ignored", "--test-threads=1"]).env("TIER2_TABLE_MUTANT", mutant.to_string())
        .output().unwrap();
    let ok = child(0);
    assert!(ok.status.success(), "the child is green without the mutant: {}", String::from_utf8_lossy(&ok.stdout));
    assert!(!child(14).status.success(), "mutant 14 (dispatch table kept across a flush) stayed GREEN");
}

#[test]
#[ignore = "run by negative_control_a_dispatch_table_kept_across_a_flush_goes_red in a child process"]
fn table_child() {
    let mutant: u8 = std::env::var("TIER2_TABLE_MUTANT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let prog = program(false);
    let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
    for slice in [37, 1_000] {
        let (got, _) = run(Tier::Jit, mutant, &prog, slice);
        assert!(got == want, "slice {slice}");
    }
}

/// SUPERBLOCKS: a loop whose block runs on through a conditional branch's not-taken path. The branch is taken every 4th
/// trip with r3 written just before it (held dirty in a host register by the register cache), and the taken target reads
/// r3 from the Cpu: the side exit must store it. Tier 0's state in every slicing and under lockstep, superblocks really
/// compiled; mutant 16 (side exits store nothing) RED.
fn side_exit_program() -> Vec<u32> {
    let mut p = vec![d(14, 31, 0, 120)]; // li r31,120
    let top = p.len();
    p.extend([d(14, 3, 3, 1), d(14, 3, 3, 1), d(14, 3, 3, 1)]); // r3 += 3 (dirty)
    p.push(d(28, 31, 8, 3)); // andi. r8,r31,3
    p.push(bc(12, 2, 4 * 5)); // beq side (5 instructions on)
    p.push(d(14, 4, 4, 1)); // not taken: addi r4,r4,1
    p.push(d(13, 31, 31, 0xFFFF)); // addic. r31,r31,-1
    p.push(bc(4, 2, -(4 * (p.len() - top) as i32))); // bne top
    p.push(18 << 26 | 4 * 5); // b out
    // side:
    p.push(d(14, 5, 3, 0)); // addi r5,r3,0: r3 as the side exit left it
    p.push(d(13, 31, 31, 0xFFFF));
    p.push(bc(4, 2, -(4 * (p.len() - top) as i32)));
    // out:
    p.push(d(15, 24, 0, EXIT >> 16));
    p.push(d(24, 24, 24, EXIT & 0xFFFF));
    p.push(mtspr(8, 24));
    p.push(BLR);
    p
}

#[test]
fn a_superblock_side_exit_stores_the_registers_dirty_at_its_branch() {
    let prog = side_exit_program();
    let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
    assert!(want.last.halted && want.gpr[5] != 0, "the reference run takes the side exit: {:?}", want.last);
    for slice in [1, 5, 37, 1_000_000] {
        let (got, e) = run(Tier::Jit, 0, &prog, slice);
        assert!(got == want, "slice {slice}: r3 {:#x}/{:#x} r5 {:#x}/{:#x} {:?}", got.gpr[3], want.gpr[3], got.gpr[5], want.gpr[5], e.jit_stats());
        if slice > 5 { assert!(e.jit_stats().superblocks > 0, "slice {slice}: a superblock was compiled: {:?}", e.jit_stats()); }
    }
    let (got, e) = run(Tier::JitLockstep, 0, &prog, 50);
    assert!(e.divergence().is_none() && got == want, "lockstep: {:?}", e.divergence());
    let (got, _) = run(Tier::Jit, 16, &prog, 1_000_000);
    assert!(got != want, "mutant 16 (a side exit stores no dirty register) stayed GREEN");
}

/// CHAINED ENTRIES PAST THE GENERATION CHECK: a chained jump from a block on the same code page compiled at the same
/// generation lands past the target's page-generation check. Block A (the loop top) jumps to block B on its page every 4th
/// trip; a native rewrites B's instruction at trip 50 (on A's other path); A is recompiled at the new generation on the
/// next trip, which does NOT reach B, so B is still the stale code of the old generation when A's next jump to it comes: it
/// must take B's full entry (and bail), never the inner one. Tier 0's state; mutant 17 (same page, generation ignored) RED.
fn same_page_program() -> Vec<u32> {
    let b = |from: u32, to: u32| 18 << 26 | (4 * (to as i32 - from as i32)) as u32 & 0x03FF_FFFC;
    vec![
        d(14, 31, 0, 101),          // 0: li r31,101
        d(13, 31, 31, 0xFFFF),      // 1 top (A): addic. r31,r31,-1
        bc(12, 2, 4 * 8),           // 2: beq out
        d(28, 31, 8, 3),            // 3: andi. r8,r31,3
        bc(12, 2, 4 * 4),           // 4: beq B (every 4th trip)
        mtspr(9, 15), BCTRL,        // 5, 6: N_PATCH (rewrites instruction 8 at r31 = 50)
        b(7, 1),                    // 7: b top
        d(14, 6, 6, 1),             // 8 (B): addi r6,r6,1 (-> addi r6,r6,2)
        b(9, 1),                    // 9: b top
        d(15, 24, 0, EXIT >> 16),   // 10 out
        d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24), BLR,
    ]
}

#[test]
fn a_chained_jump_skips_the_generation_check_only_into_a_block_of_its_own_generation() {
    let prog = same_page_program();
    let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
    assert!(want.last.halted && want.gpr[6] > 25, "the reference run takes B with both instructions: r6 = {}", want.gpr[6]);
    for slice in [1, 7, 37, 1_000_000] {
        let (got, e) = run(Tier::Jit, 0, &prog, slice);
        assert!(got == want, "slice {slice}: r6 {}/{} {:?}", got.gpr[6], want.gpr[6], e.jit_stats());
    }
    let (got, e) = run(Tier::JitLockstep, 0, &prog, 50);
    assert!(e.divergence().is_none() && got == want, "lockstep: {:?}", e.divergence());
    let (got, _) = run(Tier::Jit, 17, &prog, 1_000_000);
    assert!(got != want, "mutant 17 (a same-page chained jump skips the check into a block of an older generation) stayed GREEN");
}

/// A provider that refuses publish calls: `refuse(n)` answers for the n-th call (0 = the stub page at creation).
struct Refusing { inner: HostCodeMem, calls: u32, refuse: fn(u32) -> bool }
impl CodeMem for Refusing {
    fn reserve(&mut self, len: usize) -> Option<*mut u8> { self.inner.reserve(len) }
    fn publish(&mut self, ptr: *mut u8, len: usize) -> bool {
        let n = self.calls;
        self.calls += 1;
        // the pages really become RX either way (as the kernel's NOT-GLOBAL leaves them locally): only the answer is no
        self.inner.publish(ptr, len) && !(self.refuse)(n)
    }
    fn reset(&mut self, ptr: *mut u8, len: usize) -> bool { self.inner.reset(ptr, len) }
}
fn refusing(refuse: fn(u32) -> bool) -> Box<dyn CodeMem> { Box::new(Refusing { inner: HostCodeMem::new(), calls: 0, refuse }) }

/// EXECUTABLE MEMORY THAT SAYS NO (on CoralOS: a TLB shootdown not acknowledged in time, EXEC_CODE_PROTECT's NOT-GLOBAL). A
/// refused batch is dropped, never run and never answered with a whole-arena withdraw; a provider that keeps refusing
/// stops tier 2 by name. Tier 0's state in every case, natives-set changes included (they drop code without a withdraw);
/// mutant 18 (a refusal resets the whole arena, the loop the Z83 showed) RED.
#[test]
fn a_refused_publish_drops_its_batch_and_never_withdraws_the_arena() {
    for prog in [program(false), same_page_program()] {
        let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
        for slice in [37, 1_000] {
            let (got, e) = run_with(Tier::Jit, 0, &prog, slice, refusing(|n| n % 3 == 2));
            let s = e.jit_stats();
            assert!(got == want, "every 3rd publish refused, slice {slice}: {s:?}");
            assert!(s.codemem_failures > 0 && s.withdraw_pages == 0 && !s.refused && s.insns > 0, "refusals absorbed, nothing withdrawn: {s:?}");
            let (got, e) = run_with(Tier::Jit, 0, &prog, slice, refusing(|n| n > 0));
            let s = e.jit_stats();
            assert!(got == want, "every publish refused, slice {slice}: {s:?}");
            assert!(s.refused && s.codemem_failures == 8 && s.withdraw_pages == 0 && s.insns == 0, "tier 2 stopped by name after 8: {s:?}");
        }
        let (_, e) = run_with(Tier::Jit, 18, &prog, 37, refusing(|n| n % 3 == 2));
        assert!(e.jit_stats().withdraw_pages > 0, "mutant 18 (a refusal withdraws the whole arena) stayed GREEN: {:?}", e.jit_stats());
    }
}

/// THE STORE MAP (jit_smap) mirrors the code watch AFTER it was built: the JIT's loop runs (the map built while page B is
/// unused), then the loop starts calling a function on page B (B decoded and marked: its map byte must drop to 0), then the
/// loop's own INLINE store writes a new instruction into B with no icbi: it must take the slow path, bump B's generation, and
/// the next call must run the new instruction. Tier 0's state; mutant 19 (the map stops mirroring) RED, mutant 3 (no watch
/// test at all) RED.
fn store_into_fresh_code_program() -> Vec<u32> {
    const B: u32 = 0x3000; // another page than the loop's
    let b = |from: u32, to: u32| 18 << 26 | (4 * (to as i32 - from as i32)) as u32 & 0x03FF_FFFC;
    vec![
        d(14, 10, 0, B),                            // 0: r10 = B
        d(15, 11, 0, 0x38A5), d(24, 11, 11, 1),     // 1, 2: r11 = addi r5,r5,1
        d(36, 11, 10, 0),                           // 3: B[0] = addi r5,r5,1
        d(15, 12, 0, 0x4E80), d(24, 12, 12, 0x20),  // 4, 5: r12 = blr
        d(36, 12, 10, 4),                           // 6: B[1] = blr
        d(24, 11, 11, 3),                           // 7: r11 = addi r5,r5,3 (ori: the new instruction)
        d(14, 31, 0, 200),                          // 8: li r31,200
        d(13, 31, 31, 0xFFFF),                      // 9 top: addic. r31,r31,-1
        bc(12, 2, 4 * 10),                          // 10: beq out (20)
        d(11, 0, 31, 140),                          // 11: cmpwi r31,140
        bc(12, 1, 4 * 7),                           // 12: bgt skip (19): B is not called for the first 59 trips
        mtspr(9, 10), BCTRL,                        // 13, 14: call B
        d(11, 0, 31, 100),                          // 15: cmpwi r31,100
        bc(4, 2, 4 * 2),                            // 16: bne skip (18 -> 19)
        d(36, 11, 10, 0),                           // 17: stw r11,0(r10): B[0] = the new instruction, no icbi
        b(18, 9),                                   // 18: skip: b top
        b(19, 9),                                   // 19: b top
        d(15, 24, 0, EXIT >> 16),                   // 20 out
        d(24, 24, 24, EXIT & 0xFFFF), mtspr(8, 24), BLR,
    ]
}

#[test]
fn an_inline_store_into_code_decoded_after_the_store_map_was_built_takes_the_slow_path() {
    let prog = store_into_fresh_code_program();
    let (want, _) = run(Tier::Interp, 0, &prog, 1_000_000);
    // B was called 140 times: 41 before the store (+1 each) and 99 after it (+3 each)
    assert!(want.last.halted && want.gpr[5] == 41 + 99 * 3, "the reference run: r5 = {}", want.gpr[5]);
    for slice in [1, 7, 37, 1_000_000] {
        let (got, e) = run(Tier::Jit, 0, &prog, slice);
        assert!(got == want, "slice {slice}: r5 {}/{} {:?}", got.gpr[5], want.gpr[5], e.jit_stats());
    }
    let (got, e) = run(Tier::JitLockstep, 0, &prog, 50);
    assert!(e.divergence().is_none() && got == want, "lockstep: {:?}", e.divergence());
    for mutant in [19, 3] {
        let (got, _) = run(Tier::Jit, mutant, &prog, 1_000_000);
        assert!(got != want, "mutant {mutant} stayed GREEN: r5 {}", got.gpr[5]);
    }
}
