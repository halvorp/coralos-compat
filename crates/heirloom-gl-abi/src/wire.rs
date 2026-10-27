//! Little-endian wire primitives. Every multi-byte field on the ring is little-endian, explicitly, whatever the host is.
use alloc::vec::Vec;

/// The ABI version: the FIRST field of every frame, so a reader can refuse an unknown layout before parsing it.
/// 2: `CallData` and `ReplyData` (texture pixels and read-backs) joined the frame kinds; a version-1 peer refuses a version-2 ring.
pub const ABI_VERSION: u16 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    /// The frame's leading version is not [`ABI_VERSION`].
    Version { got: u16 },
    UnknownKind(u16),
    /// A declared length disagrees with the bytes present or with the fields it must equal.
    LengthMismatch,
    /// A size computation overflowed.
    Overflow,
    BadValue(&'static str),
}

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.b.len() - self.pos
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.pos.checked_add(n).ok_or(WireError::Overflow)?;
        let s = self.b.get(self.pos..end).ok_or(WireError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn arr<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.arr::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.arr()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_little_endian_whatever_the_host() {
        let mut w = Writer::default();
        w.u16(0x0102);
        w.u32(0x03040506);
        assert_eq!(w.buf, [0x02, 0x01, 0x06, 0x05, 0x04, 0x03]);
        let mut r = Reader::new(&w.buf);
        assert_eq!((r.u16(), r.u32()), (Ok(0x0102), Ok(0x03040506)));
        assert_eq!(r.u8(), Err(WireError::Truncated));
    }
}
