//! `objc_msgSend` and its variants: assembly trampolines that resolve the implementation in Rust and tail-jump to it with every
//! argument register (integer and vector) and the stack arguments exactly as the caller left them.
//!
//! The System V calling convention is the Darwin one on x86_64, so the guest calls these as ordinary functions. The trampolines
//! are not exported under the Apple names (no clash with a host libobjc); the loader binds `_objc_msgSend` and friends to their
//! addresses through [`crate::libobjc::symbol`].
use crate::runtime::{self, Class, Id, Imp, Sel};
use core::arch::global_asm;

/// Resolves a message to an instance or class.
#[no_mangle]
extern "C" fn osxobjc_lookup(receiver: Id, sel: Sel) -> Imp {
    // SAFETY: the trampoline calls this only with a non-nil receiver, whose first word is its class.
    unsafe { runtime::resolve(receiver, sel) }
}

/// Resolves a `[super ...]` message; `current` is the class the sending method belongs to.
#[no_mangle]
extern "C" fn osxobjc_lookup_super(receiver: Id, sel: Sel, current: Class) -> Imp {
    runtime::resolve_super(receiver, sel, current)
}

// The save area: rbp frame, rdi rsi rdx rcx r8 r9 rax pushed, then 128 bytes for xmm0-7 and 8 bytes of padding, which keeps rsp
// 16-byte aligned at the call. rdi/rsi/rdx are saved at [rbp-8], [rbp-16], [rbp-24].
global_asm!(
    ".text",
    ".globl osxobjc_msgsend",
    "osxobjc_msgsend:",
    "test rdi, rdi",
    "jz 2f",
    "push rbp",
    "mov rbp, rsp",
    "push rdi", "push rsi", "push rdx", "push rcx", "push r8", "push r9", "push rax",
    "sub rsp, 136",
    "movups [rsp], xmm0", "movups [rsp + 16], xmm1", "movups [rsp + 32], xmm2", "movups [rsp + 48], xmm3",
    "movups [rsp + 64], xmm4", "movups [rsp + 80], xmm5", "movups [rsp + 96], xmm6", "movups [rsp + 112], xmm7",
    "call {lookup}",
    "mov r11, rax",
    "movups xmm0, [rsp]", "movups xmm1, [rsp + 16]", "movups xmm2, [rsp + 32]", "movups xmm3, [rsp + 48]",
    "movups xmm4, [rsp + 64]", "movups xmm5, [rsp + 80]", "movups xmm6, [rsp + 96]", "movups xmm7, [rsp + 112]",
    "add rsp, 136",
    "pop rax", "pop r9", "pop r8", "pop rcx", "pop rdx", "pop rsi", "pop rdi",
    "pop rbp",
    "jmp r11",
    "2:",
    "xor eax, eax", "xor edx, edx", "xorps xmm0, xmm0", "xorps xmm1, xmm1",
    "ret",
    // stret: rdi = result buffer, rsi = receiver, rdx = selector
    ".globl osxobjc_msgsend_stret",
    "osxobjc_msgsend_stret:",
    "test rsi, rsi",
    "jz 3f",
    "push rbp",
    "mov rbp, rsp",
    "push rdi", "push rsi", "push rdx", "push rcx", "push r8", "push r9", "push rax",
    "sub rsp, 136",
    "movups [rsp], xmm0", "movups [rsp + 16], xmm1", "movups [rsp + 32], xmm2", "movups [rsp + 48], xmm3",
    "movups [rsp + 64], xmm4", "movups [rsp + 80], xmm5", "movups [rsp + 96], xmm6", "movups [rsp + 112], xmm7",
    "mov rdi, rsi",
    "mov rsi, rdx",
    "call {lookup}",
    "mov r11, rax",
    "movups xmm0, [rsp]", "movups xmm1, [rsp + 16]", "movups xmm2, [rsp + 32]", "movups xmm3, [rsp + 48]",
    "movups xmm4, [rsp + 64]", "movups xmm5, [rsp + 80]", "movups xmm6, [rsp + 96]", "movups xmm7, [rsp + 112]",
    "add rsp, 136",
    "pop rax", "pop r9", "pop r8", "pop rcx", "pop rdx", "pop rsi", "pop rdi",
    "pop rbp",
    "jmp r11",
    "3:",
    "mov rax, rdi",
    "ret",
    // super2: rdi = &objc_super { receiver, class }, rsi = selector
    ".globl osxobjc_msgsend_super2",
    "osxobjc_msgsend_super2:",
    "push rbp",
    "mov rbp, rsp",
    "push rdi", "push rsi", "push rdx", "push rcx", "push r8", "push r9", "push rax",
    "sub rsp, 136",
    "movups [rsp], xmm0", "movups [rsp + 16], xmm1", "movups [rsp + 32], xmm2", "movups [rsp + 48], xmm3",
    "movups [rsp + 64], xmm4", "movups [rsp + 80], xmm5", "movups [rsp + 96], xmm6", "movups [rsp + 112], xmm7",
    "mov r10, [rdi]",
    "mov rdx, [rdi + 8]",
    "mov [rbp - 8], r10",
    "mov rdi, r10",
    "call {lookup_super}",
    "mov r11, rax",
    "movups xmm0, [rsp]", "movups xmm1, [rsp + 16]", "movups xmm2, [rsp + 32]", "movups xmm3, [rsp + 48]",
    "movups xmm4, [rsp + 64]", "movups xmm5, [rsp + 80]", "movups xmm6, [rsp + 96]", "movups xmm7, [rsp + 112]",
    "add rsp, 136",
    "pop rax", "pop r9", "pop r8", "pop rcx", "pop rdx", "pop rsi", "pop rdi",
    "pop rbp",
    "jmp r11",
    // super2 stret: rdi = result buffer, rsi = &objc_super, rdx = selector
    ".globl osxobjc_msgsend_super2_stret",
    "osxobjc_msgsend_super2_stret:",
    "push rbp",
    "mov rbp, rsp",
    "push rdi", "push rsi", "push rdx", "push rcx", "push r8", "push r9", "push rax",
    "sub rsp, 136",
    "movups [rsp], xmm0", "movups [rsp + 16], xmm1", "movups [rsp + 32], xmm2", "movups [rsp + 48], xmm3",
    "movups [rsp + 64], xmm4", "movups [rsp + 80], xmm5", "movups [rsp + 96], xmm6", "movups [rsp + 112], xmm7",
    "mov r10, [rsi]",
    "mov rcx, [rsi + 8]",
    "mov [rbp - 16], r10",
    "mov rdi, r10",
    "mov rsi, rdx",
    "mov rdx, rcx",
    "call {lookup_super}",
    "mov r11, rax",
    "movups xmm0, [rsp]", "movups xmm1, [rsp + 16]", "movups xmm2, [rsp + 32]", "movups xmm3, [rsp + 48]",
    "movups xmm4, [rsp + 64]", "movups xmm5, [rsp + 80]", "movups xmm6, [rsp + 96]", "movups xmm7, [rsp + 112]",
    "add rsp, 136",
    "pop rax", "pop r9", "pop r8", "pop rcx", "pop rdx", "pop rsi", "pop rdi",
    "pop rbp",
    "jmp r11",
    lookup = sym osxobjc_lookup,
    lookup_super = sym osxobjc_lookup_super,
);

extern "C" {
    pub(crate) fn osxobjc_msgsend();
    pub(crate) fn osxobjc_msgsend_stret();
    pub(crate) fn osxobjc_msgsend_super2();
    pub(crate) fn osxobjc_msgsend_super2_stret();
}
