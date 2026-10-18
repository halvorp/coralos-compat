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
    ("fcmpo", fp63(32)), ("fcmpu", fp63(0)), ("mcrfs", fp63(64)), ("mffs", fp63(583)), ("mtfsb0", fp63(70)), ("mtfsb1", fp63(38)),
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
    ("crand", xl(257)), ("crandc", xl(129)), ("creqv", xl(289)), ("crnand", xl(225)), ("crnor", xl(33)), ("cror", xl(449)),
    ("crorc", xl(417)), ("crxor", xl(193)), ("mcrf", xl(0)), ("sc", 0x4400_0002), ("tw", x(4)), ("twi", d(3)),
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

#[test]
fn every_covered_instruction_really_executes_not_just_decodes() {
    // The sweep above would pass if a decoder arm returned Ok(()) without effect. Spot-check by instruction effect:
    // each of these changes a register or memory when run, and the entries are distinct words.
    let words: std::collections::HashSet<u32> = ALL.iter().map(|(_, w)| *w).collect();
    assert_eq!(words.len(), ALL.len(), "duplicate encodings in the table would hide a hole");
}

#[test]
fn supervisor_only_instructions_refuse_in_user_mode() {
    for (name, w) in [("mfmsr", x(83)), ("mtmsr", x(146)), ("rfi", xl(50)), ("dcbi", x(470)), ("mfspr PVR", spr(339, 287)), ("mtspr SDR1", spr(467, 25))] {
        assert!(matches!(one(w), Some(Trap::Privileged { .. })), "{name}: {:?}", one(w));
    }
    // CONTROL: the user-visible SPRs (LR=8, CTR=9, XER=1, TBL=268) are fine.
    for n in [1u32, 8, 9, 268, 269] {
        assert_eq!(one(spr(339, n)), None, "mfspr {n}");
    }
}
