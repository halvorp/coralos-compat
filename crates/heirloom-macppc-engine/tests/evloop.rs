use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::gestalt_ppc;
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;

type M = MacPpc<RecordingGuestUi>;
const TV: u32 = 0x0003_0000;
const CODE: u32 = 0x0003_1000;
const TOC: u32 = 0x0003_8000;
const SP: u32 = 0x0004_0000;
const OUT: u32 = 0x0009_0000;
const MAIN_LOOP: u32 = 0x0051_0001;
const PARAM_ERR: u32 = (-50i32) as u32;

fn setup() -> (M, Cpu) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_u32(TV, CODE).unwrap();
    m.mac.mem.write_u32(TV + 4, TOC).unwrap();
    m.mac.ev.ticks_total = 0;
    m.mac.ev.ticks = 0;
    (m, Cpu::new(0x5000, SP))
}

fn bind_and_stub(m: &mut M, name: &str) -> u32 {
    m.bind(name);
    m.glue.intern(&mut m.mac, "CFM", name).unwrap();
    let tv = m.glue.tvector_of("CFM", name).unwrap();
    m.mac.mem.read_u32(tv).unwrap()
}

fn call(m: &mut M, cpu: &mut Cpu, stub: u32) {
    cpu.pc = stub;
    m.native(cpu, stub);
    assert!(m.fault().is_none(), "{:?}", m.fault());
}

fn advance(m: &mut M, ticks: u32) {
    m.mac.ev.ticks_total += u64::from(ticks);
    m.mac.ev.ticks += ticks;
}

fn install(m: &mut M, cpu: &mut Cpu, delay: f64, interval: f64) -> u32 {
    let stub = bind_and_stub(m, "InstallEventLoopTimer");
    cpu.gpr[3] = MAIN_LOOP;
    cpu.fpr[1] = delay.to_bits();
    cpu.fpr[2] = interval.to_bits();
    // Poison the shadow slots: proc/data/outTimer must come from r8/r9/r10.
    cpu.gpr[4] = 0;
    cpu.gpr[5] = 0;
    cpu.gpr[6] = 0;
    cpu.gpr[7] = 0;
    cpu.gpr[8] = TV;
    cpu.gpr[9] = 0x1234;
    cpu.gpr[10] = OUT;
    call(m, cpu, stub);
    assert_eq!(cpu.gpr[3], 0);
    let id = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(id, 0);
    assert_eq!(id & 1, 1);
    id
}

fn finish_callback(m: &mut M, cpu: &mut Cpu, interrupted: u32) {
    let ret = cpu.lr;
    call(m, cpu, ret);
    assert_eq!(m.callback_depth(), 0);
    assert_eq!(cpu.pc, interrupted);
}

#[test]
fn get_main_and_current_event_loop_return_same_ref() {
    let (mut m, mut cpu) = setup();
    let stub = bind_and_stub(&mut m, "GetMainEventLoop");
    call(&mut m, &mut cpu, stub);
    let main_ref = cpu.gpr[3];
    assert_eq!(main_ref, MAIN_LOOP);

    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
    call(&mut m, &mut cpu, stub);
    assert_eq!(cpu.gpr[3], main_ref);
}

#[test]
fn install_timer_fires_at_safepoint_and_repeats() {
    let (mut m, mut cpu) = setup();
    let timer_id = install(&mut m, &mut cpu, 0.5, 0.25);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");

    advance(&mut m, 24); // 0.4s: not due
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0);
    assert_ne!(cpu.pc, CODE);
    assert_eq!(cpu.gpr[3], MAIN_LOOP);

    advance(&mut m, 12); // 0.6s: first fire
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.pc, CODE);
    assert_eq!(cpu.gpr[2], TOC);
    assert_eq!(cpu.gpr[3], timer_id);
    assert_eq!(cpu.gpr[4], 0x1234);

    finish_callback(&mut m, &mut cpu, stub);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0, "return must not immediately fire again");

    advance(&mut m, 15); // 0.85s: second fire
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.pc, CODE);
    assert_eq!(cpu.gpr[3], timer_id);
}

#[test]
fn missed_periods_are_coalesced_and_phase_is_preserved() {
    let (mut m, mut cpu) = setup();
    let id = install(&mut m, &mut cpu, 0.5, 0.25);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");

    advance(&mut m, 66); // 1.1s: 0.5, 0.75 and 1.0 have elapsed
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.gpr[3], id);
    finish_callback(&mut m, &mut cpu, stub);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0);

    advance(&mut m, 9); // 1.25s, not 1.35s
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.gpr[3], id);
}

#[test]
fn callbacks_do_not_nest_at_glue_safepoints() {
    let (mut m, mut cpu) = setup();
    let first = install(&mut m, &mut cpu, 0.5, 0.0);
    let second = install(&mut m, &mut cpu, 0.5, 0.0);
    assert_ne!(first, second);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");

    advance(&mut m, 30);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    let ret = cpu.lr;

    call(&mut m, &mut cpu, stub);
    assert_eq!(cpu.gpr[3], MAIN_LOOP);
    assert_eq!(m.callback_depth(), 1);
    call(&mut m, &mut cpu, ret);
    assert_eq!(m.callback_depth(), 0);

    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.gpr[3], second);
}

