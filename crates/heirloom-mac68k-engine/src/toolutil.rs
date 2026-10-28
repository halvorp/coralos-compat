//! Toolbox Utilities that work on bytes: `Munger` (find / replace / delete / insert in a Handle), `PackBits` and `UnpackBits`
//! (the run-length scheme MacPaint files and PICT bitmaps use). Inside Macintosh: Toolbox Utilities / OS Utilities; the
//! `Munger` search-and-replace rules and its results (the offset after the replacement, the offset of a match, -1 for none)
//! are the book's, cross-read against Executor's `C_Munger` (MIT).
use crate::machine::{Call, Done, Mac};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register(0xA9E0, &[Arg::P, Arg::L, Arg::P, Arg::L, Arg::P, Arg::L], Some(Arg::L), munger::<U>)?;
    m.register(0xA8CF, &[Arg::P, Arg::P, Arg::W], None, pack_bits::<U>)?;
    m.register(0xA8D0, &[Arg::P, Arg::P, Arg::W], None, unpack_bits::<U>)
}

/// The Munger algorithm over plain bytes: returns the new contents and the function result. `pat` is `ptr1`/`len1`, `rep`
/// is `ptr2`/`len2`; a NIL pointer is `None`. `len1` of a NIL-pattern call is the number of bytes to replace (negative =
/// everything from `offset` on). `None` = the pattern was not found (-1) or `offset` is outside the data.
pub fn munger_bytes(data: &[u8], offset: i32, pat: Option<&[u8]>, len1: i32, rep: Option<&[u8]>, len2: i32) -> Option<(Vec<u8>, i32)> {
    if offset < 0 || offset as usize > data.len() { return None; }
    let mut off = offset as usize;
    let cut = match pat {
        Some(p) if !p.is_empty() => {
            off += data[off..].windows(p.len()).position(|w| w == p)?;
            if rep.is_none() { return Some((data.to_vec(), off as i32)); } // find only
            p.len()
        }
        _ if len1 < 0 => data.len() - off,
        _ => (len1 as usize).min(data.len() - off),
    };
    let rep = rep.map_or(&[][..], |r| &r[..(len2.max(0) as usize).min(r.len())]);
    let mut out = Vec::with_capacity(data.len() - cut + rep.len());
    out.extend_from_slice(&data[..off]);
    out.extend_from_slice(rep);
    out.extend_from_slice(&data[off + cut..]);
    Some((out, (off + rep.len()) as i32))
}

fn munger<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (h, offset, p1, len1, p2, len2) = (c.args.get(0), c.args.get(1) as i32, c.args.get(2), c.args.get(3) as i32, c.args.get(4), c.args.get(5) as i32);
    let data = m.handle_bytes(h).ok_or_else(|| Refusal(format!("Munger: {h:#x} is not a Handle")))?;
    let read = |m: &Mac<U>, p: u32, n: i32| -> Result<Option<Vec<u8>>, Refusal> {
        if p == 0 { return Ok(None); }
        let n = n.max(0) as u32;
        let e = Refusal::mem("Munger data");
        m.mem.span(p, n).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map(Some).map_err(&e)
    };
    let pat = read(m, p1, len1)?;
    let rep = read(m, p2, len2)?;
    let Some((out, result)) = munger_bytes(&data, offset, pat.as_deref(), len1, rep.as_deref(), len2) else {
        return Ok(Done { result: -1i32 as u32, log: format!("[toolbox] Munger {h:#x} @{offset}: no match -> -1") });
    };
    if out != data {
        m.mm.set_handle_size(&mut m.mem, &mut m.heap, h, out.len() as u32).map_err(|e| Refusal(format!("Munger: SetHandleSize failed (MemErr {e})")))?;
        let ptr = m.mm.block(h).map(|b| b.ptr).ok_or_else(|| Refusal(format!("Munger: {h:#x} vanished")))?;
        m.mem.write_bytes(ptr, &out).map_err(Refusal::mem("Munger result"))?;
    }
    Ok(Done { result: result as u32, log: format!("[toolbox] Munger {h:#x} @{offset}: {} -> {} bytes, result {result}", data.len(), out.len()) })
}

/// PackBits: runs of three or more equal bytes become `[1-n, byte]`, everything else is literal groups `[n-1, bytes..]` of
/// at most 128 bytes (a two-byte run inside literals is kept literal, as the book's encoder does).
pub fn pack_bits_bytes(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, lit: &mut Vec<u8>| { for ch in lit.chunks(128) { out.push((ch.len() - 1) as u8); out.extend_from_slice(ch); } lit.clear(); };
    let mut i = 0;
    while i < src.len() {
        let run = src[i..].iter().take(128).take_while(|&&b| b == src[i]).count();
        if run >= 3 { flush(&mut out, &mut lit); out.push((1 - run as i32) as u8); out.push(src[i]); i += run; } else { lit.push(src[i]); i += 1; }
    }
    flush(&mut out, &mut lit);
    out
}

