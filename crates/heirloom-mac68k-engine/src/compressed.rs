//! Compressed resources, the way System 7's Resource Manager reads them (`resCompressed`, resource attribute bit 0).
//!
//! A compressed resource's data starts with an 18-byte header, then the compressed bytes:
//!
//! ```text
//! 0  u32  signature 0xA89F6572
//! 4  u16  header length (0x12)
//! 6  u8   header version (8 or 9)      7  u8  attributes (bit 0: the data is compressed)
//! 8  u32  decompressed size
//! version 8:  12 u8 working-buffer fractional size   13 u8 expansion buffer size   14 i16 'dcmp' id   16 u16 reserved
//! version 9:  12 i16 'dcmp' id   14 u32 decompressor-specific parameters
//! ```
//!
//! The decompressor is NOT the Resource Manager's: it is the 68k code of the `'dcmp'` resource the header names (an
//! application ships its own, or the System file's set supplies it). The engine runs it IN THE GUEST (this module only lays
//! out the call; there is no host-side reimplementation of Apple's compression). A resource whose `'dcmp'` is missing, or
//! whose header is not understood, is a named refusal with `CantDecompress` (-186) in `ResErr`. Two interfaces exist, told
//! apart by the header version (what the corpus applications' own `'dcmp'` resources show):
//!
//! * version 8, the classic one (`'dcmp'` 0 and 1): the entry is the first byte of the resource (a `bra.w` over a `'dcmp'`
//!   type tag), a Pascal procedure `dcmp(src, dst, working: Ptr; len: LongInt)`: `src` points just past the header, `len` is
//!   the compressed length, `dst` has room for the decompressed size, `working` is scratch the Resource Manager supplies;
//!   the callee pops the 16 argument bytes.
//! * version 9, the dispatch-table one (`'dcmp'` 2 and 3): the resource begins with three offsets `[init, decompress, exit]`
//!   (words, from the start of the resource) and the decompress routine is a Pascal FUNCTION
//!   `dcmp(src, dst: Ptr; header: Ptr): OSErr`, `header` pointing at the compressed-resource header itself (the routine reads
//!   the decompressed size at +8 and its own parameters at +14..+17). `init` and `exit` each take one longword; the two
//!   corpus implementations either do nothing or park two registers in scratch, and are NOT called here.
//!
//! Facts: the header and the `dcmp` call from the Executor Resource Manager (MIT, `src/res/resOpen.cpp`), the version 8 / 9
//! field offsets read off the headers of the corpus applications, the argument order from the stack accesses of their own
//! `'dcmp'` code (`link a6,#0` ... `movea.l 20(a6)` = src, `16(a6)` = dst, `12(a6)` = working, `8(a6)` = len), and the
//! `resCompressed` bit from the Advanced Mac Substitute Resource Manager (`ams-rsrc`).
//!
//! The call cannot be made from inside the trap handler (the one CPU is running the trap), so a load that needs it is
//! DEFERRED: the handle is allocated and cached, [`Mac::load_entry`] answers [`PENDING`], the A-line bridge
//! discards the trap's outcome, runs the decompressor through the completion trampoline and then executes the SAME trap word
//! again; the second run finds the finished handle in the cache. The registers and condition codes the trap was entered with
//! are restored after the decompressor, so the retried trap sees exactly what the first run saw.
use crate::guestcall::TRAMPOLINE;
use crate::machine::Mac;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// `resCompressed`: the resource attribute bit that says the data is compressed.
pub const RES_COMPRESSED: u8 = 0x01;
pub const SIGNATURE: u32 = 0xA89F_6572;
/// `CantDecompress` (Resources.h): the Resource Manager could not expand the resource.
pub const CANT_DECOMPRESS: i16 = -186;
/// The private answer of `load_entry` for "the decompressor has been scheduled; abandon this run of the trap".
pub const PENDING: i16 = i16::MIN;
const HEADER_LEN: usize = 0x12;

/// The parts of a compressed-resource header the Resource Manager needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub size: u32,
    pub dcmp_id: i16,
    pub header_len: usize,
}

/// `Ok(None)`: the data does not carry the signature, so it is an ordinary resource whose bit 0 means nothing (the System
/// treats it the same way). `Err`: a compressed header this engine will not guess at (named).
pub fn parse(data: &[u8]) -> Result<Option<Header>, String> {
    if data.len() < 4 || u32::from_be_bytes([data[0], data[1], data[2], data[3]]) != SIGNATURE { return Ok(None); }
    if data.len() < HEADER_LEN { return Err(format!("compressed resource: {} bytes is shorter than its {HEADER_LEN}-byte header", data.len())); }
    let be16 = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);
    let header_len = usize::from(be16(4));
    if header_len < HEADER_LEN || header_len > data.len() { return Err(format!("compressed resource: header length {header_len:#x} is outside {HEADER_LEN:#x}..{:#x}", data.len())); }
    let version = data[6];
    let dcmp_id = match version { 8 => be16(14), 9 => be16(12), v => return Err(format!("compressed resource: header version {v} is not one of 8, 9")) } as i16;
    let size = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    Ok(Some(Header { version, size, dcmp_id, header_len }))
}

/// How the decompressor is entered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// `dcmp(src, dst, working, len)`.
    Classic { working: u32, len: u32 },
    /// `FUNCTION dcmp(src, dst, header): OSErr`.
    Table { header: u32 },
}

/// One decompression waiting for the A-line bridge to run it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Decomp {
    /// The guest address of the routine to enter.
    pub code: u32,
    pub src: u32,
    pub dst: u32,
    pub entry: Entry,
    /// The block holding the compressed bytes (freed afterwards).
    pub block: u32,
}

