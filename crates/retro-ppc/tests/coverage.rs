//! Instruction coverage against the PEM's user-mode instruction list (32-bit implementations: 601/603/604/G3,
//! without AltiVec). Each entry is executed once; "covered" means anything other than `Unimplemented` (an
//! InvalidForm or a Privileged refusal is a recognised instruction). The expected-missing list is exact, so a
//! newly implemented instruction or a regression both fail this test until the list is updated.
mod common;
use common::*;
use retro_ppc::{resume, Cpu, Trap};

const fn d(op: u32) -> u32 { op << 26 | 3 << 21 | 4 << 16 }
const fn x(xo: u32) -> u32 { 31 << 26 | 3 << 21 | 4 << 16 | 5 << 11 | xo << 1 }
/// mfspr/mtspr/mftb form: RT=3 and the SPR number with its two 5-bit halves swapped.
const fn spr(xo: u32, n: u32) -> u32 { 31 << 26 | 3 << 21 | (n & 31) << 16 | (n >> 5) << 11 | xo << 1 }
const fn xl(xo: u32) -> u32 { 19 << 26 | xo << 1 }
/// CR logical on BA=4 BB=5 (both 0 in the preset CR) writing bit `bt`: pick one holding the opposite of the result.
const fn xlc(xo: u32, bt: u32) -> u32 { xl(xo) | bt << 21 | 4 << 16 | 5 << 11 }
const fn fp63(xo: u32) -> u32 { 63 << 26 | 1 << 21 | 2 << 16 | 3 << 11 | xo << 1 }
const fn fp63a(xo5: u32) -> u32 { 63 << 26 | 1 << 21 | 2 << 16 | 3 << 11 | 4 << 6 | xo5 << 1 }
const fn fp59a(xo5: u32) -> u32 { 59 << 26 | 1 << 21 | 2 << 16 | 3 << 11 | 4 << 6 | xo5 << 1 }

