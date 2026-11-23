//! ARM EHABI exception unwinding for Symbian EKA2 `TRAP` / `User::Leave` (`__LEAVE_EQUALS_THROW__`).
//!
//! Symbian EKA2 ARM binaries built with RVCT or GCCE emit standard ARM EHABI `.ARM.exidx` / `.ARM.extab` tables
//! referenced by `E32ImageHeaderV::iExceptionDescriptor` (`TExceptionDescriptor`: `exidx_base`, `exidx_limit`,
//! `ro_base`, `ro_limit`). `User::Leave(reason)` walks the guest stack through those tables, runs any intermediate
//! C++ cleanup landing pads (which resume via `__cxa_end_cleanup`), drains `CleanupStack` down to the enclosing
//! `TRAP` mark, and enters the `catch (XLeaveException&)` or `catch (...)` landing pad with `r0` pointing at the
//! guest `XLeaveException` object.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::callback::CALLBACK_STUB;
use crate::layout::{EXIT_STUB, STUB_BASE};
use crate::loader::Process;
use crate::natives::NativeOutcome;

const EXIDX_CANTUNWIND: u32 = 1;
const MAX_UNWIND_FRAMES: usize = 64;

/// Saved unwinding state across a C++ cleanup landing pad (`__cxa_end_cleanup` resumes from `next_regs`).
#[derive(Debug, Clone)]
pub struct LeaveState {
    pub reason: i32,
    pub exc_obj: u32,
    /// Unwound caller registers to resume from when a cleanup landing pad calls `__cxa_end_cleanup`.
    pub next_regs: Option<[u32; 16]>,
}

fn prel31(base: u32, word: u32) -> u32 {
    let mut off = (word & 0x7FFF_FFFF) as i32;
    if (off & 0x4000_0000) != 0 {
        off |= !0x7FFF_FFFFu32 as i32;
    }
    base.wrapping_add(off as u32)
}

