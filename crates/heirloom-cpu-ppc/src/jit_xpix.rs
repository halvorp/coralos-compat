//! Native routines (jit_xnat) for Warcraft III's PIXEL LOOPS (and clearBitset's word scan), chosen by the gameplay census
//! (callgrind over the JIT arena, per guest loop: /build/patches/h86/native): each runs row by row in Rust, every pixel in the
//! original's order on real guest memory (a source the destination overlaps reads exactly what the original would), every
//! register the loop leaves exact.
//! * [`PALBLIT`] (resizePaletteAndBlit, 2.5 % of the JIT's host instructions): rows of 16 palette-indexed bytes to 16-bit
//!   pixels through a 512-entry table, the upper half chosen per pixel by a mask byte's bit 4 while a flag word is set.
//! * [`TEXCONV`] (texture_format_convert, ~1.4 %): rows of 16-bit pixels rotated left by a nibble (ARGB4444 -> RGBA4444), an
//!   8-pixel unrolled loop and a 1-pixel tail.
//! * [`ALPHASET`] / [`ALPHAMASK`] (alphaBlendMaskRows: the simulation tick's fog-of-war mask, h86 heavy - 40k JIT host
//!   instructions a gameplay frame on average, ~6x that in a tick frame): rows of 16-bit pixels whose top bit becomes whether
//!   their low 15 bits are not zero (where a mask word's bits allow it); the JIT's four 8-instruction srawi carries a word go.
use crate::jit_xnat::{Out, X};

/// The 53 words of [`crate::jit_xnat::Routine::PalBlit`] (the outer loop over rows, at its head): `li r6,8; mr r12,r4; mr r24,r5;
/// mr r25,r10; li r27,15; mtctr r6`, 8 x two pixels (`lwz r6,8604(r26); [li r9,0;] cmpwi r6,0; beq; lbz r6,j(r12); rlwinm.
/// r6,r6,0,27,27; beq; li r9,1; clrlwi r8,r9,24; lbz r6,j(r24); neg r7,r8; or r7,r7,r8; rlwinm r6,r6,1,23,30; rlwinm
/// r7,r7,10,22,22; add r6,r7,r6; addi r6,r6,6552; lhzx r6,r26,r6; sth r6,2j(r25)`, the pointer steps, bdnz), then `cmpwi
/// r11,0; add r4,r4,r0; add r5,r5,r3; addi r10,r10,32; addi r11,r11,-1; bne` back to the head.
pub const PALBLIT: [u32; 53] = [
    0x38C0_0008, 0x7C8C_2378, 0x7CB8_2B78, 0x7D59_5378, 0x3B60_000F, 0x7CC9_03A6, 0x80DA_219C, 0x3920_0000, 0x2C06_0000,
    0x4182_0014, 0x88CC_0000, 0x54C6_06F7, 0x4182_0008, 0x3920_0001, 0x5528_063E, 0x88D8_0000, 0x7CE8_00D0, 0x3920_0000,
    0x7CE7_4378, 0x54C6_0DFC, 0x54E7_55AC, 0x7CC7_3214, 0x38C6_1998, 0x7CDA_322E, 0xB0D9_0000, 0x80DA_219C, 0x2C06_0000,
    0x4182_0014, 0x88CC_0001, 0x54C6_06F7, 0x4182_0008, 0x3920_0001, 0x5528_063E, 0x88D8_0001, 0x7CE8_00D0, 0x398C_0002,
    0x7CE7_4378, 0x54C6_0DFC, 0x54E7_55AC, 0x3B18_0002, 0x7CC7_3214, 0x3B7B_FFFE, 0x38C6_1998, 0x7CDA_322E, 0xB0D9_0002,
    0x3B39_0004, 0x4200_FF60, 0x2C0B_0000, 0x7C84_0214, 0x7CA5_1A14, 0x394A_0020, 0x396B_FFFF, 0x4082_FF30,
];
/// One row's instructions: 12 (its head and tail) + 8 x (33 + per pixel 3 when the flag is set + 1 when the mask bit is).
pub(crate) const PALBLIT_ROW_MIN: u32 = 12 + 8 * 33;
pub(crate) const PALBLIT_ROW_MAX: u32 = 12 + 8 * 41;
/// The flag word and the table, at r26 + these.
const PB_FLAG: u32 = 8604;
const PB_TABLE: u32 = 6552;

