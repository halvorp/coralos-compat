//! Native routine (jit_xnat) for Warcraft III's hottest loop: setupVertexArrays's per-vertex SKINNING (5.5 % of the JIT's host
//! instructions on the gameplay census, ~1,000 vertices a frame, 84 guest instructions each). Per vertex: a bone index byte
//! selects a 4x4 single-precision matrix (64 bytes at [r31 + 440] + 64 i); the position (3 floats) goes through its upper
//! 3x4 (rotation + translation) into [r31 + 1516] + r7, the normal (3 floats) through its 3x3 into [r31 + 1528] + r7; the three
//! stream pointers in the TOC's globals (r2 + 19304 ..) advance by their strides.
//!
//! EXACT as tier 0 computes it (fpu.rs): lfs widens exactly (a finite single: the host's widening); fmuls = (a * c) rounded
//! once to single (the product is exact in f64); fadds = (a + b) rounded to single (an f64 sum of two singles rounds
//! innocuously); fmadds = the exact a * c + b rounded ONCE to single - the f64 sum s rounds to the same single unless s is a
//! single's rounding MIDPOINT or below the single normal range, and those take fpu::fma_single; stfs of a single value is
//! its bits; FPSCR[FPRF] = the class of the LAST arithmetic result (the loop's final fmadds). Refused (tier 1's):
//! FPSCR[RN] != 0, a loaded float that is an infinity or a NaN, a result that is not finite (tier 0 would raise OX).
//!
//! TWO PATHS. The BATCH: when no output of the next k vertices can reach anything the loop reads (the three streams over k
//! vertices, the 256 matrices an index byte can select, the globals, the struct fields) and every range is inside RAM, the
//! vertices run back to back with no per-vertex test but the floats', the globals and the registers written once at the end.
//! Else the CAREFUL path, vertex by vertex: a vertex whose stores reach anything it loads after them stops it (so a vertex's
//! loads may all be taken first).
use crate::jit_xnat::{overlap, Out, X};

/// The 84 words of [`crate::jit_xnat::Routine::Skin`] (the loop at its head; straight-line, `bne` back to the head).
pub const SKIN: [u32; 84] = [
    0x8062_4B6C, 0x3908_0001, 0x80A2_4B84, 0x8803_0000, 0x807F_01B8, 0x5406_3032, 0x801F_05EC, 0x7C63_3214, 0xC065_0004,
    0xC003_0010, 0x7C80_3A14, 0xC023_0014, 0xEC03_0032, 0xC043_0018, 0xEC83_0072, 0xC0C5_0000, 0xC023_0000, 0xEC43_00B2,
    0xC0A3_0004, 0xEC06_007A, 0xC063_0008, 0xEC86_217A, 0xC0E5_0008, 0xC023_0020, 0xEC46_10FA, 0xC0A3_0024, 0xEC07_007A,
    0xC023_0030, 0xEC87_217A, 0xC063_0028, 0xC0A3_0034, 0xEC01_002A, 0xEC47_10FA, 0xC063_0038, 0xEC85_202A, 0xD004_0000,
    0xEC23_102A, 0xD084_0004, 0xD024_0008, 0x8082_4B7C, 0x801F_01B8, 0xC0C4_0004, 0x7C60_3214, 0xC0E4_0000, 0xC003_0010,
    0xC063_0014, 0xC043_0018, 0xEC06_0032, 0xC023_0000, 0xEC86_00F2, 0xC0A3_0004, 0xEC46_00B2, 0xEC07_007A, 0xECA7_217A,
    0xC104_0008, 0xC023_0020, 0x801F_05F8, 0xC063_0008, 0xEC08_007A, 0x7C80_3A14, 0xC0C3_0024, 0xC083_0028, 0xEC27_10FA,
    0xEC48_29BA, 0xD004_0000, 0x38E7_000C, 0xEC08_093A, 0xD044_0004, 0xD004_0008, 0x8062_4B84, 0x8002_4B80, 0x80A2_4B7C,
    0x7CC3_0214, 0x8082_4B78, 0x8062_4B6C, 0x8002_4B68, 0x7C85_2214, 0x90C2_4B84, 0x7C03_0214, 0x9082_4B7C, 0x9002_4B6C,
    0x8002_4B88, 0x7C08_0040, 0x4082_FEB4,
];
/// One vertex's instructions.
pub(crate) const SKIN_TRIP: u32 = 84;

// The globals (r2 + these): the bone-index stream, its stride, the normal stream, its stride, the position stream, its stride,
// the vertex count; the struct fields (r31 + these): the matrices, the position output, the normal output.
const G_IDX: u32 = 19308;
const G_IDX_STEP: u32 = 19304;
const G_NRM: u32 = 19324;
const G_NRM_STEP: u32 = 19320;
const G_POS: u32 = 19332;
const G_POS_STEP: u32 = 19328;
const G_END: u32 = 19336;
const S_MAT: u32 = 440;
const S_OPOS: u32 = 1516;
const S_ONRM: u32 = 1528;

