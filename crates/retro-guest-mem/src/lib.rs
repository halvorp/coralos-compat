//! Checked big-endian guest RAM. Every access is bounds-checked and returns a named
//! [`MemError`]; nothing here panics on a guest-controlled address.
use std::fmt;

/// Default guest RAM size (16 MiB, the 24-bit 68000 address space).
pub const DEFAULT_SIZE: usize = 16 << 20;
/// Longest C string [`GuestMem::read_cstr`] will walk.
pub const MAX_CSTR: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    /// `addr..addr+len` is not inside guest RAM (includes u32 wrap).
    OutOfRange { addr: u32, len: u32 },
    /// No NUL within `max` bytes.
    Unterminated { addr: u32, max: usize },
}

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemError::OutOfRange { addr, .. } => write!(f, "pointer {addr:#010x} outside guest RAM"),
            MemError::Unterminated { addr, max } => {
                write!(f, "string at {addr:#010x} unterminated within {max} bytes")
            }
        }
    }
}
impl std::error::Error for MemError {}

/// A guest address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span { pub addr: u32, pub len: u32 }

pub struct GuestMem { ram: Vec<u8> }

impl GuestMem {
    pub fn new(size: usize) -> Self { GuestMem { ram: vec![0; size] } }
    pub fn len(&self) -> usize { self.ram.len() }
    pub fn is_empty(&self) -> bool { self.ram.is_empty() }

    pub fn contains(&self, addr: u32, len: u32) -> bool {
        (addr as u64) + (len as u64) <= self.ram.len() as u64
    }
    pub fn check(&self, addr: u32, len: u32) -> Result<(), MemError> {
        if self.contains(addr, len) { Ok(()) } else { Err(MemError::OutOfRange { addr, len }) }
    }
    pub fn span(&self, addr: u32, len: u32) -> Result<Span, MemError> {
        self.check(addr, len).map(|_| Span { addr, len })
    }
    pub fn bytes(&self, s: Span) -> Result<&[u8], MemError> {
        self.check(s.addr, s.len)?;
        Ok(&self.ram[s.addr as usize..(s.addr + s.len) as usize])
    }
    pub fn write_bytes(&mut self, addr: u32, src: &[u8]) -> Result<(), MemError> {
        let len = u32::try_from(src.len()).map_err(|_| MemError::OutOfRange { addr, len: u32::MAX })?;
        self.check(addr, len)?;
        self.ram[addr as usize..addr as usize + src.len()].copy_from_slice(src);
        Ok(())
    }
    pub fn zero(&mut self, addr: u32, len: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        self.ram[addr as usize..(addr + len) as usize].fill(0);
        Ok(())
    }

    fn read_be(&self, addr: u32, len: u32) -> Result<u32, MemError> {
        self.check(addr, len)?;
        let a = addr as usize;
        Ok(self.ram[a..a + len as usize].iter().fold(0u32, |v, b| (v << 8) | *b as u32))
    }
    fn write_be(&mut self, addr: u32, len: u32, val: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        for i in 0..len {
            self.ram[(addr + i) as usize] = (val >> (8 * (len - 1 - i))) as u8;
        }
        Ok(())
    }
    pub fn read_u8(&self, addr: u32) -> Result<u8, MemError> { self.read_be(addr, 1).map(|v| v as u8) }
    pub fn read_u16(&self, addr: u32) -> Result<u16, MemError> { self.read_be(addr, 2).map(|v| v as u16) }
    pub fn read_u32(&self, addr: u32) -> Result<u32, MemError> { self.read_be(addr, 4) }
    pub fn read_i16(&self, addr: u32) -> Result<i16, MemError> { self.read_u16(addr).map(|v| v as i16) }
    pub fn write_u8(&mut self, addr: u32, v: u8) -> Result<(), MemError> { self.write_be(addr, 1, v as u32) }
    pub fn write_u16(&mut self, addr: u32, v: u16) -> Result<(), MemError> { self.write_be(addr, 2, v as u32) }
    pub fn write_u32(&mut self, addr: u32, v: u32) -> Result<(), MemError> { self.write_be(addr, 4, v) }

    /// Read a NUL-terminated string (lossy UTF-8, at most [`MAX_CSTR`] bytes).
    pub fn read_cstr(&self, addr: u32) -> Result<String, MemError> {
        let mut s = Vec::new();
        for i in 0..MAX_CSTR as u32 {
            let a = addr.checked_add(i).ok_or(MemError::OutOfRange { addr, len: 1 })?;
            let b = self.read_u8(a).map_err(|_| MemError::OutOfRange { addr, len: 1 })?;
            if b == 0 { return Ok(String::from_utf8_lossy(&s).into_owned()); }
            s.push(b);
        }
        Err(MemError::Unterminated { addr, max: MAX_CSTR })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_endian_round_trip() {
        let mut m = GuestMem::new(64);
        m.write_u32(4, 0x1122_3344).unwrap();
        assert_eq!(m.bytes(m.span(4, 4).unwrap()).unwrap(), &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(m.read_u16(4).unwrap(), 0x1122);
        assert_eq!(m.read_i16(6).unwrap(), 0x3344);
    }

    #[test]
    fn out_of_range_and_wrap_are_refused() {
        let mut m = GuestMem::new(64);
        assert_eq!(m.read_u32(62), Err(MemError::OutOfRange { addr: 62, len: 4 }));
        assert!(m.write_u8(64, 1).is_err());
        assert!(m.span(u32::MAX, 2).is_err(), "u32 wrap must not alias low RAM");
        assert!(m.read_cstr(0xFFFF_FFF0).is_err());
        assert_eq!(m.read_u8(63), Ok(0));
    }

    #[test]
    fn cstr_terminated_unterminated_and_edge() {
        let mut m = GuestMem::new(1024);
        m.write_bytes(10, b"Hello\0").unwrap();
        assert_eq!(m.read_cstr(10).unwrap(), "Hello");
        m.write_bytes(100, &[b'a'; 300]).unwrap();
        assert_eq!(m.read_cstr(100), Err(MemError::Unterminated { addr: 100, max: MAX_CSTR }));
        // string running off the end of RAM is an out-of-range refusal, not a panic
        let mut small = GuestMem::new(8);
        small.write_bytes(4, b"abcd").unwrap();
        assert!(matches!(small.read_cstr(4), Err(MemError::OutOfRange { .. })));
    }

    #[test]
    fn refusal_text_is_stable() {
        let e = GuestMem::new(8).read_u32(0xFFFF_0000).unwrap_err();
        assert_eq!(e.to_string(), "pointer 0xffff0000 outside guest RAM");
    }
}
