//! The architectural state a case starts from, and the seeded generators for it.
use crate::rng::Rng;

/// Guest RAM: [0, RAM). Code sits at CODE; the random data window is [DATA, DATA+DATA_LEN).
pub const RAM: usize = 0x4000;
pub const CODE: u32 = 0x200;
pub const DATA: u32 = 0x1000;
pub const DATA_LEN: u32 = 0x2000;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct State {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: u32,
    pub xer: u32,
    pub lr: u32,
    pub ctr: u32,
    pub fpscr: u32,
    pub tb: u64,
    /// Seed of the random fill of the data window.
    pub memseed: u64,
    /// Reservation address (lwarx), if any, set before the case (stwcx. paths).
    pub reservation: Option<u32>,
}

pub fn int_val(r: &mut Rng) -> u32 {
    match r.below(10) {
        0 => r.pick(&[0, 1, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 63, 64, 0xFF, 0x100]),
        1 => r.pick(&[0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFF, 0xFFFF_FFFE, 0x7FFF_FFFE, 0xFFFF, 0x8000, 0x1_0000]),
        2 => r.below(64),
        3 => 0u32.wrapping_sub(r.below(64)),
        4 => 1u32 << r.below(32),
        5 => !(1u32 << r.below(32)),
        _ => r.u32(),
    }
}

const D_SPECIAL: [u64; 24] = [
    0x0000_0000_0000_0000, 0x8000_0000_0000_0000, 0x7FF0_0000_0000_0000, 0xFFF0_0000_0000_0000, 0x7FF8_0000_0000_0000,
    0xFFF8_0000_0000_0000, 0x7FF0_0000_0000_0001, 0xFFF4_0000_0000_0000, 0x7FF7_FFFF_E000_0000, 0x7FEF_FFFF_FFFF_FFFF,
    0x0000_0000_0000_0001, 0x000F_FFFF_FFFF_FFFF, 0x0010_0000_0000_0000, 0x3FF0_0000_0000_0000, 0xBFF0_0000_0000_0000,
    0x3FE0_0000_0000_0000, 0x4000_0000_0000_0000, 0x41DF_FFFF_FFC0_0000, 0xC1E0_0000_0000_0000, 0x4330_0000_0000_0000,
    0x47EF_FFFF_E000_0000, 0x3810_0000_0000_0000, 0x37F0_0000_0000_0000, 0x7FF8_0000_dead_beef,
];

/// A double bit pattern biased towards the corners of the FP model.
pub fn fp_val(r: &mut Rng) -> u64 {
    match r.below(12) {
        0 | 1 => r.pick(&D_SPECIAL),
        2 => widen(f32::from_bits(r.u32())),
        3 => { let v = (r.u32() as i32 as f64) / (1u64 << r.below(34)) as f64; v.to_bits() }
        4 => { let v = (r.u32() as i32 as f64) + [0.0, 0.5, 0.25, 0.75, -0.5, 1.5, 2.5][r.below(7) as usize]; v.to_bits() }
        5 => { // near an integer / half-integer boundary
            let n = (r.u32() as i32 >> r.below(31)) as f64;
            let e = [0.0, 0.5, 0.4999999999999999, 0.5000000000000001, 1e-9, -1e-9, 0.9999999999999999][r.below(7) as usize];
            (n + e).to_bits()
        }
        6 => { // exponent near the single-precision denormal / overflow edges
            let e = 860 + r.below(60) as u64;
            (r.u64() & 0x800F_FFFF_FFFF_FFFF) | e << 52
        }
        7 => { let e = 1150 + r.below(30) as u64; (r.u64() & 0x800F_FFFF_FFFF_FFFF) | e << 52 }
        _ => r.u64(),
    }
}

/// `fp_val`, but exactly representable in single precision (NaNs excepted: those widen exactly too).
pub fn fp_single_val(r: &mut Rng) -> u64 {
    let b = fp_val(r);
    let f = f64::from_bits(b);
    if f.is_nan() {
        // keep sign + top 23 fraction bits; the low fraction bits of a widened single are zero
        return b & 0xFFFF_FFFF_E000_0000 | (b & 0x0008_0000_0000_0000);
    }
    match r.below(4) {
        0 => widen(f32::from_bits(r.u32())),
        _ => widen(f as f32),
    }
}
/// Widen a single to double bits without quieting a signalling NaN.
pub fn widen(x: f32) -> u64 {
    let s = x.to_bits();
    if x.is_nan() { (s as u64 & 0x8000_0000) << 32 | 0x7FF << 52 | (s as u64 & 0x007F_FFFF) << 29 } else { (x as f64).to_bits() }
}

impl State {
    pub fn random(r: &mut Rng) -> State {
        let mut gpr = [0u32; 32];
        for g in gpr.iter_mut() { *g = int_val(r); }
        let mut fpr = [0u64; 32];
        for f in fpr.iter_mut() { *f = fp_val(r); }
        // FPSCR: any enables / sticky bits; RN mostly 0; NI (bit 29, mask 4) clear: non-IEEE mode is outside both models.
        let mut fpscr = r.u32() & !0x4;
        if r.pct(80) { fpscr &= !0x3; }
        if r.pct(50) { fpscr &= !(0xFFF8_0000 | 0x0000_0700); } // no sticky bits: FX transitions become visible
        // Debug aid for triage: PPC_FP_MODE=clean clears RN and every exception enable, =enables keeps VE..XE random.
        if std::env::var("PPC_FP_MODE").as_deref() == Ok("clean") { fpscr &= !(0x3 | 0xF8); }
        fpscr = normalize_fpscr(fpscr);
        State {
            gpr, fpr,
            cr: r.u32(),
            xer: r.u32() & 0xE000_0000 | (if r.pct(70) { r.below(48) } else { r.below(128) }),
            lr: r.u32() & !3,
            ctr: int_val(r),
            fpscr,
            tb: r.u64() >> r.below(40),
            memseed: r.u64(),
            reservation: None,
        }
    }
}

/// Fill the data window deterministically from `seed`.
pub fn fill_window(ram: &mut [u8], seed: u64) {
    let mut r = Rng::new(seed);
    for b in ram[DATA as usize..(DATA + DATA_LEN) as usize].iter_mut() { *b = r.u32() as u8; }
}

/// Make FPSCR[VX] and FPSCR[FEX] consistent with the bits they summarise (PEM 2.1.4): a state the hardware can be in.
pub fn normalize_fpscr(f: u32) -> u32 {
    let b = |n: u32| 1u32 << (31 - n);
    let vx_all = [7, 8, 9, 10, 11, 12, 21, 22, 23].iter().fold(0, |m, &n| m | b(n));
    let mut f = f & !(b(1) | b(2));
    if f & vx_all != 0 { f |= b(2); }
    let fex = (f & b(2) != 0 && f & b(24) != 0) || (f & b(3) != 0 && f & b(25) != 0) || (f & b(4) != 0 && f & b(26) != 0)
        || (f & b(5) != 0 && f & b(27) != 0) || (f & b(6) != 0 && f & b(28) != 0);
    if fex { f |= b(1); }
    f
}
