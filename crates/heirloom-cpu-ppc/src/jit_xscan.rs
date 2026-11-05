//! Native routines (jit_xnat) for Warcraft III's simulation-tick SCANS (h86 heavy, 2026-11-05): the heavy frames of WC3
//! gameplay are its periodic tick (every 6-7 frames on the host harness: compat tools/ppc-profile-frames.py), whose code is
//! fetched cold. [`GRIDSCAN`] is checkCollisionDetection's cell loop (~119k guest instructions a tick frame): per pathing cell
//! four 16-bit flags masked, a cell record's bit compared, and on a MISMATCH (rare) the original code runs the update calls.
use crate::jit_xnat::{Out, X};

/// The 67 words of [`crate::jit_xnat::Routine::GridScan`] (the cell loop, at its head): `lhz r0,0(r21); li r3,0; and.
/// r0,r0,r31; ble out`, the same for `2(r21)`, `0(r16)` and `lhzx r0,r30,r21+2` (`addi r0,r21,2`), `li r3,1`, `out: lhz
/// r4,12(r19); clrlwi r18,r3,24; rlwinm r0,r4,30,31,31; cmpw r18,r0; beq next`, the mismatch path (words 20..60: bounds tests,
/// the record's neighbour, two calls), `next: addi r21,r21,2; addi r16,r16,2; addi r19,r19,28; addi r25,r25,1; cmplw r25,r23;
/// blt` back to the head.
pub const GRIDSCAN: [u32; 67] = [
    0xA015_0000, 0x3860_0000, 0x7C00_F839, 0x4081_0030, 0xA015_0002, 0x7C00_F839, 0x4081_0024, 0xA010_0000, 0x7FE0_0039,
    0x4081_0018, 0x3815_0002, 0x7C1E_022E, 0x7FE0_0039, 0x4081_0008, 0x3860_0001, 0xA093_000C, 0x5472_063E, 0x5480_F7FE,
    0x7C12_0000, 0x4182_00A8, 0x5480_05AD, 0x4182_000C, 0x5480_05EF, 0x4182_0098, 0x8091_00A4, 0x3B59_0001, 0x3860_0000,
    0x7C1A_2040, 0x4181_0014, 0x8011_00A8, 0x7C1B_0040, 0x4181_0008, 0x3860_0001, 0x5460_063F, 0x4182_0028, 0x3804_0001,
    0x8091_00D4, 0x7C1B_01D6, 0x7C1A_0214, 0x1C60_001C, 0x3803_000C, 0x7C04_022E, 0x5400_F7FE, 0x4800_0008, 0x3800_0001,
    0x7C12_0000, 0x4182_003C, 0x2C12_0000, 0x4182_001C, 0x7E23_8B78, 0x7F44_D378, 0x7F65_DB78, 0x4800_0359, 0x2C03_0000,
    0x4182_001C, 0x7E23_8B78, 0x7F44_D378, 0x7F65_DB78, 0x7E46_9378, 0x38E0_0000, 0x4800_0049, 0x3AB5_0002, 0x3A10_0002,
    0x3A73_001C, 0x3B39_0001, 0x7C19_B840, 0x4180_FEF8,
];
/// The longest trip that stays in the routine (all four flags tested, the record matching): 15 + 5 + 6.
pub(crate) const GRIDSCAN_TRIP_MAX: u32 = 26;
/// The word after `beq next` (the mismatch path's first), and the loop's exit.
const MISMATCH: u32 = 20;
const EXIT: u32 = 67;

/// One cell's flags test: (u = all four masked flags non-zero, the instructions it ran) or None where a load it would make is
/// outside RAM. `and.` sets CR0 GT on a positive result: a flag's test passes iff `(flag & r31) as i32 > 0`.
#[inline(always)]
fn cell(x: &X, r21: u32, r16: u32, r30: u32, r31: u32) -> Option<(bool, u64)> {
    let at = [r21, r21.wrapping_add(2), r16, r30.wrapping_add(r21.wrapping_add(2))];
    let cost = [4u64, 3, 3, 4];
    let mut k = 0;
    for i in 0..4 {
        if !x.in_ram(at[i], 2) { return None; }
        k += cost[i];
        if (x.rd16(at[i]) & r31) as i32 <= 0 { return Some((false, k)); }
    }
    Some((true, k + 1))
}

