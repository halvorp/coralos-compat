//! Tier 2's x86-64 encoder: only the instruction forms `jit_emit` / `jit_inline` / `jit_regs` use, base x86-64 (nothing
//! past x86-64-v2: no VEX, no BMI; MOVBE only behind the runtime CPUID check `cpu_has_movbe`). Memory forms take an `M`, and
//! the register cache's forms an `O` (a register or memory: a guest register the block holds in a host register, or not).
//! Encodings follow the Intel SDM vol. 2 opcode tables; the unit tests at the bottom pin each form against bytes an
//! assembler produces.
use alloc::vec::Vec;

pub(crate) const RAX: u8 = 0;
pub(crate) const RCX: u8 = 1;
pub(crate) const RDX: u8 = 2;
pub(crate) const RBX: u8 = 3;
pub(crate) const RSP: u8 = 4;
pub(crate) const RBP: u8 = 5;
pub(crate) const RSI: u8 = 6;
pub(crate) const RDI: u8 = 7;
pub(crate) const R8: u8 = 8;
pub(crate) const R9: u8 = 9;
pub(crate) const R10: u8 = 10;
pub(crate) const R11: u8 = 11;
pub(crate) const R12: u8 = 12;
pub(crate) const R13: u8 = 13;
pub(crate) const R14: u8 = 14;
pub(crate) const R15: u8 = 15;

/// Condition codes (the low nibble of Jcc / SETcc / CMOVcc).
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Cc { B = 2, Ae = 3, E = 4, Ne = 5, A = 7, S = 8, Ns = 9, P = 10, L = 12, G = 15 }

/// ALU group-1 operations: the /digit of 0x81 and (digit << 3) | 1 is the `r/m, reg` opcode.
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Alu { Add = 0, Or = 1, And = 4, Sub = 5, Xor = 6, Cmp = 7 }

/// Shift / rotate group-2 operations (the /digit of 0xC1 / 0xD3).
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Sh { Rol = 0, Shl = 4, Shr = 5, Sar = 7 }

/// A memory operand `[base + index * scale + disp]` (scale 1, 2, 4 or 8; no index = `[base + disp]`).
#[derive(Clone, Copy)]
pub(crate) struct M { base: u8, index: Option<(u8, u8)>, disp: i32 }

pub(crate) fn m(base: u8, disp: i32) -> M { M { base, index: None, disp } }
pub(crate) fn mx(base: u8, index: u8, scale: u8) -> M { M { base, index: Some((index, scale)), disp: 0 } }
pub(crate) fn mxd(base: u8, index: u8, scale: u8, disp: i32) -> M { M { base, index: Some((index, scale)), disp } }

/// An r/m operand: a register or memory (the register cache hands out either for a guest register: jit_emit `Loc`).
#[derive(Clone, Copy)]
pub(crate) enum O { R(u8), M(M) }

/// A forward jump whose rel32 is patched when its target is known.
#[derive(Clone, Copy)]
pub(crate) struct Fix(usize);

#[derive(Default)]
pub(crate) struct Asm { pub(crate) b: Vec<u8> }

impl Asm {
    pub(crate) fn pos(&self) -> usize { self.b.len() }
    fn u8(&mut self, v: u8) { self.b.push(v) }
    fn u32(&mut self, v: u32) { self.b.extend_from_slice(&v.to_le_bytes()) }

    /// REX with W/R/X/B; `byte_reg` forces one for spl/bpl/sil/dil as 8-bit operands.
    fn rex(&mut self, w: bool, r: u8, x: u8, b: u8, byte_reg: bool) {
        let v = 0x40 | (w as u8) << 3 | (r >> 3 & 1) << 2 | (x >> 3 & 1) << 1 | (b >> 3 & 1);
        if v != 0x40 || byte_reg { self.u8(v) }
    }
    fn modrm_mem(&mut self, reg: u8, mm: M) {
        let base = mm.base & 7;
        // mod 00 has no displacement except with base rbp/r13 (that encoding means rip/disp32)
        let md = if mm.disp == 0 && base != 5 { 0 } else if i8::try_from(mm.disp).is_ok() { 1 } else { 2 };
        match mm.index {
            None => {
                self.u8(md << 6 | (reg & 7) << 3 | base);
                if base == 4 { self.u8(0x24) }
            }
            Some((ix, sc)) => {
                debug_assert!(ix != RSP);
                let ss = match sc { 1 => 0, 2 => 1, 4 => 2, _ => 3 };
                self.u8(md << 6 | (reg & 7) << 3 | 4);
                self.u8(ss << 6 | (ix & 7) << 3 | base);
            }
        }
        match md { 1 => self.u8(mm.disp as i8 as u8), 2 => self.u32(mm.disp as u32), _ => {} }
    }
    fn op_m(&mut self, pre: Option<u8>, w: bool, opc: &[u8], reg: u8, mm: M, byte_reg: bool) {
        if let Some(p) = pre { self.u8(p) }
        self.rex(w, reg, mm.index.map_or(0, |i| i.0), mm.base, byte_reg);
        self.b.extend_from_slice(opc);
        self.modrm_mem(reg, mm);
    }
    fn op_r(&mut self, w: bool, opc: &[u8], reg: u8, rm: u8, byte_reg: bool) {
        self.rex(w, reg, 0, rm, byte_reg);
        self.b.extend_from_slice(opc);
        self.u8(0xC0 | (reg & 7) << 3 | (rm & 7));
    }