/// [`PALBLIT`] at its head `pc`: rows while the row count r11 was not zero before its decrement. Per row (validated first: the
/// flag word, the 16 source and 16 mask bytes, the table's 1 KiB inside RAM, the 32 destination bytes storable, the budget
/// for the row's longest path), per pixel j in order: F = [r26 + 8604] (re-read every pixel, as the original does), sel = F != 0
/// && mask[j] & 0x10, dst[j] = table[(src[j] << 1 & 0x1FE) + (sel ? 0x200 : 0)]. Leaves r6 r7 r8 r9 r12 r24 r25 r27 CTR CR0
/// of the last pixel / row as the original does; after the last row the pc is the word after the loop, after a refusal at a
/// later row the head.
pub(crate) fn palblit(x: &mut X, pc: u32) -> Out {
    let (mut n, mut rows) = (0u64, 0u32);
    let flag = x.cpu.gpr[26].wrapping_add(PB_FLAG);
    let table = x.cpu.gpr[26].wrapping_add(PB_TABLE);
    loop {
        let g = &x.cpu.gpr;
        let (mask, src, dst) = (g[4], g[5], g[10]);
        let ok = x.in_ram(flag, 4) && x.in_ram(table, 0x400) && x.in_ram(mask, 16) && x.in_ram(src, 16) && x.storable(dst, 32)
            && n + PALBLIT_ROW_MAX as u64 <= x.left;
        if !ok { return if rows == 0 { None } else { Some((pc, n)) }; }
        // the row: li r6,8; mr r12,r4; mr r24,r5; mr r25,r10; li r27,15; mtctr r6
        let mut k = 6u64;
        let (mut r6, mut r7, mut r8, mut r9) = (0u32, 0u32, 0u32, 0u32);
        let reads = [(flag, 4), (table, 0x400), (mask, 16), (src, 16)];
        if !reads.iter().any(|&(a, l)| crate::jit_xnat::overlap(dst, 32, a, l)) {
            // FAST: the row writes nothing it reads - the flag is one value, every pixel independent
            x.note(dst, 32);
            let f = x.rd32(flag) != 0;
            let mut sel = false;
            for j in 0..16u32 {
                sel = f && x.rd8(mask.wrapping_add(j)) & 0x10 != 0;
                r6 = x.rd16(table.wrapping_add(x.rd8(src.wrapping_add(j)) << 1 | (sel as u32) << 9));
                x.st16(dst.wrapping_add(2 * j), r6);
                k += sel as u64;
            }
            // per pixel pair 4 + 11 and 3 + 15, with the flag set 3 more each; the last pixel's sel in r7 r8 r9
            k += 8 * 33 + if f { 16 * 3 } else { 0 };
            (r7, r8, r9) = (if sel { 0x200 } else { 0 }, sel as u32, sel as u32);
            // (with the flag set the last pixel's CR0 is its mask test's; the row's tail compare overwrites it below)
        } else {
        for j in 0..16u32 {
            if j % 2 == 0 { r9 = 0; } // pixel 0 of a pair: li r9,0 before its test (pixel 1 finds pixel 0's tail li r9,0)
            r6 = x.rd32(flag);
            x.cmpw0(r6, 0);
            k += if j % 2 == 0 { 4 } else { 3 }; // lwz [li] cmpwi beq
            if r6 != 0 {
                r6 = x.rd8(mask.wrapping_add(j)) & 0x10;
                x.cmpw0(r6, 0);
                k += 3; // lbz rlwinm. beq
                if r6 != 0 { r9 = 1; k += 1; }
            }
            r8 = r9 & 0xFF;
            r7 = (r8.wrapping_neg() | r8).rotate_left(10) & 0x200;
            if j % 2 == 0 { r9 = 0; }
            r6 = (x.rd8(src.wrapping_add(j)) << 1 & 0x1FE) + r7 + PB_TABLE;
            r6 = x.rd16(x.cpu.gpr[26].wrapping_add(r6));
            x.wr16(dst.wrapping_add(2 * j), r6);
            // clrlwi lbz neg [li] or rlwinm rlwinm add addi lhzx sth (11); pixel 1 also addi r12 / r24 / r27 / r25 and bdnz
            k += if j % 2 == 0 { 11 } else { 15 };
        }
        }
        let g = &mut x.cpu.gpr;
        (g[6], g[7], g[8], g[9]) = (r6, r7, r8, r9);
        (g[12], g[24], g[25], g[27]) = (mask.wrapping_add(16), src.wrapping_add(16), dst.wrapping_add(32), 15u32.wrapping_sub(16));
        x.cpu.ctr = 0;
        // cmpwi r11,0; add r4,r4,r0; add r5,r5,r3; addi r10,r10,32; addi r11,r11,-1; bne
        let r11 = x.cpu.gpr[11];
        x.cmpw0(r11, 0);
        let g = &mut x.cpu.gpr;
        g[4] = g[4].wrapping_add(g[0]);
        g[5] = g[5].wrapping_add(g[3]);
        g[10] = g[10].wrapping_add(32);
        g[11] = r11.wrapping_sub(1);
        k += 6;
        if x.mutant == crate::jit_mutants::MUT_NAT_PB_COUNT { k -= 1; }
        n += k;
        rows += 1;
        if r11 == 0 { return Some((pc.wrapping_add(4 * PALBLIT.len() as u32), n)); }
    }
}

/// The 63 words of [`crate::jit_xnat::Routine::TexConv`] (the loop over rows, at its head): `cmplwi r5,0; li r8,0; ble tail;
/// cmplwi r5,8; addi r9,r5,-8; ble ones; addi r0,r9,7; srwi r0,r0,3; mtctr r0; cmplwi r9,0; ble ones`, 8 x `lhz r9,2j(r3);
/// rlwinm r0,r9,20,28,31; rlwimi r0,r9,4,16,27; sth r0,2j(r7)` (with `addi r8,r8,8`, `addi r3,r3,16`, `addi r7,r7,16`, bdnz),
/// `ones: subf r0,r8,r5; mtctr r0; cmplw r8,r5; bge tail`, the same for one pixel (`addi r3,r3,2; addi r8,r8,1; ...; addi
/// r7,r7,2; bdnz`), `tail: add r3,r3,r10; addi r4,r4,1; cmplw r4,r6; blt` back to the head.
pub const TEXCONV: [u32; 63] = [
    0x2805_0000, 0x3900_0000, 0x4081_00E4, 0x2805_0008, 0x3925_FFF8, 0x4081_00A8, 0x3809_0007, 0x5400_E8FE, 0x7C09_03A6,
    0x2809_0000, 0x4081_0094, 0xA123_0000, 0x3908_0008, 0x5520_A73E, 0x5120_2436, 0xB007_0000, 0xA123_0002, 0x5520_A73E,
    0x5120_2436, 0xB007_0002, 0xA123_0004, 0x5520_A73E, 0x5120_2436, 0xB007_0004, 0xA123_0006, 0x5520_A73E, 0x5120_2436,
    0xB007_0006, 0xA123_0008, 0x5520_A73E, 0x5120_2436, 0xB007_0008, 0xA123_000A, 0x5520_A73E, 0x5120_2436, 0xB007_000A,
    0xA123_000C, 0x5520_A73E, 0x5120_2436, 0xB007_000C, 0xA123_000E, 0x3863_0010, 0x5520_A73E, 0x5120_2436, 0xB007_000E,
    0x38E7_0010, 0x4200_FF74, 0x7C08_2850, 0x7C09_03A6, 0x7C08_2840, 0x4080_0024, 0xA123_0000, 0x3863_0002, 0x3908_0001,
    0x5520_A73E, 0x5120_2436, 0xB007_0000, 0x38E7_0002, 0x4200_FFE4, 0x7C63_5214, 0x3884_0001, 0x7C04_3040, 0x4180_FF08,
];
/// The shortest row (no pixels): `cmplwi li ble` + the 4 of its tail.
pub(crate) const TEXCONV_ROW_MIN: u32 = 7;
/// Pixels above which a row bails (its count needs a bound; no texture row is near it).
const TC_MAX: u32 = 1 << 20;