fn find_exidx_entry(p: &Process, pc_aligned: u32) -> Option<(u32, u32, u32)> {
    for &desc_addr in &p.exception_descriptors {
        let Ok(exidx_base) = p.rd(desc_addr) else { continue };
        let Ok(exidx_limit) = p.rd(desc_addr.wrapping_add(4)) else { continue };
        let Ok(ro_base) = p.rd(desc_addr.wrapping_add(8)) else { continue };
        let Ok(ro_limit) = p.rd(desc_addr.wrapping_add(12)) else { continue };
        if pc_aligned < (ro_base & !1) || pc_aligned >= ro_limit || exidx_limit <= exidx_base {
            continue;
        }
        let count = (exidx_limit - exidx_base) / 8;
        if count == 0 {
            continue;
        }
        let mut lo = 0u32;
        let mut hi = count;
        while lo + 1 < hi {
            let mid = lo + (hi - lo) / 2;
            let addr = exidx_base + mid * 8;
            let Ok(w0) = p.rd(addr) else { return None };
            let fn_addr = prel31(addr, w0) & !1;
            if fn_addr <= pc_aligned {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let entry_addr = exidx_base + lo * 8;
        let w0 = p.rd(entry_addr).ok()?;
        let w1 = p.rd(entry_addr.wrapping_add(4)).ok()?;
        let fn_addr = prel31(entry_addr, w0) & !1;
        if fn_addr <= pc_aligned {
            return Some((entry_addr, fn_addr, w1));
        }
    }
    None
}

struct FrameUnwind {
    pers: u8,
    ops: Vec<u8>,
    lsda: Option<u32>,
}

fn decode_frame_unwind(p: &Process, entry_addr: u32, w1: u32) -> Result<FrameUnwind, String> {
    if w1 == EXIDX_CANTUNWIND {
        return Err(String::from("EXIDX_CANTUNWIND"));
    }
    if (w1 & 0x8000_0000) != 0 {
        let pers = ((w1 >> 24) & 0x0F) as u8;
        let ops = alloc::vec![((w1 >> 16) & 0xFF) as u8, ((w1 >> 8) & 0xFF) as u8, (w1 & 0xFF) as u8];
        return Ok(FrameUnwind { pers, ops, lsda: None });
    }
    let extab = prel31(entry_addr.wrapping_add(4), w1);
    let w0 = p.rd(extab).map_err(|_| format!("unreadable extab at {extab:#x}"))?;
    if (w0 & 0x8000_0000) != 0 {
        let pers = ((w0 >> 24) & 0x0F) as u8;
        let mut ops = Vec::new();
        let lsda = if pers == 0 {
            ops.push(((w0 >> 16) & 0xFF) as u8);
            ops.push(((w0 >> 8) & 0xFF) as u8);
            ops.push((w0 & 0xFF) as u8);
            extab.wrapping_add(4)
        } else {
            let extra = ((w0 >> 16) & 0xFF) as u32;
            ops.push(((w0 >> 8) & 0xFF) as u8);
            ops.push((w0 & 0xFF) as u8);
            for i in 0..extra {
                let wi = p.rd(extab.wrapping_add(4 + i * 4)).map_err(|_| format!("unreadable extab word at {extab:#x}"))?;
                ops.push(((wi >> 24) & 0xFF) as u8);
                ops.push(((wi >> 16) & 0xFF) as u8);
                ops.push(((wi >> 8) & 0xFF) as u8);
                ops.push((wi & 0xFF) as u8);
            }
            extab.wrapping_add(4 + extra * 4)
        };
        Ok(FrameUnwind { pers, ops, lsda: Some(lsda) })
    } else {
        // Generic model: w0 is prel31 to personality routine; w1 starts unwind opcodes (count in bits 24..31).
        let w1e = p.rd(extab.wrapping_add(4)).map_err(|_| format!("unreadable generic extab at {extab:#x}"))?;
        let extra = ((w1e >> 24) & 0xFF) as u32;
        let mut ops = Vec::new();
        ops.push(((w1e >> 16) & 0xFF) as u8);
        ops.push(((w1e >> 8) & 0xFF) as u8);
        ops.push((w1e & 0xFF) as u8);
        for i in 0..extra {
            let wi = p.rd(extab.wrapping_add(8 + i * 4)).map_err(|_| format!("unreadable generic extab word at {extab:#x}"))?;
            ops.push(((wi >> 24) & 0xFF) as u8);
            ops.push(((wi >> 16) & 0xFF) as u8);
            ops.push(((wi >> 8) & 0xFF) as u8);
            ops.push((wi & 0xFF) as u8);
        }
        Ok(FrameUnwind { pers: 1, ops, lsda: Some(extab.wrapping_add(8 + extra * 4)) })
    }
}

fn is_xleave_rtti(p: &Process, rtti_addr: u32, word: u32) -> bool {
    if word == 0xFFFF_FFFF {
        return true; // catch (...)
    }
    if word == 0xFFFF_FFFE || word == 0 {
        return false;
    }
    let check_stub = |addr: u32| -> bool {
        if addr >= STUB_BASE && (addr & 7) == 0 {
            let idx = ((addr - STUB_BASE) / 8) as usize;
            if let Some(s) = p.stubs.get(idx) {
                return s.name.as_deref() == Some("_ZTI15XLeaveException");
            }
        }
        false
    };
    if check_stub(word) {
        return true;
    }
    let rel = prel31(rtti_addr, word);
    if check_stub(rel) {
        return true;
    }
    if let Ok(indirect) = p.rd(rel) {
        if check_stub(indirect) {
            return true;
        }
    }
    false
}

enum LsdeAction {
    None,
    Cleanup(u32),
    Catch(u32),
}

fn search_lsda(p: &Process, pers: u8, lsda: u32, fn_addr: u32, call_site: u32) -> LsdeAction {
    let rel_pc = call_site.wrapping_sub(fn_addr);
    let mut pos = lsda;
    for _ in 0..256 {
        let Ok(d0) = p.rd(pos) else { break };
        if d0 == 0 {
            break;
        }
        if pers == 2 {
            // Long 32-bit descriptor format (pr2)
            let Ok(d1) = p.rd(pos.wrapping_add(4)) else { break };
            let k0 = d0 & 1;
            let k1 = d1 & 1;
            let off = d0 & !1;
            let len = d1 & !1;
            if k0 == 0 && k1 == 0 {
                let Ok(lp_w) = p.rd(pos.wrapping_add(8)) else { break };
                if rel_pc >= off && rel_pc < off.wrapping_add(len) {
                    return LsdeAction::Cleanup(prel31(pos.wrapping_add(8), lp_w));
                }
                pos = pos.wrapping_add(12);
            } else if k0 == 1 && k1 == 0 {
                let Ok(lp_w) = p.rd(pos.wrapping_add(8)) else { break };
                let Ok(rtti) = p.rd(pos.wrapping_add(12)) else { break };
                if rel_pc >= off && rel_pc < off.wrapping_add(len) && is_xleave_rtti(p, pos.wrapping_add(12), rtti) {
                    return LsdeAction::Catch(prel31(pos.wrapping_add(8), lp_w));
                }
                pos = pos.wrapping_add(16);
            } else if k0 == 0 && k1 == 1 {
                let count = d0 >> 1;
                pos = pos.wrapping_add(4 + count.wrapping_mul(4));
            } else {
                break;
            }
        } else {
            // Short 16-bit descriptor format (pr0 / pr1)
            let k0 = d0 & 1;
            let k1 = (d0 >> 16) & 1;
            let off = d0 & 0xFFFE;
            let len = (d0 >> 16) & 0xFFFE;
            if k0 == 0 && k1 == 0 {
                let Ok(lp_w) = p.rd(pos.wrapping_add(4)) else { break };
                if rel_pc >= off && rel_pc < off.wrapping_add(len) {
                    return LsdeAction::Cleanup(prel31(pos.wrapping_add(4), lp_w));
                }
                pos = pos.wrapping_add(8);
            } else if k0 == 1 && k1 == 0 {
                let Ok(lp_w) = p.rd(pos.wrapping_add(4)) else { break };
                let Ok(rtti) = p.rd(pos.wrapping_add(8)) else { break };
                if rel_pc >= off && rel_pc < off.wrapping_add(len) && is_xleave_rtti(p, pos.wrapping_add(8), rtti) {
                    return LsdeAction::Catch(prel31(pos.wrapping_add(4), lp_w));
                }
                pos = pos.wrapping_add(12);
            } else if k0 == 0 && k1 == 1 {
                let count = (d0 >> 17) & 0x7FFF;
                pos = pos.wrapping_add(4 + count.wrapping_mul(4));
            } else {
                break;
            }
        }
    }
    LsdeAction::None
}

fn execute_unwind_ops(p: &Process, ops: &[u8], regs: &mut [u32; 16]) -> Result<(), String> {
    let mut vsp = regs[13];
    let mut pc_set = false;
    let mut i = 0usize;
    while i < ops.len() {
        let b = ops[i];
        i += 1;
        match b {
            0x00..=0x3F => {
                vsp = vsp.wrapping_add(((b as u32 & 0x3F) << 2) + 4);
            }
            0x40..=0x7F => {
                vsp = vsp.wrapping_sub(((b as u32 & 0x3F) << 2) + 4);
            }
            0x80..=0x8F => {
                if i >= ops.len() { break; }
                let b2 = ops[i];
                i += 1;
                let mask = (((b & 0x0F) as u16) << 8) | (b2 as u16);
                if mask == 0 {
                    return Err(String::from("EHABI REFUSE_UNWIND (0x80 0x00)"));
                }
                let mut new_sp = None;
                for bit in 0..12u32 {
                    if (mask & (1 << bit)) != 0 {
                        let reg = (bit + 4) as usize;
                        let val = p.rd(vsp).map_err(|_| format!("EHABI stack read at {vsp:#x} failed"))?;
                        vsp = vsp.wrapping_add(4);
                        if reg == 13 {
                            new_sp = Some(val);
                        } else {
                            regs[reg] = val;
                            if reg == 15 {
                                pc_set = true;
                            }
                        }
                    }
                }
                if let Some(sp) = new_sp {
                    vsp = sp;
                }
            }
            0x90..=0x9F => {
                let r = (b & 0x0F) as usize;
                if r == 13 || r == 15 {
                    return Err(format!("EHABI reserved vsp=r{r}"));
                }
                vsp = regs[r];
            }
            0xA0..=0xAF => {
                let n = (b & 0x07) as usize;
                for reg in 4..=(4 + n) {
                    regs[reg] = p.rd(vsp).map_err(|_| format!("EHABI stack read at {vsp:#x} failed"))?;
                    vsp = vsp.wrapping_add(4);
                }
                if (b & 0x08) != 0 {
                    regs[14] = p.rd(vsp).map_err(|_| format!("EHABI stack read lr at {vsp:#x} failed"))?;
                    vsp = vsp.wrapping_add(4);
                }
            }
            0xB0 => break, // Finish
            0xB1 => {
                if i >= ops.len() { break; }
                let b2 = ops[i];
                i += 1;
                let mask = b2 & 0x0F;
                if mask == 0 {
                    return Err(String::from("EHABI spare 0xB1 0x00"));
                }
                for reg in 0..4usize {
                    if (mask & (1 << reg)) != 0 {
                        regs[reg] = p.rd(vsp).map_err(|_| format!("EHABI stack read r{reg} at {vsp:#x} failed"))?;
                        vsp = vsp.wrapping_add(4);
                    }
                }
            }
            0xB2 => {
                let mut u = 0u32;
                let mut shift = 0u32;
                while i < ops.len() {
                    let byte = ops[i];
                    i += 1;
                    u |= ((byte & 0x7F) as u32) << shift;
                    if (byte & 0x80) == 0 { break; }
                    shift += 7;
                }
                vsp = vsp.wrapping_add(0x204 + (u << 2));
            }
            0xB3 => {
                if i >= ops.len() { break; }
                let b2 = ops[i];
                i += 1;
                let count = (b2 & 0x0F) as u32 + 1;
                vsp = vsp.wrapping_add(count * 8 + 4);
            }
            0xC8 | 0xC9 => {
                if i >= ops.len() { break; }
                let b2 = ops[i];
                i += 1;
                let count = (b2 & 0x0F) as u32 + 1;
                vsp = vsp.wrapping_add(count * 8);
            }
            0xD0..=0xD7 => {
                let count = (b & 0x07) as u32 + 1;
                vsp = vsp.wrapping_add(count * 8);
            }
            other => return Err(format!("EHABI unsupported unwind opcode {other:#04x}")),
        }
    }
    regs[13] = vsp;
    if !pc_set {
        regs[15] = regs[14];
    }
    Ok(())
}

/// Unwind the guest stack from `start_regs` for a `User::Leave(reason)` exception.
fn unwind_from(p: &mut Process, reason: i32, exc_obj: u32, mut regs: [u32; 16]) -> Result<(), String> {
    for _ in 0..MAX_UNWIND_FRAMES {
        let cur_pc = regs[15];
        if cur_pc == 0 || cur_pc == EXIT_STUB || cur_pc == CALLBACK_STUB {
            return Err(format!("uncaught User::Leave({reason}) reached frame boundary {cur_pc:#x}"));
        }
        let call_site = (cur_pc & !1).wrapping_sub(1);
        let Some((entry_addr, fn_addr, w1)) = find_exidx_entry(p, call_site) else {
            return Err(format!("uncaught User::Leave({reason}): no .ARM.exidx entry for {cur_pc:#x}"));
        };
        let frame = decode_frame_unwind(p, entry_addr, w1)?;
        if let Some(lsda) = frame.lsda {
            match search_lsda(p, frame.pers, lsda, fn_addr, call_site) {
                LsdeAction::Cleanup(lp) => {
                    // Compute the unwound caller frame now so `__cxa_end_cleanup` can resume past out-of-line cleanup pads.
                    let mut next = regs;
                    let next_opt = execute_unwind_ops(p, &frame.ops, &mut next).ok().map(|_| next);
                    p.nat.active_leave = Some(LeaveState { reason, exc_obj, next_regs: next_opt });
                    p.cpu.regs = regs;
                    p.cpu.regs[0] = exc_obj;
                    p.cpu.set_pc_interworking(lp);
                    return Ok(());
                }
                LsdeAction::Catch(lp) => {
                    p.nat.active_leave = Some(LeaveState { reason, exc_obj, next_regs: None });
                    p.cpu.regs = regs;
                    p.cpu.regs[0] = exc_obj;
                    p.cpu.set_pc_interworking(lp);
                    // Drain CleanupStack items pushed inside the TRAP bracket before entering the catch handler.
                    if let Some(mark) = p.nat.trap_marks.pop() {
                        let have = p.nat.cleanup.len() as u32;
                        if have > mark {
                            if let Err(o) = crate::cleanup::destroy_next(p, have - mark) {
                                return Err(format!("CleanupStack destroy during Leave({reason}) failed: {o:?}"));
                            }
                        }
                    }
                    return Ok(());
                }
                LsdeAction::None => {}
            }
        }
        execute_unwind_ops(p, &frame.ops, &mut regs)?;
    }
    Err(format!("User::Leave({reason}) exceeded {MAX_UNWIND_FRAMES} unwind frames"))
}

/// Perform a Symbian `User::Leave(reason)` from a native call (caller's return address is in `p.cpu.regs[14]`).
pub fn leave(p: &mut Process, reason: i32) -> NativeOutcome {
    if p.exception_descriptors.is_empty() {
        return NativeOutcome::Refused(format!("NEXT-NEEDED leave User::Leave({reason}): image has no E32 exception descriptor"));
    }
    let exc_obj = match p.nat.leave_exc_obj {
        Some(a) => a,
        None => match p.alloc_zeroed(16) {
            Some(a) => { p.nat.leave_exc_obj = Some(a); a }
            None => return NativeOutcome::Refused(format!("NEXT-NEEDED leave User::Leave({reason}): no guest heap for XLeaveException")),
        },
    };
    let _ = p.wr(exc_obj, reason as u32);
    let mut regs = p.cpu.regs;
    regs[15] = regs[14];
    match unwind_from(p, reason, exc_obj, regs) {
        Ok(()) => NativeOutcome::Return,
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED leave {m}")),
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let r0 = p.cpu.regs[0];
    Some(match name {
        "_ZN4User5LeaveEi" => leave(p, r0 as i32),
        "_ZN4User12LeaveIfErrorEi" => {
            if (r0 as i32) < 0 {
                leave(p, r0 as i32)
            } else {
                NativeOutcome::Return
            }
        }
        // TInt XLeaveException::GetReason() const
        "_ZNK15XLeaveException9GetReasonEv" => {
            let reason = p.rd(r0).ok().map(|v| v as i32)
                .or_else(|| p.nat.active_leave.as_ref().map(|l| l.reason))
                .unwrap_or(-1);
            p.cpu.regs[0] = reason as u32;
            NativeOutcome::Return
        }
        // void* __cxa_begin_catch(void* exceptionObject)
        "__cxa_begin_catch" => {
            let obj = if r0 != 0 { r0 } else { p.nat.active_leave.as_ref().map(|l| l.exc_obj).unwrap_or(0) };
            p.cpu.regs[0] = obj;
            NativeOutcome::Return
        }
        // void __cxa_end_catch()
        "__cxa_end_catch" => {
            p.nat.active_leave = None;
            NativeOutcome::Return
        }
        // void __cxa_end_cleanup(): resume unwinding after a C++ cleanup landing pad finishes.
        "__cxa_end_cleanup" => {
            let Some(state) = p.nat.active_leave.clone() else {
                return Some(NativeOutcome::Refused(String::from("NEXT-NEEDED ehabi __cxa_end_cleanup: no active leave")));
            };
            let regs = match state.next_regs {
                Some(r) => r,
                None => {
                    let mut r = p.cpu.regs;
                    r[15] = r[14];
                    r
                }
            };
            match unwind_from(p, state.reason, state.exc_obj, regs) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED leave {m}")),
            }
        }
        // C++ ABI one-time local static initialization guards (__cxa_guard_acquire/release/abort).
        "__cxa_guard_acquire" => {
            let done = p.mem.vm.read_u8(r0).unwrap_or(1) != 0;
            p.cpu.regs[0] = if done { 0 } else { 1 };
            NativeOutcome::Return
        }
        "__cxa_guard_release" => {
            let _ = p.mem.vm.write_u8(r0, 1);
            NativeOutcome::Return
        }
        "__cxa_guard_abort" => {
            let _ = p.mem.vm.write_u8(r0, 0);
            NativeOutcome::Return
        }
        // ARM C++ ABI array construction/destruction helpers (IHI 0041 §3.2.2).
        "__cxa_vec_ctor" => {
            let (arr, count, elem_sz, ctor) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
            let mut calls = Vec::new();
            if ctor != 0 {
                for i in 0..count {
                    calls.push((ctor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            match p.call_chain(calls, arr) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi __cxa_vec_ctor: {m}")),
            }
        }
        "__cxa_vec_dtor" => {
            let (arr, count, elem_sz, dtor) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
            let mut calls = Vec::new();
            if dtor != 0 {
                for i in (0..count).rev() {
                    calls.push((dtor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            match p.call_chain(calls, arr) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi __cxa_vec_dtor: {m}")),
            }
        }
        "__cxa_vec_new" | "__cxa_vec_new2" => {
            let (count, elem_sz, pad, ctor) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
            let total = pad.saturating_add(count.saturating_mul(elem_sz)).max(1);
            let Some(base) = p.alloc_zeroed(total) else {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: no guest heap for {total} bytes")));
            };
            if pad >= 8 {
                let _ = p.wr(base, elem_sz);
                let _ = p.wr(base.wrapping_add(4), count);
            }
            let arr = base.wrapping_add(pad);
            let mut calls = Vec::new();
            if ctor != 0 {
                for i in 0..count {
                    calls.push((ctor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            match p.call_chain(calls, arr) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: {m}")),
            }
        }
        // ARM EABI C++ vector helpers (IHI 0041 §3.2.2.3): (r0=array, r1=ctor/dtor, r2=elem_size, r3=count)
        "__aeabi_vec_ctor_nocookie_nodtor" | "__aeabi_vec_ctor_cookie_nodtor" => {
            let (raw_arr, ctor, elem_sz, count) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
            let arr = if name.contains("_cookie_") {
                let _ = p.wr(raw_arr, elem_sz);
                let _ = p.wr(raw_arr.wrapping_add(4), count);
                raw_arr.wrapping_add(8)
            } else {
                raw_arr
            };
            let mut calls = Vec::new();
            if ctor != 0 {
                for i in 0..count {
                    calls.push((ctor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            match p.call_chain(calls, arr) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: {m}")),
            }
        }
        "__aeabi_vec_dtor" => {
            let (arr, dtor, elem_sz, count) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
            let mut calls = Vec::new();
            if dtor != 0 {
                for i in (0..count).rev() {
                    calls.push((dtor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            match p.call_chain(calls, arr) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: {m}")),
            }
        }
        "__aeabi_vec_delete" | "__aeabi_vec_delete3" => {
            let (arr, dtor) = (p.cpu.regs[0], p.cpu.regs[1]);
            if arr == 0 {
                return Some(NativeOutcome::Return);
            }
            let cookie = arr.wrapping_sub(8);
            let elem_sz = p.rd(cookie).unwrap_or(0);
            let count = p.rd(cookie.wrapping_add(4)).unwrap_or(0);
            let mut calls = Vec::new();
            if dtor != 0 {
                for i in (0..count).rev() {
                    calls.push((dtor, [arr.wrapping_add(i.wrapping_mul(elem_sz)), 0, 0, 0]));
                }
            }
            if let Some(h) = p.heap.as_mut() { let _ = h.free(cookie); }
            match p.call_chain(calls, 0) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED drtaeabi {name}: {m}")),
            }
        }
        _ => return None,
    })
}
