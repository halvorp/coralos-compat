//! Checked big-endian guest RAM. Every access is bounds-checked and returns a named
//! [`MemError`]; nothing here panics on a guest-controlled address.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod heap;
mod watch;
pub use watch::CODE_PAGE_SHIFT;
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use core::fmt;

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
impl core::error::Error for MemError {}

/// A guest address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span { pub addr: u32, pub len: u32 }

/// The full 32-bit address space: no stripping (the default, and the Amiga's).
pub const MASK_32: u32 = 0xFFFF_FFFF;
/// The classic Mac 24-bit address space (68000 / 24-bit Memory Manager): the bus has 24 address lines, so the top
/// byte of every address is ignored (Inside Macintosh: Memory, "24-bit addressing"; apps keep flags there).
pub const MASK_24: u32 = 0x00FF_FFFF;

/// Guest RAM could not grow to `requested` bytes (the host refused the allocation); the memory is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamExhausted { pub requested: usize }
impl fmt::Display for GuestRamExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "cannot grow guest RAM to {} bytes", self.requested) }
}
impl core::error::Error for GuestRamExhausted {}

pub struct GuestMem { ram: Vec<u8>, mask: u32, hooks: watch::Hooks }

impl GuestMem {
    pub fn new(size: usize) -> Self { GuestMem { ram: vec![0; size], mask: MASK_32, hooks: watch::Hooks::default() } }
    /// Select the address bus width: every access (guest CPU and host handlers alike) drops the bits outside `mask`.
    pub fn set_addr_mask(&mut self, mask: u32) { self.mask = mask; }
    pub fn addr_mask(&self) -> u32 { self.mask }
    /// `addr` as the bus sees it (StripAddress).
    pub fn strip(&self, addr: u32) -> u32 { addr & self.mask }
    pub fn len(&self) -> usize { self.ram.len() }
    /// Make the address space `new_len` bytes (zero-filled); never shrinks. For machines whose personality has more RAM than the default.
    pub fn grow_to(&mut self, new_len: usize) { if new_len > self.ram.len() { self.ram.resize(new_len, 0); } }
    /// As [`GuestMem::grow_to`], but a host that cannot supply the memory is a named refusal and nothing changes.
    pub fn try_grow_to(&mut self, new_len: usize) -> Result<(), GuestRamExhausted> {
        let len = self.ram.len();
        if new_len > len {
            self.ram.try_reserve_exact(new_len - len).map_err(|_| GuestRamExhausted { requested: new_len })?;
            self.ram.resize(new_len, 0);
        }
        Ok(())
    }
    pub fn is_empty(&self) -> bool { self.ram.is_empty() }

    pub fn contains(&self, addr: u32, len: u32) -> bool {
        let addr = self.strip(addr);
        (addr as u64) + (len as u64) <= self.ram.len() as u64
    }
    pub fn check(&self, addr: u32, len: u32) -> Result<(), MemError> {
        if self.contains(addr, len) { Ok(()) } else { Err(MemError::OutOfRange { addr, len }) }
    }
    pub fn span(&self, addr: u32, len: u32) -> Result<Span, MemError> {
        self.check(addr, len).map(|_| Span { addr: self.strip(addr), len })
    }
    pub fn bytes(&self, s: Span) -> Result<&[u8], MemError> {
        self.check(s.addr, s.len)?;
        let a = self.strip(s.addr);
        Ok(&self.ram[a as usize..(a + s.len) as usize])
    }
    pub fn write_bytes(&mut self, addr: u32, src: &[u8]) -> Result<(), MemError> {
        let len = u32::try_from(src.len()).map_err(|_| MemError::OutOfRange { addr, len: u32::MAX })?;
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.hooks.before_write(&self.ram, addr, len);
        self.ram[addr as usize..addr as usize + src.len()].copy_from_slice(src);
        Ok(())
    }
    pub fn zero(&mut self, addr: u32, len: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.hooks.before_write(&self.ram, addr, len);
        self.ram[addr as usize..(addr + len) as usize].fill(0);
        Ok(())
    }