/// One pixel: rlwinm r0,r9,20,28,31; rlwimi r0,r9,4,16,27 (r9 a zero-extended halfword).
#[inline(always)]
fn tc_px(r9: u32) -> u32 { (r9 >> 12 & 0xF) | (r9 << 4 & 0xFFF0) }

/// [`TEXCONV`] at its head `pc`: rows while r4 < r6 (unsigned) after each. Per row of n = r5 pixels (validated first: the 2n
/// source bytes inside RAM, the 2n destination bytes storable, n <= 2^20, the row's exact count within the budget), pixel i
/// in order: dst[i] = tc_px(src[i]). Leaves r0 r8 r9 CTR (untouched by an empty row, as the original), r3 += 2n + r10,
/// r7 += 2n, r4 += 1, CR0 = cmplw r4, r6.
pub(crate) fn texconv(x: &mut X, pc: u32) -> Out {
    let (mut n, mut rows) = (0u64, 0u32);
    loop {
        let g = &x.cpu.gpr;
        let (w, src, dst) = (g[5], g[3], g[7]);
        // the row's exact count: n = 0: 7; n <= 8: 14 + 8n; else (k = ceil((n - 8) / 8) eight-pixel trips) 19 + 36k + 8(n - 8k)
        let k8 = if w > 8 { (w - 8 + 7) / 8 } else { 0 };
        let cost = match w { 0 => 7, 1..=8 => 14 + 8 * w as u64, _ => 19 + 36 * k8 as u64 + 8 * (w - 8 * k8) as u64 };
        let ok = w <= TC_MAX && (w == 0 || (x.in_ram(src, 2 * w) && x.storable(dst, 2 * w))) && n + cost <= x.left;
        if !ok { return if rows == 0 { None } else { Some((pc, n)) }; }
        if w > 0 {
            let mut r9 = 0;
            x.note(dst, 2 * w);
            for i in 0..w {
                r9 = x.rd16(src.wrapping_add(2 * i));
                x.st16(dst.wrapping_add(2 * i), tc_px(r9));
            }
            let g = &mut x.cpu.gpr;
            (g[0], g[8], g[9]) = (tc_px(r9), w, r9);
            x.cpu.ctr = 0;
        } else {
            x.cpu.gpr[8] = 0;
        }
        let g = &mut x.cpu.gpr;
        g[3] = src.wrapping_add(2 * w).wrapping_add(g[10]);
        g[7] = dst.wrapping_add(2 * w);
        g[4] = g[4].wrapping_add(1);
        let (r4, r6) = (g[4], g[6]);
        x.cmplw0(r4, r6);
        n += if x.mutant == crate::jit_mutants::MUT_NAT_TC_COUNT && w > 8 { cost - 36 } else { cost };
        rows += 1;
        if r4 >= r6 { return Some((pc.wrapping_add(4 * TEXCONV.len() as u32), n)); }
    }
}

/// The 21 words of [`crate::jit_xnat::Routine::BitScan`] (clearBitset's loop over a bitset's words, at its head): `lwz
/// r3,152(r24); lwzx r29,r3,r27; cmplwi r29,0; beq next`, the set word's bit loop (a call per set bit, then `stwx r31` clears
/// it), `next: addi r27,r27,4; addi r26,r26,32; addi r25,r25,1; cmplw r25,r28; blt` back to the head.
pub const BITSCAN: [u32; 21] = [
    0x8078_0098, 0x7FA3_D82E, 0x281D_0000, 0x4182_0034, 0x7F5E_D378, 0x6000_0000, 0x57A0_07FF, 0x4182_0010, 0x7F03_C378,
    0x7FC4_F378, 0x4800_09C1, 0x57BD_F87F, 0x3BDE_0001, 0x4082_FFE4, 0x8078_0098, 0x7FE3_D92E, 0x3B7B_0004, 0x3B5A_0020,
    0x3B39_0001, 0x7C19_E040, 0x4180_FFB0,
];
/// One zero word: words 0-3 and 16-20.
pub(crate) const BITSCAN_ZERO: u32 = 9;

/// [`BITSCAN`] at its head `pc`: ZERO words are skipped (r27 += 4, r26 += 32, r25 += 1, CR0 = cmplw r25, r28; r3 = the bitset's
/// base [r24 + 152], r29 = 0) while r25 < r28; the first SET word is loaded (r3, r29, CR0 = cmplwi r29, 0) and the routine
/// leaves at the bit loop (word 4) - the original code runs it, with its calls. No stores: nothing can alias.
pub(crate) fn bitscan(x: &mut X, pc: u32) -> Out {
    let (mut n, mut words) = (0u64, 0u32);
    let stop = |words: u32, n: u64| if words == 0 { None } else { Some((pc, n)) };
    let base_at = x.cpu.gpr[24].wrapping_add(152);
    if !x.in_ram(base_at, 4) { return None; }
    let r3 = x.rd32(base_at);
    let g = &x.cpu.gpr;
    let (mut r25, mut r26, mut r27, r28) = (g[25], g[26], g[27], g[28]);
    let leave = |x: &mut X, r25: u32, r26: u32, r27: u32, r29: u32| { let g = &mut x.cpu.gpr; (g[3], g[25], g[26], g[27], g[29]) = (r3, r25, r26, r27, r29); };
    loop {
        let ea = r3.wrapping_add(r27);
        if n + BITSCAN_ZERO as u64 > x.left || !x.in_ram(ea, 4) {
            // at the head after `words` zero words: the last `cmplw r25,r28` (LT) is CR0
            if words > 0 { leave(x, r25, r26, r27, 0); x.cmplw0(r25, r28); }
            return stop(words, n);
        }
        let w = x.rd32(ea);
        if w != 0 {
            // a set word: its load and test, then on at the bit loop
            leave(x, r25, r26, r27, w);
            x.cmplw0(w, 0);
            n += if x.mutant == crate::jit_mutants::MUT_NAT_BITSCAN_COUNT { 3 } else { 4 };
            return Some((pc.wrapping_add(16), n));
        }
        (r25, r26, r27) = (r25.wrapping_add(1), r26.wrapping_add(32), r27.wrapping_add(4));
        n += BITSCAN_ZERO as u64;
        words += 1;
        if r25 >= r28 {
            leave(x, r25, r26, r27, 0);
            x.cmplw0(r25, r28);
            return Some((pc.wrapping_add(4 * BITSCAN.len() as u32), n));
        }
    }
}

