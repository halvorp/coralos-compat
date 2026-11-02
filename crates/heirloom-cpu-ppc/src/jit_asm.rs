//! Tier 2's x86-64 encoder: only the instruction forms `jit_emit` uses, base x86-64 (nothing past x86-64-v2: no VEX,
//! no MOVBE/BMI yet - those come behind runtime CPUID dispatch). Encodings follow the Intel SDM vol. 2 opcode tables;
//! the unit tests at the bottom pin each form against bytes an assembler produces.
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
pub(crate) const R12: u8 = 12;
pub(crate) const R13: u8 = 13;
pub(crate) const R14: u8 = 14;
pub(crate) const R15: u8 = 15;

/// Condition codes (the low nibble of Jcc / SETcc / CMOVcc).
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Cc { B = 2, Ae = 3, E = 4, Ne = 5, A = 7, S = 8, L = 12, G = 15 }

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

    // ---- moves ----
    pub(crate) fn ld32(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x8B], r, mm, false) }
    pub(crate) fn ld64(&mut self, r: u8, mm: M) { self.op_m(None, true, &[0x8B], r, mm, false) }
    pub(crate) fn st32(&mut self, mm: M, r: u8) { self.op_m(None, false, &[0x89], r, mm, false) }
    pub(crate) fn st64(&mut self, mm: M, r: u8) { self.op_m(None, true, &[0x89], r, mm, false) }
    pub(crate) fn st16(&mut self, mm: M, r: u8) { self.op_m(Some(0x66), false, &[0x89], r, mm, false) }
    pub(crate) fn st8(&mut self, mm: M, r: u8) { self.op_m(None, false, &[0x88], r, mm, r >= 4) }
    pub(crate) fn movzx8(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xB6], r, mm, false) }
    pub(crate) fn movzx16(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xB7], r, mm, false) }
    pub(crate) fn movsx8(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xBE], r, mm, false) }
    pub(crate) fn movsx16(&mut self, r: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xBF], r, mm, false) }
    /// movzx r32, r8.
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
    /// op dword [m], imm32.
    pub(crate) fn alu_mi(&mut self, op: Alu, mm: M, v: u32) {
        self.op_m(None, false, &[0x81], op as u8, mm, false);
        self.u32(v);
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
    /// imul r32, [m].
    pub(crate) fn imul_rm(&mut self, d: u8, mm: M) { self.op_m(None, false, &[0x0F, 0xAF], d, mm, false) }
    /// imul r32, [m], imm32.
    pub(crate) fn imul_rmi(&mut self, d: u8, mm: M, v: u32) {
        self.op_m(None, false, &[0x69], d, mm, false);
        self.u32(v);
    }
    pub(crate) fn neg(&mut self, r: u8) { self.op_r(false, &[0xF7], 3, r, false) }
    pub(crate) fn not(&mut self, r: u8) { self.op_r(false, &[0xF7], 2, r, false) }
    pub(crate) fn test_rr(&mut self, a: u8, b: u8) { self.op_r(false, &[0x85], b, a, false) }
    pub(crate) fn test_ri(&mut self, r: u8, v: u32) {
        self.op_r(false, &[0xF7], 0, r, false);
        self.u32(v);
    }
    pub(crate) fn setcc(&mut self, cc: Cc, r: u8) { self.op_r(false, &[0x0F, 0x90 + cc as u8], 0, r, r >= 4) }
    pub(crate) fn cmov(&mut self, cc: Cc, d: u8, s: u8) { self.op_r(false, &[0x0F, 0x40 + cc as u8], d, s, false) }
    /// bt dword [m], imm8 (CF = the bit).
    pub(crate) fn bt_mi(&mut self, mm: M, bit: u8) {
        self.op_m(None, false, &[0x0F, 0xBA], 4, mm, false);
        self.u8(bit);
    }
    /// bt r64, r64 (bit index mod 64).
    pub(crate) fn bt_rr64(&mut self, r: u8, bit: u8) { self.op_r(true, &[0x0F, 0xA3], bit, r, false) }

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

    /// Each expected byte string is what GNU as (Intel syntax) assembles for the comment beside it.
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
        assert_eq!(enc(|a| a.alu_mi(Alu::Cmp, m(RBX, 12), 9)), [0x81, 0x7B, 0x0C, 9, 0, 0, 0]); // cmp dword [rbx+12],9
        assert_eq!(enc(|a| a.and8_rr(RAX, RCX)), [0x20, 0xC8]); // and al,cl
        assert_eq!(enc(|a| a.sh_ri(Sh::Rol, RDX, 3)), [0xC1, 0xC2, 3]); // rol edx,3
        assert_eq!(enc(|a| a.sh_cl(Sh::Shl, RDX)), [0xD3, 0xE2]); // shl edx,cl
        assert_eq!(enc(|a| a.bswap32(RDX)), [0x0F, 0xCA]);
        assert_eq!(enc(|a| a.bswap64(RDX)), [0x48, 0x0F, 0xCA]);
        assert_eq!(enc(|a| a.imul_rmi(RDX, m(RBX, 8), 3)), [0x69, 0x53, 0x08, 3, 0, 0, 0]); // imul edx,[rbx+8],3
        assert_eq!(enc(|a| a.imul_rm(RDX, m(RBX, 8))), [0x0F, 0xAF, 0x53, 0x08]);
        assert_eq!(enc(|a| a.setcc(Cc::L, RAX)), [0x0F, 0x9C, 0xC0]); // setl al
        assert_eq!(enc(|a| a.setcc(Cc::B, R8)), [0x41, 0x0F, 0x92, 0xC0]); // setb r8b
        assert_eq!(enc(|a| a.movzx8_rr(RAX, RAX)), [0x0F, 0xB6, 0xC0]); // movzx eax,al
        assert_eq!(enc(|a| a.cmov(Cc::Ne, RDX, RAX)), [0x0F, 0x45, 0xD0]); // cmovne edx,eax
        assert_eq!(enc(|a| a.bt_mi(m(RBX, 0x100), 5)), [0x0F, 0xBA, 0xA3, 0x00, 0x01, 0, 0, 5]); // bt dword [rbx+0x100],5
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
        assert_eq!(enc(|a| a.movsx16(RDX, m(RBX, 8))), [0x0F, 0xBF, 0x53, 0x08]);
        let mut a = Asm::default();
        let f = a.jcc(Cc::Ne);
        a.ret();
        a.here(f);
        assert_eq!(a.b, [0x0F, 0x85, 1, 0, 0, 0, 0xC3]);
    }
}