#[test]
fn zero_and_forever_intervals_fire_once_and_can_be_rearmed() {
    for interval in [0.0, -1.0] {
        let (mut m, mut cpu) = setup();
        let id = install(&mut m, &mut cpu, 0.5, interval);
        let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
        advance(&mut m, 30);
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 1);
        finish_callback(&mut m, &mut cpu, stub);

        advance(&mut m, 600);
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 0);

        let set = bind_and_stub(&mut m, "SetEventLoopTimerNextFireTime");
        cpu.gpr[3] = id;
        cpu.fpr[1] = 0.25_f64.to_bits();
        call(&mut m, &mut cpu, set);
        assert_eq!(cpu.gpr[3], 0);
        advance(&mut m, 15);
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 1);
        assert_eq!(cpu.gpr[3], id);
        finish_callback(&mut m, &mut cpu, stub);

        let remove = bind_and_stub(&mut m, "RemoveEventLoopTimer");
        cpu.gpr[3] = id;
        call(&mut m, &mut cpu, remove);
        assert_eq!(cpu.gpr[3], 0);
    }
}

#[test]
fn negative_next_fire_disarms_until_explicitly_rearmed() {
    let (mut m, mut cpu) = setup();
    let id = install(&mut m, &mut cpu, 0.5, 0.25);
    let set = bind_and_stub(&mut m, "SetEventLoopTimerNextFireTime");
    cpu.gpr[3] = id;
    cpu.fpr[1] = (-1.0_f64).to_bits();
    call(&mut m, &mut cpu, set);
    assert_eq!(cpu.gpr[3], 0);

    advance(&mut m, 600);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0);

    cpu.gpr[3] = id;
    cpu.fpr[1] = 0.25_f64.to_bits();
    call(&mut m, &mut cpu, set);
    assert_eq!(cpu.gpr[3], 0);
    advance(&mut m, 15);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 1);
    assert_eq!(cpu.gpr[3], id);
}

#[test]
fn removed_timers_do_not_fire_and_unknown_refs_return_param_err() {
    let (mut m, mut cpu) = setup();
    let id = install(&mut m, &mut cpu, 0.5, 0.25);
    let remove = bind_and_stub(&mut m, "RemoveEventLoopTimer");
    cpu.gpr[3] = id;
    call(&mut m, &mut cpu, remove);
    assert_eq!(cpu.gpr[3], 0);
    cpu.gpr[3] = id;
    call(&mut m, &mut cpu, remove);
    assert_eq!(cpu.gpr[3], PARAM_ERR);

    let set = bind_and_stub(&mut m, "SetEventLoopTimerNextFireTime");
    cpu.gpr[3] = id;
    cpu.fpr[1] = 0.0_f64.to_bits();
    call(&mut m, &mut cpu, set);
    assert_eq!(cpu.gpr[3], PARAM_ERR);

    advance(&mut m, 600);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0);
}

#[test]
fn bad_loop_and_null_proc_return_param_err_without_faulting() {
    for (loop_ref, proc) in [(0, TV), (MAIN_LOOP, 0)] {
        let (mut m, mut cpu) = setup();
        let stub = bind_and_stub(&mut m, "InstallEventLoopTimer");
        m.mac.mem.write_u32(OUT, 0xfeed_beef).unwrap();
        cpu.gpr[3] = loop_ref;
        cpu.fpr[1] = 0.5_f64.to_bits();
        cpu.fpr[2] = 0.25_f64.to_bits();
        cpu.gpr[8] = proc;
        cpu.gpr[9] = 0;
        cpu.gpr[10] = OUT;
        call(&mut m, &mut cpu, stub);
        assert_eq!(cpu.gpr[3], PARAM_ERR);
        assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0xfeed_beef);

        advance(&mut m, 60);
        let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 0);
    }
}

#[test]
fn bad_output_pointer_is_a_named_memory_fault() {
    let (mut m, mut cpu) = setup();
    let stub = bind_and_stub(&mut m, "InstallEventLoopTimer");
    cpu.gpr[3] = MAIN_LOOP;
    cpu.fpr[1] = 0.5_f64.to_bits();
    cpu.fpr[2] = 0.25_f64.to_bits();
    cpu.gpr[8] = TV;
    cpu.gpr[9] = 0;
    cpu.gpr[10] = 0xffff_fffc;
    cpu.pc = stub;
    m.native(&mut cpu, stub);
    assert!(m.fault().unwrap().contains("InstallEventLoopTimer"));
}