/// srawi rX,rY,31: (the result, XER[CA] - the value negative with a 1 shifted out of its low 31 bits).
#[inline(always)]
fn sraw31(v: u32) -> (u32, bool) { ((v as i32 >> 31) as u32, (v as i32) < 0 && v & 0x7FFF_FFFF != 0) }

/// The 62 words of [`crate::jit_xnat::Routine::AlphaSet`] (alphaBlendMaskRows's unmasked row loop, at its head - h86 heavy: the
/// simulation tick's fog-of-war mask): 4 x `lwz r0,4k(r21); clrlwi r29,r0,17; rlwinm r11,r0,0,1,15; neg r7,r29; and r0,r0,r5;
/// or r29,r7,r29; neg r7,r11; srawi r29,r29,31; or r7,r7,r11; srawi r7,r7,31; and r11,r12,r29; and r7,r6,r7; or r11,r11,r7; or
/// r0,r0,r11; stw r0,4k(r21)`, `addi r21,r21,16; bdnz` - each 16-bit half's top bit set from whether its low 15 bits are not
/// zero (r5 = 0x7FFF7FFF, r12 = 0x8000, r6 = 0x80000000 in the game; any values are reproduced).
pub const ALPHASET: [u32; 62] = [
    0x8015_0000, 0x541D_047E, 0x540B_005E, 0x7CFD_00D0, 0x7C00_2838, 0x7CFD_EB78, 0x7CEB_00D0, 0x7FBD_FE70, 0x7CE7_5B78,
    0x7CE7_FE70, 0x7D8B_E838, 0x7CC7_3838, 0x7D6B_3B78, 0x7C00_5B78, 0x9015_0000, 0x8015_0004, 0x541D_047E, 0x540B_005E,
    0x7CFD_00D0, 0x7C00_2838, 0x7CFD_EB78, 0x7CEB_00D0, 0x7FBD_FE70, 0x7CE7_5B78, 0x7CE7_FE70, 0x7D8B_E838, 0x7CC7_3838,
    0x7D6B_3B78, 0x7C00_5B78, 0x9015_0004, 0x8015_0008, 0x541D_047E, 0x540B_005E, 0x7CFD_00D0, 0x7C00_2838, 0x7CFD_EB78,
    0x7CEB_00D0, 0x7FBD_FE70, 0x7CE7_5B78, 0x7CE7_FE70, 0x7D8B_E838, 0x7CC7_3838, 0x7D6B_3B78, 0x7C00_5B78, 0x9015_0008,
    0x8015_000C, 0x541D_047E, 0x540B_005E, 0x7CFD_00D0, 0x7C00_2838, 0x7CFD_EB78, 0x7CEB_00D0, 0x7FBD_FE70, 0x7CE7_5B78,
    0x7CE7_FE70, 0x7D8B_E838, 0x7CC7_3838, 0x7D6B_3B78, 0x7C00_5B78, 0x9015_000C, 0x3AB5_0010, 0x4200_FF0C,
];
/// One trip of [`ALPHASET`]: every word of it.
pub(crate) const ALPHASET_TRIP: u32 = 62;

/// One word of [`ALPHASET`]: (the word stored, r29, r7, r11, XER[CA] of its second srawi - the later one).
#[inline(always)]
fn alphaset_word(w: u32, r5: u32, r6: u32, r12: u32, mutant: u8) -> (u32, u32, u32, u32, bool) {
    let (a, b) = (w & 0x7FFF, w & 0x7FFF_0000);
    let (t29, ca29) = sraw31(a.wrapping_neg() | a);
    let (t7, ca7) = sraw31(b.wrapping_neg() | b);
    let r7 = r6 & t7;
    let r11 = (r12 & t29) | r7;
    (w & r5 | r11, t29, r7, r11, if mutant == crate::jit_mutants::MUT_NAT_ALPHA_CA { ca29 } else { ca7 })
}

/// [`ALPHASET`] at its head `pc`: CTR trips (CTR = 0 at the head would run 2^32: refused) of 4 words at r21, each rewritten in
/// place in order; trips while the budget holds a whole trip and its 16 bytes are storable (the row checked once: then
/// journaled once). Leaves r0 r7 r11 r29 and XER[CA] of the last word, r21 += 16 a trip, CTR -= trips; the pc after the loop
/// once CTR reaches 0, else the head (a refusal at the first trip is a bail).
pub(crate) fn alphaset(x: &mut X, pc: u32) -> Out {
    let ctr = x.cpu.ctr;
    let g = &x.cpu.gpr;
    let (r5, r6, r12, base) = (g[5], g[6], g[12], g[21]);
    let max = (x.left / ALPHASET_TRIP as u64).min(ctr as u64) as u32;
    if max == 0 { return None; }
    let whole = (max as u64) * 16 <= u32::MAX as u64 && x.storable(base, 16 * max);
    if whole { x.note(base, 16 * max); }
    let mut last = (0, 0, 0, 0, false);
    let mut trips = 0u32;
    while trips < max {
        let at = base.wrapping_add(16 * trips);
        if !whole && !x.storable(at, 16) { break; }
        for k in 0..4u32 {
            let ea = at.wrapping_add(4 * k);
            last = alphaset_word(x.rd32(ea), r5, r6, r12, x.mutant);
            if whole { x.st32(ea, last.0) } else { x.wr32(ea, last.0) }
        }
        trips += 1;
    }
    if trips == 0 { return None; }
    let g = &mut x.cpu.gpr;
    (g[0], g[29], g[7], g[11]) = (last.0, last.1, last.2, last.3);
    g[21] = base.wrapping_add(16 * trips);
    x.set_ca(last.4);
    x.cpu.ctr = ctr - trips;
    let next = if x.cpu.ctr == 0 { pc.wrapping_add(4 * ALPHASET.len() as u32) } else { pc };
    Some((next, ALPHASET_TRIP as u64 * trips as u64))
}

