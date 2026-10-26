//! The export hash function and table sizing (Mac OS Runtime Architectures, "Export Hash Table").
//!
//! The hash word is `length << 16 | value`, where `value` folds the name with PseudoRotate (`(x << 1) - (x >> 16)`,
//! as Executor's MIT `pef_hash.cpp` has it) and xors the two halves. The slot is `(word ^ (word >> power)) & mask`.

/// Hash word of `name`: length in the high 16 bits, folded hash in the low 16.
pub fn hash_word(name: &[u8]) -> u32 {
    let mut v: i32 = 0;
    for &c in name { v = (v << 1).wrapping_sub(v >> 16) ^ i32::from(c); }
    let v = v as u32;
    ((name.len() as u32) << 16) | ((v ^ (v >> 16)) & 0xFFFF)
}

/// The slot a full hash word lands in for a table of `1 << power` slots.
pub fn slot(word: u32, power: u32) -> u32 {
    let mask = if power >= 32 { u32::MAX } else { (1u32 << power) - 1 };
    (word ^ word.checked_shr(power).unwrap_or(0)) & mask
}

/// The table exponent the format's reference code picks for `count` exported symbols (average chain length 10, at most
/// 2^16 slots); writers use it, readers accept any power.
pub fn table_power(count: usize) -> u32 {
    let mut p = 0;
    while p < 16 && count / (1usize << p) >= 10 { p += 1; }
    p
}