#[inline(always)]
fn finite(v: u32) -> bool { v & 0x7F80_0000 != 0x7F80_0000 }
/// lfs of a finite single.
#[inline(always)]
fn f(b: u32) -> f64 { f32::from_bits(b) as f64 }
#[inline(always)]
fn muls(a: f64, c: f64) -> f64 { ((a * c) as f32) as f64 }
#[inline(always)]
fn adds(a: f64, b: f64) -> f64 { ((a + b) as f32) as f64 }
/// fmadds of singles: s = a * c + b (the product exact, the sum rounded to f64) rounds to the exact sum's single unless s is a
/// single midpoint (its 29 bits below a single's mantissa 1 then zeros) or under 2^-126 (fewer single bits there).
#[inline(always)]
fn madds<const MID: bool>(a: f64, c: f64, b: f64) -> f64 {
    let s = a * c + b;
    let bits = s.to_bits();
    if (!MID || bits & 0x1FFF_FFFF != 0x1000_0000) && bits & 0x7FF0_0000_0000_0000 >= 897 << 52 { return (s as f32) as f64; }
    madds_exact(a, c, b)
}
#[cold]
#[inline(never)]
fn madds_exact(a: f64, c: f64, b: f64) -> f64 { (crate::fpu::fma_single(a, c, b) as f32) as f64 }
/// stfs of a single value.
#[inline(always)]
fn st(v: f64) -> u32 { (v as f32).to_bits() }

/// One vertex: the position's three results and the normal's, and f0..f8 as the vertex leaves them; None = a float that is
/// not finite (in or out).
#[inline(always)]
/// MID = false is the planted mutant MUT_NAT_SKIN_MIDPOINT: a midpoint sum takes the f64 sum's rounding (ties to even).
fn vertex<const MID: bool>(p: [u32; 3], q: [u32; 3], m: &[u32; 15]) -> Option<([f64; 6], [f64; 9])> {
    let madds = madds::<MID>;
    let fin = p.iter().chain(q.iter()).chain([0usize, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14].iter().map(|&k| &m[k])).fold(true, |o, &v| o & finite(v));
    if !fin { return None; }
    let mm = |k: usize| f(m[k]);
    // the position (words 8 .. 38)
    let f3 = f(p[1]);
    let (f6, f7) = (f(p[0]), f(p[2]));
    let f0 = madds(f7, mm(8), madds(f6, mm(0), muls(f3, mm(4))));
    let f4 = madds(f7, mm(9), madds(f6, mm(1), muls(f3, mm(5))));
    let f2 = madds(f7, mm(10), madds(f6, mm(2), muls(f3, mm(6))));
    let (t0, t4, t1) = (adds(mm(12), f0), adds(mm(13), f4), adds(mm(14), f2));
    // the normal (words 41 .. 66)
    let (g6, g7, g8) = (f(q[1]), f(q[0]), f(q[2]));
    let h0 = madds(g8, mm(8), madds(g7, mm(0), muls(g6, mm(4))));
    let h5 = madds(g7, mm(1), muls(g6, mm(5)));
    let h1 = madds(g7, mm(2), muls(g6, mm(6)));
    let h2b = madds(g8, mm(9), h5);
    let h0b = madds(g8, mm(10), h1);
    let out = [t0, t4, t1, h0, h2b, h0b];
    if !out.iter().fold(true, |o, v| o & v.is_finite()) { return None; }
    // f0..f8 as their last writers left them: fmadds (66), fmadds (62), fmadds (63), lfs m2 (57), lfs m10 (61), fmadds (53),
    // lfs m9 (60), lfs q0 (43), lfs q2 (54)
    Some((out, [h0b, h1, h2b, mm(2), mm(10), h5, mm(9), g7, g8]))
}

fn vx(mutant: u8) -> fn([u32; 3], [u32; 3], &[u32; 15]) -> Option<([f64; 6], [f64; 9])> {
    if mutant == crate::jit_mutants::MUT_NAT_SKIN_MIDPOINT { vertex::<false> } else { vertex::<true> }
}

