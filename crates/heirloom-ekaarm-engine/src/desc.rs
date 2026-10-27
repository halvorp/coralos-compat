//! Guest descriptors (`TDesC8` / `TDesC16` / `TDes16`), read and written in place in guest memory. Text stays UTF-16 in guest
//! memory: these hand back `Vec<u16>`, and the caller converts to UTF-8 only where it crosses the `GuestUi` boundary.
//!
//! Layout (e32des16.h / e32des8.h): word 0 is `iLength | type << KShiftDesType` (shift 28, e32des16.h:16); then by type
//! (EKA2L1 utils/des.h:50-55, facts: buf_const 0, ptr_const 1, ptr 2, buf 3; `EBufCPtr` 4 is `TPtr` over an `HBufC`):
//! * `TBufC` (0): data at +4.        * `TPtrC` (1): `iPtr` at +4.
//! * `TPtr` (2): `iMaxLength` at +4, `iPtr` at +8.   * `TBuf` (3): `iMaxLength` at +4, data at +8.
//! * `EBufCPtr` (4): `iMaxLength` at +4, `iPtr` at +8 = the `HBufC` (a `TBufC`: data at its +4).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;

pub const KSHIFT_DES_TYPE: u32 = 28; // e32des16.h:16
const LENGTH_MASK: u32 = (1 << KSHIFT_DES_TYPE) - 1;
pub const DES_BUF_C: u32 = 0;
pub const DES_PTR_C: u32 = 1;
pub const DES_PTR: u32 = 2;
pub const DES_BUF: u32 = 3;
pub const DES_BUF_C_PTR: u32 = 4;

/// Where a descriptor's elements are: the guest address of the first, the length, and the maximum length when it is modifiable.
#[derive(Debug, Clone, Copy)]
pub struct DesView {
    pub data: u32,
    pub len: u32,
    pub max: Option<u32>,
}

fn word(p: &Process, addr: u32, what: &str) -> Result<u32, String> {
    p.mem.vm.read_u32(addr).map_err(|_| format!("descriptor at {addr:#x}: {what} is unreadable"))
}

/// Locate the elements of the descriptor at `addr`.
pub fn locate(p: &Process, addr: u32) -> Result<DesView, String> {
    let w0 = word(p, addr, "its length word")?;
    let (len, ty) = (w0 & LENGTH_MASK, w0 >> KSHIFT_DES_TYPE);
    Ok(match ty {
        DES_BUF_C => DesView { data: addr.wrapping_add(4), len, max: None },
        DES_PTR_C => DesView { data: word(p, addr.wrapping_add(4), "iPtr")?, len, max: None },
        DES_PTR => DesView { data: word(p, addr.wrapping_add(8), "iPtr")?, len, max: Some(word(p, addr.wrapping_add(4), "iMaxLength")?) },
        DES_BUF => DesView { data: addr.wrapping_add(8), len, max: Some(word(p, addr.wrapping_add(4), "iMaxLength")?) },
        DES_BUF_C_PTR => {
            let hbuf = word(p, addr.wrapping_add(8), "iPtr (the HBufC)")?;
            DesView { data: hbuf.wrapping_add(4), len, max: Some(word(p, addr.wrapping_add(4), "iMaxLength")?) }
        }
        other => return Err(format!("descriptor at {addr:#x} has unknown type {other}")),
    })
}

/// The 16-bit elements of the descriptor at `addr`.
pub fn read16(p: &Process, addr: u32) -> Result<Vec<u16>, String> {
    let v = locate(p, addr)?;
    (0..v.len).map(|i| p.mem.vm.read_u16(v.data.wrapping_add(i * 2)).map_err(|_| format!("descriptor at {addr:#x}: element {i} at {:#x} is unreadable", v.data.wrapping_add(i * 2)))).collect()
}

/// The 8-bit elements of the descriptor at `addr`.
pub fn read8(p: &Process, addr: u32) -> Result<Vec<u8>, String> {
    let v = locate(p, addr)?;
    (0..v.len).map(|i| p.mem.vm.read_u8(v.data.wrapping_add(i)).map_err(|_| format!("descriptor at {addr:#x}: byte {i} is unreadable"))).collect()
}

/// `TDes16::Copy`-style write: the elements and the new length. A source longer than `iMaxLength` is an error (a USER 11 panic on a device).
pub fn write16(p: &mut Process, addr: u32, text: &[u16]) -> Result<(), String> {
    let v = locate(p, addr)?;
    let max = v.max.ok_or_else(|| format!("descriptor at {addr:#x} is not modifiable"))?;
    if text.len() as u32 > max {
        return Err(format!("descriptor at {addr:#x}: {} elements into a maximum length of {max}", text.len()));
    }
    for (i, u) in text.iter().enumerate() {
        p.mem.vm.write_u16(v.data.wrapping_add(i as u32 * 2), *u).map_err(|_| format!("descriptor at {addr:#x}: element {i} is unwritable"))?;
    }
    let w0 = word(p, addr, "its length word")?;
    p.mem.vm.write_u32(addr, (w0 & !LENGTH_MASK) | text.len() as u32).map_err(|_| format!("descriptor at {addr:#x}: length word is unwritable"))
}

/// Convert UTF-16 elements to UTF-8 (lossy on unpaired surrogates): the GuestUi boundary.
pub fn to_utf8(units: &[u16]) -> String {
    char::decode_utf16(units.iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

/// The natives of this module (descriptor members); `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<crate::natives::NativeOutcome> {
    use crate::natives::NativeOutcome;
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    Some(match name {
        // TDes16::Copy(const TDesC16& aDes) (e32des16.h): replace the contents; longer than the maximum length panics USER 11.
        "_ZN6TDes164CopyERK7TDesC16" => {
            let r = read16(p, r1).and_then(|text| write16(p, this, &text));
            match r {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        _ => return None,
    })
}