const ALL: &[(&str, u32)] = &[
    // integer arithmetic
    ("add", x(266)), ("addc", x(10)), ("adde", x(138)), ("addi", d(14)), ("addic", d(12)), ("addic.", d(13)), ("addis", d(15)),
    ("addme", x(234)), ("addze", x(202)), ("divw", x(491)), ("divwu", x(459)), ("mulhw", x(75)), ("mulhwu", x(11)),
    ("mulli", d(7)), ("mullw", x(235)), ("neg", x(104)), ("subf", x(40)), ("subfc", x(8)), ("subfe", x(136)),
    ("subfic", d(8)), ("subfme", x(232)), ("subfze", x(200)),
    // compare
    ("cmp", x(0)), ("cmpi", d(11)), ("cmpl", x(32)), ("cmpli", d(10)),
    // logical
    ("and", x(28)), ("andc", x(60)), ("andi.", d(28)), ("andis.", d(29)), ("cntlzw", x(26)), ("eqv", x(284)), ("extsb", x(954)),
    ("extsh", x(922)), ("nand", x(476)), ("nor", x(124)), ("or", x(444)), ("orc", x(412)), ("ori", d(24)), ("oris", d(25)),
    ("xor", x(316)), ("xori", d(26)), ("xoris", d(27)),
    // rotate and shift
    ("rlwimi", d(20)), ("rlwinm", d(21)), ("rlwnm", d(23)), ("slw", x(24)), ("sraw", x(792)), ("srawi", x(824)), ("srw", x(536)),
    // floating-point arithmetic and conversion
    ("fadd", fp63a(21)), ("fadds", fp59a(21)), ("fdiv", fp63a(18)), ("fdivs", fp59a(18)), ("fmul", fp63a(25)), ("fmuls", fp59a(25)),
    ("fres", fp59a(24)), ("frsqrte", fp63a(26)), ("fsub", fp63a(20)), ("fsubs", fp59a(20)), ("fsel", fp63a(23)),
    ("fmadd", fp63a(29)), ("fmadds", fp59a(29)), ("fmsub", fp63a(28)), ("fmsubs", fp59a(28)), ("fnmadd", fp63a(31)),
    ("fnmadds", fp59a(31)), ("fnmsub", fp63a(30)), ("fnmsubs", fp59a(30)), ("fctiw", fp63(14)), ("fctiwz", fp63(15)),
    ("frsp", fp63(12)), ("fsqrt", fp63a(22)), ("fsqrts", fp59a(22)),
    // floating-point compare, status and move
    ("fcmpo", fp63(32)), ("fcmpu", fp63(0)), ("mcrfs", fp63(64)), ("mffs", fp63(583)), ("mtfsb0", fp63(70) + (4 << 21)), ("mtfsb1", fp63(38) + (2 << 21)),
    ("mtfsf", fp63(711)), ("mtfsfi", fp63(134)), ("fabs", fp63(264)), ("fmr", fp63(72)), ("fnabs", fp63(136)), ("fneg", fp63(40)),
    // integer loads and stores
    ("lbz", d(34)), ("lbzu", d(35)), ("lbzux", x(119)), ("lbzx", x(87)), ("lha", d(42)), ("lhau", d(43)), ("lhaux", x(375)),
    ("lhax", x(343)), ("lhbrx", x(790)), ("lhz", d(40)), ("lhzu", d(41)), ("lhzux", x(311)), ("lhzx", x(279)), ("lmw", d(46)),
    ("lswi", x(597)), ("lswx", x(533)), ("lwarx", x(20)), ("lwbrx", x(534)), ("lwz", d(32)), ("lwzu", d(33)), ("lwzux", x(55)),
    ("lwzx", x(23)), ("stb", d(38)), ("stbu", d(39)), ("stbux", x(247)), ("stbx", x(215)), ("sth", d(44)), ("sthbrx", x(918)),
    ("sthu", d(45)), ("sthux", x(439)), ("sthx", x(407)), ("stmw", d(47)), ("stswi", x(725)), ("stswx", x(661)), ("stw", d(36)),
    ("stwbrx", x(662)), ("stwcx.", x(150) | 1), ("stwu", d(37)), ("stwux", x(183)), ("stwx", x(151)),
    // floating-point loads and stores
    ("lfd", d(50)), ("lfdu", d(51)), ("lfdux", x(631)), ("lfdx", x(599)), ("lfs", d(48)), ("lfsu", d(49)), ("lfsux", x(567)),
    ("lfsx", x(535)), ("stfd", d(54)), ("stfdu", d(55)), ("stfdux", x(759)), ("stfdx", x(727)), ("stfiwx", x(983)), ("stfs", d(52)),
    ("stfsu", d(53)), ("stfsux", x(695)), ("stfsx", x(663)),
    // branch, condition register, system call, trap
    ("b", 18 << 26), ("bc", 16 << 26 | 20 << 21), ("bcctr", xl(528) | 20 << 21), ("bclr", xl(16) | 20 << 21),
    ("crand", xlc(257, 3)), ("crandc", xlc(129, 3)), ("creqv", xlc(289, 4)), ("crnand", xlc(225, 4)), ("crnor", xlc(33, 4)), ("cror", xlc(449, 3)),
    ("crorc", xlc(417, 4)), ("crxor", xlc(193, 3)), ("mcrf", xl(0) | 3 << 23 | 4 << 18), ("sc", 0x4400_0002), ("tw", x(4)), ("twi", d(3)),
    // processor control, cache and ordering
    ("mfcr", x(19)), ("mfspr", spr(339, 8)), ("mftb", spr(371, 268)), ("mtcrf", x(144)), ("mtspr", spr(467, 8)),
    ("mcrxr", x(512)), ("dcbf", x(86)), ("dcbst", x(54)), ("dcbt", x(278)), ("dcbtst", x(246)), ("dcbz", x(1014)), ("icbi", x(982)),
    ("eieio", x(854)), ("isync", xl(150)), ("sync", x(598)),
    // optional / external-control, not in the 603/G3 user subset
    ("eciwx", x(310)), ("ecowx", x(438)),
];

/// Executed once with benign state; returns the trap, if any.
fn one(word: u32) -> Option<Trap> {
    let mut v = vm();
    v.mem.write_u32(CODE, word).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    c.gpr = [0x4000; 32];
    c.gpr[1] = STACK;
    c.lr = EXIT;
    c.ctr = EXIT;
    c.bind_native(EXIT);
    resume(&mut v, &mut c, 1).trap
}