    /// `opc` with an r/m operand that is a register (mod 11) or memory.
    fn op_o(&mut self, w: bool, opc: &[u8], reg: u8, o: O) {
        match o {
            O::M(mm) => self.op_m(None, w, opc, reg, mm, false),
            O::R(rm) => self.op_r(w, opc, reg, rm, false),
        }
    }

    // ---- r/m forms over a register-or-memory operand (the register cache's guest registers) ----
    /// mov r32, r/m32.
    pub(crate) fn ld32o(&mut self, r: u8, o: O) { if let O::R(s) = o { if s == r { return } } self.op_o(false, &[0x8B], r, o) }
    /// mov r/m32, r32.
    pub(crate) fn st32o(&mut self, o: O, r: u8) { if let O::R(d) = o { if d == r { return } } self.op_o(false, &[0x89], r, o) }
    /// mov r/m32, imm32.
    pub(crate) fn st32io(&mut self, o: O, v: u32) { self.op_o(false, &[0xC7], 0, o); self.u32(v) }
    /// op r32, r/m32.
    pub(crate) fn alu_ro(&mut self, op: Alu, d: u8, o: O) { self.op_o(false, &[(op as u8) << 3 | 3], d, o) }
    /// op r/m32, r32.
    pub(crate) fn alu_or(&mut self, op: Alu, o: O, s: u8) { self.op_o(false, &[(op as u8) << 3 | 1], s, o) }
    /// op r/m32, imm32.
    pub(crate) fn alu_oi(&mut self, op: Alu, o: O, v: u32) { self.op_o(false, &[0x81], op as u8, o); self.u32(v) }
    /// imul r32, r/m32 / imul r32, r/m32, imm32.
    pub(crate) fn imul_ro(&mut self, d: u8, o: O) { self.op_o(false, &[0x0F, 0xAF], d, o) }
    pub(crate) fn imul_roi(&mut self, d: u8, o: O, v: u32) { self.op_o(false, &[0x69], d, o); self.u32(v) }
    /// movsx r32, r/m16 (a register source is its low 16 bits) / movsx r32, r/m8 (a register source is its low byte:
    /// spl..dil need a REX, which op_r gives a register >= 4 here).
    pub(crate) fn movsx16o(&mut self, d: u8, o: O) { self.op_o(false, &[0x0F, 0xBF], d, o) }
    pub(crate) fn movsx8o(&mut self, d: u8, o: O) {
        match o { O::R(s) => self.op_r(false, &[0x0F, 0xBE], d, s, s >= 4), O::M(mm) => self.op_m(None, false, &[0x0F, 0xBE], d, mm, false) }
    }
    /// bsr r32, r/m32 (ZF = the source was zero, the destination then undefined).
    pub(crate) fn bsr_ro(&mut self, d: u8, o: O) { self.op_o(false, &[0x0F, 0xBD], d, o) }
    /// The one-operand group-3 forms on r/m32: 4 mul, 5 imul (edx:eax = eax * r/m), 6 div, 7 idiv (edx:eax / r/m).
    pub(crate) fn grp3_o(&mut self, ext: u8, o: O) { self.op_o(false, &[0xF7], ext, o) }
    /// adc r32, r/m32.
    pub(crate) fn adc_ro(&mut self, d: u8, o: O) { self.op_o(false, &[0x13], d, o) }
    /// bt r/m32, imm8.
    pub(crate) fn bt_oi(&mut self, o: O, bit: u8) { self.op_o(false, &[0x0F, 0xBA], 4, o); self.u8(bit) }

