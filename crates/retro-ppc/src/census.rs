//! Opcode and block census of a run: which instructions the guest really executes, and how long its straight-line
//! runs are. Collected only by [`crate::Exec`] with a census attached (a separate monomorphised loop), so a run
//! without one pays nothing per instruction.
//!
//! A dynamic BLOCK here is what a decoded-block interpreter would see: a run of instructions entered at one pc and
//! left by a branch, `sc`, a native call, a host redirect, or the end of a 4 KiB page.
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

/// The census key of an instruction word: primary opcode, and the extended opcode for the four opcode groups that
/// have one (19, 31: 10 bits; 59, 63: A-form 5 bits, else 10 bits).
pub fn key(w: u32) -> u32 {
    let op = w >> 26;
    let xo = match op {
        19 | 31 => w >> 1 & 0x3FF,
        59 => w >> 1 & 0x1F,
        63 => if w >> 1 & 0x10 != 0 { w >> 1 & 0x1F } else { w >> 1 & 0x3FF },
        _ => 0,
    };
    op << 10 | xo
}

/// A readable name for a census key: the mnemonic of the common instructions, else `op<N>/xo<M>`.
pub fn name(key: u32) -> alloc::string::String {
    let (op, xo) = (key >> 10, key & 0x3FF);
    let n = match (op, xo) {
        (3, _) => "twi", (7, _) => "mulli", (8, _) => "subfic", (10, _) => "cmpli", (11, _) => "cmpi", (12, _) => "addic",
        (13, _) => "addic.", (14, _) => "addi", (15, _) => "addis", (16, _) => "bc", (17, _) => "sc", (18, _) => "b",
        (19, 16) => "bclr", (19, 528) => "bcctr", (19, 0) => "mcrf", (19, 150) => "isync", (19, 193) => "crxor",
        (19, 449) => "cror", (19, 289) => "creqv", (19, 33) => "crnor", (19, 129) => "crandc", (19, 257) => "crand",
        (20, _) => "rlwimi", (21, _) => "rlwinm", (23, _) => "rlwnm", (24, _) => "ori", (25, _) => "oris", (26, _) => "xori",
        (27, _) => "xoris", (28, _) => "andi.", (29, _) => "andis.", (32, _) => "lwz", (33, _) => "lwzu", (34, _) => "lbz",
        (35, _) => "lbzu", (36, _) => "stw", (37, _) => "stwu", (38, _) => "stb", (39, _) => "stbu", (40, _) => "lhz",
        (41, _) => "lhzu", (42, _) => "lha", (43, _) => "lhau", (44, _) => "sth", (45, _) => "sthu", (46, _) => "lmw",
        (47, _) => "stmw", (48, _) => "lfs", (49, _) => "lfsu", (50, _) => "lfd", (51, _) => "lfdu", (52, _) => "stfs",
        (53, _) => "stfsu", (54, _) => "stfd", (55, _) => "stfdu",
        (31, 0) => "cmp", (31, 32) => "cmpl", (31, 4) => "tw", (31, 8) | (31, 520) => "subfc", (31, 10) | (31, 522) => "addc",
        (31, 11) => "mulhwu", (31, 19) => "mfcr", (31, 20) => "lwarx", (31, 23) => "lwzx", (31, 24) => "slw",
        (31, 26) => "cntlzw", (31, 28) => "and", (31, 40) | (31, 552) => "subf", (31, 55) => "lwzux", (31, 60) => "andc",
        (31, 75) => "mulhw", (31, 87) => "lbzx", (31, 104) | (31, 616) => "neg", (31, 119) => "lbzux", (31, 124) => "nor",
        (31, 136) | (31, 648) => "subfe", (31, 138) | (31, 650) => "adde", (31, 144) => "mtcrf", (31, 150) => "stwcx.",
        (31, 151) => "stwx", (31, 183) => "stwux", (31, 200) | (31, 712) => "subfze", (31, 202) | (31, 714) => "addze",
        (31, 215) => "stbx", (31, 232) | (31, 744) => "subfme", (31, 234) | (31, 746) => "addme", (31, 235) | (31, 747) => "mullw",
        (31, 247) => "stbux", (31, 266) | (31, 778) => "add", (31, 279) => "lhzx", (31, 284) => "eqv", (31, 316) => "xor",
        (31, 339) => "mfspr", (31, 343) => "lhax", (31, 371) => "mftb", (31, 407) => "sthx", (31, 412) => "orc",
        (31, 444) => "or", (31, 459) | (31, 971) => "divwu", (31, 467) => "mtspr", (31, 476) => "nand",
        (31, 491) | (31, 1003) => "divw", (31, 512) => "mcrxr", (31, 534) => "lwbrx", (31, 535) => "lfsx", (31, 536) => "srw",
        (31, 599) => "lfdx", (31, 597) => "lswi", (31, 725) => "stswi", (31, 663) => "stfsx", (31, 727) => "stfdx",
        (31, 792) => "sraw", (31, 824) => "srawi", (31, 854) => "eieio", (31, 598) => "sync", (31, 922) => "extsh",
        (31, 954) => "extsb", (31, 982) => "icbi", (31, 1014) => "dcbz", (31, 86) => "dcbf", (31, 54) => "dcbst",
        (31, 278) => "dcbt", (31, 246) => "dcbtst",
        (59, 18) => "fdivs", (59, 20) => "fsubs", (59, 21) => "fadds", (59, 25) => "fmuls", (59, 28) => "fmsubs",
        (59, 29) => "fmadds", (59, 30) => "fnmsubs", (59, 31) => "fnmadds", (59, 24) => "fres", (59, 22) => "fsqrts",
        (63, 18) => "fdiv", (63, 20) => "fsub", (63, 21) => "fadd", (63, 25) => "fmul", (63, 28) => "fmsub",
        (63, 29) => "fmadd", (63, 30) => "fnmsub", (63, 31) => "fnmadd", (63, 23) => "fsel", (63, 22) => "fsqrt",
        (63, 26) => "frsqrte", (63, 0) => "fcmpu", (63, 32) => "fcmpo", (63, 12) => "frsp", (63, 14) => "fctiw",
        (63, 15) => "fctiwz", (63, 72) => "fmr", (63, 40) => "fneg", (63, 264) => "fabs", (63, 136) => "fnabs",
        (63, 583) => "mffs", (63, 711) => "mtfsf", (63, 134) => "mtfsfi", (63, 70) => "mtfsb0", (63, 38) => "mtfsb1",
        (63, 64) => "mcrfs",
        _ => return alloc::format!("op{op}/xo{xo}"),
    };
    n.into()
}