#[test]
fn every_user_mode_instruction_is_implemented_except_the_listed_ones() {
    let missing: Vec<&str> = ALL.iter()
        .filter(|(_, w)| matches!(one(*w), Some(Trap::Unimplemented { .. })))
        .map(|(n, _)| *n).collect();
    let covered = ALL.len() - missing.len();
    println!("COVERAGE: {covered}/{} user-mode instructions executed; missing: {missing:?}", ALL.len());
    assert_eq!(missing, vec!["eciwx", "ecowx"]);
    assert!(ALL.len() >= 170);
}

/// Everything an instruction can change: registers, CR/XER/LR/CTR/FPSCR and a window of guest memory.
fn effect(word: u32) -> (Option<Trap>, bool) {
    let mut v = vm();
    for i in 0..0x5000u32 { v.mem.write_u8(0x4000 + i, (i.wrapping_mul(37) ^ 0x5A) as u8).unwrap(); }
    v.mem.write_u32(CODE, word).unwrap();
    let mut c = Cpu::new(CODE, STACK);
    for i in 0..32 { c.gpr[i] = 0x4000 + 4 * i as u32 + if i == 7 { 3 << 16 } else { 0 }; c.fpr[i] = (0x4000_0000_0000_0000u64).wrapping_add(i as u64 * 0x0123_4567_89AB); }
    c.gpr[1] = STACK;
    c.cr = 0x1234_5678;
    c.xer = 0x2000_0004; // string count 4: lswx/stswx move exactly r3
    c.fpscr = 0x0400_0000; // ZX set, so mtfsb0 on it has something to clear
    c.gpr[3] |= 0x8000_0000;
    c.lr = EXIT;
    c.ctr = EXIT;
    c.bind_native(EXIT);
    let snap = |c: &Cpu, v: &mut Vm| {
        let mut m = vec![0u8; 0x5000];
        for (k, b) in m.iter_mut().enumerate() { *b = v.mem.read_u8(0x4000 + k as u32).unwrap(); }
        (c.gpr, c.fpr, c.cr, c.xer, c.lr, c.ctr, c.fpscr, m)
    };
    let before = snap(&c, &mut v);
    let st = resume(&mut v, &mut c, 1);
    (st.trap, snap(&c, &mut v) != before)
}

/// Instructions that legitimately change nothing the snapshot sees (hints with no cache to act on, branches that
/// only move pc, a non-trapping trap word, a handled-or-absent sc) - each is covered by its own behaviour test.
const NO_SNAPSHOT_EFFECT: &[&str] = &["b", "bc", "dcbf", "dcbst", "dcbt", "dcbtst", "icbi", "eieio", "isync", "sync", "tw", "twi", "sc", "bcctr", "bclr"];

#[test]
fn every_covered_instruction_really_executes_not_just_decodes() {
    // A decoder arm that returned Ok(()) with no effect would pass the sweep above. Run each instruction against a
    // distinct, non-degenerate state and demand that something observable changed.
    let mut silent = vec![];
    for (name, w) in ALL {
        let (trap, changed) = effect(*w);
        if matches!(trap, Some(Trap::Unimplemented { .. })) || NO_SNAPSHOT_EFFECT.contains(name) { continue; }
        if trap.is_none() && !changed { silent.push(*name); }
    }
    assert!(silent.is_empty(), "executed but changed nothing observable: {silent:?}");
    // CONTROL: the detector does see a no-op as a no-op (ori 0,0,0) and a real effect as one (addi 3,4,1).
    assert_eq!(effect(24 << 26), (None, false));
    assert_eq!(effect(14 << 26 | 3 << 21 | 4 << 16 | 1), (None, true));
}

#[test]
fn supervisor_only_instructions_refuse_in_user_mode() {
    for (name, w) in [("mfmsr", x(83)), ("mtmsr", x(146)), ("tlbsync", x(566)), ("rfi", xl(50)), ("dcbi", x(470)), ("mfspr PVR", spr(339, 287)), ("mtspr SDR1", spr(467, 25))] {
        assert!(matches!(one(w), Some(Trap::Privileged { .. })), "{name}: {:?}", one(w));
    }
    // CONTROL: the user-visible SPRs (LR=8, CTR=9, XER=1, TBL=268) are fine.
    for n in [1u32, 8, 9, 268, 269] {
        assert_eq!(one(spr(339, n)), None, "mfspr {n}");
    }
}
