//! LoadSeg / UnLoadSeg: Hunk executables from the volume into the guest heap as a real AmigaDOS SEGLIST.
//!
//! One heap region holds the whole image (heirloom-fmt-hunk lays the hunks out back to back, with 8 bytes of room before
//! each). Per segment the standard header sits in that room: the longword at `seg-4` is the segment's length
//! (including the header) and the longword at `seg` is the BPTR of the next segment (0 ends the chain), so a
//! seglist BPTR is `(first segment address) >> 2` and the code starts at `seg + 4`. Host-side `fs.segs` remembers
//! each list's region so UnLoadSeg frees exactly what LoadSeg took.
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::dos_err::*;
use crate::fs::{cur_dir, fail, ok, str_arg, DOSTRUE};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_fmt_hunk::HunkError;
use heirloom_cpu_68k::Cpu;

/// Room before every hunk: its length word and its next-segment link.
const HEADER: u32 = 8;

/// A loaded seglist.
#[derive(Debug, Clone)]
pub struct SegList {
    /// BPTR of the first segment.
    pub bptr: u32,
    /// Address of every hunk's contents (the code of hunk 0 is where execution starts).
    pub bases: Vec<u32>,
    pub sizes: Vec<u32>,
}

fn hunk_err(e: &HunkError) -> DosErr {
    match e { HunkError::NotAHunkFile => ERROR_FILE_NOT_OBJECT, _ => ERROR_BAD_HUNK }
}

/// Load the Hunk executable `file` into the guest heap. The error is the AmigaDOS code LoadSeg would leave in IoErr.
pub fn load_bytes<U: GuestUi>(a: &mut Amiga<U>, file: &[u8]) -> Result<SegList, (DosErr, String)> {
    let sizes = heirloom_fmt_hunk::hunk_sizes(file).map_err(|e| (hunk_err(&e), format!("{e}")))?;
    let total = sizes.iter().try_fold(0u32, |t, &s| t.checked_add(s)?.checked_add(HEADER))
        .ok_or((ERROR_BAD_HUNK, "hunk sizes overflow".into()))?;
    let region = a.heap.alloc(total).ok_or((ERROR_NO_FREE_STORE, format!("no guest memory for {total} bytes")))?;
    let loaded = match heirloom_fmt_hunk::load_spaced(&mut a.mem, file, region, HEADER) {
        Ok(l) => l,
        Err(e) => { let _ = a.heap.free(region, total); return Err((hunk_err(&e), format!("{e}"))); }
    };
    let seg_bptr = |i: usize| (loaded.bases[i] - 4) >> 2;
    for i in 0..loaded.bases.len() {
        let next = if i + 1 < loaded.bases.len() { seg_bptr(i + 1) } else { 0 };
        let w = a.mem.write_u32(loaded.bases[i] - 8, sizes[i] + HEADER).and_then(|_| a.mem.write_u32(loaded.bases[i] - 4, next));
        if let Err(e) = w { let _ = a.heap.free(region, total); return Err((ERROR_BAD_HUNK, format!("{e}"))); }
    }
    let bptr = seg_bptr(0);
    a.fs.segs.insert(bptr, (region, total));
    Ok(SegList { bptr, bases: loaded.bases, sizes })
}

/// Free a seglist LoadSeg made. Ok(false) = `bptr` is not one.
pub fn unload<U: GuestUi>(a: &mut Amiga<U>, bptr: u32) -> Result<bool, Refusal> {
    let Some((region, total)) = a.fs.segs.remove(&bptr) else { return Ok(false) };
    a.heap.free(region, total).map_err(|e| Refusal(format!("UnLoadSeg: {e}")))?;
    Ok(true)
}

/// LoadSeg(d1 = name) -> seglist BPTR, or 0 with IoErr.
pub fn load_seg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let name = str_arg(a, cpu.dreg(1), "LoadSeg.name")?;
    let what = format!("[dos] LoadSeg \"{name}\"");
    let node = match a.fs.vol.resolve(cur_dir(a), &name) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    let Some(bytes) = a.fs.vol.file_data(node).map(|d| d.to_vec()) else { return fail(a, cpu, ERROR_FILE_NOT_OBJECT, 0, what) };
    match load_bytes(a, &bytes) {
        Ok(s) => ok(a, cpu, s.bptr, format!("{what} -> {:#x} ({} hunks)", s.bptr << 2, s.bases.len())),
        Err((e, why)) => fail(a, cpu, e, 0, format!("{what} ({why})")),
    }
}

/// UnLoadSeg(d1 = seglist) -> DOSTRUE. UnLoadSeg(0) is a no-op; a BPTR LoadSeg never returned is a named refusal.
pub fn unload_seg<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let b = cpu.dreg(1);
    if b != 0 && !unload(a, b)? { return Err(Refusal(format!("UnLoadSeg {:#x}: not a seglist LoadSeg returned", b << 2))); }
    cpu.set_d0(DOSTRUE);
    Ok(format!("[dos] UnLoadSeg {:#x}", b << 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;
    use heirloom_fmt_hunk::writer::{build, Hunk};

    fn two_hunks() -> Vec<u8> {
        // hunk 0: code holding a pointer to hunk 1 at offset 4 (relocated); hunk 1: data.
        build(&[Hunk::code(alloc::vec![0x4E, 0x75, 0, 0, 0, 0, 0, 0], alloc::vec![(1, 4)]), Hunk::data(alloc::vec![1, 2, 3, 4], alloc::vec![])], None)
    }

    #[test]
    fn seglist_chain_headers_and_relocation() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        let live = a.heap.live_blocks();
        let s = load_bytes(&mut a, &two_hunks()).unwrap();
        assert_eq!(s.bases.len(), 2);
        let (s0, s1) = ((s.bases[0] - 4), (s.bases[1] - 4));
        assert_eq!(s.bptr << 2, s0, "the BPTR is the first segment's address");
        assert_eq!(a.mem.read_u32(s0).unwrap(), s1 >> 2, "next link");
        assert_eq!(a.mem.read_u32(s1).unwrap(), 0, "the chain ends");
        assert_eq!(a.mem.read_u32(s0 - 4).unwrap(), 8 + 8, "length includes the header");
        assert_eq!(a.mem.read_u32(s.bases[0] + 4).unwrap(), s.bases[1], "HUNK_RELOC32 applied against the spaced base");
        assert_eq!(a.mem.read_u32(s.bases[1]).unwrap(), 0x01020304);
        assert_eq!(a.heap.live_blocks(), live + 1);
        assert!(unload(&mut a, s.bptr).unwrap());
        assert_eq!(a.heap.live_blocks(), live, "UnLoadSeg freed the region");
        assert!(!unload(&mut a, s.bptr).unwrap(), "negative control: a second unload finds nothing");
    }

    #[test]
    fn bad_files_leave_nothing_allocated_and_name_the_dos_error() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        let live = a.heap.live_blocks();
        assert_eq!(load_bytes(&mut a, b"hello world, not a hunk").unwrap_err().0, ERROR_FILE_NOT_OBJECT);
        let mut cut = two_hunks();
        cut.truncate(cut.len() - 8);
        assert_eq!(load_bytes(&mut a, &cut).unwrap_err().0, ERROR_BAD_HUNK);
        assert_eq!(a.heap.live_blocks(), live);
    }
}
