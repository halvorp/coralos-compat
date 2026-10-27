//! The ARM run-time ABI's memory helpers (`drtaeabi.dll`, ARM IHI 0043 "Run-time ABI" section 4.3): compiler support the guest's own
//! code calls for struct copies, zeroing and 64-bit division. memcpy/memmove (dest, src, n), memset (dest, n, value: the value LAST), memclr (dest, n).
use alloc::format;
use alloc::vec::Vec;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// `__aeabi_ldivmod` / `__aeabi_uldivmod` (Run-time ABI 4.3.1): (r1:r0) / (r3:r2), quotient in r1:r0 and remainder in r3:r2. Division by zero
/// is a named refusal (the helper calls `__aeabi_ldiv0`, which on a device raises an exception the guest has not asked for).
fn long_divmod(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let signed = match name {
        "__aeabi_ldivmod" => true,
        "__aeabi_uldivmod" => false,
        _ => return None,
    };
    let r = p.cpu.regs;
    let (n, d) = ((r[1] as u64) << 32 | r[0] as u64, (r[3] as u64) << 32 | r[2] as u64);
    if d == 0 {
        return Some(NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: division by zero (__aeabi_ldiv0)")));
    }
    let (q, m) = if signed { ((n as i64).wrapping_div(d as i64) as u64, (n as i64).wrapping_rem(d as i64) as u64) } else { (n / d, n % d) };
    p.cpu.regs[0] = q as u32;
    p.cpu.regs[1] = (q >> 32) as u32;
    p.cpu.regs[2] = m as u32;
    p.cpu.regs[3] = (m >> 32) as u32;
    Some(NativeOutcome::Return)
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    if let Some(o) = long_divmod(p, name) {
        return Some(o);
    }
    let rest = name.strip_prefix("__aeabi_mem")?;
    let kind = rest.trim_end_matches(|c: char| c.is_ascii_digit());
    if !matches!(kind, "cpy" | "move" | "set" | "clr") {
        return None;
    }
    let (dst, a, b) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let bad = |what: &str| Some(NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: {what} is unmapped")));
    match kind {
        "cpy" | "move" => {
            let src: Result<Vec<u8>, _> = (0..b).map(|i| p.mem.vm.read_u8(a.wrapping_add(i))).collect();
            let Ok(src) = src else { return bad("the source") };
            for (i, v) in src.iter().enumerate() {
                if p.mem.vm.write_u8(dst.wrapping_add(i as u32), *v).is_err() { return bad("the destination"); }
            }
        }
        _ => {
            let (n, v) = if kind == "set" { (a, b as u8) } else { (a, 0) };
            for i in 0..n {
                if p.mem.vm.write_u8(dst.wrapping_add(i), v).is_err() { return bad("the destination"); }
            }
        }
    }
    Some(NativeOutcome::Return)
}
