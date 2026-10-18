//! mathieeesingbas.library and mathieeedoubbas.library: IEEE single and double basic arithmetic.
//!
//! Registers (AROS workbench/libs/mathieeesingbas/mathieeesingbas.conf, mathieeedoubbas.conf): a single is one longword in D0
//! (second operand D1); a double is D0 (high) : D1 (low), the second operand D2:D3; results come back in D0 / D0:D1.
//! Semantics from the AROS sources: Fix truncates toward zero and saturates at the longword limits (NaN gives 0); Cmp and
//! Tst return +1 / 0 / -1; Sub is `y - z` and Div `y / z` (y is the first operand in D0 / D0:D1); Floor and Ceil round toward
//! -inf / +inf. The 68k condition codes the real library also sets (zero, negative, overflow) are not modelled: Musashi's SR
//! is not reachable from a handler, and programs read the D0 result. The transcendental libraries are not implemented.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

fn sp(cpu: &Cpu, n: u32) -> f32 { f32::from_bits(cpu.dreg(n)) }
fn dp(cpu: &Cpu, n: u32) -> f64 { f64::from_bits(((cpu.dreg(n) as u64) << 32) | cpu.dreg(n + 1) as u64) }
fn set_sp(cpu: &mut Cpu, v: f32) { cpu.set_d0(v.to_bits()); }
fn set_dp(cpu: &mut Cpu, v: f64) { let b = v.to_bits(); cpu.set_d0((b >> 32) as u32); cpu.set_dreg(1, b as u32); }
fn sign<T: PartialOrd + Default>(a: T, b: T) -> u32 { if a == b { 0 } else if a > b { 1 } else { u32::MAX } }
fn floor_f64(v: f64) -> f64 { let t = v as i64 as f64; if v.is_finite() && v.abs() < 9.0e15 { if t > v { t - 1.0 } else { t } } else { v } }
fn ceil_f64(v: f64) -> f64 { let t = v as i64 as f64; if v.is_finite() && v.abs() < 9.0e15 { if t < v { t + 1.0 } else { t } } else { v } }

macro_rules! op {
    ($name:ident, $text:literal, |$cpu:ident| $body:expr) => {
        fn $name<U: GuestUi>(_: &mut Amiga<U>, $cpu: &mut Cpu) -> Result<String, Refusal> { $body; Ok(format!("[math] {}", $text)) }
    };
}

op!(sp_fix, "IEEESPFix", |c| { let v = sp(c, 0); c.set_d0(v as i32 as u32) });
op!(sp_flt, "IEEESPFlt", |c| { let v = c.dreg(0) as i32 as f32; set_sp(c, v) });
op!(sp_cmp, "IEEESPCmp", |c| { let r = sign(sp(c, 0), sp(c, 1)); c.set_d0(r) });
op!(sp_tst, "IEEESPTst", |c| { let r = sign(sp(c, 0), 0.0); c.set_d0(r) });
op!(sp_abs, "IEEESPAbs", |c| { let v = sp(c, 0).abs(); set_sp(c, v) });
op!(sp_neg, "IEEESPNeg", |c| { let v = -sp(c, 0); set_sp(c, v) });
op!(sp_add, "IEEESPAdd", |c| { let v = sp(c, 0) + sp(c, 1); set_sp(c, v) });
op!(sp_sub, "IEEESPSub", |c| { let v = sp(c, 0) - sp(c, 1); set_sp(c, v) });
op!(sp_mul, "IEEESPMul", |c| { let v = sp(c, 0) * sp(c, 1); set_sp(c, v) });
op!(sp_div, "IEEESPDiv", |c| { let v = sp(c, 0) / sp(c, 1); set_sp(c, v) });
op!(sp_floor, "IEEESPFloor", |c| { let v = floor_f64(sp(c, 0) as f64) as f32; set_sp(c, v) });
op!(sp_ceil, "IEEESPCeil", |c| { let v = ceil_f64(sp(c, 0) as f64) as f32; set_sp(c, v) });

op!(dp_fix, "IEEEDPFix", |c| { let v = dp(c, 0); c.set_d0(v as i32 as u32) });
op!(dp_flt, "IEEEDPFlt", |c| { let v = c.dreg(0) as i32 as f64; set_dp(c, v) });
op!(dp_cmp, "IEEEDPCmp", |c| { let r = sign(dp(c, 0), dp(c, 2)); c.set_d0(r) });
op!(dp_tst, "IEEEDPTst", |c| { let r = sign(dp(c, 0), 0.0); c.set_d0(r) });
op!(dp_abs, "IEEEDPAbs", |c| { let v = dp(c, 0).abs(); set_dp(c, v) });
op!(dp_neg, "IEEEDPNeg", |c| { let v = -dp(c, 0); set_dp(c, v) });
op!(dp_add, "IEEEDPAdd", |c| { let v = dp(c, 0) + dp(c, 2); set_dp(c, v) });
op!(dp_sub, "IEEEDPSub", |c| { let v = dp(c, 0) - dp(c, 2); set_dp(c, v) });
op!(dp_mul, "IEEEDPMul", |c| { let v = dp(c, 0) * dp(c, 2); set_dp(c, v) });
op!(dp_div, "IEEEDPDiv", |c| { let v = dp(c, 0) / dp(c, 2); set_dp(c, v) });
op!(dp_floor, "IEEEDPFloor", |c| { let v = floor_f64(dp(c, 0)); set_dp(c, v) });
op!(dp_ceil, "IEEEDPCeil", |c| { let v = ceil_f64(dp(c, 0)); set_dp(c, v) });

pub fn register_single<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("IEEESPFix", sp_fix::<U>).named("IEEESPFlt", sp_flt::<U>).named("IEEESPCmp", sp_cmp::<U>).named("IEEESPTst", sp_tst::<U>)
        .named("IEEESPAbs", sp_abs::<U>).named("IEEESPNeg", sp_neg::<U>).named("IEEESPAdd", sp_add::<U>).named("IEEESPSub", sp_sub::<U>)
        .named("IEEESPMul", sp_mul::<U>).named("IEEESPDiv", sp_div::<U>).named("IEEESPFloor", sp_floor::<U>).named("IEEESPCeil", sp_ceil::<U>)
}

pub fn register_double<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("IEEEDPFix", dp_fix::<U>).named("IEEEDPFlt", dp_flt::<U>).named("IEEEDPCmp", dp_cmp::<U>).named("IEEEDPTst", dp_tst::<U>)
        .named("IEEEDPAbs", dp_abs::<U>).named("IEEEDPNeg", dp_neg::<U>).named("IEEEDPAdd", dp_add::<U>).named("IEEEDPSub", dp_sub::<U>)
        .named("IEEEDPMul", dp_mul::<U>).named("IEEEDPDiv", dp_div::<U>).named("IEEEDPFloor", dp_floor::<U>).named("IEEEDPCeil", dp_ceil::<U>)
}