/// The 59 words of [`crate::jit_xnat::Routine::AlphaMask`] (alphaBlendMaskRows's MASKED row loop, at its head): two words a
/// trip, each `lwz r30,j(r4); lwz r27,j(r5)`, its halves' low-15-bit tests (as [`ALPHASET`]) and the mask word's
/// `and r25,r27,r11` / `and r27,r27,r12` tests, four srawi and the merge `andc r28,r30,r23; and r30,r23,r25; or r30,r28,r30;
/// stw r30,j(r4)` (the second word's instructions in another order; `addi r5,r5,8` after its loads), `addi r4,r4,8; bdnz` -
/// a half's top bit, where the mask word's matching bits are set, becomes whether its low 15 bits are not zero.
pub const ALPHAMASK: [u32; 59] = [
    0x83C4_0000, 0x8365_0000, 0x57D7_047E, 0x57D6_005E, 0x7EB7_00D0, 0x7F79_5838, 0x7EB5_BB78, 0x7EF6_00D0, 0x7F59_00D0,
    0x7F7B_6038, 0x7F9B_00D0, 0x7EB5_FE70, 0x7EF7_B378, 0x7F5A_CB78, 0x7EF7_FE70, 0x7F9C_DB78, 0x7F5A_FE70, 0x7C19_A838,
    0x7F15_B838, 0x7F9C_FE70, 0x7C17_D038, 0x7F16_E038, 0x7F39_AB78, 0x7EF7_B378, 0x7FDC_B878, 0x7EFE_C838, 0x7F9E_F378,
    0x93C4_0000, 0x83C4_0004, 0x8365_0004, 0x38A5_0008, 0x57D7_047E, 0x57D6_005E, 0x7F79_5838, 0x7F7B_6038, 0x7EB7_00D0,
    0x7EB5_BB78, 0x7EF6_00D0, 0x7F59_00D0, 0x7F9B_00D0, 0x7EB5_FE70, 0x7EF7_B378, 0x7F5A_CB78, 0x7F9C_DB78, 0x7EF7_FE70,
    0x7C19_A838, 0x7F5A_FE70, 0x7F15_B838, 0x7F9C_FE70, 0x7C17_D038, 0x7F16_E038, 0x7F39_AB78, 0x7EF7_B378, 0x7FDC_B878,
    0x7EFE_C838, 0x7F9E_F378, 0x93C4_0004, 0x3884_0008, 0x4200_FF18,
];
pub(crate) const ALPHAMASK_TRIP: u32 = 59;

/// One word of [`ALPHAMASK`] (`d` the destination word, `m` the mask word): r21 r22 r23 r25 r26 r27 r28 r30 (= the word
/// stored) and XER[CA] of its last srawi (r28's).
#[inline(always)]
fn alphamask_word(d: u32, m: u32, r0: u32, r11: u32, r12: u32, r24: u32) -> ([u32; 8], bool) {
    let (a, b) = (d & 0x7FFF, d & 0x7FFF_0000);
    let (ma, mb) = (m & r11, m & r12);
    let (t21, _) = sraw31(a.wrapping_neg() | a);
    let (t23, _) = sraw31(b.wrapping_neg() | b);
    let (t26, _) = sraw31(ma.wrapping_neg() | ma);
    let (t28, ca) = sraw31(mb.wrapping_neg() | mb);
    let r21 = r24 & t23;
    let r22 = r24 & t28;
    let r23 = (r0 & t26) | r22;
    let r25 = (r0 & t21) | r21;
    let r28 = d & !r23;
    ([r21, r22, r23, r25, t26, mb, r28, r28 | (r23 & r25)], ca)
}

/// [`ALPHAMASK`] at its head `pc`: CTR trips (0 refused, as [`alphaset`]) of 2 words, destination r4 (rewritten in place) and
/// mask r5, each word read and written in the original's order; a trip runs while the budget holds it and its 8 destination
/// bytes are storable and its 8 mask bytes in RAM. A row whose destination does not overlap its mask is checked once (then
/// journaled once); else trip by trip, word by word on real memory. Leaves the last word's r21 r22 r23 r25 r26 r27 r28 r30
/// and XER[CA], r4 and r5 += 8 a trip, CTR -= trips; the pc after the loop once CTR reaches 0, else the head.
pub(crate) fn alphamask(x: &mut X, pc: u32) -> Out {
    let ctr = x.cpu.ctr;
    let g = &x.cpu.gpr;
    let (r0, r11, r12, r24, dst, src) = (g[0], g[11], g[12], g[24], g[4], g[5]);
    let max = (x.left / ALPHAMASK_TRIP as u64).min(ctr as u64) as u32;
    if max == 0 { return None; }
    let span = 8 * max as u64;
    let whole = span <= u32::MAX as u64 && x.storable(dst, 8 * max) && x.in_ram(src, 8 * max) && !crate::jit_xnat::overlap(dst, 8 * max, src, 8 * max);
    if whole { x.note(dst, 8 * max); }
    let mut last = ([0u32; 8], false);
    let mut trips = 0u32;
    while trips < max {
        let (d, m) = (dst.wrapping_add(8 * trips), src.wrapping_add(8 * trips));
        if !whole && !(x.storable(d, 8) && x.in_ram(m, 8)) { break; }
        for k in 0..2u32 {
            let (de, me) = (d.wrapping_add(4 * k), m.wrapping_add(4 * k));
            last = alphamask_word(x.rd32(de), x.rd32(me), r0, r11, r12, r24);
            if whole { x.st32(de, last.0[7]) } else { x.wr32(de, last.0[7]) }
        }
        trips += 1;
    }
    if trips == 0 { return None; }
    let g = &mut x.cpu.gpr;
    let r = last.0;
    (g[21], g[22], g[23], g[25], g[26], g[27], g[28], g[30]) = (r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]);
    (g[4], g[5]) = (dst.wrapping_add(8 * trips), src.wrapping_add(8 * trips));
    x.set_ca(last.1);
    x.cpu.ctr = ctr - trips;
    let next = if x.cpu.ctr == 0 { pc.wrapping_add(4 * ALPHAMASK.len() as u32) } else { pc };
    Some((next, ALPHAMASK_TRIP as u64 * trips as u64))
}

