//! `CallUniversalProc(upp, procInfo, ...)` (Mixed Mode, InterfaceLib; Inside Macintosh: PowerPC System Software 2-31): call the routine
//! a UPP names with the arguments `procInfo` describes. Served by the run loop through the ONE callback machine ([`crate::callback`]).
//! Which routine, by Mixed Mode's rule ([`heirloom_mac68k_engine::mixed_mode::resolve`]: a routine descriptor names its records' ISAs;
//! any other pointer is a 68k ProcPtr):
//! * a PowerPC record -> the program's own PowerPC code runs; its r3 comes back as the result;
//! * a 68k TRAP's stub (the address `GetTrapAddress` gave, heirloom-mac68k-engine `traptable::in_stub`) whose trap the engine serves BY
//!   VALUE -> the same body the 68k trap runs (today `_CursorDeviceDispatch` $AADB, D0-dispatched Pascal: Warcraft III's CursorDeviceLib);
//!   the `_Unimplemented` stub -> refused by name (a real Mac calling it stops with a system error);
//! * any other 68k code (a 68k record, a plain ProcPtr, another trap's stub) -> runs on the 68k CPU ([`crate::xisa`]), marshalled by
//!   `procInfo` (stack-based Pascal or C; other conventions are named refusals);
//! * NIL -> refused by name.
//! `procInfo`: bits 0-3 the calling convention, 4-5 the result size; for a D0-dispatched convention bits 6-7 the selector size and
//! the parameters from bit 8, two bits each (Universal Interfaces MixedMode.h).
use crate::callback::{Continuation, Frame, Saved};
use crate::cpu::Cpu;
use crate::invoke_c::Result_;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;

/// kD0DispatchedPascalStackBased.
const D0_DISPATCHED_PASCAL: u32 = 8;
/// `_Unimplemented` (the stub every unimplemented trap's address is).
const UNIMPLEMENTED_TRAP: u16 = 0xA89F;
/// `_CursorDeviceDispatch`.
const CURSOR_DEVICE_DISPATCH: u16 = 0xAADB;

/// The parameter sizes `procInfo` gives from bit `from` (0 = none from there on): how many parameters.
fn param_count(proc_info: u32, from: u32) -> usize {
    (0..13).take_while(|k| from + 2 * k < 32 && (proc_info >> (from + 2 * k)) & 3 != 0).count()
}

