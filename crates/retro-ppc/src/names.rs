//! Names for the opcode groups this interpreter does not execute, so a trap says WHAT, never just "unknown".
pub(crate) fn primary(op: u32) -> &'static str {
    match op {
        0 => "primary opcode 0 (reserved/zero word)",
        1 | 2 | 5 | 6 | 9 | 22 | 30 | 58 | 62 => "64-bit PowerPC / reserved opcode",
        4 => "AltiVec / paired-single (opcode 4)",
        56 | 57 | 60 | 61 => "reserved opcode (POWER2 quad / VSX, not 6xx)",
        _ => "reserved opcode",
    }
}

/// Opcode-31 extended opcodes that are real instructions but outside the user-mode subset, or reserved.
pub(crate) fn x31(xo: u32) -> &'static str {
    match xo {
        83 => "mfmsr (supervisor)", 146 => "mtmsr (supervisor)", 210 | 242 | 306 | 370 | 434 | 466 | 498 | 566 => "TLB/segment management (supervisor)",
        339 | 467 => "mfspr/mtspr", 595 | 659 | 627 | 693 | 723 => "segment register access (supervisor)",
        310 | 438 => "eciwx/ecowx (external control, not in user subset)",
        6 | 7 | 38 | 39 | 71 | 103 | 135 | 167 | 199 | 231 | 342 | 359 | 487 | 822 => "AltiVec load/store (lvx/stvx family)",
        _ => "reserved opcode-31 extended opcode",
    }
}
