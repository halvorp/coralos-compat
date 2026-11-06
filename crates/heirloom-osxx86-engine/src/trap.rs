//! The raw-trap admission scan: finds the instructions a Mach-O app must not execute itself, the `syscall` and `sysenter`
//! instructions and the legacy Darwin software-interrupt gates (`int 0x80` BSD, `int 0x81` Mach, `int 0x82` machine-dependent).
//!
//! The scan is a linear instruction sweep over code bytes, not a byte search. Measured on the host's own x86_64 code, the byte
//! pairs `0F 05`, `CD 80` and `0F 34` occur by chance inside immediates and displacements about once per 100 KiB (8 hits in the
//! 1 MiB text of /usr/bin/bash, 42 in the 3.8 MiB text of /usr/bin/git) while the sweep finds none in either, so a byte search would
//! refuse nearly every real application. The sweep resynchronises one byte after a decode failure.
//!
//! What it does not do: a hostile image can hide a trap by jumping into the middle of an instruction. The scan is admission
//! hygiene for honest apps; the boundary against a hostile guest is the engine's capability confinement and the kernel's refusal
//! of Darwin system call numbers (docs/heirloom/25-OSXX86-ENGINE.md section 2).
use crate::TrapKind;
use alloc::vec::Vec;
use yaxpeax_arch::LengthedInstruction;
use yaxpeax_x86::long_mode::{InstDecoder, Opcode, Operand};

/// Every trap instruction in `code`, with its offset, in order.
pub fn scan_all(code: &[u8]) -> Vec<(usize, TrapKind)> {
    let decoder = InstDecoder::default();
    let mut found = Vec::new();
    let mut at = 0usize;
    while at < code.len() {
        match decoder.decode_slice(&code[at..]) {
            Ok(inst) => {
                if let Some(kind) = trap_kind(&inst) {
                    found.push((at, kind));
                }
                at += (inst.len().to_const() as usize).max(1);
            }
            Err(_) => at += 1,
        }
    }
    found
}

/// The first trap instruction in `code`.
pub fn scan_code(code: &[u8]) -> Option<(usize, TrapKind)> {
    scan_all(code).into_iter().next()
}

fn trap_kind(inst: &yaxpeax_x86::long_mode::Instruction) -> Option<TrapKind> {
    match inst.opcode() {
        Opcode::SYSCALL => Some(TrapKind::Syscall),
        Opcode::SYSENTER => Some(TrapKind::Sysenter),
        Opcode::INT => match inst.operand(0) {
            Operand::ImmediateU8 { imm } if (0x80..=0x82).contains(&imm) => Some(TrapKind::Int80),
            _ => None,
        },
        _ => None,
    }
}
