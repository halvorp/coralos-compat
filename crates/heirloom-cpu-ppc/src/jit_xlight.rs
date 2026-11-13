//! Native routine (jit_xnat) for updateLightState's per-light loop - its UNCHANGED-LIGHT path: Warcraft III's GL state cache
//! asks, for each of 8 lights, whether the light's 4x4 float matrix equals the cached one (a call of jit_xlate's CMP16F, the
//! matrix compare, ~2,000 of its ~2,200 calls a frame come from here), and when it does and the light needs no GL call, the
//! iteration only steps its pointers. That path - 93 guest instructions, two dispatches through the IBTC, the matrix compare's
//! block - runs here as one Rust step per light.
//!
//! A light it does not take (the matrices differ in any bit, a float with exponent 0xFF - CMP16F's own fast-path rule -, the
//! light's dirty word [r30 + 1252] set, the loop's GL flag r26 set, an aliasing store) stops the routine at the loop head:
//! the original code runs that light (its first light: a bail).
use crate::fpmem::{double_to_single, single_to_double};
use crate::jit_xnat::{overlap, Out, X};

/// The 302 words of [`crate::jit_xnat::Routine::Lights`] (the loop from its head to its `bne` back; word 7 is the `bl` to
/// CMP16F, whose 68 words recognise() also checks at the call's target).
pub const LIGHTS: [u32; 302] = [
    0xD3E1_0038, 0x3B19_4000, 0x387C_06F4, 0x3897_0138, 0xD3E1_003C, 0xD3E1_0040, 0xD3E1_0044, 0x4BFA_E9F1, 0x2C03_0000,
    0x4182_0084, 0xC017_0138, 0xD01C_06F4, 0xC017_013C, 0xD01C_06F8, 0xC017_0140, 0xD01C_06FC, 0xC017_0144, 0xD01C_0700,
    0xC017_0148, 0xD01C_0704, 0xC017_014C, 0xD01C_0708, 0xC017_0150, 0xD01C_070C, 0xC017_0154, 0xD01C_0710, 0xC017_0158,
    0xD01C_0714, 0xC017_015C, 0xD01C_0718, 0xC017_0160, 0xD01C_071C, 0xC017_0164, 0xD01C_0720, 0xC017_0168, 0xD01C_0724,
    0xC017_016C, 0xD01C_0728, 0xC017_0170, 0xD01C_072C, 0xC017_0174, 0xD01C_0730, 0x801E_04E4, 0x2C00_0000, 0x4082_000C,
    0x2C03_0000, 0x4182_01A4, 0x801D_0248, 0x2C00_0000, 0x4182_0198, 0x2C03_0000, 0x4082_0054, 0x807D_03C8, 0x801D_024C,
    0x7C03_0000, 0x4082_0044, 0xC03D_03CC, 0x3800_0000, 0xC01D_0250, 0xFC01_0000, 0x4082_0024, 0xC03D_03D0, 0xC01D_0254,
    0xFC01_0000, 0x4082_0014, 0xC03D_03D4, 0xC01D_0258, 0xFC01_0000, 0x4182_0008, 0x3800_0001, 0x5400_063F, 0x4182_0140,
    0x2C1B_0000, 0x4082_0014, 0x7EE3_BB78, 0x3897_0138, 0x4800_16CD, 0x3B60_0001, 0x801D_024C, 0x2C00_0000, 0x4182_0028,
    0xC07D_0258, 0xC05D_0254, 0xC03D_0250, 0xC01F_0014, 0xD021_0038, 0xD041_003C, 0xD061_0040, 0xD001_0044, 0x4800_00E4,
    0xC03D_0254, 0xC01D_0250, 0xFC40_0850, 0xC03D_0258, 0xFC00_0050, 0xC09F_0000, 0xFC60_0850, 0xC03F_0014, 0xD001_0038,
    0xC01F_0018, 0xD041_003C, 0xC0A1_0038, 0xC041_003C, 0xD061_0040, 0xEC65_0172, 0xEC42_00B2, 0xC0A1_0040, 0xD081_0044,
    0xEC43_102A, 0xEC85_0172, 0xC061_0044, 0xEC63_00F2, 0xEC44_102A, 0xEC43_102A, 0xEC22_0828, 0xFC20_0A10, 0xFC20_0818,
    0xFC01_0040, 0x7C00_0026, 0x5400_0FFF, 0x4082_0068, 0xC001_0038, 0xC021_003C, 0xEC00_0032, 0xC041_0040, 0xC061_0044,
    0xEC01_007A, 0xEC02_00BA, 0xEC23_00FA, 0x485E_0B3D, 0x8041_0014, 0xFC20_0818, 0xC01F_0014, 0xC061_0038, 0xC041_003C,
    0xEC80_0824, 0xC021_0040, 0xC001_0044, 0xEC63_0132, 0xEC42_0132, 0xEC21_0132, 0xEC00_0132, 0xD061_0038, 0xD041_003C,
    0xD021_0040, 0xD001_0044, 0x7F03_C378, 0x38A1_0038, 0x3880_1203, 0x485E_0AD5, 0x8041_0014, 0x801E_04E4, 0x2C00_0000,
    0x4182_020C, 0x801D_03C4, 0x807D_0248, 0x7C00_1800, 0x4182_0028, 0x2C03_0000, 0x4182_0014, 0x7F03_C378, 0x485E_05C5,
    0x8041_0014, 0x4800_0010, 0x7F03_C378, 0x485E_05CD, 0x8041_0014, 0x801D_0248, 0x2C00_0000, 0x4182_017C, 0x807D_03D8,
    0x801D_025C, 0x7C03_0040, 0x4082_0014, 0xC03D_03E0, 0xC01D_0264, 0xFC01_0000, 0x4182_00A0, 0x895D_025C, 0x3D00_4330,
    0x88FD_025F, 0x7F03_C378, 0x88DD_025E, 0x38A1_0038, 0x881D_025D, 0x3880_1200, 0x8122_1044, 0x9141_00E4, 0xC03F_000C,
    0x9101_00E0, 0xC01D_0264, 0x90E1_00DC, 0xC869_0000, 0xECA1_0032, 0x9101_00D8, 0xC821_00E0, 0xC801_00D8, 0xEC41_1828,
    0x90C1_00D4, 0xEC20_1828, 0x9101_00D0, 0xEC82_0172, 0xC801_00D0, 0xEC41_0172, 0x9001_00CC, 0xEC20_1828, 0x9101_00C8,
    0xC801_00C8, 0xEC21_0172, 0xD041_0040, 0xEC00_1828, 0xD021_003C, 0xEC00_0172, 0xD081_0044, 0xD001_0038, 0x485E_09CD,
    0x8041_0014, 0x807D_03DC, 0x801D_0260, 0x7C03_0040, 0x4082_0014, 0xC03D_03E4, 0xC01D_0268, 0xFC01_0000, 0x4182_00A0,
    0x895D_0260, 0x3D00_4330, 0x88FD_0263, 0x7F03_C378, 0x88DD_0262, 0x38A1_0038, 0x881D_0261, 0x3880_1201, 0x8122_1044,
    0x9141_00CC, 0xC03F_000C, 0x9101_00C8, 0xC01D_0268, 0x90E1_00D4, 0xC869_0000, 0xECA1_0032, 0x9101_00D0, 0xC821_00C8,
    0xC801_00D0, 0xEC41_1828, 0x90C1_00DC, 0xEC20_1828, 0x9101_00D8, 0xEC82_0172, 0xC801_00D8, 0xEC41_0172, 0x9001_00E4,
    0xEC20_1828, 0x9101_00E0, 0xC801_00E0, 0xEC21_0172, 0xD041_0040, 0xEC00_1828, 0xD021_003C, 0xEC00_0172, 0xD081_0044,
    0xD001_0038, 0x485E_0911, 0x8041_0014, 0x807D_0248, 0x3800_0000, 0x907D_03C4, 0x807D_024C, 0x907D_03C8, 0x809D_0250,
    0x807D_0254, 0x909D_03CC, 0x907D_03D0, 0x807D_0258, 0x907D_03D4, 0x807D_025C, 0x907D_03D8, 0x807D_0260, 0x907D_03DC,
    0xC01D_0264, 0xD01D_03E0, 0xC01D_0268, 0xD01D_03E4, 0x901E_04E4, 0x2C1A_0000, 0x4182_002C, 0xC037_0388, 0x7F03_C378,
    0x3880_1208, 0x485E_08D5, 0x8041_0014, 0xC037_038C, 0x7F03_C378, 0x3880_1209, 0x485E_08C1, 0x8041_0014, 0x3B39_0001,
    0x3BBD_0024, 0x2819_0008, 0x3B9C_0040, 0x3BDE_0004, 0x4082_FB4C,
];
/// The `bl` to the matrix compare: word 7.
pub(crate) const LIGHTS_BL: u32 = 7;
/// One unchanged light: words 0-7, the compare (67), 8-9, 42-46, 151-153, 284-285, 296-301.
pub(crate) const LIGHT_TRIP: u32 = 8 + 67 + 2 + 5 + 3 + 2 + 6;