/// The 101 words of [`crate::jit_xnat::Routine::TileExpand`] (terrainTileBlend's 4 x 4 corner expansion of one dirty tile, at
/// its head - the word after the 5-column average loop's bdnz): 16 words that load the tile's step r0 ([r27]), the averages'
/// base r4 = r1 + 120, r5 = 0, the row step r17 = r31 * r0 and the destination r6 from the tile's column / row ([r1 + 56],
/// [r1 + 60]) and the terrain map's origin and base ([r28 + 580 / 584 / 604]); then 4 rows (r5 < 4) of `li r8,4; mr r7,r6; mr
/// r3,r4; mtctr r8` and 4 trips of 76 words: four average bytes at r3 + 0 / 1 / 8 / 9 as 8.8 fixed point, the corner deltas
/// (srawi 2), 16 bytes (srawi 8) stored as four rows of 4 at r7, r7 + r31, + 2 r31, + 3 r31, r7 += r0; then `addi r5,r5,1;
/// add r6,r6,r17; cmpwi r5,4; addi r4,r4,1; blt` - 44 srawi a trip, each an 8-instruction carry in JIT code.
pub const TILEEXPAND: [u32; 101] = [
    0x801B_0000, 0x3881_0078, 0x8061_0038, 0x38A0_0000, 0x80E1_003C, 0x7E3F_01D6, 0x80DC_0244, 0x813C_025C, 0x811C_0248,
    0x7C63_01D6, 0x7C66_1850, 0x7CC7_01D6, 0x7C63_F9D6, 0x7CE8_3050, 0x7CC3_4A14, 0x7CC7_3214, 0x3900_0004, 0x7CC7_3378,
    0x7C83_2378, 0x7D09_03A6, 0x8923_0000, 0x7D07_FA14, 0x8983_0001, 0x8963_0008, 0x5529_402E, 0x8943_0009, 0x558C_402E,
    0x7E49_6050, 0x556B_402E, 0x554C_402E, 0x3863_0008, 0x7E4A_1670, 0x7E49_5850, 0x7D8B_6050, 0x7D8C_1670, 0x7E50_1670,
    0x7D32_4670, 0x7D6B_6214, 0x7DE9_8214, 0x9A47_0000, 0x7DF2_4670, 0x7D29_5214, 0x7DEF_8214, 0x9A47_0001, 0x7DF3_4670,
    0x7E49_5850, 0x7DEF_8214, 0x9A67_0002, 0x7DF3_4670, 0x7D6B_6214, 0x7E50_1670, 0x9A67_0003, 0x7D32_4670, 0x7CE7_0214,
    0x7DE9_8214, 0x9A48_0000, 0x7DF2_4670, 0x7D29_5214, 0x7DEF_8214, 0x9A48_0001, 0x7DF3_4670, 0x7E49_5850, 0x7DEF_8214,
    0x9A68_0002, 0x7DF3_4670, 0x7D6B_6214, 0x7E50_1670, 0x9A68_0003, 0x7D2C_4670, 0x7D08_FA14, 0x7DE9_8214, 0x9988_0000,
    0x7DEC_4670, 0x7D29_5214, 0x7DEF_8214, 0x9988_0001, 0x7DEC_4670, 0x7D49_5850, 0x7DEF_8214, 0x9988_0002, 0x7DEB_4670,
    0x7D4C_1670, 0x9968_0003, 0x7D2A_4670, 0x7D08_FA14, 0x7D69_6214, 0x9948_0000, 0x7D69_4670, 0x7D6B_6214, 0x9928_0001,
    0x7D69_4670, 0x7D6B_6214, 0x9928_0002, 0x7D69_4670, 0x9928_0003, 0x4200_FED4, 0x38A5_0001, 0x7CC6_8A14, 0x2C05_0004,
    0x3884_0001, 0x4180_FEB0,
];
/// The prefix (16), one row (4 + 4 trips of 76 + 5).
pub(crate) const TILE_PREFIX: u32 = 16;
pub(crate) const TILE_ROW: u32 = 4 + 4 * 76 + 5;
/// The word index of the row loop's head (`li r8,4`).
const TILE_HEAD: u32 = 16;

/// srawi rA,rS,n (n < 32): the result and XER[CA].
#[inline(always)]
fn srawi(v: u32, n: u32) -> (u32, bool) { ((v as i32 >> n) as u32, (v as i32) < 0 && v & ((1u32 << n) - 1) != 0) }

/// One trip's 16 bytes as four rows (each row's bytes in memory order, the first in the low byte) from its four average bytes as the original computes them: row k's left edge l = a << 8 +
/// k (b - a) >> 2 ... (each a step of srawi 2 of the 8.8 differences), its step d = (right - left) >> 2, byte j = (l + j d)
/// >> 8 - arithmetic shifts, the edges stepped as the original steps them (`tile_trip` is the word-for-word form).
#[inline(always)]
fn tile_rows(b: [u32; 4]) -> [u32; 4] {
    #[cfg(target_feature = "sse2")]
    { tile_rows_sse2(b) }
    #[cfg(not(target_feature = "sse2"))]
    { tile_rows_scalar(b) }
}

