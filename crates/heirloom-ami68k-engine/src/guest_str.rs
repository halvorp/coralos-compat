//! Byte-exact guest strings. `GuestMem::read_cstr` is lossy UTF-8 and capped at 256 bytes, which is right for path
//! names but not for icon tool types or variable values (Latin-1 text, long lines).
use alloc::vec::Vec;
use heirloom_mem_flat32::{GuestMem, MemError};

/// A NUL-terminated string's bytes (without the NUL), at most `max` of them.
pub fn read_bytes_z(mem: &GuestMem, addr: u32, max: usize) -> Result<Vec<u8>, MemError> {
    let mut s = Vec::new();
    for i in 0..=max as u32 {
        let b = mem.read_u8(addr.checked_add(i).ok_or(MemError::OutOfRange { addr, len: 1 })?)?;
        if b == 0 { return Ok(s); }
        s.push(b);
    }
    Err(MemError::Unterminated { addr, max })
}

/// `s` plus a NUL at `addr`.
pub fn write_z(mem: &mut GuestMem, addr: u32, s: &[u8]) -> Result<(), MemError> {
    mem.write_bytes(addr, s)?;
    mem.write_u8(addr + s.len() as u32, 0)
}
