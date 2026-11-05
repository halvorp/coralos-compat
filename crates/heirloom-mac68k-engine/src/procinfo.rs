//! Mixed Mode calling conventions on the 68k side: what a `procInfo` (MixedMode.h) says about a 68k routine's stack frame, and the
//! frame a caller from another ISA builds for it (heirloom-macppc-engine `xisa`, `CallUniversalProc` of 68k code).
//!
//! `procInfo` bits 0-3 are the calling convention, bits 4-5 the result size, and for a stack-based convention two bits per parameter
//! from bit 6 (`kNoByteCode` 0 ends the list, `kOneByteCode` 1, `kTwoByteCode` 2, `kFourByteCode` 3). Two conventions are built here:
//! * `kPascalStackBased` (0): the caller reserves the result's space, pushes the parameters LEFT TO RIGHT, the callee pops them; a 1-byte
//!   parameter or result occupies a word with its value in the HIGH byte (a 68k `MOVE.B` to `-(A7)` moves A7 by two);
//! * `kCStackBased` (1): the caller pushes the parameters RIGHT TO LEFT and pops them; the result is in D0. A 1- or 2-byte parameter
//!   occupies a word with its value in the low byte(s).
//! The register-based, dispatched and special-case conventions are named refusals here (owner heirloom-macppc-engine; trigger: the first
//! program whose 68k routine needs one).
use crate::mixed_mode::MixedModeError;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_mem_flat32::GuestMem;

pub const K_PASCAL_STACK_BASED: u32 = 0;
pub const K_C_STACK_BASED: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackConv { Pascal, C }

/// A stack-based call decoded from a procInfo: the convention, the result's size in bytes (0, 1, 2 or 4), each parameter's size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackCall { pub conv: StackConv, pub result: u8, pub params: Vec<u8> }

/// The frame a caller built ([`push_frame`]): the 68k SP the routine is entered with (the return address on top), where a Pascal
/// result is read, and the SP the routine must leave behind when it returns (callee-popped for Pascal, caller-popped for C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame68k { pub sp: u32, pub result_at: u32, pub sp_after: u32 }

fn size_of_code(code: u32) -> u8 { [0, 1, 2, 4][(code & 3) as usize] }

/// Decode `proc_info`; a convention this module does not build is a named refusal.
pub fn stack_call(proc_info: u32) -> Result<StackCall, String> {
    let conv = match proc_info & 0xF {
        K_PASCAL_STACK_BASED => StackConv::Pascal,
        K_C_STACK_BASED => StackConv::C,
        c => return Err(format!("procInfo {proc_info:#x}: calling convention {c} ({}) is not built for a 68k routine yet", conv_name(c))),
    };
    let params = (0..13).map(|k| (proc_info >> (6 + 2 * k)) & 3).take_while(|&c| c != 0).map(size_of_code).collect();
    Ok(StackCall { conv, result: size_of_code(proc_info >> 4), params })
}

fn conv_name(c: u32) -> &'static str {
    match c {
        2 => "kRegisterBased", 5 => "kThinkCStackBased", 8 => "kD0DispatchedPascalStackBased", 9 => "kD0DispatchedCStackBased",
        12 => "kD1DispatchedPascalStackBased", 14 => "kStackDispatchedPascalStackBased", 15 => "kSpecialCase", _ => "reserved",
    }
}

fn push(mem: &mut GuestMem, sp: &mut u32, size: u8, v: u32) -> Result<(), MixedModeError> {
    let at = sp.checked_sub(if size == 4 { 4 } else { 2 }).ok_or(MixedModeError::Unreadable(*sp))?;
    let e = |_| MixedModeError::Unreadable(at);
    if size == 4 { mem.write_u32(at, v).map_err(e)?; } else { mem.write_u16(at, v as u16).map_err(e)?; }
    *sp = at;
    Ok(())
}

/// Build `call`'s frame below `sp` (word-aligned) with `args` (the parameters' values, low bits significant) and `ret` as the return
/// address. `args` must hold one value per parameter.
pub fn push_frame(mem: &mut GuestMem, sp: u32, call: &StackCall, args: &[u32], ret: u32) -> Result<Frame68k, MixedModeError> {
    if args.len() != call.params.len() { return Err(MixedModeError::NoRunnableRecord { at: 0, why: format!("{} arguments for {} parameters", args.len(), call.params.len()) }); }
    let mut sp = sp & !1;
    let byte_hi = |size: u8, v: u32| if size == 1 { (v & 0xFF) << 8 } else { v };
    let result_at = match call.conv {
        StackConv::Pascal => {
            if call.result != 0 { push(mem, &mut sp, call.result, 0)?; }
            let result_at = sp;
            for (&size, &v) in call.params.iter().zip(args) { push(mem, &mut sp, size, byte_hi(size, v))?; }
            result_at
        }
        StackConv::C => {
            for (&size, &v) in call.params.iter().zip(args).rev() { push(mem, &mut sp, size, if size == 1 { v & 0xFF } else { v })?; }
            sp
        }
    };
    let sp_after = match call.conv { StackConv::Pascal => result_at, StackConv::C => sp };
    push(mem, &mut sp, 4, ret)?;
    Ok(Frame68k { sp, result_at, sp_after })
}

