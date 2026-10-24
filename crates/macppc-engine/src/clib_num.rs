//! `strtol` / `strtoul` / `atoi` number parsing over a byte string (ISO C 7.20.1.4), with the classic Mac's 32-bit `long`.

fn digit(c: u8) -> Option<u32> {
    match c { b'0'..=b'9' => Some(u32::from(c - b'0')), b'a'..=b'z' => Some(u32::from(c - b'a') + 10), b'A'..=b'Z' => Some(u32::from(c - b'A') + 10), _ => None }
}
fn is_space(c: u8) -> bool { matches!(c, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r') }

/// (value, bytes consumed, overflowed). `unsigned` selects strtoul's range; a minus sign negates modulo 2^32 there. No digits: (0, 0, false).
pub fn strtol_full(s: &[u8], base: u32, unsigned: bool) -> (u32, usize, bool) {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) { i += 1; }
    let neg = match s.get(i) { Some(b'-') => { i += 1; true } Some(b'+') => { i += 1; false } _ => false };
    let mut base = base;
    let has_0x = s.get(i) == Some(&b'0') && matches!(s.get(i + 1), Some(b'x' | b'X')) && s.get(i + 2).and_then(|&c| digit(c)).is_some_and(|d| d < 16);
    if (base == 0 || base == 16) && has_0x { i += 2; base = 16; }
    else if base == 0 { base = if s.get(i) == Some(&b'0') { 8 } else { 10 }; }
    if !(2..=36).contains(&base) { return (0, 0, false); }
    let start = i;
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = s.get(i).and_then(|&c| digit(c)).filter(|&d| d < base) {
        v = v.saturating_mul(u64::from(base)).saturating_add(u64::from(d));
        if v > u64::from(u32::MAX) * 2 { over = true; v = u64::from(u32::MAX) * 2; }
        i += 1;
    }
    if i == start { return (0, 0, false); }
    if unsigned {
        if v > u64::from(u32::MAX) { return (u32::MAX, i, true); }
        let r = v as u32;
        (if neg { r.wrapping_neg() } else { r }, i, over)
    } else if neg {
        if v > 0x8000_0000 { (0x8000_0000, i, true) } else { ((v as i64).wrapping_neg() as i32 as u32, i, false) }
    } else if v > 0x7FFF_FFFF { (0x7FFF_FFFF, i, true) } else { (v as u32, i, false) }
}

/// `atoi`/`atol`: base 10, no endptr: (value, bytes used).
pub fn strtol(s: &[u8], base: u32) -> (i32, usize) { let (v, n, _) = strtol_full(s, base, false); (v as i32, n) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_like_the_c_library() {
        assert_eq!(strtol_full(b"  -42xyz", 10, false), (-42i32 as u32, 5, false));
        assert_eq!(strtol_full(b"0x1F", 0, false), (31, 4, false));
        assert_eq!(strtol_full(b"0x", 0, false), (0, 1, false), "'0' alone: the x has no digit after it");
        assert_eq!(strtol_full(b"077", 0, false), (63, 3, false));
        assert_eq!(strtol_full(b"zz", 36, false), (35 * 36 + 35, 2, false));
        assert_eq!(strtol_full(b"abc", 10, false), (0, 0, false), "no digits: nothing consumed");
        assert_eq!(strtol_full(b"99999999999", 10, false), (0x7FFF_FFFF, 11, true), "LONG_MAX with ERANGE");
        assert_eq!(strtol_full(b"-99999999999", 10, false), (0x8000_0000, 12, true));
        assert_eq!(strtol_full(b"4294967295", 10, true), (u32::MAX, 10, false));
        assert_eq!(strtol_full(b"4294967296", 10, true), (u32::MAX, 10, true));
        assert_eq!(strtol_full(b"-1", 10, true), (u32::MAX, 2, false), "strtoul negates modulo 2^32");
        assert_eq!(strtol(b"12", 10), (12, 2));
    }
}