/// [`tile_rows`] four rows at a time in SSE2 lanes (lane k = row k): the edges L and R, the steps D = (R - L) >> 2, the
/// columns L + j D >> 8 masked to a byte and shifted into byte j of the lane - the lanes ARE the rows' four bytes.
#[cfg(target_feature = "sse2")]
#[inline(always)]
fn tile_rows_sse2(b: [u32; 4]) -> [u32; 4] {
    use core::arch::x86_64::*;
    let (l0, r0) = ((b[0] << 8) as i32, (b[2] << 8) as i32);
    let dl = ((b[1] << 8) as i32).wrapping_sub(l0) >> 2;
    let dr = ((b[3] << 8) as i32).wrapping_sub(r0) >> 2;
    let mut o = [0u32; 4];
    // SAFETY: SSE2 is enabled for this build (cfg above); the store writes the 16 bytes of `o`.
    unsafe {
        let l = _mm_setr_epi32(l0, l0.wrapping_add(dl), l0.wrapping_add(dl.wrapping_mul(2)), l0.wrapping_add(dl.wrapping_mul(3)));
        let r = _mm_setr_epi32(r0, r0.wrapping_add(dr), r0.wrapping_add(dr.wrapping_mul(2)), r0.wrapping_add(dr.wrapping_mul(3)));
        let d = _mm_srai_epi32(_mm_sub_epi32(r, l), 2);
        let ff = _mm_set1_epi32(0xFF);
        let x1 = _mm_add_epi32(l, d);
        let x2 = _mm_add_epi32(x1, d);
        let x3 = _mm_add_epi32(x2, d);
        let b0 = _mm_and_si128(_mm_srai_epi32(l, 8), ff);
        let b1 = _mm_slli_epi32(_mm_and_si128(_mm_srai_epi32(x1, 8), ff), 8);
        let b2 = _mm_slli_epi32(_mm_and_si128(_mm_srai_epi32(x2, 8), ff), 16);
        let b3 = _mm_slli_epi32(_mm_srai_epi32(x3, 8), 24);
        let w = _mm_or_si128(_mm_or_si128(b0, b1), _mm_or_si128(b2, b3));
        _mm_storeu_si128(o.as_mut_ptr() as *mut __m128i, w);
    }
    o
}

/// [`tile_rows`] one row and byte at a time (the build without SSE2; the tests hold the two equal).
#[cfg(any(test, not(target_feature = "sse2")))]
#[inline(always)]
fn tile_rows_scalar(b: [u32; 4]) -> [u32; 4] {
    let (l0, r0) = ((b[0] << 8) as i32, (b[2] << 8) as i32);
    let dl = ((b[1] << 8) as i32).wrapping_sub(l0) >> 2;
    let dr = ((b[3] << 8) as i32).wrapping_sub(r0) >> 2;
    let mut o = [0u32; 4];
    let (mut l, mut r) = (l0, r0);
    for row in o.iter_mut() {
        let d = r.wrapping_sub(l) >> 2;
        let mut v = l;
        for j in 0..4 { *row |= ((v >> 8) as u32 & 0xFF) << (8 * j); v = v.wrapping_add(d); }
        l = l.wrapping_add(dl);
        r = r.wrapping_add(dr);
    }
    o
}

/// One trip of [`TILEEXPAND`]'s inner loop on its four average bytes (r3 + 0 / 1 / 8 / 9), word for word on locals: the 16
/// bytes in store order (rows of 4), r9 r10 r11 r12 r15 r16 r18 r19 after it, and XER[CA] of its last srawi (every earlier
/// srawi's carry is overwritten before anything reads it).
#[inline(always)]
fn tile_trip(b: [u32; 4]) -> ([u8; 16], [u32; 8], bool) {
    let s = |v: u32, n: u32| srawi(v, n).0;
    let mut o = [0u8; 16];
    let (mut r9, mut r12, mut r11, mut r10) = (b[0], b[1], b[2], b[3]);
    r9 <<= 8;
    r12 <<= 8;
    let mut r18 = r12.wrapping_sub(r9);
    r11 <<= 8;
    r12 = r10 << 8;
    r10 = s(r18, 2);
    r18 = r11.wrapping_sub(r9);
    r12 = r12.wrapping_sub(r11);
    r12 = s(r12, 2);
    let mut r16 = s(r18, 2);
    r18 = s(r9, 8);
    r11 = r11.wrapping_add(r12);
    let mut r15 = r9.wrapping_add(r16);
    o[0] = r18 as u8;
    r18 = s(r15, 8);
    r9 = r9.wrapping_add(r10);
    r15 = r15.wrapping_add(r16);
    o[1] = r18 as u8;
    let mut r19 = s(r15, 8);
    r18 = r11.wrapping_sub(r9);
    r15 = r15.wrapping_add(r16);
    o[2] = r19 as u8;
    r19 = s(r15, 8);
    r11 = r11.wrapping_add(r12);
    r16 = s(r18, 2);
    o[3] = r19 as u8;
    r18 = s(r9, 8);
    r15 = r9.wrapping_add(r16);
    o[4] = r18 as u8;
    r18 = s(r15, 8);
    r9 = r9.wrapping_add(r10);
    r15 = r15.wrapping_add(r16);
    o[5] = r18 as u8;
    r19 = s(r15, 8);
    r18 = r11.wrapping_sub(r9);
    r15 = r15.wrapping_add(r16);
    o[6] = r19 as u8;
    r19 = s(r15, 8);
    r11 = r11.wrapping_add(r12);
    r16 = s(r18, 2);
    o[7] = r19 as u8;
    r12 = s(r9, 8);
    r15 = r9.wrapping_add(r16);
    o[8] = r12 as u8;
    r12 = s(r15, 8);
    r9 = r9.wrapping_add(r10);
    r15 = r15.wrapping_add(r16);
    o[9] = r12 as u8;
    r12 = s(r15, 8);
    r10 = r11.wrapping_sub(r9);
    r15 = r15.wrapping_add(r16);
    o[10] = r12 as u8;
    r11 = s(r15, 8);
    r12 = s(r10, 2);
    o[11] = r11 as u8;
    r10 = s(r9, 8);
    r11 = r9.wrapping_add(r12);
    o[12] = r10 as u8;
    r9 = s(r11, 8);
    r11 = r11.wrapping_add(r12);
    o[13] = r9 as u8;
    r9 = s(r11, 8);
    r11 = r11.wrapping_add(r12);
    o[14] = r9 as u8;
    let (r9, ca) = srawi(r11, 8);
    o[15] = r9 as u8;
    (o, [r9, r10, r11, r12, r15, r16, r18, r19], ca)
}