/// [`GRIDSCAN`] at its head `pc` (entered with r25 < r23, as the loop's `blt` leaves it). Trips while the budget holds the
/// longest trip and every load the trip makes is inside RAM (the row's four flag streams and its records validated at once,
/// else load by load); a trip whose record bit equals its flags' u goes on, one that differs leaves at the mismatch path (word
/// 20) with r0 r3 r4 r18 and CR0 = cmpw r18,r0 as the original leaves them there. Leaves r0 = the record's bit, r3 = r18 = u,
/// r4 = the record's halfword, r21 / r16 += 2, r19 += 28, r25 += 1 a trip, CR0 = cmplw r25,r23; the pc after the loop once r25
/// reaches r23, else the head. No stores.
pub(crate) fn gridscan(x: &mut X, pc: u32) -> Out {
    let g = &x.cpu.gpr;
    let (r23, r30) = (g[23], g[30]);
    let (r21, r16, r19, r25) = (g[21], g[16], g[19], g[25]);
    if r25 >= r23 { return None; }
    // the row's ranges (r23 - r25 trips): the grid's [r21, +2R+2), the next row's [r16, +2R), [r30 + r21 + 2, +2R), the
    // records' halfwords at r19 + 12 + 28i
    let rem = (r23 - r25) as u64;
    let fits = |a: u32, n: u64| n <= u32::MAX as u64 && x.in_ram(a, n as u32);
    let whole = fits(r21, 2 * rem + 2) && fits(r16, 2 * rem) && fits(r30.wrapping_add(r21).wrapping_add(2), 2 * rem)
        && fits(r19.wrapping_add(12), 28 * (rem - 1) + 2);
    if whole { fast(x, pc) } else { checked(x, pc) }
}

/// The trip's instructions by where its flags test stopped (the first test failing ... all four passing), with the record's
/// test and the loop's tail: 4 / 7 / 10 / 14 / 15, + 5 + 6.
const TRIP: [u64; 5] = [15, 18, 21, 25, 26];

/// [`gridscan`] over a VALIDATED row (every load the remaining trips can make is inside RAM): the flags tested on the bytes
/// as loaded against the mask's swapped low half (a big-endian halfword AND the mask is not zero iff the swapped bytes AND
/// the swapped mask are not), the record's bit 2 at bit 10 of its swapped halfword; the trips counted by path and the
/// budget taken once (whole trips of the longest path).
fn fast(x: &mut X, pc: u32) -> Out {
    let g = &x.cpu.gpr;
    let (r23, r30, r31) = (g[23], g[30], g[31]);
    let (r21, r16, r19, r25) = (g[21], g[16], g[19], g[25]);
    let max = ((r23 - r25) as u64).min(x.left / GRIDSCAN_TRIP_MAX as u64) as u32;
    if max == 0 { return None; }
    let m = (r31 as u16).swap_bytes();
    let mut paths = [0u64; 5];
    let mut i = 0u32;
    let (mut u, mut rec) = (false, 0u16);
    while i < max {
        let a = r21.wrapping_add(2 * i);
        let p = if x.rd16_raw(a) & m == 0 { 0 }
            else if x.rd16_raw(a.wrapping_add(2)) & m == 0 { 1 }
            else if x.rd16_raw(r16.wrapping_add(2 * i)) & m == 0 { 2 }
            else if x.rd16_raw(r30.wrapping_add(a).wrapping_add(2)) & m == 0 { 3 }
            else { 4 };
        u = p == 4;
        rec = x.rd16_raw(r19.wrapping_add(12).wrapping_add(28 * i));
        if (rec >> 10 & 1 != 0) != u {
            // the mismatch path at word 20: this trip's flags and record test (no tail), the trips before it whole
            let n = paths.iter().zip(TRIP).map(|(c, t)| c * t).sum::<u64>() + TRIP[p] - 6;
            let h = rec.swap_bytes() as u32;
            let gr = &mut x.cpu.gpr;
            (gr[0], gr[3], gr[4], gr[18]) = (h >> 2 & 1, u as u32, h, u as u32);
            (gr[21], gr[16], gr[19], gr[25]) = (a, r16.wrapping_add(2 * i), r19.wrapping_add(28 * i), r25.wrapping_add(i));
            x.cmpw0(u as u32, h >> 2 & 1);
            return Some((pc.wrapping_add(4 * MISMATCH), n));
        }
        paths[p] += 1;
        i += 1;
    }
    let mut n = paths.iter().zip(TRIP).map(|(c, t)| c * t).sum::<u64>();
    if x.mutant == crate::jit_mutants::MUT_NAT_GRID_COUNT { n -= paths[..4].iter().sum::<u64>(); }
    let h = rec.swap_bytes() as u32;
    let r25 = r25.wrapping_add(i);
    let gr = &mut x.cpu.gpr;
    (gr[0], gr[3], gr[4], gr[18]) = (h >> 2 & 1, u as u32, h, u as u32);
    (gr[21], gr[16], gr[19], gr[25]) = (r21.wrapping_add(2 * i), r16.wrapping_add(2 * i), r19.wrapping_add(28 * i), r25);
    x.cmplw0(r25, r23);
    Some((if r25 >= r23 { pc.wrapping_add(4 * EXIT) } else { pc }, n))
}

