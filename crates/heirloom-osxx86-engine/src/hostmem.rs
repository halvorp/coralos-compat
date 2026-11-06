//! [`CodeMem`] over the host's `mmap` / `mprotect`, for host tests and tools. Pages are published one range at a time with an
//! exact [`Prot`]; there is no way to ask for write and execute together.
use crate::{CodeMem, Prot};

/// Reservations are zero-filled read-write anonymous mappings; publishing narrows each range to its final protection.
#[derive(Default)]
pub struct HostMem;

const PAGE: usize = 4096;

fn prot_flags(p: Prot) -> i32 {
    match p {
        Prot::None => libc::PROT_NONE,
        Prot::R => libc::PROT_READ,
        Prot::RW => libc::PROT_READ | libc::PROT_WRITE,
        Prot::RX => libc::PROT_READ | libc::PROT_EXEC,
    }
}

impl CodeMem for HostMem {
    fn reserve(&mut self, len: usize) -> Option<*mut u8> {
        let len = len.checked_add(PAGE - 1)? & !(PAGE - 1);
        // SAFETY: an anonymous private mapping with no fixed address; the result is checked against MAP_FAILED.
        let p = unsafe { libc::mmap(core::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) };
        if p == libc::MAP_FAILED { None } else { Some(p as *mut u8) }
    }
    fn publish(&mut self, base: *mut u8, ranges: &[(usize, usize, Prot)]) -> Result<(), &'static str> {
        for &(off, len, prot) in ranges {
            if off % PAGE != 0 || len % PAGE != 0 {
                return Err("publish range is not page aligned");
            }
            // SAFETY: `base` came from `reserve` and the caller keeps the ranges inside that reservation.
            let r = unsafe { libc::mprotect(base.add(off) as *mut libc::c_void, len, prot_flags(prot)) };
            if r != 0 {
                return Err("mprotect refused");
            }
        }
        Ok(())
    }
    fn release(&mut self, base: *mut u8, len: usize) {
        let len = (len + PAGE - 1) & !(PAGE - 1);
        // SAFETY: `base`/`len` describe a mapping `reserve` returned and the caller no longer uses.
        unsafe { libc::munmap(base as *mut libc::c_void, len) };
    }
}
