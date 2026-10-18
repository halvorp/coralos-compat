//! Pattern-initialised data: a byte-oriented unpacker. Each instruction is `opcode:3 count:5`; a zero count means the
//! count follows as a variable-length integer (7 bits per byte, big-endian, high bit = more). Opcodes: 0 zero `count`
//! bytes; 1 copy `count` literal bytes; 2 repeat the `count`-byte block (varint repeatCount + 1) times; 3 repeat-block:
//! `count` common bytes, varint customSize, varint repeatCount, then the common bytes and `customSize` bytes for each
//! of the `repeatCount` gaps; 4 repeat-zero: as 3 with the common bytes all zero. The output must be filled exactly.
use crate::PefError;

fn varint(src: &[u8], at: &mut usize) -> Result<u32, PefError> {
    let mut v: u32 = 0;
    loop {
        let b = *src.get(*at).ok_or(PefError::BadPidata("a variable-length count runs past the end of the stream"))?;
        *at += 1;
        if v > (u32::MAX >> 7) { return Err(PefError::BadPidata("a variable-length count overflows 32 bits")); }
        v = (v << 7) | u32::from(b & 0x7F);
        if b & 0x80 == 0 { return Ok(v); }
    }
}

struct Out<'a> { buf: &'a mut [u8], at: usize }
impl Out<'_> {
    fn room(&self, n: usize) -> Result<(), PefError> {
        if self.buf.len() - self.at < n { Err(PefError::BadPidata("the stream writes past the unpacked size")) } else { Ok(()) }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), PefError> { self.room(bytes.len())?; self.buf[self.at..self.at + bytes.len()].copy_from_slice(bytes); self.at += bytes.len(); Ok(()) }
    fn zeros(&mut self, n: usize) -> Result<(), PefError> { self.room(n)?; self.buf[self.at..self.at + n].fill(0); self.at += n; Ok(()) }
}

fn take<'s>(src: &'s [u8], at: &mut usize, n: usize) -> Result<&'s [u8], PefError> {
    let end = at.checked_add(n).ok_or(PefError::BadPidata("size overflow"))?;
    let s = src.get(*at..end).ok_or(PefError::BadPidata("a literal runs past the end of the stream"))?;
    *at = end;
    Ok(s)
}

/// Expand `src` into `dst`, which must be exactly the unpacked size.
pub fn unpack(src: &[u8], dst: &mut [u8]) -> Result<(), PefError> {
    let mut out = Out { buf: dst, at: 0 };
    let mut at = 0usize;
    while at < src.len() {
        let (op, mut count) = (src[at] >> 5, u32::from(src[at] & 0x1F));
        at += 1;
        if count == 0 { count = varint(src, &mut at)?; }
        let count = count as usize;
        match op {
            0 => out.zeros(count)?,
            1 => { let b = take(src, &mut at, count)?; out.put(b)?; }
            2 => {
                let reps = varint(src, &mut at)? as usize + 1;
                let block = take(src, &mut at, count)?;
                out.room(count.checked_mul(reps).ok_or(PefError::BadPidata("repeat size overflow"))?)?;
                for _ in 0..reps { out.put(block)?; }
            }
            3 | 4 => {
                let custom = varint(src, &mut at)? as usize;
                let reps = varint(src, &mut at)? as usize;
                let common = if op == 3 { take(src, &mut at, count)? } else { &[][..] };
                let per = count.checked_add(custom).ok_or(PefError::BadPidata("size overflow"))?;
                let total = reps.checked_mul(per).and_then(|t| t.checked_add(count)).ok_or(PefError::BadPidata("repeat size overflow"))?;
                out.room(total)?;
                for i in 0..=reps {
                    if op == 3 { out.put(common)?; } else { out.zeros(count)?; }
                    if i < reps { let c = take(src, &mut at, custom)?; out.put(c)?; }
                }
            }
            _ => return Err(PefError::BadPidata("opcode 5, 6 or 7 is not defined")),
        }
    }
    if out.at != out.buf.len() { return Err(PefError::BadPidata("the stream ends before the unpacked size is filled")); }
    Ok(())
}
