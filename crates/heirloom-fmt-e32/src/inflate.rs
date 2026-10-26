//! Symbian's image "deflate" (KUidCompressionDeflate): its own Huffman format, not RFC 1951.
//!
//! Format facts (the bitstream, not any implementation): bits are read most-significant first. The stream opens with
//! the code lengths of two canonical Huffman alphabets read as one table of 329 entries - 285 literal/length symbols
//! (256 literals, 28 length codes, end-of-stream) then 44 distance codes. That table is itself coded: each entry is a
//! symbol of a fixed 29-symbol metacode, where 0 and 1 are bijective base-2 digits of a run of the previous length and
//! 2..28 pick a length by its position in a move-to-front list. A length or distance code `c` carries `(c >> 2) - 1`
//! extra bits once `c >= 8`; its value is `((c - 4 * extra) << extra) | bits`, a match length being value + 3 and a
//! distance value + 1, reaching back at most 4096 bytes.
use alloc::vec::Vec;

use crate::E32Error;

const LITERALS: usize = 256;
const LENGTH_CODES: usize = 28;
const LIT_LEN: usize = LITERALS + LENGTH_CODES + 1;
const END_OF_STREAM: usize = LITERALS + LENGTH_CODES;
const DIST_CODES: usize = 44;
const MAX_CODE_LEN: usize = 27;
const MIN_MATCH: usize = 3;
const MAX_DIST: usize = 4096;
/// Code lengths of the fixed metacode (symbols 0..28), canonical in symbol order.
const META_LENGTHS: [u8; 29] = [2, 3, 2, 3, 4, 4, 5, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 9, 10, 11, 12, 14, 15, 15, 15, 15, 15, 16, 16];
/// The move-to-front list holds every length a code can have, 0..=27.
const MTF_LEN: usize = MAX_CODE_LEN + 1;

const BAD: E32Error = E32Error::Malformed("Symbian deflate stream");

struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<u32, E32Error> {
        let byte = *self.bytes.get(self.at >> 3).ok_or(E32Error::Truncated("Symbian deflate stream"))?;
        let bit = (byte >> (7 - (self.at & 7))) & 1;
        self.at += 1;
        Ok(u32::from(bit))
    }

    fn bits(&mut self, n: u32) -> Result<u32, E32Error> {
        (0..n).try_fold(0, |v, _| Ok(v << 1 | self.bit()?))
    }
}

/// A canonical Huffman decoder: per-length counts and the symbols in code order.
struct Huffman {
    counts: [u16; MAX_CODE_LEN + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, E32Error> {
        let mut counts = [0u16; MAX_CODE_LEN + 1];
        for &l in lengths {
            if usize::from(l) > MAX_CODE_LEN {
                return Err(BAD);
            }
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        // Kraft: a complete code, or at most one code (a single symbol is coded with one bit).
        let used: usize = counts.iter().map(|&c| usize::from(c)).sum();
        let mut space: i64 = 1;
        for &c in &counts[1..] {
            space = (space << 1) - i64::from(c);
            if space < 0 {
                return Err(BAD);
            }
        }
        if space != 0 && used > 1 {
            return Err(BAD);
        }
        let mut symbols = Vec::with_capacity(used);
        for len in 1..=MAX_CODE_LEN as u8 {
            symbols.extend((0..lengths.len()).filter(|&s| lengths[s] == len).map(|s| s as u16));
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<usize, E32Error> {
        if self.symbols.len() == 1 {
            bits.bit()?;
            return Ok(usize::from(self.symbols[0]));
        }
        let (mut code, mut first, mut index) = (0i64, 0i64, 0i64);
        for &count in &self.counts[1..] {
            code |= i64::from(bits.bit()?);
            let count = i64::from(count);
            if code - first < count {
                return Ok(usize::from(self.symbols[(index + code - first) as usize]));
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(BAD)
    }
}

/// The 329 code lengths, read through the metacode.
fn code_lengths(bits: &mut Bits) -> Result<Vec<u8>, E32Error> {
    let meta = Huffman::new(&META_LENGTHS)?;
    let total = LIT_LEN + DIST_CODES;
    let mut mtf: [u8; MTF_LEN] = core::array::from_fn(|i| i as u8);
    let mut out = Vec::with_capacity(total);
    let (mut last, mut run) = (0u8, 0usize);
    let push_run = |out: &mut Vec<u8>, run: &mut usize, last: u8| -> Result<(), E32Error> {
        if out.len() + *run > total {
            return Err(BAD);
        }
        out.extend(core::iter::repeat(last).take(*run));
        *run = 0;
        Ok(())
    };
    while out.len() + run < total {
        let c = meta.decode(bits)?;
        if c < 2 {
            run = run.checked_mul(2).and_then(|r| r.checked_add(c + 1)).ok_or(BAD)?;
            continue;
        }
        push_run(&mut out, &mut run, last)?;
        let pos = c - 1;
        mtf[0] = last;
        last = mtf[pos];
        mtf.copy_within(0..pos, 1);
        out.push(last);
    }
    push_run(&mut out, &mut run, last)?;
    Ok(out)
}

fn extra(code: usize, bits: &mut Bits) -> Result<usize, E32Error> {
    if code < 8 {
        return Ok(code);
    }
    let x = (code >> 2) - 1;
    Ok(((code - (x << 2)) << x) | bits.bits(x as u32)? as usize)
}

/// Decompress exactly `size` bytes, or refuse by name.
pub(crate) fn inflate(stream: &[u8], size: usize) -> Result<Vec<u8>, E32Error> {
    let mut bits = Bits { bytes: stream, at: 0 };
    let lengths = code_lengths(&mut bits)?;
    let lit = Huffman::new(&lengths[..LIT_LEN])?;
    let dist = Huffman::new(&lengths[LIT_LEN..])?;
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        let sym = lit.decode(&mut bits)?;
        if sym < LITERALS {
            out.push(sym as u8);
            continue;
        }
        if sym == END_OF_STREAM {
            break;
        }
        let len = extra(sym - LITERALS, &mut bits)? + MIN_MATCH;
        let back = extra(dist.decode(&mut bits)?, &mut bits)? + 1;
        if back > out.len() || back > MAX_DIST || out.len() + len > size {
            return Err(BAD);
        }
        let from = out.len() - back;
        for i in 0..len {
            out.push(out[from + i]);
        }
    }
    if out.len() != size {
        return Err(E32Error::Malformed("Symbian deflate stream ended short of iUncompressedSize"));
    }
    Ok(out)
}
