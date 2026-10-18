//! The Pascal calling convention of the classic Toolbox, implemented ONCE for every trap.
//!
//! The caller reserves the function result (if any) on the stack, then pushes the arguments left to right, then
//! executes the A-line word. The callee pops its arguments and leaves the result where the caller reserved it. So at
//! the trap, SP points at the LAST argument; the first argument is deepest; the result slot lies just above all of
//! the arguments. A one-byte value (BOOLEAN) occupies a 16-bit slot with the byte at the LOW address (a `move.b` push
//! on A7 steps SP by 2). [`read_args`] gives the values back in declaration order; [`complete`] pops them and stores
//! the result.
use alloc::vec::Vec;
use retro_common::Refusal;
use retro_guest_mem::GuestMem;

/// One argument or result in a Pascal frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    /// BOOLEAN / Byte: a 16-bit slot, the value in its first (low-address) byte.
    B,
    /// INTEGER: 16 bits.
    W,
    /// LONGINT / any pointer or handle: 32 bits.
    L,
}

impl Arg {
    /// Bytes the argument occupies on the stack.
    pub const fn size(self) -> u32 { match self { Arg::B | Arg::W => 2, Arg::L => 4 } }
}

/// Total stack bytes of an argument list (what the callee pops).
pub fn frame_size(layout: &[Arg]) -> u32 { layout.iter().map(|a| a.size()).sum() }

/// Argument values in declaration order (B and W zero-extended).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args(Vec<u32>);