/// [`LIGHTS`] at its head `pc`: unchanged lights while r25 (incremented) != 8. Per light: `stfs f31` x 4 to [r1 + 56 ..],
/// r24 = r25 + 16384, r3 = r28 + 1780, r4 = r23 + 312, LR = the word after the `bl`; the compare of the 16 floats at r3 / r4
/// answers 0 (r0 = r3 = 0, f1 / f0 = the 16th pair widened, FPSCR[FPCC] = EQ); [r30 + 1252] (= 0) into r0; r25 += 1, r29 +=
/// 36, r28 += 64, r30 += 4, CR0 = cmplwi r25, 8.
pub(crate) fn lights(x: &mut X, pc: u32) -> Out {
    let g = x.cpu.gpr;
    let stop = |trips: u32| trips == 0;
    // the light's matrix (b) and the stack slots (tmp) are the same for every light; r26 too
    let (tmp, b) = (g[1].wrapping_add(56), g[23].wrapping_add(312));
    let alias_test = x.mutant != crate::jit_mutants::MUT_NAT_LIGHT_ALIAS;
    if g[26] != 0 || !x.storable(tmp, 16) || !x.in_ram(b, 64) || (alias_test && overlap(tmp, 16, b, 64)) { return None; }
    let s31 = double_to_single(x.cpu.fpr[31]);
    let (mut r25, mut r28, mut r29, mut r30) = (g[25], g[28], g[29], g[30]);
    let (mut n, mut trips) = (0u64, 0u32);
    // exponent 0xFF in either big-endian float of 8 bytes read as a little-endian u64: (byte 0 & 0x7F) and (byte 1 & 0x80) set
    const EXP: u64 = 0x0000_807F_0000_807F;
    let done = loop {
        let (a, dirty) = (r28.wrapping_add(1780), r30.wrapping_add(1252));
        if n + LIGHT_TRIP as u64 > x.left || !x.in_ram(a, 64) || !x.in_ram(dirty, 4) { break false; }
        // the four stores come before the compare's loads and the dirty word's: they may reach none of them
        if alias_test && (overlap(tmp, 16, a, 64) || overlap(tmp, 16, dirty, 4)) { break false; }
        if x.rd32(dirty) != 0 { break false; }
        // CMP16F's fast-path rule: the 64 bytes bit-identical, no float with exponent 0xFF (else the original runs this light)
        let same = (0..8u32).all(|k| {
            let (u, v) = (x.raw64(a + 8 * k), x.raw64(b + 8 * k));
            let w = (u & EXP) ^ EXP;
            u == v && w & 0xFFFF != 0 && w & 0xFFFF_0000_0000 != 0
        });
        if !same { break false; }
        if trips == 0 { x.note(tmp, 16); }
        for k in 0..4 { x.st32(tmp + 4 * k, s31); }
        r25 = r25.wrapping_add(1);
        (r28, r29, r30) = (r28.wrapping_add(64), r29.wrapping_add(36), r30.wrapping_add(4));
        n += LIGHT_TRIP as u64;
        trips += 1;
        if r25 == 8 { break true; }
    };
    if stop(trips) { return None; }
    // the registers as the last light's instructions left them: r24 from its r25, the compare's r0 / r3 then [dirty] (0) in r0,
    // f1 / f0 = its 16th pair (CMP16F's lfs), FPSCR[FPCC] = EQ, LR after the bl, CR0 = cmplwi r25, 8
    let last = single_to_double(x.rd32(r28.wrapping_sub(64).wrapping_add(1780 + 60)));
    let c = &mut *x.cpu;
    c.gpr[24] = r25.wrapping_sub(1).wrapping_add(16384);
    (c.gpr[0], c.gpr[3], c.gpr[4]) = (0, 0, b);
    (c.gpr[25], c.gpr[28], c.gpr[29], c.gpr[30]) = (r25, r28, r29, r30);
    c.lr = pc.wrapping_add(4 * (LIGHTS_BL + 1));
    (c.fpr[0], c.fpr[1]) = (last, last);
    c.fpscr = (c.fpscr & !0xF000) | 0x2 << 12;
    x.cmplw0(r25, 8);
    Some((if done { pc.wrapping_add(4 * LIGHTS.len() as u32) } else { pc }, n))
}