    fn read_be(&self, addr: u32, len: u32) -> Result<u32, MemError> {
        self.check(addr, len)?;
        let a = self.strip(addr) as usize;
        Ok(self.ram[a..a + len as usize].iter().fold(0u32, |v, b| (v << 8) | *b as u32))
    }
    fn write_be(&mut self, addr: u32, len: u32, val: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.hooks.before_write(&self.ram, addr, len);
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

    /// Like [`read_cstr`](Self::read_cstr) for TEXT a program owns (a label, a long About text, a Text object's contents): no 256-byte limit, the
    /// only bound is the end of guest RAM, so a string with no NUL before it is `Unterminated { max: <bytes to the end> }`, a named refusal.
    pub fn read_cstr_long(&self, addr: u32) -> Result<String, MemError> {
        let mut s = Vec::new();
        let mut a = addr;
        loop {
            let b = self.read_u8(a).map_err(|_| if s.is_empty() { MemError::OutOfRange { addr, len: 1 } } else { MemError::Unterminated { addr, max: s.len() } })?;
            if b == 0 { return Ok(String::from_utf8_lossy(&s).into_owned()); }
            s.push(b);
            a = a.checked_add(1).ok_or(MemError::Unterminated { addr, max: s.len() })?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_string_reads_whole_with_read_cstr_long_and_a_runaway_is_a_named_error() {
        let mut m = GuestMem::new(1024);
        let text = "x".repeat(700);
        m.write_bytes(10, text.as_bytes()).unwrap();
        m.write_u8(710, 0).unwrap();
        assert_eq!(m.read_cstr_long(10).unwrap(), text, "more than MAX_CSTR bytes");
        assert_eq!(m.read_cstr(10), Err(MemError::Unterminated { addr: 10, max: MAX_CSTR }), "the bounded read keeps its limit");
        m.write_bytes(900, &[b'y'; 124]).unwrap();
        assert_eq!(m.read_cstr_long(900), Err(MemError::Unterminated { addr: 900, max: 124 }), "NEGATIVE CONTROL: no NUL before the end of RAM");
    }

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

    /// 24-bit bus: the top byte is ignored by loads, stores, spans and strings; 32-bit leaves it significant.
    #[test]
    fn a_24_bit_bus_ignores_the_top_byte_and_a_32_bit_bus_does_not() {
        let mut m = GuestMem::new(0x100);
        m.write_u32(0x10, 0xCAFE_F00D).unwrap();
        assert!(m.read_u32(0x8000_0010).is_err(), "32-bit: 0x80000010 is outside RAM");
        m.set_addr_mask(MASK_24);
        assert_eq!(m.read_u32(0x8000_0010), Ok(0xCAFE_F00D), "24-bit: the flag byte is ignored");
        m.write_u8(0xFF00_0020, 7).unwrap();
        assert_eq!(m.read_u8(0x20), Ok(7));
        assert_eq!(m.span(0x4000_0010, 4), Ok(Span { addr: 0x10, len: 4 }));
        assert_eq!(m.strip(0x8020_A9FA), 0x20_A9FA);
        assert!(m.read_u8(0x0000_0100).is_err(), "past RAM is still a refusal");
        m.set_addr_mask(MASK_32);
        assert!(m.write_u8(0xFF00_0020, 7).is_err());
    }

    #[test]
    fn refusal_text_is_stable() {
        let e = GuestMem::new(8).read_u32(0xFFFF_0000).unwrap_err();
        assert_eq!(e.to_string(), "pointer 0xffff0000 outside guest RAM");
    }

    #[test]
    fn an_impossible_growth_is_refused_and_changes_nothing() {
        let mut m = GuestMem::new(4096);
        assert_eq!(m.try_grow_to(usize::MAX / 2), Err(GuestRamExhausted { requested: usize::MAX / 2 }));
        assert_eq!(m.len(), 4096);
        assert_eq!(m.try_grow_to(8192), Ok(()));
        assert_eq!(m.len(), 8192);
    }
}