/// Whether `w` ends a dynamic block by itself (a branch or `sc`).
pub(crate) fn ends_block(w: u32) -> bool {
    match w >> 26 { 16..=18 => true, 19 => matches!(w >> 1 & 0x3FF, 16 | 528), _ => false }
}

pub struct Census {
    /// Indexed by [`key`]: dynamic executions.
    pub ops: Vec<u64>,
    /// Block entry pc -> (dynamic entries, instructions retired from it).
    pub blocks: BTreeMap<u32, (u64, u64)>,
    pub natives: u64,
    open: Option<(u32, u64)>,
}

impl Default for Census { fn default() -> Self { Census { ops: vec![0; 64 << 10], blocks: BTreeMap::new(), natives: 0, open: None } } }

impl Census {
    #[inline]
    pub(crate) fn insn(&mut self, pc: u32, w: u32) {
        self.ops[key(w) as usize] += 1;
        match &mut self.open { Some((_, n)) => *n += 1, None => self.open = Some((pc, 1)) }
    }
    /// The block that is open ends after the instruction just counted (`next` is where execution goes).
    #[inline]
    pub(crate) fn after(&mut self, pc: u32, w: Option<u32>, next: u32) {
        let end = match w { None => true, Some(w) => ends_block(w) || next != pc.wrapping_add(4) || next & 0xFFF == 0 };
        if end { self.close(); }
    }
    pub(crate) fn close(&mut self) {
        if let Some((start, n)) = self.open.take() {
            let e = self.blocks.entry(start).or_default();
            e.0 += 1;
            e.1 += n;
        }
    }
    pub fn total(&self) -> u64 { self.ops.iter().sum() }
    /// The `n` most executed instruction kinds, (name, count), most first.
    pub fn top_ops(&self, n: usize) -> Vec<(alloc::string::String, u64)> {
        let mut v: Vec<(u32, u64)> = self.ops.iter().enumerate().filter(|(_, c)| **c > 0).map(|(k, c)| (k as u32, *c)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.into_iter().take(n).map(|(k, c)| (name(k), c)).collect()
    }
    /// (distinct blocks, dynamic block entries, instructions in blocks): mean dynamic block length = third / second.
    pub fn block_summary(&self) -> (usize, u64, u64) {
        let (mut e, mut i) = (0, 0);
        for (n, k) in self.blocks.values() { e += n; i += k; }
        (self.blocks.len(), e, i)
    }
    /// The `n` hottest blocks by instructions retired: (entry pc, entries, instructions).
    pub fn top_blocks(&self, n: usize) -> Vec<(u32, u64, u64)> {
        let mut v: Vec<(u32, u64, u64)> = self.blocks.iter().map(|(p, (e, i))| (*p, *e, *i)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2));
        v.truncate(n);
        v
    }
}