/// [`TILEEXPAND`] at its head `pc`: the prefix (its loads inside RAM, else a bail), then rows while r5 < 4 - each validated
/// first (its 34 average bytes inside RAM, its 16 four-byte destination runs storable) and the budget holding a whole row - with
/// every trip a word-for-word transcription on registers (its four loads first, then its 16 byte stores in the original's
/// order on real memory: a destination over the averages is read back as the original reads it). Leaves every register the
/// words write, CTR, XER[CA] (the last srawi's), CR0 = cmpwi r5,4; the pc after the routine after the last row, the row head
/// (word 16) after a refusal at a later row.
pub(crate) fn tileexpand(x: &mut X, pc: u32) -> Out {
    let mut r = x.cpu.gpr;
    let ld = |x: &X, ea: u32| if x.in_ram(ea, 4) { Some(x.rd32(ea)) } else { None };
    if (TILE_PREFIX + TILE_ROW) as u64 > x.left { return None; }
    let (Some(a0), Some(a3), Some(a7), Some(a6), Some(a9), Some(a8)) = (ld(x, r[27]), ld(x, r[1].wrapping_add(56)), ld(x, r[1].wrapping_add(60)),
        ld(x, r[28].wrapping_add(580)), ld(x, r[28].wrapping_add(604)), ld(x, r[28].wrapping_add(584))) else { return None };
    // the prefix, word by word
    r[0] = a0;
    r[4] = r[1].wrapping_add(120);
    r[3] = a3;
    r[5] = 0;
    r[7] = a7;
    r[17] = r[31].wrapping_mul(r[0]);
    r[6] = a6;
    r[9] = a9;
    r[8] = a8;
    r[3] = r[3].wrapping_mul(r[0]);
    r[3] = r[3].wrapping_sub(r[6]);
    r[6] = r[7].wrapping_mul(r[0]);
    r[3] = r[3].wrapping_mul(r[31]);
    r[7] = r[6].wrapping_sub(r[8]);
    r[6] = r[3].wrapping_add(r[9]);
    r[6] = r[7].wrapping_add(r[6]);
    let mut n = TILE_PREFIX as u64;
    let mut ca = x.cpu.xer & crate::cpu::XER_CA != 0;
    let mut ctr = x.cpu.ctr;
    loop {
        // validate the row: its averages and its 16 destination runs
        // (steps under 64 KiB: the runs lie in one box from r6, tested at once - stricter than the runs themselves)
        let small = r[0] < 1 << 16 && r[31] < 1 << 16;
        let ok = n + TILE_ROW as u64 <= x.left && x.in_ram(r[4], 34)
            && ((small && x.storable(r[6], 3 * r[0] + 3 * r[31] + 4))
                || (0..4u32).all(|t| (0..4u32).all(|k| x.storable(r[6].wrapping_add(t.wrapping_mul(r[0])).wrapping_add(k.wrapping_mul(r[31])), 4))));
        if !ok {
            x.cpu.gpr = r;
            x.cpu.ctr = ctr;
            x.set_ca(ca);
            return Some((pc.wrapping_add(4 * TILE_HEAD), n));
        }
        // the row's 16 runs journaled once (lockstep only), then stored without a test each
        if x.journaling() { for t in 0..4u32 { for k in 0..4u32 { x.note(r[6].wrapping_add(t.wrapping_mul(r[0])).wrapping_add(k.wrapping_mul(r[31])), 4); } } }
        r[8] = 4;
        r[7] = r[6];
        r[3] = r[4];
        ctr = r[8];
        let mut b = [0u32; 4];
        for _ in 0..4 {
            b = [x.rd8(r[3]), x.rd8(r[3].wrapping_add(1)), x.rd8(r[3].wrapping_add(8)), x.rd8(r[3].wrapping_add(9))];
            let out = tile_rows(b);
            // the 16 stores in the original's order: rows r7, r7 + r31, + 2 r31, + 3 r31, 4 bytes each (a row's 4 bytes are
            // distinct addresses, so one 4-byte write each, in row order, leaves what the byte stores leave even where rows
            // or trips overlap)
            for k in 0..4u32 { x.st4_bytes(r[7].wrapping_add(k.wrapping_mul(r[31])), out[k as usize]); }
            r[8] = r[7].wrapping_add(r[31].wrapping_mul(3));
            r[7] = r[7].wrapping_add(r[0]);
            r[3] = r[3].wrapping_add(8);
            ctr = ctr.wrapping_sub(1);
        }
        // the row's last trip leaves the registers (its word-for-word transcription, once a row)
        let (_, v, c) = tile_trip(b);
        (r[9], r[10], r[11], r[12], r[15], r[16], r[18], r[19]) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
        ca = c;
        // addi r5,r5,1; add r6,r6,r17; cmpwi r5,4; addi r4,r4,1; blt
        r[5] = r[5].wrapping_add(1);
        r[6] = r[6].wrapping_add(r[17]);
        x.cmpw0(r[5], 4);
        r[4] = r[4].wrapping_add(1);
        n += TILE_ROW as u64 - (x.mutant == crate::jit_mutants::MUT_NAT_TILE_COUNT) as u64;
        if (r[5] as i32) >= 4 {
            x.cpu.gpr = r;
            x.cpu.ctr = ctr;
            x.set_ca(ca);
            return Some((pc.wrapping_add(4 * TILEEXPAND.len() as u32), n));
        }
    }
}

#[cfg(test)]
mod tests {
    /// The tile expansion's closed form gives the word-for-word transcription's 16 bytes for every corner pattern sampled (the
    /// extremes and a pseudo-random sweep of all four bytes).
    #[test]
    fn tile_rows_agree_with_the_transcription() {
        let mut s = 0x1234_5678u32;
        let mut cases: alloc::vec::Vec<[u32; 4]> = alloc::vec::Vec::new();
        for a in [0u32, 1, 127, 128, 254, 255] { for b in [0u32, 3, 128, 255] { cases.push([a, b, 255 - a, 255 - b]); cases.push([a, a, b, b]); } }
        for _ in 0..200_000 {
            s ^= s << 13; s ^= s >> 17; s ^= s << 5;
            cases.push([s & 0xFF, s >> 8 & 0xFF, s >> 16 & 0xFF, s >> 24]);
        }
        for b in cases {
            let rows = super::tile_rows(b);
            assert_eq!(rows, super::tile_rows_scalar(b), "{b:?}");
            let bytes: alloc::vec::Vec<u8> = rows.iter().flat_map(|w| w.to_le_bytes()).collect();
            assert_eq!(bytes[..], super::tile_trip(b).0[..], "{b:?}");
        }
    }
}