/// The registers and FPSCR after vertex `last` (its stream pointers, the vertex count), the loop's exit test.
fn leave(x: &mut X, r7: u32, r8: u32, end: u32, idx_p: u32, nrm_p: u32, nnrm: u32, npos: u32, fr: &[f64; 9]) {
    let c = &mut *x.cpu;
    (c.gpr[0], c.gpr[3], c.gpr[4], c.gpr[5], c.gpr[6], c.gpr[7], c.gpr[8]) = (end, idx_p, nnrm, nrm_p, npos, r7, r8);
    for (k, v) in fr.iter().enumerate() { c.fpr[k] = v.to_bits(); }
    crate::fpscr::set_fprf(c, crate::fpscr::classify(fr[0], true));
    x.cmplw0(r8, end);
}

/// [`SKIN`] at its head `pc`: vertices while r8 (incremented first) != [r2 + 19336] after each.
pub(crate) fn skin(x: &mut X, pc: u32) -> Out {
    if x.cpu.fpscr & crate::fpscr::RN != 0 { return None; }
    let (r2, r31) = (x.cpu.gpr[2], x.cpu.gpr[31]);
    let g = |o: u32| r2.wrapping_add(o);
    let s = |o: u32| r31.wrapping_add(o);
    if !(x.in_ram(g(G_IDX_STEP), 36) && x.in_ram(s(S_MAT), 4) && x.in_ram(s(S_OPOS), 4) && x.in_ram(s(S_ONRM), 4) && x.storable(g(G_IDX), 28)) { return None; }
    if let Some(out) = batch(x, pc) { return out; }
    careful(x, pc)
}

/// The batch path (see the module doc): None = its conditions do not hold (the careful path runs instead).
fn batch(x: &mut X, pc: u32) -> Option<Out> {
    let (r2, r31) = (x.cpu.gpr[2], x.cpu.gpr[31]);
    let g = |o: u32| r2.wrapping_add(o);
    let s = |o: u32| r31.wrapping_add(o);
    let (idx_p, pos_p, nrm_p, end) = (x.rd32(g(G_IDX)), x.rd32(g(G_POS)), x.rd32(g(G_NRM)), x.rd32(g(G_END)));
    let (sidx, spos, snrm) = (x.rd32(g(G_IDX_STEP)), x.rd32(g(G_POS_STEP)), x.rd32(g(G_NRM_STEP)));
    let (mat0, opos0, onrm0) = (x.rd32(s(S_MAT)), x.rd32(s(S_OPOS)), x.rd32(s(S_ONRM)));
    let (r7, r8) = (x.cpu.gpr[7], x.cpu.gpr[8]);
    // trips until r8 == end (0: 2^32), within the budget and 4096 (with strides <= 0xFFFF every length below fits 32 bits)
    let k = (end.wrapping_sub(r8).wrapping_sub(1) as u64 + 1).min(x.left / SKIN_TRIP as u64).min(4096) as u32;
    if k == 0 || sidx > 0xFFFF || spos > 0xFFFF || snrm > 0xFFFF { return None; }
    let (opos, onrm) = (opos0.wrapping_add(r7), onrm0.wrapping_add(r7));
    let ins = [(idx_p, (k - 1) * sidx + 1), (pos_p, (k - 1) * spos + 12), (nrm_p, (k - 1) * snrm + 12), (mat0, 256 * 64), (g(G_IDX_STEP), 36), (s(S_MAT), 4), (s(S_OPOS), 4), (s(S_ONRM), 4)];
    if !(ins.iter().all(|&(a, l)| x.in_ram(a, l)) && x.storable(opos, 12 * k) && x.storable(onrm, 12 * k)) { return None; }
    if x.mutant != crate::jit_mutants::MUT_NAT_SKIN_ALIAS && ins.iter().any(|&(a, l)| overlap(opos, 12 * k, a, l) || overlap(onrm, 12 * k, a, l)) { return None; }
    x.note(opos, 12 * k);
    x.note(onrm, 12 * k);
    let mut fr = [0f64; 9];
    let mut v = 0u32;
    while v < k {
        let (ip, pp, np) = (idx_p.wrapping_add(v * sidx), pos_p.wrapping_add(v * spos), nrm_p.wrapping_add(v * snrm));
        let mat = mat0.wrapping_add(x.rd8(ip) << 6);
        let mut m = [0u32; 15];
        for (j, w) in m.iter_mut().enumerate() { *w = x.rd32(mat + 4 * j as u32); }
        let Some((out, regs)) = vx(x.mutant)([x.rd32(pp), x.rd32(pp + 4), x.rd32(pp + 8)], [x.rd32(np), x.rd32(np + 4), x.rd32(np + 8)], &m) else { break };
        for (j, &r) in out.iter().enumerate() {
            let at = if j < 3 { opos + 12 * v + 4 * j as u32 } else { onrm + 12 * v + 4 * (j as u32 - 3) };
            x.st32(at, st(r));
        }
        fr = regs;
        v += 1;
    }
    if v == 0 { return Some(None); }
    // the globals and registers after the last vertex run (index v - 1)
    let (lip, lpp, lnp) = (idx_p.wrapping_add((v - 1) * sidx), pos_p.wrapping_add((v - 1) * spos), nrm_p.wrapping_add((v - 1) * snrm));
    let (nidx, npos, nnrm) = (lip.wrapping_add(sidx), lpp.wrapping_add(spos), lnp.wrapping_add(snrm));
    x.note(g(G_IDX), 28);
    x.st32(g(G_POS), npos);
    x.st32(g(G_NRM), nnrm);
    x.st32(g(G_IDX), nidx);
    let r8n = r8.wrapping_add(v);
    leave(x, r7.wrapping_add(12 * v), r8n, end, lip, lnp, nnrm, npos, &fr);
    let next = if r8n == end { pc.wrapping_add(4 * SKIN.len() as u32) } else { pc };
    Some(Some((next, SKIN_TRIP as u64 * v as u64)))
}

