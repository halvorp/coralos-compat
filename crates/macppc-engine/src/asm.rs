//! A tiny PowerPC assembler for test programs: the handful of integer instructions a hand-written Mac application
//! needs, with labels. Encodings are from the Programming Environments Manual (PEM) instruction-format tables (D-form,
//! X-form, B-form, I-form, XFX-form for the SPR moves). No `powerpc-*-as` exists on every build host, and a fixture
//! built from words you can read here is cheaper to audit than a binary blob.
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Condition-register bit of CR0 that `beq`/`bne` test.
const CR0_EQ: u32 = 2;

#[derive(Default)]
pub struct Asm {
    words: Vec<u32>,
    labels: BTreeMap<&'static str, usize>,
    /// (word index, label, kind): a branch to patch once all labels are known.
    fixups: Vec<(usize, &'static str, Fix)>,
}

#[derive(Clone, Copy)]
enum Fix { Always, IfEq, IfNe }

fn d(op: u32, rt: u32, ra: u32, imm: i32) -> u32 { op << 26 | rt << 21 | ra << 16 | (imm as u32 & 0xFFFF) }
fn x(rt: u32, ra: u32, rb: u32, xo: u32) -> u32 { 31 << 26 | rt << 21 | ra << 16 | rb << 11 | xo << 1 }
fn spr(rt: u32, spr: u32, xo: u32) -> u32 { 31 << 26 | rt << 21 | ((spr & 0x1F) << 5 | (spr >> 5)) << 11 | xo << 1 }

impl Asm {
    pub fn new() -> Self { Self::default() }
    fn push(&mut self, w: u32) -> &mut Self { self.words.push(w); self }
    /// Byte offset of the next instruction.
    pub fn here(&self) -> u32 { 4 * self.words.len() as u32 }
    pub fn label(&mut self, name: &'static str) -> &mut Self { self.labels.insert(name, self.words.len()); self }

    pub fn li(&mut self, rt: u32, v: i32) -> &mut Self { self.push(d(14, rt, 0, v)) }
    /// `lis rt,v` = `addis rt,0,v`.
    pub fn lis(&mut self, rt: u32, v: i32) -> &mut Self { self.push(d(15, rt, 0, v)) }
    /// `ori ra,rs,v`.
    pub fn ori(&mut self, ra: u32, rs: u32, v: i32) -> &mut Self { self.push(d(24, rs, ra, v)) }
    pub fn addi(&mut self, rt: u32, ra: u32, v: i32) -> &mut Self { self.push(d(14, rt, ra, v)) }
    pub fn lwz(&mut self, rt: u32, off: i32, ra: u32) -> &mut Self { self.push(d(32, rt, ra, off)) }
    pub fn stw(&mut self, rs: u32, off: i32, ra: u32) -> &mut Self { self.push(d(36, rs, ra, off)) }
    pub fn stwu(&mut self, rs: u32, off: i32, ra: u32) -> &mut Self { self.push(d(37, rs, ra, off)) }
    pub fn lhz(&mut self, rt: u32, off: i32, ra: u32) -> &mut Self { self.push(d(40, rt, ra, off)) }
    pub fn lbz(&mut self, rt: u32, off: i32, ra: u32) -> &mut Self { self.push(d(34, rt, ra, off)) }
    pub fn sth(&mut self, rs: u32, off: i32, ra: u32) -> &mut Self { self.push(d(44, rs, ra, off)) }
    pub fn cmpwi(&mut self, ra: u32, v: i32) -> &mut Self { self.push(d(11, 0, ra, v)) }
    /// `mr ra,rs` = `or ra,rs,rs`.
    pub fn mr(&mut self, ra: u32, rs: u32) -> &mut Self { self.push(x(rs, ra, rs, 444)) }
    pub fn mflr(&mut self, rt: u32) -> &mut Self { self.push(spr(rt, 8, 339)) }
    pub fn mtlr(&mut self, rs: u32) -> &mut Self { self.push(spr(rs, 8, 467)) }
    pub fn mtctr(&mut self, rs: u32) -> &mut Self { self.push(spr(rs, 9, 467)) }
    pub fn bctrl(&mut self) -> &mut Self { self.push(0x4E80_0421) }
    pub fn blr(&mut self) -> &mut Self { self.push(0x4E80_0020) }
    pub fn b(&mut self, to: &'static str) -> &mut Self { self.fix(to, Fix::Always) }
    pub fn beq(&mut self, to: &'static str) -> &mut Self { self.fix(to, Fix::IfEq) }
    pub fn bne(&mut self, to: &'static str) -> &mut Self { self.fix(to, Fix::IfNe) }
    fn fix(&mut self, to: &'static str, k: Fix) -> &mut Self { self.fixups.push((self.words.len(), to, k)); self.push(0) }

    /// The CFM pointer-glue call sequence through the TVector whose address sits in the TOC word at `toc_off(r2)`:
    /// r12 = TVector, save RTOC at 20(r1), CTR = code, r2 = the TVector's toc, call, restore RTOC.
    pub fn call_toc(&mut self, toc_off: i32) -> &mut Self {
        self.lwz(12, toc_off, 2).lwz(0, 0, 12).stw(2, 20, 1).mtctr(0).lwz(2, 4, 12).bctrl().lwz(2, 20, 1)
    }

    /// The program's bytes, big-endian, branches resolved; panics on an undefined label (a bug in the fixture).
    pub fn finish(&self) -> Vec<u8> {
        let mut w = self.words.clone();
        for &(at, name, k) in &self.fixups {
            let to = *self.labels.get(name).unwrap_or_else(|| panic!("asm: undefined label {name}"));
            let off = (to as i32 - at as i32) * 4;
            w[at] = match k {
                Fix::Always => 18 << 26 | (off as u32 & 0x03FF_FFFC),
                Fix::IfEq => 16 << 26 | 12 << 21 | CR0_EQ << 16 | (off as u32 & 0xFFFC),
                Fix::IfNe => 16 << 26 | 4 << 21 | CR0_EQ << 16 | (off as u32 & 0xFFFC),
            };
        }
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodings_match_known_words() {
        let mut a = Asm::new();
        a.li(3, 0).blr().mflr(0).mtctr(0).mtlr(0).stwu(1, -96, 1).lwz(12, 8, 2).mr(31, 3).cmpwi(3, 0);
        let w: Vec<u32> = a.finish().chunks(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect();
        // li r3,0 ; blr ; mflr r0 ; mtctr r0 ; mtlr r0 ; stwu r1,-96(r1) ; lwz r12,8(r2) ; mr r31,r3 ; cmpwi r3,0
        assert_eq!(w, [0x3860_0000, 0x4E80_0020, 0x7C08_02A6, 0x7C09_03A6, 0x7C08_03A6, 0x9421_FFA0, 0x8182_0008, 0x7C7F_1B78, 0x2C03_0000]);
    }
    #[test]
    fn branches_resolve_forwards_and_backwards() {
        let mut a = Asm::new();
        a.label("top").li(3, 1).bne("end").b("top").label("end").blr();
        let w: Vec<u32> = a.finish().chunks(4).map(|c| u32::from_be_bytes(c.try_into().unwrap())).collect();
        assert_eq!(w[1], 0x4082_0008, "bne +8"); assert_eq!(w[2], 0x4BFF_FFF8, "b -8");
    }
}