impl<U: GuestUi> MacPpc<U> {
    /// The glue call was reached with r3 = upp, r4 = procInfo, r5.. the routine's arguments.
    pub(crate) fn call_universal_proc(&mut self, cpu: &mut Cpu) -> Result<(), String> {
        let (upp, info) = (cpu.gpr[3], cpu.gpr[4]);
        if upp == 0 { return Err(String::from("[HEIRLOOM-MACPPC CallUniversalProc REFUSED upp=NIL] the program called a NIL procedure")); }
        let conv = info & 0xF;
        if heirloom_mac68k_engine::traptable::in_stub(upp) {
            let word = self.mac.mem.read_u16(upp).map_err(|e| format!("CallUniversalProc: the trap stub at {upp:#x}: {e}"))?;
            if word == CURSOR_DEVICE_DISPATCH && conv == D0_DISPATCHED_PASCAL {
                let n = param_count(info, 8);
                let args: Vec<u32> = cpu.gpr[6..6 + n.min(5)].to_vec();
                let e = self.mac.cursor_device_call(cpu.gpr[5], &args).map_err(|r| format!("CallUniversalProc: {}", r.0))?;
                cpu.gpr[3] = e as i32 as u32;
                return Ok(());
            }
            if word == UNIMPLEMENTED_TRAP {
                let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED trap={word:#06x} procInfo={info:#x}] the stub behind {upp:#x} is _Unimplemented (a real Mac stops with a system error)");
                self.mac.refused.push(why.clone());
                return Err(why);
            }
            // any other trap's stub is 68k code: it runs on the 68k CPU below, its trap served by the Toolbox like a 68k program's
        }
        let refuse = |m: &mut Self, e: String| { let why = format!("[HEIRLOOM-MACPPC CallUniversalProc REFUSED upp={upp:#x}] {e}"); m.mac.refused.push(why.clone()); why };
        let effective_upp = self.prepare_accelerated_upp(upp).map_err(|e| refuse(self, e))?;
        let routine = heirloom_mac68k_engine::mixed_mode::resolve(&self.mac.mem, effective_upp).map_err(|e| refuse(self, format!("{e}")))?;
        let n = param_count(info, if matches!(conv, 8 | 9 | 12) { 8 } else { 6 });
        let args = crate::xisa::universal_args(&self.mac.mem, cpu, n).map_err(|e| refuse(self, e))?;
        match routine {
            heirloom_mac68k_engine::mixed_mode::Routine::PowerPc(record) => {
                if n > 8 { return Err(refuse(self, format!("a PowerPC routine with {n} parameters (more than r3..r10) is not entered yet"))); }
                let res = match (info >> 4) & 3 { 0 => Result_::None, 1 => Result_::Boolean, _ => Result_::Status };
                let frame = Frame { saved: Saved::of(cpu, cpu.lr & !3), cont: Continuation::Invoke(res) };
                let sp = cpu.gpr[1];
                self.enter_tvector(cpu, record.proc_descriptor, &args, sp).map_err(|e| refuse(self, e))?;
                self.rt.callbacks.push(frame);
            }
            // the caller's procInfo describes the call (Mixed Mode marshals by it, not by the record's)
            heirloom_mac68k_engine::mixed_mode::Routine::M68k { code, .. } => {
                self.enter_68k(cpu, code, info, &args).map_err(|e| refuse(self, format!("68k code at {code:#x}: {e}")))?;
            }
        }
        Ok(())
    }

    /// Follow a 68k-ISA descriptor that wraps an inner RoutineDescriptor (`NewRoutineDescriptor(codeRsrc, info, kM68kISA)`
    /// on an accelerated resource) and prepare any PowerPC accelerated resource (`kProcDescriptorIsRelative |
    /// kFragmentNeedsPreparing`) in place, rewriting its RoutineRecord to the prepared fragment's main TVector exactly as
    /// Apple's Mixed Mode Manager does (Inside Macintosh: PowerPC System Software, "Accelerated Resources").
    fn prepare_accelerated_upp(&mut self, mut addr: u32) -> Result<u32, String> {
        use heirloom_mac68k_engine::mixed_mode::{
            parse, FRAGMENT_NEEDS_PREPARING, HEADER_LEN, ISA_M68K, ISA_PPC, PROC_DESCRIPTOR_IS_RELATIVE, RECORD_LEN,
        };
        // Unwrap at most 4 chained RoutineDescriptors (a 68k record pointing directly at another `$AAFE` descriptor).
        let mut desc = None;
        for _ in 0..4 {
            let Ok(d) = parse(&self.mac.mem, addr) else { break };
            if d.records.iter().any(|r| r.isa == ISA_PPC) {
                desc = Some(d);
                break;
            }
            let next = d.records.iter().find(|r| r.isa == ISA_M68K && r.flags & PROC_DESCRIPTOR_IS_RELATIVE == 0 && r.proc_descriptor != 0)
                .map(|r| r.proc_descriptor);
            match next {
                Some(target) if target != addr && parse(&self.mac.mem, target).is_ok() => addr = target,
                _ => { desc = Some(d); break; }
            }
        }
        let Some(d) = desc else { return Ok(addr); };
        let Some((idx, rec)) = d.records.iter().enumerate().find(|(_, r)| r.isa == ISA_PPC) else { return Ok(d.at); };
        if rec.flags & (PROC_DESCRIPTOR_IS_RELATIVE | FRAGMENT_NEEDS_PREPARING) == 0 {
            return Ok(d.at);
        }
        let pef_addr = if rec.flags & PROC_DESCRIPTOR_IS_RELATIVE != 0 {
            d.at.checked_add(rec.proc_descriptor).ok_or_else(|| format!("accelerated resource at {:#x}: relative offset {:#x} overflows", d.at, rec.proc_descriptor))?
        } else {
            rec.proc_descriptor
        };
        let len = pef_container_len(&self.mac.mem, pef_addr)
            .map_err(|e| format!("accelerated resource at {:#x}: PEF container at {pef_addr:#x}: {e}", d.at))?;
        let bytes = self.mac.mem.span(pef_addr, len)
            .and_then(|sp| self.mac.mem.bytes(sp).map(<[u8]>::to_vec))
            .map_err(|e| format!("accelerated resource at {:#x}: PEF bytes at {pef_addr:#x}+{len}: {e}", d.at))?;
        let lib = format!("accelerated@{:#x}", d.at);
        self.provide_library(&lib, &bytes);
        self.ensure_loaded(&lib);
        let main = match self.libs.get(lib.as_str()) {
            Some(crate::machine::LibSlot::Loaded(_, Some(main))) => *main,
            Some(crate::machine::LibSlot::Failed(why)) => return Err(format!("accelerated resource at {:#x}: PEF does not prepare ({why})", d.at)),
            _ => return Err(format!("accelerated resource at {:#x}: PEF has no main symbol", d.at)),
        };
        let rec_at = d.at + HEADER_LEN + (idx as u32) * RECORD_LEN;
        let new_flags = rec.flags & !(PROC_DESCRIPTOR_IS_RELATIVE | FRAGMENT_NEEDS_PREPARING);
        self.mac.mem.write_u16(rec_at + 6, new_flags).map_err(|e| format!("accelerated resource at {:#x}: {e}", d.at))?;
        self.mac.mem.write_u32(rec_at + 8, main).map_err(|e| format!("accelerated resource at {:#x}: {e}", d.at))?;
        self.mac.log.push(format!("[cfm] accelerated resource at {:#x} prepared ({len} byte PEF at {pef_addr:#x}) -> main {main:#x}; record is now absolute", d.at));
        Ok(d.at)
    }
}

/// Exact byte length of a PEF container residing in guest memory at `addr` (40-byte container header, `sectionCount`
/// 28-byte section headers, section name table strings, and all section bodies).
fn pef_container_len(mem: &heirloom_mem_flat32::GuestMem, addr: u32) -> Result<u32, String> {
    let hdr = mem.span(addr, 40).and_then(|s| mem.bytes(s).map(<[u8]>::to_vec)).map_err(|e| format!("header unreadable: {e}"))?;
    if &hdr[..8] != b"Joy!peff" { return Err(String::from("missing Joy!peff magic")); }
    let sec_count = u32::from(u16::from_be_bytes([hdr[32], hdr[33]]));
    if sec_count > 256 { return Err(format!("implausible sectionCount {sec_count}")); }
    let names_base = 40u32.checked_add(sec_count * 28).ok_or_else(|| String::from("section table overflow"))?;
    let mut end = names_base;
    for i in 0..sec_count {
        let s_at = addr.checked_add(40 + i * 28).ok_or_else(|| String::from("section header address overflow"))?;
        let sh = mem.span(s_at, 28).and_then(|s| mem.bytes(s).map(<[u8]>::to_vec)).map_err(|e| format!("section {i} header unreadable: {e}"))?;
        let name_off = i32::from_be_bytes([sh[0], sh[1], sh[2], sh[3]]);
        let packed = u32::from_be_bytes([sh[16], sh[17], sh[18], sh[19]]);
        let c_off = u32::from_be_bytes([sh[20], sh[21], sh[22], sh[23]]);
        let sec_end = c_off.checked_add(packed).ok_or_else(|| format!("section {i} extent overflow"))?;
        end = end.max(sec_end);
        if name_off >= 0 {
            let mut p = names_base.checked_add(name_off as u32).ok_or_else(|| format!("section {i} name offset overflow"))?;
            for _ in 0..1024 {
                let b = mem.read_u8(addr.wrapping_add(p)).map_err(|e| format!("section {i} name unreadable: {e}"))?;
                p += 1;
                if b == 0 { break; }
            }
            end = end.max(p);
        }
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::{param_count, pef_container_len};
    use crate::MacPpc;
    use heirloom_ui_abi::RecordingGuestUi;

    #[test]
    fn procinfo_parameter_fields_are_counted_until_the_first_empty_one() {
        // Warcraft III's CursorDeviceNextDevice: kD0DispatchedPascalStackBased, OSErr, a 4-byte selector, one 4-byte parameter
        assert_eq!(param_count(0x3E8, 8), 1);
        assert_eq!(param_count(0x0, 6), 0);
        assert_eq!(param_count(0xFFC1, 6), 5);
    }

    #[test]
    fn pef_container_len_measures_sections_and_rejects_non_pef() {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        let addr = 0x4000u32;
        // Negative control: not a PEF container
        m.mac.mem.write_bytes(addr, b"NotAPEF!").unwrap();
        assert!(pef_container_len(&m.mac.mem, addr).is_err());
        // Minimal 1-section PEF header (40 + 28 bytes) with section at offset 80, packedSize 16 -> total 96
        let mut hdr = [0u8; 68];
        hdr[..8].copy_from_slice(b"Joy!peff");
        hdr[8..12].copy_from_slice(b"pwpc");
        hdr[32..34].copy_from_slice(&1u16.to_be_bytes());
        hdr[40..44].copy_from_slice(&(-1i32).to_be_bytes()); // nameOffset = -1
        hdr[56..60].copy_from_slice(&16u32.to_be_bytes()); // packedSize = 16
        hdr[60..64].copy_from_slice(&80u32.to_be_bytes()); // containerOffset = 80
        m.mac.mem.write_bytes(addr, &hdr).unwrap();
        assert_eq!(pef_container_len(&m.mac.mem, addr).unwrap(), 96);
    }
}
