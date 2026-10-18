//! The instruction families the differential fuzz draws from. Each entry fixes the opcode/extended-opcode bits and
//! every field the architecture marks reserved (zero), and leaves the register/immediate/OE/Rc/LK/AA bits random.
//!
//! Bit numbers in comments are the manual's (0 = MSB); masks are host masks.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Int,
    /// Integer loads/stores: the base registers are pointed into the data window.
    Mem,
    Fp,
    /// op 59: the operands are made single-representable (anything else is architecturally undefined).
    FpSingle,
    FpMem,
    Br,
    Spr,
    Sys,
}

#[derive(Clone, Copy, Debug)]
pub struct Ins {
    pub name: &'static str,
    /// Bits that are fixed.
    pub mask: u32,
    pub fixed: u32,
    pub cat: Cat,
}

const OPM: u32 = 0xFC00_0000;
const XM: u32 = 0xFC00_07FE; // op + 10-bit XO
const XOM: u32 = 0xFC00_03FE; // op + 9-bit XO at word bits 22-30 (OE, bit 21, free)
const AM: u32 = 0xFC00_003E; // op + 5-bit A-form XO

fn d(name: &'static str, op: u32, cat: Cat) -> Ins { Ins { name, mask: OPM, fixed: op << 26, cat } }
fn x(name: &'static str, op: u32, xo: u32, cat: Cat) -> Ins { Ins { name, mask: XM, fixed: op << 26 | xo << 1, cat } }
fn xo(name: &'static str, x9: u32) -> Ins { Ins { name, mask: XOM, fixed: 31 << 26 | x9 << 1, cat: Cat::Int } }
fn a(name: &'static str, op: u32, x5: u32, cat: Cat) -> Ins { Ins { name, mask: AM, fixed: op << 26 | x5 << 1, cat } }
impl Ins {
    /// Reserved (must-be-zero) bits.
    fn z(mut self, m: u32) -> Ins { self.mask |= m; self }
    /// No Rc bit: bit 31 is zero.
    fn norc(self) -> Ins { self.z(1) }
    /// Bit 31 must be one.
    fn rc1(mut self) -> Ins { self.mask |= 1; self.fixed |= 1; self }
    pub fn matches(&self, w: u32) -> bool { w & self.mask == self.fixed }
    pub fn word(&self, rnd: u32) -> u32 { self.fixed | (rnd & !self.mask) }
}

const RB: u32 = 0x0000_F800;
const RA: u32 = 0x001F_0000;
const RT: u32 = 0x03E0_0000;
const FRC: u32 = 0x0000_07C0;
const LBIT: u32 = 0x0020_0000;

pub fn table() -> Vec<Ins> {
    use Cat::*;
    let mut t = vec![
        // D-form arithmetic / logical (PEM 4.2.1, 4.2.2)
        d("addi", 14, Int), d("addis", 15, Int), d("addic", 12, Int), d("addic.", 13, Int), d("subfic", 8, Int),
        d("mulli", 7, Int), d("ori", 24, Int), d("oris", 25, Int), d("xori", 26, Int), d("xoris", 27, Int),
        d("andi.", 28, Int), d("andis.", 29, Int), d("cmpi", 11, Int).z(LBIT | 0x0040_0000), d("cmpli", 10, Int).z(LBIT | 0x0040_0000),
        d("twi", 3, Int),
        // rotate (PEM 4.2.1.4)
        d("rlwimi", 20, Int), d("rlwinm", 21, Int), d("rlwnm", 23, Int),
        // XO-form arithmetic
        xo("add", 266), xo("addc", 10), xo("adde", 138), xo("addze", 202).z(RB), xo("addme", 234).z(RB), xo("subf", 40),
        xo("subfc", 8), xo("subfe", 136), xo("subfze", 200).z(RB), xo("subfme", 232).z(RB), xo("neg", 104).z(RB),
        xo("mullw", 235), xo("mulhw", 75).z(0x400), xo("mulhwu", 11).z(0x400), xo("divw", 491), xo("divwu", 459),
        // X-form logical / shift / compare
        x("and", 31, 28, Int), x("andc", 31, 60, Int), x("or", 31, 444, Int), x("orc", 31, 412, Int),
        x("xor", 31, 316, Int), x("nor", 31, 124, Int), x("nand", 31, 476, Int), x("eqv", 31, 284, Int),
        x("slw", 31, 24, Int), x("srw", 31, 536, Int), x("sraw", 31, 792, Int), x("srawi", 31, 824, Int),
        x("cntlzw", 31, 26, Int).z(RB), x("extsb", 31, 954, Int).z(RB), x("extsh", 31, 922, Int).z(RB),
        x("cmp", 31, 0, Int).z(LBIT | 0x0040_0000 | 1), x("cmpl", 31, 32, Int).z(LBIT | 0x0040_0000 | 1),
        x("tw", 31, 4, Int).norc(),
        // integer loads/stores
        d("lwz", 32, Mem), d("lwzu", 33, Mem), d("lbz", 34, Mem), d("lbzu", 35, Mem), d("stw", 36, Mem), d("stwu", 37, Mem),
        d("stb", 38, Mem), d("stbu", 39, Mem), d("lhz", 40, Mem), d("lhzu", 41, Mem), d("lha", 42, Mem), d("lhau", 43, Mem),
        d("sth", 44, Mem), d("sthu", 45, Mem), d("lmw", 46, Mem), d("stmw", 47, Mem),
        x("lwzx", 31, 23, Mem).norc(), x("lwzux", 31, 55, Mem).norc(), x("lbzx", 31, 87, Mem).norc(), x("lbzux", 31, 119, Mem).norc(),
        x("stwx", 31, 151, Mem).norc(), x("stwux", 31, 183, Mem).norc(), x("stbx", 31, 215, Mem).norc(), x("stbux", 31, 247, Mem).norc(),
        x("lhzx", 31, 279, Mem).norc(), x("lhzux", 31, 311, Mem).norc(), x("lhax", 31, 343, Mem).norc(), x("lhaux", 31, 375, Mem).norc(),
        x("sthx", 31, 407, Mem).norc(), x("sthux", 31, 439, Mem).norc(),
        x("lhbrx", 31, 790, Mem).norc(), x("lwbrx", 31, 534, Mem).norc(), x("sthbrx", 31, 918, Mem).norc(), x("stwbrx", 31, 662, Mem).norc(),
        x("lwarx", 31, 20, Mem).norc(), x("stwcx.", 31, 150, Mem).rc1(),
        x("lswi", 31, 597, Mem).norc(), x("lswx", 31, 533, Mem).norc(), x("stswi", 31, 725, Mem).norc(), x("stswx", 31, 661, Mem).norc(),
        x("dcbz", 31, 1014, Mem).norc().z(RT), x("dcbf", 31, 86, Mem).norc().z(RT), x("dcbst", 31, 54, Mem).norc().z(RT),
        x("dcbt", 31, 278, Mem).norc().z(RT), x("dcbtst", 31, 246, Mem).norc().z(RT), x("icbi", 31, 982, Mem).norc().z(RT),
        x("sync", 31, 598, Sys).norc().z(RT | RA | RB), x("eieio", 31, 854, Sys).norc().z(RT | RA | RB),
        x("isync", 19, 150, Sys).norc().z(RT | RA | RB),
        // FP loads/stores
        d("lfs", 48, FpMem), d("lfsu", 49, FpMem), d("lfd", 50, FpMem), d("lfdu", 51, FpMem),
        d("stfs", 52, FpMem), d("stfsu", 53, FpMem), d("stfd", 54, FpMem), d("stfdu", 55, FpMem),
        x("lfsx", 31, 535, FpMem).norc(), x("lfsux", 31, 567, FpMem).norc(), x("lfdx", 31, 599, FpMem).norc(), x("lfdux", 31, 631, FpMem).norc(),
        x("stfsx", 31, 663, FpMem).norc(), x("stfsux", 31, 695, FpMem).norc(), x("stfdx", 31, 727, FpMem).norc(),
        x("stfdux", 31, 759, FpMem).norc(), x("stfiwx", 31, 983, FpMem).norc(),
        // branches and the condition register
        d("b", 18, Br), d("bc", 16, Br), x("bclr", 19, 16, Br).z(RB), x("bcctr", 19, 528, Br).z(RB),
        x("mcrf", 19, 0, Br).norc().z(0x0063_F800 | 0x0000_0000), x("crand", 19, 257, Br).norc(), x("cror", 19, 449, Br).norc(),
        x("crxor", 19, 193, Br).norc(), x("crnand", 19, 225, Br).norc(), x("crnor", 19, 33, Br).norc(),
        x("creqv", 19, 289, Br).norc(), x("crandc", 19, 129, Br).norc(), x("crorc", 19, 417, Br).norc(),
        // SPR / CR moves
        x("mfcr", 31, 19, Spr).norc().z(RA | RB), x("mtcrf", 31, 144, Spr).norc().z(0x0010_0800),
        x("mcrxr", 31, 512, Spr).norc().z(0x007F_F800), x("mfspr", 31, 339, Spr).norc(), x("mtspr", 31, 467, Spr).norc(),
        x("mftb", 31, 371, Spr).norc(),
        x("mfmsr", 31, 83, Spr).norc().z(RA | RB), x("mtmsr", 31, 146, Spr).norc().z(RA | RB),
        x("rfi", 19, 50, Sys).norc().z(RT | RA | RB),
        Ins { name: "sc", mask: 0xFFFF_FFFF, fixed: 0x4400_0002, cat: Sys },
        // FP single (op 59)
        a("fadds", 59, 21, FpSingle).z(FRC), a("fsubs", 59, 20, FpSingle).z(FRC), a("fmuls", 59, 25, FpSingle).z(RB),
        a("fdivs", 59, 18, FpSingle).z(FRC), a("fsqrts", 59, 22, FpSingle).z(RA | FRC), a("fres", 59, 24, FpSingle).z(RA | FRC),
        a("fmadds", 59, 29, FpSingle), a("fmsubs", 59, 28, FpSingle), a("fnmadds", 59, 31, FpSingle), a("fnmsubs", 59, 30, FpSingle),
        // FP double (op 63)
        a("fadd", 63, 21, Fp).z(FRC), a("fsub", 63, 20, Fp).z(FRC), a("fmul", 63, 25, Fp).z(RB), a("fdiv", 63, 18, Fp).z(FRC),
        a("fsqrt", 63, 22, Fp).z(RA | FRC), a("frsqrte", 63, 26, Fp).z(RA | FRC), a("fsel", 63, 23, Fp),
        a("fmadd", 63, 29, Fp), a("fmsub", 63, 28, Fp), a("fnmadd", 63, 31, Fp), a("fnmsub", 63, 30, Fp),
        x("fcmpu", 63, 0, Fp).z(0x0060_0001), x("fcmpo", 63, 32, Fp).z(0x0060_0001),
        x("frsp", 63, 12, Fp).z(RA), x("fctiw", 63, 14, Fp).z(RA), x("fctiwz", 63, 15, Fp).z(RA),
        x("fneg", 63, 40, Fp).z(RA), x("fmr", 63, 72, Fp).z(RA), x("fnabs", 63, 136, Fp).z(RA), x("fabs", 63, 264, Fp).z(RA),
        x("mffs", 63, 583, Fp).z(RA | RB),
        x("mtfsf", 63, 711, Fp).z(0x0201_0000), x("mtfsfi", 63, 134, Fp).z(0x007F_0800 | 0x0000_0000),
        x("mtfsb0", 63, 70, Fp).z(RA | RB), x("mtfsb1", 63, 38, Fp).z(RA | RB),
        x("mcrfs", 63, 64, Fp).norc().z(0x0060_0000 | 0x0003_F800),
    ];
    // mtfsfi: BF in bits 6-8, imm in bits 16-19; the compiler-free zero mask above covers 9-15 and 20.
    // (RA|RB of mtfsfi are the reserved range 9-15 and 16 is the imm's top bit, handled by the explicit mask.)
    for i in t.iter_mut() {
        if i.name == "mtfsfi" { i.mask = XM | 0x007F_0800; }
    }
    t
}