/// The routine's result, zero-extended: from the reserved space (Pascal; a 1-byte result is the HIGH byte of its word) or D0 (C).
pub fn read_result(mem: &GuestMem, call: &StackCall, frame: &Frame68k, d0: u32) -> Result<u32, MixedModeError> {
    let e = |_| MixedModeError::Unreadable(frame.result_at);
    Ok(match (call.conv, call.result) {
        (_, 0) => 0,
        (StackConv::Pascal, 1) => u32::from(mem.read_u8(frame.result_at).map_err(e)?),
        (StackConv::Pascal, 2) => u32::from(mem.read_u16(frame.result_at).map_err(e)?),
        (StackConv::Pascal, _) => mem.read_u32(frame.result_at).map_err(e)?,
        (StackConv::C, 1) => d0 & 0xFF,
        (StackConv::C, 2) => d0 & 0xFFFF,
        (StackConv::C, _) => d0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procinfo_fields_decode_and_unbuilt_conventions_are_refused_by_name() {
        // a "returns A5" routine (the first program measured): Pascal, a 4-byte result, no parameters
        assert_eq!(stack_call(0x30).unwrap(), StackCall { conv: StackConv::Pascal, result: 4, params: vec![] });
        // C, 2-byte result, params 4, 1, 2
        assert_eq!(stack_call(0x1 | (2 << 4) | (3 << 6) | (1 << 8) | (2 << 10)).unwrap(), StackCall { conv: StackConv::C, result: 2, params: vec![4, 1, 2] });
        assert!(stack_call(0x2).unwrap_err().contains("kRegisterBased"));
        assert!(stack_call(0x3E8).unwrap_err().contains("kD0DispatchedPascalStackBased"));
    }

    #[test]
    fn a_pascal_frame_reserves_the_result_pushes_left_to_right_with_bytes_high() {
        let mut mem = GuestMem::new(0x10000);
        let call = stack_call(0x30 | (1 << 6) | (3 << 8)).unwrap(); // (Boolean, long): long
        let f = push_frame(&mut mem, 0x8000, &call, &[1, 0xCAFE_F00D], 0x518).unwrap();
        assert_eq!((f.result_at, f.sp_after, f.sp), (0x7FFC, 0x7FFC, 0x7FFC - 2 - 4 - 4));
        assert_eq!(mem.read_u16(0x7FFA).unwrap(), 0x0100, "the Boolean is the high byte of its word, pushed first");
        assert_eq!(mem.read_u32(0x7FF6).unwrap(), 0xCAFE_F00D);
        assert_eq!(mem.read_u32(f.sp).unwrap(), 0x518, "the return address on top");
        mem.write_u32(0x7FFC, 0x1234_5678).unwrap();
        assert_eq!(read_result(&mem, &call, &f, 0).unwrap(), 0x1234_5678, "the result comes from the reserved space, not D0");
    }

    #[test]
    fn a_c_frame_pushes_right_to_left_and_the_result_is_d0() {
        let mut mem = GuestMem::new(0x10000);
        let call = stack_call(0x1 | (1 << 4) | (3 << 6) | (2 << 8)).unwrap(); // char f(long, short)
        let f = push_frame(&mut mem, 0x8001, &call, &[0x1111_2222, 0xBEEF], 0x518).unwrap();
        assert_eq!(mem.read_u32(f.sp + 4).unwrap(), 0x1111_2222, "the first parameter nearest the return address");
        assert_eq!(mem.read_u16(f.sp + 8).unwrap(), 0xBEEF);
        assert_eq!(f.sp_after, f.sp + 4, "the caller pops: the routine leaves only the return address gone");
        assert_eq!(read_result(&mem, &call, &f, 0xFFFF_FF41).unwrap(), 0x41);
        // NEGATIVE CONTROL: an argument count that does not match the procInfo is refused, nothing written
        assert!(push_frame(&mut mem, 0x9000, &call, &[1], 0x518).is_err());
        assert_eq!(mem.read_u32(0x9000 - 4).unwrap(), 0);
    }
}