impl Args {
    /// The arguments of a call made BY VALUE (another processor's convention: PowerPC passes them in r3..r10): one
    /// value per layout entry, each truncated to its slot width the way a 68k stack slot would hold it.
    pub fn from_values(layout: &[Arg], values: &[u32]) -> Result<Args, Refusal> {
        if layout.len() != values.len() { return Err(Refusal(alloc::format!("call passes {} arguments, the Toolbox signature takes {}", values.len(), layout.len()))); }
        Ok(Args(layout.iter().zip(values).map(|(a, &v)| match a { Arg::B => v & 0xFF, Arg::W => v & 0xFFFF, Arg::L => v }).collect()))
    }
    pub fn get(&self, i: usize) -> u32 { self.0[i] }
    pub fn len(&self) -> usize { self.0.len() }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

/// Read the arguments of a call whose last argument is at `sp`.
pub fn read_args(mem: &GuestMem, sp: u32, layout: &[Arg]) -> Result<Args, Refusal> {
    let mut at = sp.checked_add(frame_size(layout)).ok_or_else(|| Refusal("Pascal frame: sp overflows".into()))?;
    let mut out = alloc::vec![0u32; layout.len()];
    for (i, a) in layout.iter().enumerate() {
        at -= a.size();
        out[i] = match a {
            Arg::B => mem.read_u8(at).map(u32::from),
            Arg::W => mem.read_u16(at).map(u32::from),
            Arg::L => mem.read_u32(at),
        }.map_err(Refusal::mem("Pascal argument"))?;
    }
    Ok(Args(out))
}

/// Pop the arguments (the callee's job) and store `result` in the slot the caller reserved above them. Returns the
/// new SP, which the caller sets.
pub fn complete(mem: &mut GuestMem, sp: u32, layout: &[Arg], result: Option<(Arg, u32)>) -> Result<u32, Refusal> {
    let new_sp = sp.checked_add(frame_size(layout)).ok_or_else(|| Refusal("Pascal frame: sp overflows".into()))?;
    if let Some((kind, v)) = result {
        match kind {
            Arg::B => mem.write_u8(new_sp, v as u8),
            Arg::W => mem.write_u16(new_sp, v as u16),
            Arg::L => mem.write_u32(new_sp, v),
        }.map_err(Refusal::mem("Pascal result"))?;
    }
    Ok(new_sp)
}

/// A Str255 (length byte + that many bytes) at `addr`. Bytes above 0x7F are shown as their Latin-1 character (a
/// MacRoman table is a later step).
pub fn read_pstring(mem: &GuestMem, addr: u32) -> Result<alloc::string::String, Refusal> {
    Ok(read_pstring_bytes(mem, addr)?.iter().map(|&b| char::from(b)).collect())
}

/// The raw bytes of a Str255 at `addr` (no character mapping), for byte-exact name matching.
pub fn read_pstring_bytes(mem: &GuestMem, addr: u32) -> Result<Vec<u8>, Refusal> {
    let len = mem.read_u8(addr).map_err(Refusal::mem("Str255 length"))? as u32;
    let span = mem.span(addr + 1, len).map_err(Refusal::mem("Str255"))?;
    Ok(mem.bytes(span).map_err(Refusal::mem("Str255"))?.to_vec())
}

/// A classic Mac `Rect`: four 16-bit INTEGERs, top, left, bottom, right (the reverse of x,y habit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect { pub top: i16, pub left: i16, pub bottom: i16, pub right: i16 }

pub fn read_rect(mem: &GuestMem, addr: u32) -> Result<Rect, Refusal> {
    let m = Refusal::mem("Rect");
    Ok(Rect { top: mem.read_i16(addr).map_err(&m)?, left: mem.read_i16(addr + 2).map_err(&m)?,
              bottom: mem.read_i16(addr + 4).map_err(&m)?, right: mem.read_i16(addr + 6).map_err(&m)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FUNCTION F(a: LONGINT; b: BOOLEAN; c: INTEGER): LONGINT, called with a=0x11223344, b=TRUE, c=-2, SP = 0x100.
    fn frame() -> GuestMem {
        let mut m = GuestMem::new(0x1000);
        m.write_u32(0x100 + 4 + 2 + 2, 0xDEAD_BEEF).unwrap(); // reserved result slot (above the args)
        m.write_u32(0x100 + 4, 0x1122_3344).unwrap();         // a (first pushed = deepest)
        m.write_u8(0x100 + 2, 1).unwrap();                    // b: byte at the low address of its 16-bit slot
        m.write_u16(0x100, 0xFFFE).unwrap();                  // c (last pushed = at SP)
        m
    }
    const LAYOUT: [Arg; 3] = [Arg::L, Arg::B, Arg::W];

    #[test]
    fn arguments_come_back_in_declaration_order() {
        let a = read_args(&frame(), 0x100, &LAYOUT).unwrap();
        assert_eq!((a.get(0), a.get(1), a.get(2)), (0x1122_3344, 1, 0xFFFE));
        assert_eq!(frame_size(&LAYOUT), 8);
    }

    #[test]
    fn complete_pops_the_arguments_and_stores_the_result_in_the_reserved_slot() {
        let mut m = frame();
        let sp = complete(&mut m, 0x100, &LAYOUT, Some((Arg::L, 0x0020_0000))).unwrap();
        assert_eq!(sp, 0x108, "args popped: SP now points at the result");
        assert_eq!(m.read_u32(sp).unwrap(), 0x0020_0000);
        // Negative control: a procedure (no result) leaves the reserved slot alone.
        let mut m = frame();
        let sp = complete(&mut m, 0x100, &LAYOUT, None).unwrap();
        assert_eq!(m.read_u32(sp).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn boolean_result_goes_in_the_high_byte_and_a_wild_frame_is_refused() {
        let mut m = GuestMem::new(0x1000);
        let sp = complete(&mut m, 0x200, &[Arg::W], Some((Arg::B, 1))).unwrap();
        assert_eq!(m.read_u16(sp).unwrap(), 0x0100);
        assert!(read_args(&m, 0xFFFF_FFF0, &LAYOUT).is_err(), "sp overflow / outside RAM is a refusal, not a panic");
        assert!(complete(&mut m, 0x0FFC, &[Arg::L], Some((Arg::L, 1))).is_err());
    }

    #[test]
    fn strings_and_rects() {
        let mut m = GuestMem::new(0x1000);
        m.write_bytes(0x10, b"\x09Hello Mac").unwrap();
        assert_eq!(read_pstring(&m, 0x10).unwrap(), "Hello Mac");
        m.write_u16(0x20, 40).unwrap(); m.write_u16(0x22, 30).unwrap(); m.write_u16(0x24, 240).unwrap(); m.write_u16(0x26, 350).unwrap();
        assert_eq!(read_rect(&m, 0x20).unwrap(), Rect { top: 40, left: 30, bottom: 240, right: 350 });
        m.write_u8(0xFFF, 5).unwrap();
        assert!(read_pstring(&m, 0xFFF).is_err(), "length byte in range, bytes past RAM: refused");
    }
}
