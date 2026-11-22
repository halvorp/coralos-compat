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
        // TPtr8(TUint8* aBuf, TInt aLength, TInt aMaxLength) / TPtr16(TUint16*, TInt, TInt) (e32des8.h / e32des16.h).
        "_ZN5TPtr8C1EPhii" | "_ZN5TPtr8C2EPhii" | "_ZN6TPtr16C1EPtii" | "_ZN6TPtr16C2EPtii" => {
            let (buf, len, max) = (r1, p.cpu.regs[2], p.cpu.regs[3]);
            let ok = p.mem.vm.write_u32(this, (DES_PTR << KSHIFT_DES_TYPE) | (len & LENGTH_MASK)).is_ok()
                && p.mem.vm.write_u32(this.wrapping_add(4), max).is_ok()
                && p.mem.vm.write_u32(this.wrapping_add(8), buf).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: descriptor at {this:#x} is unwritable")) }
        }
        // void TDes16::Zero() / void TDes8::Zero(): set length to 0 while preserving descriptor type bits.
        "_ZN6TDes164ZeroEv" | "_ZN5TDes84ZeroEv" => {
            let r = word(p, this, "its length word").and_then(|w0| {
                p.mem.vm.write_u32(this, w0 & !LENGTH_MASK).map_err(|_| format!("descriptor at {this:#x}: length word is unwritable"))
            });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // TDes16::Copy(const TDesC16& aDes) (e32des16.h): replace the contents; longer than the maximum length panics USER 11.
        "_ZN6TDes164CopyERK7TDesC16" => {
            let r = read16(p, r1).and_then(|text| write16(p, this, &text));
            match r {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // TDes16::Copy(const TUint16* aBuf, TInt aLength) (e32des16.h)
        "_ZN6TDes164CopyEPKti" => {
            let len = p.cpu.regs[2] as i32;
            let r = if len < 0 {
                Err(format!("negative length {len}"))
            } else {
                let mut units = Vec::with_capacity(len as usize);
                let mut err = None;
                for i in 0..(len as u32) {
                    match p.mem.vm.read_u16(r1.wrapping_add(i * 2)) {
                        Ok(u) => units.push(u),
                        Err(_) => { err = Some(format!("source at {r1:#x}+{i} unreadable")); break; }
                    }
                }
                match err { Some(m) => Err(m), None => write16(p, this, &units) }
            };
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // const TUint16* TDesC16::Ptr() const / const TUint8* TDesC8::Ptr() const
        "_ZNK7TDesC163PtrEv" | "_ZNK6TDesC83PtrEv" => match locate(p, this) {
            Ok(v) => { p.cpu.regs[0] = v.data; NativeOutcome::Return }
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // TPtr16 HBufC16::Des() (e32des16.h): ARM EABI sret TPtr16 in r0, this in r1.
        "_ZN7HBufC163DesEv" => {
            let sret = this;
            let hbuf = r1;
            let r = locate(p, hbuf).and_then(|v| {
                let max = p.heap.as_ref().and_then(|h| h.len_of(hbuf)).map(|b| b.saturating_sub(4) / 2).unwrap_or(v.len).max(v.len);
                p.mem.vm.write_u32(sret, (DES_BUF_C_PTR << KSHIFT_DES_TYPE) | (v.len & LENGTH_MASK))
                    .and_then(|_| p.mem.vm.write_u32(sret.wrapping_add(4), max))
                    .and_then(|_| p.mem.vm.write_u32(sret.wrapping_add(8), hbuf))
                    .map_err(|_| format!("TPtr16 sret at {sret:#x} unwritable"))
            });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // void TDes16::Append(const TDesC16& aDes): the elements of `aDes` after those of this; past the maximum length is USER 11 on a device.
        "_ZN6TDes166AppendERK7TDesC16" => {
            let r = read16(p, this).and_then(|mut a| { a.extend(read16(p, r1)?); write16(p, this, &a) });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // void TDes16::Append(TChar aChar) (e32des16.h): append one UTF-16 code unit.
        "_ZN6TDes166AppendE5TChar" => {
            let r = read16(p, this).and_then(|mut a| { a.push(r1 as u16); write16(p, this, &a) });
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
        // void TDes16::Trim() / TDes8::Trim(): strip leading and trailing ASCII whitespace (e32des16.h / e32des8.h).
        "_ZN6TDes164TrimEv" => {
            let res = read16(p, this).and_then(|u| {
                let s = u.iter().position(|&c| !matches!(c, 0x20 | 0x09 | 0x0a | 0x0d)).unwrap_or(u.len());
                let e = u[s..].iter().rposition(|&c| !matches!(c, 0x20 | 0x09 | 0x0a | 0x0d)).map(|i| s + i + 1).unwrap_or(s);
                write16(p, this, &u[s..e])
            });
            match res { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        "_ZN5TDes84TrimEv" => {
            let res = read8(p, this).and_then(|b| {
                let s = b.iter().position(|&c| !c.is_ascii_whitespace()).unwrap_or(b.len());
                let e = b[s..].iter().rposition(|&c| !c.is_ascii_whitespace()).map(|i| s + i + 1).unwrap_or(s);
                write8(p, this, &b[s..e])
            });
            match res { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // void TDes16::UpperCase() / LowerCase() (e32des16.h).
        "_ZN6TDes169UpperCaseEv" | "_ZN6TDes169LowerCaseEv" => {
            let upper = name.contains("UpperCase");
            let res = read16(p, this).and_then(|mut u| {
                for c in &mut u {
                    if upper && (b'a' as u16..=b'z' as u16).contains(c) { *c -= 32; }
                    else if !upper && (b'A' as u16..=b'Z' as u16).contains(c) { *c += 32; }
                }
                write16(p, this, &u)
            });
            match res { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // void TDes16::SetLength(TInt aLength) (e32des16.h).
        "_ZN6TDes169SetLengthEi" => {
            let new_len = r1 as i32;
            let res = locate(p, this).and_then(|v| {
                let Some(max) = v.max else { return Err(format!("descriptor at {this:#x} is non-modifiable")); };
                if new_len < 0 || (new_len as u32) > max {
                    return Err(format!("SetLength({new_len}) exceeds max {max}"));
                }
                let ty = (p.rd(this)? >> KSHIFT_DES_TYPE) & 0xf;
                p.wr(this, (new_len as u32) | (ty << KSHIFT_DES_TYPE))
            });
            match res { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        // TInt TDesC16::LocateReverse(TChar aChar) const: last index of aChar, or KErrNotFound (-1).
        "_ZNK7TDesC1613LocateReverseE5TChar" => match read16(p, this) {
            Ok(u) => {
                let target = (r1 & 0xffff) as u16;
                let idx = u.iter().rposition(|&c| c == target).map(|i| i as i32).unwrap_or(-1);
                p.cpu.regs[0] = idx as u32;
                NativeOutcome::Return
            }
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // TInt TDesC16::FindF(const TDesC16& aDes) const: case-folded substring search, or KErrNotFound (-1).
        "_ZNK7TDesC165FindFERKS_" => match (read16(p, this), read16(p, r1)) {
            (Ok(hay), Ok(needle)) => {
                let fold = |v: &[u16]| -> Vec<u16> {
                    v.iter().map(|&c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c }).collect()
                };
                let (h, n) = (fold(&hay), fold(&needle));
                let idx = if n.is_empty() {
                    0
                } else {
                    h.windows(n.len()).position(|w| w == n.as_slice()).map(|i| i as i32).unwrap_or(-1)
                };
                p.cpu.regs[0] = idx as u32;
                NativeOutcome::Return
            }
            (Err(m), _) | (_, Err(m)) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // TInt TDesC16::CompareC(const TDesC16&) const / CompareF: ASCII case-insensitive comparison.
        "_ZNK7TDesC168CompareCERKS_" | "_ZNK7TDesC168CompareFERKS_" => match (read16(p, this), read16(p, r1)) {
            (Ok(a), Ok(b)) => {
                let fold = |c: u16| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c };
                let d = a.iter().zip(&b)
                    .find(|(&x, &y)| fold(x) != fold(y))
                    .map_or(a.len() as i32 - b.len() as i32, |(&x, &y)| i32::from(fold(x)) - i32::from(fold(y)));
                p.cpu.regs[0] = d as u32;
                NativeOutcome::Return
            }
            (Err(m), _) | (_, Err(m)) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        // void TDes16::CopyLC(const TDesC16&) / CopyUC(const TDesC16&): copy converted to lowercase / uppercase (e32des16.h).
        "_ZN6TDes166CopyLCERK7TDesC16" | "_ZN6TDes166CopyUCERK7TDesC16" => {
            let lower = name.contains("CopyLC");
            let res = read16(p, r1).and_then(|u| {
                let conv: Vec<u16> = u.into_iter().map(|c| {
                    if lower && (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 }
                    else if !lower && (b'a' as u16..=b'z' as u16).contains(&c) { c - 32 }
                    else { c }
                }).collect();
                write16(p, this, &conv)
            });
            match res {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // void TDes8::Delete(TInt aPos, TInt aLength) / TDes16::Delete(TInt aPos, TInt aLength) (e32des8.h / e32des16.h):
        // deletes `aLength` elements starting at `aPos`.
        "_ZN5TDes86DeleteEii" | "_ZN6TDes166DeleteEii" => {
            let pos = r1 as i32;
            let len = p.cpu.regs[2] as i32;
            let res = if name.contains("TDes8") {
                read8(p, this).and_then(|mut b| {
                    if pos < 0 || len < 0 || (pos as usize) > b.len() {
                        return Err(format!("Delete({pos}, {len}) outside length {}", b.len()));
                    }
                    let end = (pos as usize).saturating_add(len as usize).min(b.len());
                    b.drain(pos as usize..end);
                    write8(p, this, &b)
                })
            } else {
                read16(p, this).and_then(|mut u| {
                    if pos < 0 || len < 0 || (pos as usize) > u.len() {
                        return Err(format!("Delete({pos}, {len}) outside length {}", u.len()));
                    }
                    let end = (pos as usize).saturating_add(len as usize).min(u.len());
                    u.drain(pos as usize..end);
                    write16(p, this, &u)
                })
            };
            match res {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // void TDes8::Copy(const TDesC16& aDes): Symbian narrowing copy (each 16-bit code unit truncated to 8 bits, e32des8.h).
        "_ZN5TDes84CopyERK7TDesC16" => {
            let res = read16(p, r1).and_then(|u| {
                let bytes: Vec<u8> = u.into_iter().map(|c| (c & 0xff) as u8).collect();
                write8(p, this, &bytes)
            });
            match res {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // void TDes16::Copy(const TDesC8& aDes): Symbian widening copy (each 8-bit byte zero-extended to 16 bits, e32des16.h).
        "_ZN6TDes164CopyERK6TDesC8" => {
            let res = read8(p, r1).and_then(|b| {
                let units: Vec<u16> = b.into_iter().map(u16::from).collect();
                write16(p, this, &units)
            });
            match res {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // CnvUtfConverter::ConvertToUnicodeFromUtf8L(const TDesC8&) -> HBufC16* (utf.h)
        "_ZN15CnvUtfConverter25ConvertToUnicodeFromUtf8LERK6TDesC8" => {
            let res = read8(p, this).and_then(|bytes| {
                let s = alloc::string::String::from_utf8_lossy(&bytes);
                let units: Vec<u16> = s.encode_utf16().collect();
                let hbuf = p.alloc_zeroed(4 + units.len() as u32 * 2)
                    .ok_or_else(|| String::from("out of heap for HBufC16"))?;
                p.wr(hbuf, units.len() as u32)?; // DES_BUF_C (type 0)
                for (i, u) in units.iter().enumerate() {
                    p.mem.vm.write_u16(hbuf + 4 + i as u32 * 2, *u)
                        .map_err(|_| String::from("HBufC16 unwritable"))?;
                }
                Ok(hbuf)
            });
            match res {
                Ok(hbuf) => { p.cpu.regs[0] = hbuf; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED charconv {name}: {m}")),
            }
        }
        // TInt CnvUtfConverter::ConvertToUnicodeFromUtf8(TDes16& aUnicode, const TDesC8& aUtf8) (utf.h)
        "_ZN15CnvUtfConverter24ConvertToUnicodeFromUtf8ER6TDes16RK6TDesC8" => {
            let res = read8(p, r1).and_then(|bytes| {
                let s = alloc::string::String::from_utf8_lossy(&bytes);
                let units: Vec<u16> = s.encode_utf16().collect();
                write16(p, this, &units)
            });
            match res {
                Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED charconv {name}: {m}")),
            }
        }
        // TInt CnvUtfConverter::ConvertFromUnicodeToUtf8(TDes8& aUtf8, const TDesC16& aUnicode) (utf.h)
        "_ZN15CnvUtfConverter24ConvertFromUnicodeToUtf8ER5TDes8RK7TDesC16" => {
            let res = read16(p, r1).and_then(|units| {
                let s = to_utf8(&units);
                write8(p, this, s.as_bytes())
            });
            match res {
                Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED charconv {name}: {m}")),
            }
        }
        // TPtrC16 TDesC16::Left(TInt aLength) const (e32des16.h): ARM EABI sret in r0, this in r1, aLength in r2.
        "_ZNK7TDesC164LeftEi" => {
            let sret = p.cpu.regs[0];
            let src_addr = p.cpu.regs[1];
            let want = p.cpu.regs[2] as i32;
            match locate(p, src_addr) {
                Ok(v) => {
                    let len = (want.max(0) as u32).min(v.len);
                    let ok = p.mem.vm.write_u32(sret, len | DES_PTR_C << KSHIFT_DES_TYPE).is_ok()
                        && p.mem.vm.write_u32(sret.wrapping_add(4), v.data).is_ok();
                    if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret at {sret:#x} unwritable")) }
                }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // TPtrC16 TDesC16::Right(TInt aLength) const (e32des16.h): ARM EABI sret in r0, this in r1, aLength in r2.
        "_ZNK7TDesC165RightEi" => {
            let sret = p.cpu.regs[0];
            let src_addr = p.cpu.regs[1];
            let want = p.cpu.regs[2] as i32;
            match locate(p, src_addr) {
                Ok(v) => {
                    let len = (want.max(0) as u32).min(v.len);
                    let off = (v.len - len) * 2;
                    let ok = p.mem.vm.write_u32(sret, len | DES_PTR_C << KSHIFT_DES_TYPE).is_ok()
                        && p.mem.vm.write_u32(sret.wrapping_add(4), v.data.wrapping_add(off)).is_ok();
                    if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret at {sret:#x} unwritable")) }
                }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        // TPtrC16 TDesC16::Mid(TInt aPos) / Mid(TInt aPos, TInt aLength) const (e32des16.h):
        // ARM EABI sret in r0, this in r1, aPos in r2, optional aLength in r3.
        "_ZNK7TDesC163MidEi" | "_ZNK7TDesC163MidEii" => {
            let sret = p.cpu.regs[0];
            let src_addr = p.cpu.regs[1];
            let pos = p.cpu.regs[2] as i32;
            let has_len = name.ends_with("Eii");
            let want_len = p.cpu.regs[3] as i32;
            match locate(p, src_addr) {
                Ok(v) if pos >= 0 && (pos as u32) <= v.len => {
                    let start = pos as u32;
                    let avail = v.len - start;
                    let len = if has_len {
                        if want_len < 0 || (want_len as u32) > avail {
                            return Some(NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: Mid({pos}, {want_len}) outside length {}", v.len)));
                        }
                        want_len as u32
                    } else {
                        avail
                    };
                    let ok = p.mem.vm.write_u32(sret, len | DES_PTR_C << KSHIFT_DES_TYPE).is_ok()
                        && p.mem.vm.write_u32(sret.wrapping_add(4), v.data.wrapping_add(start * 2)).is_ok();
                    if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: sret at {sret:#x} unwritable")) }
                }
                Ok(v) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: Mid({pos}) outside length {}", v.len)),
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
            }
        }
        _ => return None,
    })
}
