//! The ARM run-time ABI's double-precision helpers (`dfpaeabi.dll`: `__aeabi_dadd`, `__aeabi_i2d`, ...): compiler support the guest's own
//! code calls for `double` arithmetic (there is no FPU). Each one is the IEEE-754 operation on the two register pairs (`r0:r1`, `r2:r3`,
//! low word first), the result in `r0:r1` (comparisons and `d2iz` in `r0`), exactly as the run-time ABI defines them.
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

fn pair(lo: u32, hi: u32) -> f64 { f64::from_bits(u64::from(hi) << 32 | u64::from(lo)) }

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let [r0, r1, r2, r3] = [p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]];
    let (a, b) = (pair(r0, r1), pair(r2, r3));
    let double = |p: &mut Process, v: f64| { let bits = v.to_bits(); p.cpu.regs[0] = bits as u32; p.cpu.regs[1] = (bits >> 32) as u32; NativeOutcome::Return };
    let word = |p: &mut Process, v: u32| { p.cpu.regs[0] = v; NativeOutcome::Return };
    Some(match name {
        "__aeabi_i2d" => double(p, f64::from(r0 as i32)),
        "__aeabi_ui2d" => double(p, f64::from(r0)),
        "__aeabi_dadd" => double(p, a + b),
        "__aeabi_dsub" => double(p, a - b),
        "__aeabi_drsub" => double(p, b - a),
        "__aeabi_dmul" => double(p, a * b),
        "__aeabi_ddiv" => double(p, a / b),
        // Truncation toward zero (the run-time ABI leaves out-of-range undefined; Rust's cast saturates).
        "__aeabi_d2iz" => word(p, a as i32 as u32),
        "__aeabi_d2uiz" => word(p, a as u32),
        "__aeabi_dcmpeq" => word(p, (a == b) as u32),
        "__aeabi_dcmplt" => word(p, (a < b) as u32),
        "__aeabi_dcmple" => word(p, (a <= b) as u32),
        "__aeabi_dcmpgt" => word(p, (a > b) as u32),
        "__aeabi_dcmpge" => word(p, (a >= b) as u32),
        _ if name.starts_with("__aeabi_d") || name.starts_with("__aeabi_i2") || name.starts_with("__aeabi_ui2") => NativeOutcome::Refused(format!("NEXT-NEEDED dfpaeabi {name}")),
        _ => return None,
    })
}