/// The careful path: vertex by vertex, each validated before it writes.
fn careful(x: &mut X, pc: u32) -> Out {
    let (mut n, mut trips) = (0u64, 0u32);
    let (r2, r31) = (x.cpu.gpr[2], x.cpu.gpr[31]);
    let g = |o: u32| r2.wrapping_add(o);
    let s = |o: u32| r31.wrapping_add(o);
    let stop = |trips: u32, n: u64| if trips == 0 { None } else { Some((pc, n)) };
    loop {
        if n + SKIN_TRIP as u64 > x.left { return stop(trips, n); }
        let r7 = x.cpu.gpr[7];
        let (idx_p, pos_p, nrm_p) = (x.rd32(g(G_IDX)), x.rd32(g(G_POS)), x.rd32(g(G_NRM)));
        let (mat0, opos0, onrm0) = (x.rd32(s(S_MAT)), x.rd32(s(S_OPOS)), x.rd32(s(S_ONRM)));
        if !x.in_ram(idx_p, 1) { return stop(trips, n); }
        let r6 = x.rd8(idx_p) << 6;
        let (mat, opos, onrm) = (mat0.wrapping_add(r6), opos0.wrapping_add(r7), onrm0.wrapping_add(r7));
        if !(x.in_ram(pos_p, 12) && x.in_ram(nrm_p, 12) && x.in_ram(mat, 60) && x.storable(opos, 12) && x.storable(onrm, 12)) { return stop(trips, n); }
        // the position's stores come before the normal half's loads, the normal's before the globals' loads: neither may
        // reach what is loaded after it (then every load can be taken first)
        let after_opos = [(g(G_NRM), 4), (s(S_MAT), 4), (nrm_p, 12), (mat, 44), (s(S_ONRM), 4), (g(G_IDX_STEP), 36)];
        let mutant_alias = x.mutant == crate::jit_mutants::MUT_NAT_SKIN_ALIAS;
        if !mutant_alias && (after_opos.iter().any(|&(a, l)| overlap(opos, 12, a, l)) || overlap(onrm, 12, g(G_IDX_STEP), 36)) { return stop(trips, n); }
        let mut m = [0u32; 15];
        for (k, v) in m.iter_mut().enumerate() { *v = x.rd32(mat + 4 * k as u32); }
        let Some((out, fr)) = vx(x.mutant)([x.rd32(pos_p), x.rd32(pos_p + 4), x.rd32(pos_p + 8)], [x.rd32(nrm_p), x.rd32(nrm_p + 4), x.rd32(nrm_p + 8)], &m) else { return stop(trips, n) };
        // the stores, in program order
        for (j, &r) in out.iter().enumerate() { x.wr32(if j < 3 { opos + 4 * j as u32 } else { onrm + 4 * (j as u32 - 3) }, st(r)); }
        // words 69 .. 80: the stream pointers advance (their steps read after the vertex's stores: none reached them)
        let (pos_step, idx_step, nrm_step) = (x.rd32(g(G_POS_STEP)), x.rd32(g(G_IDX_STEP)), x.rd32(g(G_NRM_STEP)));
        let (npos, nnrm, nidx) = (pos_p.wrapping_add(pos_step), nrm_p.wrapping_add(nrm_step), idx_p.wrapping_add(idx_step));
        x.wr32(g(G_POS), npos);
        x.wr32(g(G_NRM), nnrm);
        x.wr32(g(G_IDX), nidx);
        let end = x.rd32(g(G_END));
        let r8 = x.cpu.gpr[8].wrapping_add(1);
        leave(x, r7.wrapping_add(12), r8, end, idx_p, nrm_p, nnrm, npos, &fr);
        n += SKIN_TRIP as u64;
        trips += 1;
        if r8 == end { return Some((pc.wrapping_add(4 * SKIN.len() as u32), n)); }
    }
}
