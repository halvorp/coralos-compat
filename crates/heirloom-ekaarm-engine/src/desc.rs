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

/// `TDes8::Copy`-style write of bytes (the 8-bit counterpart of [`write16`]).
pub fn write8(p: &mut Process, addr: u32, bytes: &[u8]) -> Result<(), String> {
    let v = locate(p, addr)?;
    let max = v.max.ok_or_else(|| format!("descriptor at {addr:#x} is not modifiable"))?;
    if bytes.len() as u32 > max {
        return Err(format!("descriptor at {addr:#x}: {} bytes into a maximum length of {max}", bytes.len()));
    }
    for (i, b) in bytes.iter().enumerate() {
        p.mem.vm.write_u8(v.data.wrapping_add(i as u32), *b).map_err(|_| format!("descriptor at {addr:#x}: byte {i} is unwritable"))?;
    }
    let w0 = word(p, addr, "its length word")?;
    p.mem.vm.write_u32(addr, (w0 & !LENGTH_MASK) | bytes.len() as u32).map_err(|_| format!("descriptor at {addr:#x}: length word is unwritable"))
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
        // void TDes16::Append(const TDesC16& aDes): the elements of `aDes` after those of this; past the maximum length is USER 11 on a device.
        "_ZN6TDes166AppendERK7TDesC16" => {
            let r = read16(p, this).and_then(|mut a| { a.extend(read16(p, r1)?); write16(p, this, &a) });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // const TUint8& TDesC8::AtC(TInt) / const TUint16& TDesC16::AtC(TInt) (the `operator []`): the address of the element; an index outside
        // the descriptor is a USER 21 panic on a device (refused by name).
        "_ZNK6TDesC83AtCEi" | "_ZNK7TDesC163AtCEi" => match locate(p, this) {
            Ok(v) if (r1 as i32) >= 0 && r1 < v.len => { let w = if name.contains("TDesC16") { 2 } else { 1 }; p.cpu.regs[0] = v.data.wrapping_add(r1 * w); NativeOutcome::Return }
            Ok(v) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: index {} outside a descriptor of length {} (USER 21 on a device)", r1 as i32, v.len)),
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // TInt TDesC16::Compare(const TDesC16& aDes) const: element by element over the shorter, the difference of the first pair that
        // differs (this - other), else the difference of the lengths (e32des16.h).
        "_ZNK7TDesC167CompareERKS_" => match (read16(p, this), read16(p, r1)) {
            (Ok(a), Ok(b)) => {
                let d = a.iter().zip(&b).find(|(x, y)| x != y).map_or(a.len() as i32 - b.len() as i32, |(x, y)| i32::from(*x) - i32::from(*y));
                p.cpu.regs[0] = d as u32;
                NativeOutcome::Return
            }
            (Err(m), _) | (_, Err(m)) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // TPtrC16(const TUint16* aString) / (const TUint16*, TInt aLength) / (const TDesC16&): a pointer descriptor (type 1, e32des16.h):
        // iLength | type << 28, then iPtr. The first form measures the zero-terminated string.
        "_ZN7TPtrC16C1EPKt" | "_ZN7TPtrC16C1EPKti" | "_ZN7TPtrC16C1ERK7TDesC16" => {
            let r2 = p.cpu.regs[2];
            let (len, ptr) = if name.ends_with("RK7TDesC16") {
                match locate(p, r1) { Ok(v) => (v.len, v.data), Err(m) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"))) }
            } else if name.ends_with("EPKti") {
                (r2, r1)
            } else {
                let mut n = 0;
                while p.mem.vm.read_u16(r1.wrapping_add(n * 2)).is_ok_and(|c| c != 0) { n += 1; }
                (n, r1)
            };
            let ok = p.mem.vm.write_u32(this, len | DES_PTR_C << KSHIFT_DES_TYPE).is_ok() && p.mem.vm.write_u32(this.wrapping_add(4), ptr).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: descriptor at {this:#x} unwritable")) }
        }
        _ => return None,
    })
}