/// [`gridscan`] load by load (a row whose ranges are not all inside RAM): a trip runs only if every load it makes is.
fn checked(x: &mut X, pc: u32) -> Out {
    let g = &x.cpu.gpr;
    let (r23, r30, r31) = (g[23], g[30], g[31]);
    let (mut r21, mut r16, mut r19, mut r25) = (g[21], g[16], g[19], g[25]);
    let (mut n, mut trips) = (0u64, 0u32);
    let (mut r0, mut u, mut r4) = (0u32, false, 0u32);
    let leave = |x: &mut X, r0: u32, u: bool, r4: u32, r21: u32, r16: u32, r19: u32, r25: u32| {
        let g = &mut x.cpu.gpr;
        (g[0], g[3], g[4], g[18]) = (r0, u as u32, r4, u as u32);
        (g[21], g[16], g[19], g[25]) = (r21, r16, r19, r25);
    };
    loop {
        if n + GRIDSCAN_TRIP_MAX as u64 > x.left { break; }
        let rec = r19.wrapping_add(12);
        let Some((cu, mut k)) = cell(x, r21, r16, r30, r31) else { break };
        if !x.in_ram(rec, 2) { break; }
        let h = x.rd16(rec);
        let bit = h >> 2 & 1;
        k += 5;
        if cu as u32 != bit {
            // the mismatch path: the trip's r0 r3 r4 r18 and its cmpw, at word 20
            leave(x, bit, cu, h, r21, r16, r19, r25);
            x.cmpw0(cu as u32, bit);
            return Some((pc.wrapping_add(4 * MISMATCH), n + k));
        }
        (r0, u, r4) = (bit, cu, h);
        (r21, r16, r19, r25) = (r21.wrapping_add(2), r16.wrapping_add(2), r19.wrapping_add(28), r25.wrapping_add(1));
        k += if x.mutant == crate::jit_mutants::MUT_NAT_GRID_COUNT && !cu { 5 } else { 6 };
        n += k;
        trips += 1;
        if r25 >= r23 {
            leave(x, r0, u, r4, r21, r16, r19, r25);
            x.cmplw0(r25, r23);
            return Some((pc.wrapping_add(4 * EXIT), n));
        }
    }
    if trips == 0 { return None; }
    leave(x, r0, u, r4, r21, r16, r19, r25);
    x.cmplw0(r25, r23);
    Some((pc, n))
}