    // ---- moves ----
    pub(crate) fn ld32(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x8B], r, mm, false) }
    pub(crate) fn ld64(&mut self, r: u8, mm: M) { self.op_m(None, true, &[0x8B], r, mm, false) }
    pub(crate) fn st32(&mut self, mm: M, r: u8) { self.op_m(None, false, &[0x89], r, mm, false) }
    pub(crate) fn st64(&mut self, mm: M, r: u8) { self.op_m(None, true, &[0x89], r, mm, false) }
    pub(crate) fn st16(&mut self, mm: M, r: u8) { self.op_m(Some(0x66), false, &[0x89], r, mm, false) }
    pub(crate) fn st8(&mut self, mm: M, r: u8) { self.op_m(None, false, &[0x88], r, mm, r >= 4) }
    pub(crate) fn movzx8(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xB6], r, mm, false) }
    pub(crate) fn movzx16(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xB7], r, mm, false) }
    /// movzx r32, r8.
    /// movzx / movsx r32, r16.
    pub(crate) fn movzx16_rr(&mut self, d: u8, s: u8) { self.op_r(false, &[0x0F, 0xB7], d, s, false) }
    pub(crate) fn movsx16_rr(&mut self, d: u8, s: u8) { self.op_r(false, &[0x0F, 0xBF], d, s, false) }
    pub(crate) fn movzx8_rr(&mut self, d: u8, s: u8) { self.op_r(false, &[0x0F, 0xB6], d, s, s >= 4) }
    pub(crate) fn mov_rr(&mut self, d: u8, s: u8) { self.op_r(false, &[0x89], s, d, false) }
    pub(crate) fn mov_rr64(&mut self, d: u8, s: u8) { self.op_r(true, &[0x89], s, d, false) }
    pub(crate) fn mov_ri(&mut self, d: u8, v: u32) {
        self.rex(false, 0, 0, d, false);
        self.u8(0xB8 + (d & 7));
        self.u32(v);
    }
    pub(crate) fn mov_ri64(&mut self, d: u8, v: u64) {
        self.rex(true, 0, 0, d, false);
        self.u8(0xB8 + (d & 7));
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    /// mov dword [m], imm32.
    pub(crate) fn st32i(&mut self, mm: M, v: u32) {
        self.op_m(None, false, &[0xC7], 0, mm, false);
        self.u32(v);
    }
    pub(crate) fn lea64(&mut self, r: u8, mm: M) { self.op_m(None, true, &[0x8D], r, mm, false) }

    // ---- ALU ----
    pub(crate) fn alu_rr(&mut self, op: Alu, d: u8, s: u8) { self.op_r(false, &[(op as u8) << 3 | 1], s, d, false) }
    pub(crate) fn alu_rr64(&mut self, op: Alu, d: u8, s: u8) { self.op_r(true, &[(op as u8) << 3 | 1], s, d, false) }
    /// op r32, [m].
    pub(crate) fn alu_rm(&mut self, op: Alu, d: u8, mm: M) { self.op_m(None, false, &[(op as u8) << 3 | 3], d, mm, false) }
    /// op r64, [m].
    pub(crate) fn alu_rm64(&mut self, op: Alu, d: u8, mm: M) { self.op_m(None, true, &[(op as u8) << 3 | 3], d, mm, false) }
    pub(crate) fn alu_ri(&mut self, op: Alu, d: u8, v: u32) {
        self.op_r(false, &[0x81], op as u8, d, false);
        self.u32(v);
    }
    pub(crate) fn alu_ri64(&mut self, op: Alu, d: u8, v: i32) {
        self.op_r(true, &[0x81], op as u8, d, false);
        self.u32(v as u32);
    }
    /// op qword [m], imm32 (sign-extended).
    pub(crate) fn alu_mi64(&mut self, op: Alu, mm: M, v: u32) {
        self.op_m(None, true, &[0x81], op as u8, mm, false);
        self.u32(v);
    }
    /// and r8, r8 (8-bit).
    pub(crate) fn and8_rr(&mut self, d: u8, s: u8) { self.op_r(false, &[0x20], s, d, d >= 4 || s >= 4) }
    pub(crate) fn sh_ri(&mut self, op: Sh, d: u8, n: u8) {
        self.op_r(false, &[0xC1], op as u8, d, false);
        self.u8(n);
    }
    /// shift r32 by cl.
    pub(crate) fn sh_cl(&mut self, op: Sh, d: u8) { self.op_r(false, &[0xD3], op as u8, d, false) }
    pub(crate) fn bswap32(&mut self, r: u8) {
        self.rex(false, 0, 0, r, false);
        self.b.extend_from_slice(&[0x0F, 0xC8 + (r & 7)]);
    }
    pub(crate) fn bswap64(&mut self, r: u8) {
        self.rex(true, 0, 0, r, false);
        self.b.extend_from_slice(&[0x0F, 0xC8 + (r & 7)]);
    }
    pub(crate) fn neg(&mut self, r: u8) { self.op_r(false, &[0xF7], 3, r, false) }
    pub(crate) fn not(&mut self, r: u8) { self.op_r(false, &[0xF7], 2, r, false) }
    pub(crate) fn test_rr(&mut self, a: u8, b: u8) { self.op_r(false, &[0x85], b, a, false) }
    pub(crate) fn test_rr64(&mut self, a: u8, b: u8) { self.op_r(true, &[0x85], b, a, false) }
    pub(crate) fn test_ri(&mut self, r: u8, v: u32) {
        self.op_r(false, &[0xF7], 0, r, false);
        self.u32(v);
    }
    pub(crate) fn setcc(&mut self, cc: Cc, r: u8) { self.op_r(false, &[0x0F, 0x90 + cc as u8], 0, r, r >= 4) }
    pub(crate) fn cmov(&mut self, cc: Cc, d: u8, s: u8) { self.op_r(false, &[0x0F, 0x40 + cc as u8], d, s, false) }
    /// bt r64, r64 (bit index mod 64).
    pub(crate) fn bt_rr64(&mut self, r: u8, bit: u8) { self.op_r(true, &[0x0F, 0xA3], bit, r, false) }

    pub(crate) fn sh_ri64(&mut self, op: Sh, d: u8, n: u8) {
        self.op_r(true, &[0xC1], op as u8, d, false);
        self.u8(n);
    }

    /// adc r32, imm32 (CF in).
    pub(crate) fn adc_ri(&mut self, d: u8, v: u32) {
        self.op_r(false, &[0x81], 2, d, false);
        self.u32(v);
    }
    pub(crate) fn stc(&mut self) { self.u8(0xF9) }
    /// cdq (sign-extend eax into edx).
    pub(crate) fn cdq(&mut self) { self.u8(0x99) }
    /// MOVBE (CPUID.1:ECX[22]; callers check `cpu_has_movbe`): r <- byte-swapped [m] / [m] <- byte-swapped r, 16/32/64 bits.
    pub(crate) fn movbe_ld(&mut self, bits: u8, r: u8, mm: M) {
        self.op_m(if bits == 16 { Some(0x66) } else { None }, bits == 64, &[0x0F, 0x38, 0xF0], r, mm, false)
    }
    pub(crate) fn movbe_st(&mut self, bits: u8, mm: M, r: u8) {
        self.op_m(if bits == 16 { Some(0x66) } else { None }, bits == 64, &[0x0F, 0x38, 0xF1], r, mm, false)
    }

    // ---- SSE2 (x86-64 baseline): scalar double / single, xmm0..xmm7 only ----
    /// `pre` op xmm, [m] (pre = F2: movsd 10 / addsd 58 / mulsd 59 / subsd 5C; 66: ucomisd 2E).
    pub(crate) fn sse_m(&mut self, pre: u8, opc: u8, x: u8, mm: M) { self.op_m(Some(pre), false, &[0x0F, opc], x, mm, false) }
    /// `pre` op xmm, xmm (F2 5A cvtsd2ss, F3 5A cvtss2sd).
    pub(crate) fn sse_r(&mut self, pre: u8, opc: u8, x: u8, y: u8) {
        self.u8(pre);
        self.op_r(false, &[0x0F, opc], x, y, false);
    }
    /// movq r64, xmm / movd xmm, r32.
    pub(crate) fn movq_rx(&mut self, r: u8, x: u8) { self.u8(0x66); self.op_r(true, &[0x0F, 0x7E], x, r, false) }
    pub(crate) fn movd_xr(&mut self, x: u8, r: u8) { self.u8(0x66); self.op_r(false, &[0x0F, 0x6E], x, r, false) }
    /// movq xmm, r64.
    pub(crate) fn movq_xr(&mut self, x: u8, r: u8) { self.u8(0x66); self.op_r(true, &[0x0F, 0x6E], x, r, false) }
    /// movapd xmm, xmm.
    pub(crate) fn movapd(&mut self, x: u8, y: u8) { self.sse_r(0x66, 0x28, x, y) }
    /// cvttsd2si r32, xmm (truncating; 0x80000000 for NaN or out of range).
    pub(crate) fn cvttsd2si(&mut self, r: u8, x: u8) { self.sse_r(0xF2, 0x2C, r, x) }
    /// cvtsi2sd xmm, r32.
    pub(crate) fn cvtsi2sd(&mut self, x: u8, r: u8) { self.sse_r(0xF2, 0x2A, x, r) }

    // ---- control flow ----
    pub(crate) fn jcc(&mut self, cc: Cc) -> Fix {
        self.b.extend_from_slice(&[0x0F, 0x80 + cc as u8]);
        self.u32(0);
        Fix(self.pos())
    }
    pub(crate) fn jmp(&mut self) -> Fix {
        self.u8(0xE9);
        self.u32(0);
        Fix(self.pos())
    }
    /// Point a forward jump at `target` (an offset in this buffer).
    pub(crate) fn patch(&mut self, f: Fix, target: usize) {
        let rel = (target as i64 - f.0 as i64) as i32;
        self.b[f.0 - 4..f.0].copy_from_slice(&rel.to_le_bytes());
    }
    pub(crate) fn here(&mut self, f: Fix) { let p = self.pos(); self.patch(f, p) }
    /// jmp qword [m].
    pub(crate) fn jmp_m(&mut self, mm: M) { self.op_m(None, false, &[0xFF], 4, mm, false) }
    pub(crate) fn jmp_r(&mut self, r: u8) { self.op_r(false, &[0xFF], 4, r, false) }
    pub(crate) fn call_r(&mut self, r: u8) { self.op_r(false, &[0xFF], 2, r, false) }
    /// call qword [m].
    pub(crate) fn call_m(&mut self, mm: M) { self.op_m(None, false, &[0xFF], 2, mm, false) }
    pub(crate) fn push(&mut self, r: u8) {
        self.rex(false, 0, 0, r, false);
        self.u8(0x50 + (r & 7));
    }
    pub(crate) fn pop(&mut self, r: u8) {
        self.rex(false, 0, 0, r, false);
        self.u8(0x58 + (r & 7));
    }
    pub(crate) fn ret(&mut self) { self.u8(0xC3) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enc(f: impl FnOnce(&mut Asm)) -> Vec<u8> { let mut a = Asm::default(); f(&mut a); a.b }

    #[test]
    fn the_movbe_probe_agrees_with_std_feature_detection() {
        assert_eq!(cpu_has_movbe(), std::is_x86_feature_detected!("movbe"));
    }

    /// Each expected byte string is what GNU as (Intel syntax) assembles for the comment beside it, or (where the comment
    /// says so) another valid encoding of it that llvm-mc --disassemble reads back as the comment.
    #[test]
    fn encodings_match_an_assembler() {
        assert_eq!(enc(|a| a.ld32(RDX, m(RBX, 0x10))), [0x8B, 0x53, 0x10]); // mov edx,[rbx+0x10]
        assert_eq!(enc(|a| a.ld32(RAX, m(R13, 0))), [0x41, 0x8B, 0x45, 0x00]); // mov eax,[r13+0]
        assert_eq!(enc(|a| a.ld32(RAX, m(R12, 0))), [0x41, 0x8B, 0x04, 0x24]); // mov eax,[r12]
        assert_eq!(enc(|a| a.ld32(RDX, mx(R12, RAX, 1))), [0x41, 0x8B, 0x14, 0x04]); // mov edx,[r12+rax]
        assert_eq!(enc(|a| a.ld64(RDX, mx(RSI, RDX, 8))), [0x48, 0x8B, 0x14, 0xD6]); // mov rdx,[rsi+rdx*8]
        assert_eq!(enc(|a| a.st32(m(RBX, 0x200), RDX)), [0x89, 0x93, 0x00, 0x02, 0x00, 0x00]); // mov [rbx+0x200],edx
        assert_eq!(enc(|a| a.st16(mx(R12, RAX, 1), RDX)), [0x66, 0x41, 0x89, 0x14, 0x04]); // mov [r12+rax],dx
        assert_eq!(enc(|a| a.st8(mx(R12, RAX, 1), RDX)), [0x41, 0x88, 0x14, 0x04]); // mov [r12+rax],dl
        assert_eq!(enc(|a| a.movzx16(RDX, mx(R12, RAX, 1))), [0x41, 0x0F, 0xB7, 0x14, 0x04]); // movzx edx,word [r12+rax]
        assert_eq!(enc(|a| a.mov_rr64(R13, RDI)), [0x49, 0x89, 0xFD]); // mov r13,rdi
        assert_eq!(enc(|a| a.mov_rr(RAX, R8)), [0x44, 0x89, 0xC0]); // mov eax,r8d
        assert_eq!(enc(|a| a.mov_ri(R8, 5)), [0x41, 0xB8, 5, 0, 0, 0]); // mov r8d,5
        assert_eq!(enc(|a| a.mov_ri64(RAX, 0x1122334455667788)), [0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]);
        assert_eq!(enc(|a| a.st32i(m(RBX, 4), 7)), [0xC7, 0x43, 0x04, 7, 0, 0, 0]); // mov dword [rbx+4],7
        assert_eq!(enc(|a| a.lea64(RCX, m(RAX, 4))), [0x48, 0x8D, 0x48, 0x04]); // lea rcx,[rax+4]
        assert_eq!(enc(|a| a.lea64(R15, m(R15, 3))), [0x4D, 0x8D, 0x7F, 0x03]); // lea r15,[r15+3]
        assert_eq!(enc(|a| a.alu_rm(Alu::Add, RDX, m(RBX, 8))), [0x03, 0x53, 0x08]); // add edx,[rbx+8]
        assert_eq!(enc(|a| a.alu_rm64(Alu::Cmp, RCX, m(R13, 16))), [0x49, 0x3B, 0x4D, 0x10]); // cmp rcx,[r13+16]
        assert_eq!(enc(|a| a.alu_ri(Alu::And, RAX, 0xFFF)), [0x81, 0xE0, 0xFF, 0x0F, 0, 0]); // and eax,0xfff
        assert_eq!(enc(|a| a.alu_rr64(Alu::Cmp, RAX, R14)), [0x4C, 0x39, 0xF0]); // cmp rax,r14
        assert_eq!(enc(|a| a.alu_oi(Alu::Cmp, O::M(m(RBX, 12)), 9)), [0x81, 0x7B, 0x0C, 9, 0, 0, 0]); // cmp dword [rbx+12],9
        assert_eq!(enc(|a| a.and8_rr(RAX, RCX)), [0x20, 0xC8]); // and al,cl
        assert_eq!(enc(|a| a.sh_ri(Sh::Rol, RDX, 3)), [0xC1, 0xC2, 3]); // rol edx,3
        assert_eq!(enc(|a| a.sh_cl(Sh::Shl, RDX)), [0xD3, 0xE2]); // shl edx,cl
        assert_eq!(enc(|a| a.bswap32(RDX)), [0x0F, 0xCA]);
        assert_eq!(enc(|a| a.bswap64(RDX)), [0x48, 0x0F, 0xCA]);
        assert_eq!(enc(|a| a.imul_roi(RDX, O::M(m(RBX, 8)), 3)), [0x69, 0x53, 0x08, 3, 0, 0, 0]); // imul edx,[rbx+8],3
        assert_eq!(enc(|a| a.imul_ro(RDX, O::M(m(RBX, 8)))), [0x0F, 0xAF, 0x53, 0x08]);
        assert_eq!(enc(|a| a.setcc(Cc::L, RAX)), [0x0F, 0x9C, 0xC0]); // setl al
        assert_eq!(enc(|a| a.setcc(Cc::B, R8)), [0x41, 0x0F, 0x92, 0xC0]); // setb r8b
        assert_eq!(enc(|a| a.movzx8_rr(RAX, RAX)), [0x0F, 0xB6, 0xC0]); // movzx eax,al
        assert_eq!(enc(|a| a.cmov(Cc::Ne, RDX, RAX)), [0x0F, 0x45, 0xD0]); // cmovne edx,eax
        assert_eq!(enc(|a| a.bt_oi(O::M(m(RBX, 0x100)), 5)), [0x0F, 0xBA, 0xA3, 0x00, 0x01, 0, 0, 5]); // bt dword [rbx+0x100],5
        assert_eq!(enc(|a| a.bt_rr64(RDX, RCX)), [0x48, 0x0F, 0xA3, 0xCA]); // bt rdx,rcx
        assert_eq!(enc(|a| a.jmp_m(m(R13, 8))), [0x41, 0xFF, 0x65, 0x08]); // jmp [r13+8]
        assert_eq!(enc(|a| a.jmp_m(m(RAX, 0))), [0xFF, 0x20]); // jmp [rax]
        assert_eq!(enc(|a| a.call_r(RAX)), [0xFF, 0xD0]);
        assert_eq!(enc(|a| a.call_m(m(R13, 0x58))), [0x41, 0xFF, 0x55, 0x58]); // call [r13+0x58]
        assert_eq!(enc(|a| a.alu_mi64(Alu::Cmp, m(RAX, 0x100), 7)), [0x48, 0x81, 0xB8, 0x00, 0x01, 0, 0, 7, 0, 0, 0]); // cmp qword [rax+0x100],7
        assert_eq!(enc(|a| a.lea64(R15, mxd(R15, RBP, 1, 1))), [0x4D, 0x8D, 0x7C, 0x2F, 0x01]); // lea r15,[r15+rbp+1]
        assert_eq!(enc(|a| a.lea64(RCX, mx(R15, RBP, 1))), [0x49, 0x8D, 0x0C, 0x2F]); // lea rcx,[r15+rbp]
        assert_eq!(enc(|a| a.push(R15)), [0x41, 0x57]);
        assert_eq!(enc(|a| a.pop(RBX)), [0x5B]);
        assert_eq!(enc(|a| a.test_ri(R8, 0x20)), [0x41, 0xF7, 0xC0, 0x20, 0, 0, 0]); // test r8d,0x20
        assert_eq!(enc(|a| a.movsx16o(RDX, O::M(m(RBX, 8)))), [0x0F, 0xBF, 0x53, 0x08]);
        assert_eq!(enc(|a| a.sse_m(0xF2, 0x10, 0, m(RBX, 0x80))), [0xF2, 0x0F, 0x10, 0x83, 0x80, 0, 0, 0]); // movsd xmm0,[rbx+0x80]
        assert_eq!(enc(|a| a.sse_m(0x66, 0x2E, 0, m(RBX, 8))), [0x66, 0x0F, 0x2E, 0x43, 0x08]); // ucomisd xmm0,[rbx+8]
        assert_eq!(enc(|a| a.sse_m(0xF2, 0x58, 0, m(RBX, 8))), [0xF2, 0x0F, 0x58, 0x43, 0x08]); // addsd xmm0,[rbx+8]
        assert_eq!(enc(|a| a.sse_r(0xF2, 0x5A, 0, 0)), [0xF2, 0x0F, 0x5A, 0xC0]); // cvtsd2ss xmm0,xmm0
        assert_eq!(enc(|a| a.sse_r(0xF3, 0x5A, 0, 0)), [0xF3, 0x0F, 0x5A, 0xC0]); // cvtss2sd xmm0,xmm0
        assert_eq!(enc(|a| a.movq_rx(RDX, 0)), [0x66, 0x48, 0x0F, 0x7E, 0xC2]); // movq rdx,xmm0
        assert_eq!(enc(|a| a.movd_xr(0, RDX)), [0x66, 0x0F, 0x6E, 0xC2]); // movd xmm0,edx
        assert_eq!(enc(|a| a.sh_ri64(Sh::Shr, RAX, 53)), [0x48, 0xC1, 0xE8, 53]); // shr rax,53
        assert_eq!(enc(|a| a.test_rr64(RCX, RCX)), [0x48, 0x85, 0xC9]); // test rcx,rcx
        assert_eq!(enc(|a| a.movq_xr(1, RDX)), [0x66, 0x48, 0x0F, 0x6E, 0xCA]); // movq xmm1,rdx
        assert_eq!(enc(|a| a.movapd(5, 0)), [0x66, 0x0F, 0x28, 0xE8]); // movapd xmm5,xmm0
        assert_eq!(enc(|a| a.cvttsd2si(RAX, 0)), [0xF2, 0x0F, 0x2C, 0xC0]); // cvttsd2si eax,xmm0
        assert_eq!(enc(|a| a.cvtsi2sd(1, RAX)), [0xF2, 0x0F, 0x2A, 0xC8]); // cvtsi2sd xmm1,eax
        assert_eq!(enc(|a| a.adc_ro(RDX, O::M(m(RBX, 8)))), [0x13, 0x53, 0x08]); // adc edx,[rbx+8]
        assert_eq!(enc(|a| a.adc_ri(RDX, 0)), [0x81, 0xD2, 0, 0, 0, 0]); // adc edx,0
        assert_eq!(enc(|a| a.bsr_ro(RDX, O::M(m(RBX, 8)))), [0x0F, 0xBD, 0x53, 0x08]); // bsr edx,[rbx+8]
        assert_eq!(enc(|a| a.grp3_o(4, O::M(m(RBX, 8)))), [0xF7, 0x63, 0x08]); // mul dword [rbx+8]
        assert_eq!(enc(|a| a.grp3_o(7, O::M(m(RBX, 8)))), [0xF7, 0x7B, 0x08]); // idiv dword [rbx+8]
        assert_eq!(enc(|a| a.movbe_ld(32, RDX, mx(R12, RAX, 1))), [0x41, 0x0F, 0x38, 0xF0, 0x14, 0x04]); // movbe edx,[r12+rax]
        assert_eq!(enc(|a| a.movbe_ld(16, RDX, mx(R12, RAX, 1))), [0x66, 0x41, 0x0F, 0x38, 0xF0, 0x14, 0x04]); // movbe dx,[r12+rax]
        assert_eq!(enc(|a| a.movbe_st(64, mx(R12, RAX, 1), RDX)), [0x49, 0x0F, 0x38, 0xF1, 0x14, 0x04]); // movbe [r12+rax],rdx
        // the register-cache forms with a register operand (r9..r11, rbp, r14 hold guest fields)
        assert_eq!(enc(|a| a.ld32o(RDX, O::R(R9))), [0x41, 0x8B, 0xD1]); // mov edx,r9d (the 8B /r form)
        assert_eq!(enc(|a| a.ld32o(RDX, O::R(RDX))), []); // mov edx,edx is dropped
        assert_eq!(enc(|a| a.st32o(O::R(RBP), RDX)), [0x89, 0xD5]); // mov ebp,edx
        assert_eq!(enc(|a| a.st32o(O::R(R14), RSI)), [0x41, 0x89, 0xF6]); // mov r14d,esi
        assert_eq!(enc(|a| a.st32io(O::R(R10), 7)), [0x41, 0xC7, 0xC2, 7, 0, 0, 0]); // mov r10d,7 (the C7 /0 form; as picks B8+r)
        assert_eq!(enc(|a| a.alu_ro(Alu::Add, RDX, O::R(R11))), [0x41, 0x03, 0xD3]); // add edx,r11d (the 03 /r form)
        assert_eq!(enc(|a| a.alu_or(Alu::Or, O::R(R9), RAX)), [0x41, 0x09, 0xC1]); // or r9d,eax
        assert_eq!(enc(|a| a.alu_or(Alu::Or, O::M(m(RBX, 0x80)), RAX)), [0x09, 0x83, 0x80, 0, 0, 0]); // or [rbx+0x80],eax
        assert_eq!(enc(|a| a.alu_oi(Alu::And, O::R(RBP), 0x0FFF_FFFF)), [0x81, 0xE5, 0xFF, 0xFF, 0xFF, 0x0F]); // and ebp,0xfffffff
        assert_eq!(enc(|a| a.alu_oi(Alu::Cmp, O::R(R14), 9)), [0x41, 0x81, 0xFE, 9, 0, 0, 0]); // cmp r14d,9 (imm32; as picks imm8)
        assert_eq!(enc(|a| a.imul_ro(RDX, O::R(R10))), [0x41, 0x0F, 0xAF, 0xD2]); // imul edx,r10d
        assert_eq!(enc(|a| a.imul_roi(RDX, O::R(RBP), 3)), [0x69, 0xD5, 3, 0, 0, 0]); // imul edx,ebp,3 (imm32; as picks 6B imm8)
        assert_eq!(enc(|a| a.movsx16o(RDX, O::R(R9))), [0x41, 0x0F, 0xBF, 0xD1]); // movsx edx,r9w
        assert_eq!(enc(|a| a.movsx8o(RDX, O::R(RBP))), [0x40, 0x0F, 0xBE, 0xD5]); // movsx edx,bpl
        assert_eq!(enc(|a| a.movsx8o(RDX, O::R(R11))), [0x41, 0x0F, 0xBE, 0xD3]); // movsx edx,r11b
        assert_eq!(enc(|a| a.bsr_ro(RDX, O::R(R14))), [0x41, 0x0F, 0xBD, 0xD6]); // bsr edx,r14d
        assert_eq!(enc(|a| a.grp3_o(7, O::R(R9))), [0x41, 0xF7, 0xF9]); // idiv r9d
        assert_eq!(enc(|a| a.adc_ro(RDX, O::R(R11))), [0x41, 0x13, 0xD3]); // adc edx,r11d (the 13 /r form)
        assert_eq!(enc(|a| a.bt_oi(O::R(R10), 31)), [0x41, 0x0F, 0xBA, 0xE2, 31]); // bt r10d,31
        assert_eq!(enc(|a| a.bt_oi(O::R(RBP), 3)), [0x0F, 0xBA, 0xE5, 3]); // bt ebp,3
        let mut a = Asm::default();
        let f = a.jcc(Cc::Ne);
        a.ret();
        a.here(f);
        assert_eq!(a.b, [0x0F, 0x85, 1, 0, 0, 0, 0xC3]);
    }
}

/// CPUID.1:ECX[22] MOVBE (the Z8350 has it; x86-64-v2 does not promise it: runtime dispatch).
pub(crate) fn cpu_has_movbe() -> bool {
    let ecx: u32;
    // SAFETY: CPUID leaf 1 exists on every x86-64 processor and only writes eax/ebx/ecx/edx; rbx, which LLVM reserves, is
    // saved in a scratch register and restored. (asm rather than core::arch::__cpuid: that intrinsic is `unsafe` on the CoralOS
    // toolchain and safe on newer ones, so neither spelling builds warning-free on both.)
    unsafe {
        core::arch::asm!("mov {t}, rbx", "cpuid", "mov rbx, {t}", t = out(reg) _, inout("eax") 1u32 => _,
            inout("ecx") 0u32 => ecx, out("edx") _, options(nostack, preserves_flags));
    }
    ecx & (1 << 22) != 0
}