/// What `leave_guest` does when the decompressor returns to the trampoline: put the trap's registers back, free the scratch.
#[derive(Debug, Clone)]
pub(crate) struct DecompDone { pub regs: [u32; 15], pub sr: u32, pub block: u32, pub working: Option<u32>, pub has_result: bool }

impl<U: GuestUi> Mac<U> {
    /// Allocate everything a decompression needs and queue it. `dst_handle` is the (already cached) destination; `Err` is a
    /// `ResErr` code with the reason noted as a refusal.
    pub(crate) fn schedule_decompress(&mut self, data: &[u8], h: &Header, dst_handle: u32) -> Result<(), i16> {
        let dcmp = self.get_resource(*b"dcmp", h.dcmp_id, false).map_err(|_| self.cant(format!("the 'dcmp' {} it names is in neither the application's resources nor the system set", h.dcmp_id)))?;
        let (base, dsize) = self.mm.block(dcmp).map(|b| (b.ptr, b.size)).ok_or(CANT_DECOMPRESS)?;
        if let Some(b) = self.mm.block_mut(dcmp) { b.locked = true; }
        let _ = self.mm.sync_master(&mut self.mem, dcmp);
        let table = h.version == 9;
        let code = if table {
            let off = u32::from(self.mem.read_u16(base + 2).map_err(|_| CANT_DECOMPRESS)?);
            if dsize < 6 || off % 2 != 0 || off < 6 || off >= dsize { return Err(self.cant(format!("'dcmp' {} has no usable dispatch table (decompress offset {off:#x} of {dsize} bytes)", h.dcmp_id))); }
            base + off
        } else { base };
        let dst = self.mm.block(dst_handle).map(|b| b.ptr).ok_or(CANT_DECOMPRESS)?;
        let len = (data.len() - h.header_len) as u32;
        let block = self.mm.new_ptr(&mut self.mem, &mut self.heap, data.len() as u32, false).map_err(|_| self.cant("no memory for the compressed bytes".into()))?;
        self.mem.write_bytes(block, data).map_err(|_| CANT_DECOMPRESS)?;
        let src = block + h.header_len as u32;
        let entry = if table { Entry::Table { header: block } } else {
            // Scratch for the decompressor (the header's fractional size is a hint the Executor authors found too small; a
            // generous fixed rule costs host memory only while the call runs).
            let wlen = h.size.max(len).saturating_add(len).saturating_add(0x1000);
            let Ok(working) = self.mm.new_ptr(&mut self.mem, &mut self.heap, wlen, true) else {
                let _ = self.mm.dispose_ptr(&mut self.heap, block);
                return Err(self.cant("no memory for the decompressor's working buffer".into()));
            };
            Entry::Classic { working, len }
        };
        self.dcmp_pending = Some(Decomp { code, src, dst, entry, block });
        Ok(())
    }

    fn cant(&mut self, why: String) -> i16 {
        self.note_refusal("Resource Manager", format!("cannot decompress: {why} (ResErr {CANT_DECOMPRESS})"));
        CANT_DECOMPRESS
    }

    /// Enter the queued decompressor. The trap word at `retry` runs again when it returns.
    pub(crate) fn enter_decompress(&mut self, cpu: &mut Cpu, retry: u32, d: Decomp) {
        cpu.cancel_stop(); // the abandoned run may have halted / blocked the guest on the missing resource (LoadSeg does)
        let mut regs = [0u32; 15];
        for (i, r) in regs.iter_mut().enumerate() { *r = if i < 8 { cpu.dreg(i as u32) } else { cpu.areg(i as u32 - 8) }; }
        let mut sp = cpu.areg(7);
        let (words, working, has_result): (alloc::vec::Vec<u32>, Option<u32>, bool) = match d.entry {
            Entry::Classic { working, len } => (alloc::vec![d.src, d.dst, working, len], Some(working), false),
            Entry::Table { header } => (alloc::vec![d.src, d.dst, header], None, true),
        };
        sp = sp.wrapping_sub(4);
        let _ = self.mem.write_u32(sp, retry);
        if has_result { sp = sp.wrapping_sub(2); let _ = self.mem.write_u16(sp, 0); } // the FUNCTION's result slot, pushed first
        for v in words.into_iter().chain([TRAMPOLINE]) { sp = sp.wrapping_sub(4); let _ = self.mem.write_u32(sp, v); }
        cpu.set_areg(7, sp);
        self.call_stack.push(crate::guestcall::Frame::Decomp(DecompDone { regs, sr: cpu.sr(), block: d.block, working, has_result }));
        self.log.push(format!("[res] decompress: 'dcmp' routine at {:#x} ({}), {:#x} -> {:#x}; the trap runs again afterwards", d.code, if has_result { "table interface" } else { "classic interface" }, d.src, d.dst));
        cpu.set_pc(d.code);
    }

    /// The decompressor came back: take its result, restore the trap's registers and free the scratch.
    pub(crate) fn finish_decompress(&mut self, cpu: &mut Cpu, done: DecompDone) {
        if done.has_result {
            let sp = cpu.areg(7);
            let err = self.mem.read_u16(sp).unwrap_or(0) as i16;
            cpu.set_areg(7, sp.wrapping_add(2));
            if err != 0 { self.note_refusal("Resource Manager", format!("the 'dcmp' returned error {err}: the resource's bytes are not trustworthy")); }
        }
        for (i, v) in done.regs.iter().enumerate() { if i < 8 { cpu.set_dreg(i as u32, *v) } else { cpu.set_areg(i as u32 - 8, *v) } }
        cpu.set_sr(done.sr);
        let _ = self.mm.dispose_ptr(&mut self.heap, done.block);
        if let Some(w) = done.working { let _ = self.mm.dispose_ptr(&mut self.heap, w); }
    }
}
