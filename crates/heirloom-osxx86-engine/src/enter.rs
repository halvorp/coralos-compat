//! Entering native guest code on its own stack, and leaving it without unwinding foreign frames.
//!
//! The guest is x86_64 code compiled for the System V calling convention, so it is entered with a plain `call` on a stack the
//! engine owns. Leaving is a context restore, not an unwind: the guest's `exit` lands in a Rust function that calls
//! [`osxx86_leave`], which reloads the callee-saved registers and the stack pointer [`osxx86_enter`] saved and returns from
//! it. Rust frames between the two (the `exit` binding) are abandoned, so they must own nothing that needs a destructor.
use core::arch::global_asm;

/// The registers [`osxx86_enter`] saves: the System V callee-saved set, the stack pointer, MXCSR and the x87 control word.
#[repr(C)]
#[derive(Default)]
pub struct SavedCtx {
    rsp: u64,
    rbx: u64,
    rbp: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    mxcsr: u32,
    fpucw: u16,
    _pad: u16,
}

global_asm!(
    ".globl osxx86_enter",
    "osxx86_enter:",
    "mov rax, [rsp + 8]",
    "mov [rax + 8], rbx",
    "mov [rax + 16], rbp",
    "mov [rax + 24], r12",
    "mov [rax + 32], r13",
    "mov [rax + 40], r14",
    "mov [rax + 48], r15",
    "stmxcsr [rax + 56]",
    "fnstcw [rax + 60]",
    "mov [rax], rsp",
    "mov rsp, r9",
    "sub rsp, 16",
    "mov [rsp], rax",
    "mov r10, rdi",
    "mov rdi, rsi",
    "mov rsi, rdx",
    "mov rdx, rcx",
    "mov rcx, r8",
    "xor ebp, ebp",
    "call r10",
    "mov rdi, [rsp]",
    "mov esi, eax",
    "jmp osxx86_leave",
    ".globl osxx86_leave",
    "osxx86_leave:",
    "cld",
    "mov eax, esi",
    "mov rbx, [rdi + 8]",
    "mov rbp, [rdi + 16]",
    "mov r12, [rdi + 24]",
    "mov r13, [rdi + 32]",
    "mov r14, [rdi + 40]",
    "mov r15, [rdi + 48]",
    "ldmxcsr [rdi + 56]",
    "fldcw [rdi + 60]",
    "mov rsp, [rdi]",
    "ret",
);

extern "C" {
    /// Calls `entry(argc, argv, envp, apple)` on the stack whose (16-byte aligned) top is `stack_top`, with `save` receiving the
    /// caller's registers. Returns the guest's return value, or the code a guest `exit` handed to [`osxx86_leave`].
    pub fn osxx86_enter(
        entry: usize,
        argc: i32,
        argv: *const *const u8,
        envp: *const *const u8,
        apple: *const *const u8,
        stack_top: usize,
        save: *mut SavedCtx,
    ) -> i32;
    /// Abandons the running guest: restores `save` (filled by the matching [`osxx86_enter`]) and makes that call return `code`.
    pub fn osxx86_leave(save: *const SavedCtx, code: i32) -> !;
}