/// UnpackBits of `src` until `want` bytes exist. Returns the bytes and how many source bytes were consumed; `None` if the
/// source ends first or a run would overrun `want` (a damaged row).
pub fn unpack_bits_bytes(src: &[u8], want: usize) -> Option<(Vec<u8>, usize)> {
    let (mut out, mut i) = (Vec::with_capacity(want), 0);
    while out.len() < want {
        let n = *src.get(i)? as i8;
        i += 1;
        if n == -128 { continue; }
        if n >= 0 {
            let k = n as usize + 1;
            out.extend_from_slice(src.get(i..i + k)?);
            i += k;
        } else {
            let k = (1 - i32::from(n)) as usize;
            let b = *src.get(i)?;
            i += 1;
            out.extend(core::iter::repeat_n(b, k));
        }
        if out.len() > want { return None; }
    }
    Some((out, i))
}

fn pack_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (sp, dp, n) = (c.args.get(0), c.args.get(1), c.args.get(2) as u16 as i16);
    let e = Refusal::mem("PackBits");
    let (src, dst) = (m.mem.read_u32(sp).map_err(&e)?, m.mem.read_u32(dp).map_err(&e)?);
    let n = n.max(0) as u32;
    let data = m.mem.span(src, n).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(&e)?;
    let packed = pack_bits_bytes(&data);
    m.mem.write_bytes(dst, &packed).map_err(&e)?;
    m.mem.write_u32(sp, src + n).map_err(&e)?;
    m.mem.write_u32(dp, dst + packed.len() as u32).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[toolbox] PackBits {n} -> {} bytes", packed.len()) })
}

fn unpack_bits<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (sp, dp, n) = (c.args.get(0), c.args.get(1), c.args.get(2) as u16 as i16);
    let e = Refusal::mem("UnpackBits");
    let (src, dst) = (m.mem.read_u32(sp).map_err(&e)?, m.mem.read_u32(dp).map_err(&e)?);
    let want = n.max(0) as usize;
    // The source length is not given: read a generous window (the scheme is self-delimiting) bounded by the end of memory.
    let window = (want as u32 * 2 + 2).min((m.mem.len() as u32).saturating_sub(src));
    let data = m.mem.span(src, window).and_then(|s| m.mem.bytes(s).map(<[u8]>::to_vec)).map_err(&e)?;
    let (out, used) = unpack_bits_bytes(&data, want).ok_or_else(|| Refusal(format!("UnpackBits: the packed row does not expand to exactly {want} bytes")))?;
    m.mem.write_bytes(dst, &out).map_err(&e)?;
    m.mem.write_u32(sp, src + used as u32).map_err(&e)?;
    m.mem.write_u32(dp, dst + want as u32).map_err(&e)?;
    Ok(Done { result: 0, log: format!("[toolbox] UnpackBits {used} -> {want} bytes") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn munger_finds_replaces_inserts_and_deletes_and_says_minus_one_for_no_match() {
        let d = b"hello world, hello";
        // find only (ptr2 NIL): the offset of the first match at or after `offset`, the data untouched
        assert_eq!(munger_bytes(d, 0, Some(b"hello"), 5, None, 0), Some((d.to_vec(), 0)));
        assert_eq!(munger_bytes(d, 1, Some(b"hello"), 5, None, 0), Some((d.to_vec(), 13)));
        // replace: the result is the offset after the replacement
        assert_eq!(munger_bytes(d, 0, Some(b"world"), 5, Some(b"there!"), 6), Some((b"hello there!, hello".to_vec(), 12)));
        // insert (len1 0, NIL pattern) and delete (NIL ptr2): by offset
        assert_eq!(munger_bytes(b"abcd", 2, None, 0, Some(b"XY"), 2), Some((b"abXYcd".to_vec(), 4)));
        assert_eq!(munger_bytes(b"abcd", 1, None, 2, None, 0), Some((b"ad".to_vec(), 1)));
        // a negative len1 with a NIL pattern replaces everything from offset on
        assert_eq!(munger_bytes(b"abcd", 1, None, -1, Some(b"Z"), 1), Some((b"aZ".to_vec(), 2)));
        // NEGATIVE CONTROLS: no match is None (-1), an offset outside the data is None, a pattern is not matched across the end
        assert_eq!(munger_bytes(d, 0, Some(b"xyz"), 3, None, 0), None);
        assert_eq!(munger_bytes(b"abc", 9, None, 0, Some(b"x"), 1), None);
        assert_eq!(munger_bytes(b"abc", 0, Some(b"cd"), 2, None, 0), None);
    }

    #[test]
    fn packbits_round_trips_runs_and_literals_and_rejects_a_damaged_row() {
        let rows: [&[u8]; 5] = [b"", b"a", b"aaa", b"aabbbcdeeee", &[0u8; 300]];
        for row in rows {
            let packed = pack_bits_bytes(row);
            assert_eq!(unpack_bits_bytes(&packed, row.len()), Some((row.to_vec(), packed.len())), "{row:?}");
        }
        assert_eq!(pack_bits_bytes(&[7; 5]), vec![(1 - 5i32) as u8, 7], "a run of five is [1-5, byte]");
        assert_eq!(pack_bits_bytes(b"ab"), vec![1, b'a', b'b'], "literals are [n-1, bytes]");
        // the no-op flag byte -128 is skipped
        assert_eq!(unpack_bits_bytes(&[0x80, 0, 9], 1), Some((vec![9], 3)));
        // NEGATIVE CONTROLS: a row that ends early, and a run that would overrun the row
        assert_eq!(unpack_bits_bytes(&[3, 1, 2], 4), None);
        assert_eq!(unpack_bits_bytes(&[(1 - 9i32) as u8, 5], 4), None);
    }
}
