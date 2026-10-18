//! A tiny seeded generator (splitmix64 seeding, xorshift64*): deterministic across platforms, no dependency.

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }
    pub fn u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn u32(&mut self) -> u32 { (self.u64() >> 32) as u32 }
    /// 0..n
    pub fn below(&mut self, n: u32) -> u32 { ((self.u64() >> 32) * n as u64 >> 32) as u32 }
    /// true with probability `pct` percent.
    pub fn pct(&mut self, pct: u32) -> bool { self.below(100) < pct }
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T { xs[self.below(xs.len() as u32) as usize] }
}
