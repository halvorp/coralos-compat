//! The draw-time snapshot is byte-for-byte the reference definition - every referenced vertex copied out of guest memory in
//! order, stride removed, each component reversed from big- to little-endian - for every element type, component count,
//! packed and padded strides and ranges, and the index snapshot likewise (its range from the converted values).
use heirloom_gl_abi::snapshot::{snapshot_array, snapshot_indices, Descriptor, ElemType, Role};

const TYPES: [ElemType; 8] = [ElemType::I8, ElemType::U8, ElemType::I16, ElemType::U16, ElemType::I32, ElemType::U32, ElemType::F32, ElemType::F64];

fn reference(mem: &[u8], d: &Descriptor, first: u32, count: u32) -> Vec<u8> {
    let vbytes = d.size as usize * d.ty.bytes();
    let stride = if d.stride == 0 { vbytes } else { d.stride as usize };
    let mut out = Vec::new();
    for i in first as usize..(first + count) as usize {
        let at = d.offset + i * stride;
        for c in mem[at..at + vbytes].chunks_exact(d.ty.bytes()) {
            out.extend(c.iter().rev());
        }
    }
    out
}

fn guest(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn every_array_snapshot_is_the_reference_copy_in_wire_order() {
    let mem = guest(4096, 0x9E37_79B9_7F4A_7C15);
    let mut cases = 0;
    for ty in TYPES {
        for size in 1..=4u8 {
            let vbytes = size as u32 * ty.bytes() as u32;
            for stride in [0, vbytes, vbytes + 1, vbytes + 4, 32] {
                if stride != 0 && stride < vbytes {
                    continue;
                }
                for (offset, first, count) in [(0usize, 0u32, 1u32), (3, 0, 7), (16, 5, 11), (1, 2, 0), (40, 0, 33)] {
                    let d = Descriptor { role: Role::Vertex, unit: 0, offset, size, ty, stride };
                    let s = snapshot_array(&mem, &d, first, count).unwrap();
                    assert_eq!(s.data, reference(&mem, &d, first, count), "{ty:?} size {size} stride {stride} at {offset} {first}+{count}");
                    assert_eq!((s.first, s.count, s.size, s.ty), (first, count, size, ty));
                    cases += 1;
                }
            }
        }
    }
    assert!(cases > 400, "{cases}");
}

#[test]
fn every_index_snapshot_is_the_reference_copy_and_its_range() {
    let mem = guest(1024, 0x2545_F491_4F6C_DD1D);
    for ty in [ElemType::U8, ElemType::U16, ElemType::U32] {
        for (offset, count) in [(0usize, 1u32), (3, 9), (64, 100), (5, 0)] {
            let s = snapshot_indices(&mem, offset, count, ty).unwrap();
            let raw = &mem[offset..offset + count as usize * ty.bytes()];
            let vals: Vec<u32> = raw.chunks_exact(ty.bytes()).map(|c| c.iter().fold(0u32, |v, b| (v << 8) | u32::from(*b))).collect();
            let want: Vec<u8> = raw.chunks_exact(ty.bytes()).flat_map(|c| c.iter().rev().copied()).collect();
            assert_eq!(s.data, want, "{ty:?} {offset}+{count}");
            assert_eq!(s.range, vals.iter().min().zip(vals.iter().max()).map(|(a, b)| (*a, *b)), "{ty:?} {offset}+{count}");
        }
    }
}