#[test]
fn get_event_time_uses_the_clock_and_logs_unknown_refs() {
    let (mut m, mut cpu) = setup();
    let stub = bind_and_stub(&mut m, "GetEventTime");
    advance(&mut m, 90);
    // NAMED GAP: a known EventRef's time (crate::carbon::CarbonState::is_event) is not reached from this host test: the Carbon
    // natives bind under CarbonLib only; the unknown-ref refusals below are the tested half.

    for invalid in [0, MAIN_LOOP, 0x0052_0003] {
        cpu.gpr[3] = invalid;
        cpu.fpr[1] = 99.0_f64.to_bits();
        call(&mut m, &mut cpu, stub);
        assert_eq!(f64::from_bits(cpu.fpr[1]), 0.0);
        assert!(m.mac.log.last().unwrap().contains("GetEventTime"));
    }
}

#[test]
fn gestalt_reports_appearance_and_the_emulated_cpu_model() {
    assert_eq!(gestalt_ppc::answer(*b"apvr"), Some(0x0110));
    assert_eq!(gestalt_ppc::EMULATED_CPU, 0x108);
    assert_eq!(gestalt_ppc::EMULATED_CPU_HZ, 400_000_000);
    assert_eq!(gestalt_ppc::answer(*b"cput"), Some(gestalt_ppc::EMULATED_CPU));
    assert_eq!(gestalt_ppc::answer(*b"cpuf"), Some(gestalt_ppc::EMULATED_CPU));
    assert_eq!(gestalt_ppc::answer(*b"pclk"), Some(gestalt_ppc::EMULATED_CPU_HZ));
    assert_eq!(gestalt_ppc::answer(*b"????"), None);
    for &(selector, reason) in gestalt_ppc::UNANSWERED {
        assert_eq!(gestalt_ppc::answer(*selector), None);
        assert_eq!(gestalt_ppc::unanswered(*selector), Some(reason));
    }
}
/// The earliest DUE timer fires first whatever the install order, and equal due times fire in timer-ref order (the walk's order).
#[test]
fn due_timers_fire_earliest_first_and_ties_in_ref_order() {
    let (mut m, mut cpu) = setup();
    let late = install(&mut m, &mut cpu, 0.75, 0.0);
    let early = install(&mut m, &mut cpu, 0.25, 0.0);
    let tie = install(&mut m, &mut cpu, 0.75, 0.0);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
    advance(&mut m, 60); // 1 s: all three are due
    let mut order = Vec::new();
    for _ in 0..3 {
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 1);
        order.push(cpu.gpr[3]);
        finish_callback(&mut m, &mut cpu, stub);
    }
    assert_eq!(order, vec![early, late.min(tie), late.max(tie)]);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0, "one-shots fire once");
}

/// A timer installed, re-timed EARLIER or removed while a timer's procedure runs (a callout) is seen by the very next safepoint
/// after that procedure returns - the safepoint's "nothing is due" answer never comes from a stale earliest-due time.
#[test]
fn a_timer_installed_or_moved_earlier_during_a_callout_fires_at_the_next_safepoint() {
    let (mut m, mut cpu) = setup();
    let slow = install(&mut m, &mut cpu, 10.0, 0.0);
    let first = install(&mut m, &mut cpu, 0.1, 0.0);
    let stub = bind_and_stub(&mut m, "GetCurrentEventLoop");
    advance(&mut m, 12); // 0.2 s
    call(&mut m, &mut cpu, stub);
    assert_eq!((m.callback_depth(), cpu.gpr[3]), (1, first));
    // inside the callout: install a timer due at once, and move the 10 s one to "now"
    let fresh = install(&mut m, &mut cpu, 0.0, 0.0);
    let set = bind_and_stub(&mut m, "SetEventLoopTimerNextFireTime");
    cpu.gpr[3] = slow;
    cpu.fpr[1] = 0.0_f64.to_bits();
    call(&mut m, &mut cpu, set);
    assert_eq!(cpu.gpr[3], 0);
    assert_eq!(m.callback_depth(), 1, "no nesting while the callout runs");
    let ret = cpu.lr;
    call(&mut m, &mut cpu, ret);
    assert_eq!(m.callback_depth(), 0);
    let mut fired = Vec::new();
    for _ in 0..2 {
        call(&mut m, &mut cpu, stub);
        assert_eq!(m.callback_depth(), 1, "fired so far: {fired:x?}");
        fired.push(cpu.gpr[3]);
        finish_callback(&mut m, &mut cpu, stub);
    }
    fired.sort_unstable();
    let mut want = vec![slow, fresh];
    want.sort_unstable();
    assert_eq!(fired, want);
    // removing the only armed timer leaves nothing due; installing a later one is not due early
    let late = install(&mut m, &mut cpu, 0.5, 0.0);
    let remove = bind_and_stub(&mut m, "RemoveEventLoopTimer");
    cpu.gpr[3] = late;
    call(&mut m, &mut cpu, remove);
    advance(&mut m, 60);
    call(&mut m, &mut cpu, stub);
    assert_eq!(m.callback_depth(), 0, "a removed timer never fires");
}
