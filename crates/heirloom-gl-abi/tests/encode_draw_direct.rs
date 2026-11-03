//! `Frame::encode_draw_elements_into` (the engine's draw path, h86 gl-callout-5) writes EXACTLY the bytes of the snapshot path it
//! replaces - `Frame { token, command: Command::Draw(DrawCommand::capture(..)) }.encode_into(..)` - for every index type, packed and
//! strided arrays of every element width, empty draws, and every error (which leaves the queue as it was and is the same error).
use heirloom_gl_abi::command::{Command, DrawCommand, Frame};
use heirloom_gl_abi::snapshot::{Descriptor, ElemType, Role};
use heirloom_gl_abi::Token;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn below(&mut self, n: u32) -> u32 { self.next() % n.max(1) }
}

const TYPES: [ElemType; 8] = [ElemType::I8, ElemType::U8, ElemType::I16, ElemType::U16, ElemType::I32, ElemType::U32, ElemType::F32, ElemType::F64];
const ROLES: [Role; 4] = [Role::Vertex, Role::Normal, Role::Color, Role::TexCoord];

#[test]
fn the_direct_draw_encoding_is_the_snapshot_encoding_byte_for_byte() {
    let mut r = Lcg(0x5eed);
    let mem: Vec<u8> = (0..65536).map(|_| r.next() as u8).collect();
    let token = Token { slot: 3, incarnation: 9 };
    let (mut ok, mut err) = (0, 0);
    for case in 0..4000 {
        let ity = [ElemType::U8, ElemType::U16, ElemType::U32, ElemType::F32][r.below(if case % 50 == 0 { 4 } else { 3 }) as usize];
        let count = if case % 37 == 0 { 0 } else { r.below(200) };
        let index_off = if case % 101 == 0 { 65530 } else { r.below(4096) as usize };
        // indices in the guest's (big-endian) order, small enough to land inside the arrays mostly
        let mut m = mem.clone();
        if matches!(ity, ElemType::U8 | ElemType::U16 | ElemType::U32) && index_off + count as usize * ity.bytes() <= m.len() {
            for k in 0..count as usize {
                let v = r.below(if case % 23 == 0 { 70_000 } else { 300 });
                let at = index_off + k * ity.bytes();
                match ity {
                    ElemType::U8 => m[at] = v as u8,
                    ElemType::U16 => m[at..at + 2].copy_from_slice(&(v as u16).to_be_bytes()),
                    _ => m[at..at + 4].copy_from_slice(&v.to_be_bytes()),
                }
            }
        }
        let arrays: Vec<Descriptor> = (0..r.below(5)).map(|k| {
            let ty = TYPES[r.below(8) as usize];
            let size = if case % 41 == 0 { 5 } else { 1 + r.below(4) as u8 };
            let vbytes = size as u32 * ty.bytes() as u32;
            let stride = match r.below(3) { 0 => 0, 1 => vbytes, _ => vbytes + r.below(16) };
            Descriptor { role: ROLES[k as usize % 4], unit: k as u8, offset: 8192 + r.below(8192) as usize, size, ty, stride }
        }).collect();
        let mode = r.below(10);
        let mut old = vec![0xAB; 5];
        let mut new = old.clone();
        let want = DrawCommand::capture(&m, mode, index_off, count, ity, &arrays).map(|d| Frame { token, command: Command::Draw(d) }.encode_into(&mut old));
        let got = Frame::encode_draw_elements_into(&mut new, token, &m, mode, index_off, count, ity, &arrays);
        assert_eq!(got, want, "case {case}");
        assert_eq!(new, old, "case {case}: the bytes differ");
        if got.is_ok() { ok += 1 } else { err += 1 }
    }
    assert!(ok > 1000 && err > 100, "both paths exercised: ok={ok} err={err}");
}

#[test]
fn the_fast_call_encoding_is_the_frame_encoding() {
    let token = Token { slot: 1, incarnation: 2 };
    for n in 0..20u64 {
        let args: Vec<u64> = (0..n).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)).collect();
        let mut a = vec![1, 2, 3];
        let mut b = a.clone();
        Frame { token, command: Command::Call { entry: 0x0102 + n as u16, args: args.clone() } }.encode_into(&mut a);
        Frame::encode_call_into(&mut b, token, 0x0102 + n as u16, &args);
        assert_eq!(a, b, "{n} words");
    }
}
