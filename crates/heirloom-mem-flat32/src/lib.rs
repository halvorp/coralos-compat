//! Checked big-endian guest RAM. Every access is bounds-checked and returns a named
//! [`MemError`]; nothing here panics on a guest-controlled address.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
pub mod heap;
mod watch;
pub use watch::CODE_PAGE_SHIFT;
use heirloom_mem_abi::{CodeWatch, FlatRam};
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use core::fmt;
use core::alloc::Layout;

/// Default guest RAM size (16 MiB, the 24-bit 68000 address space).
pub const DEFAULT_SIZE: usize = 16 << 20;
/// Longest C string [`GuestMem::read_cstr`] will walk.
pub const MAX_CSTR: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemError {
    /// `addr..addr+len` is not inside guest RAM (includes u32 wrap).
    OutOfRange { addr: u32, len: u32 },
    /// No NUL within `max` bytes.
    Unterminated { addr: u32, max: usize },
}

impl fmt::Display for MemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemError::OutOfRange { addr, .. } => write!(f, "pointer {addr:#010x} outside guest RAM"),
            MemError::Unterminated { addr, max } => {
                write!(f, "string at {addr:#010x} unterminated within {max} bytes")
            }
        }
    }
}
impl core::error::Error for MemError {}

/// A guest address range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span { pub addr: u32, pub len: u32 }

/// The full 32-bit address space: no stripping (the default, and the Amiga's).
pub const MASK_32: u32 = 0xFFFF_FFFF;
/// The classic Mac 24-bit address space (68000 / 24-bit Memory Manager): the bus has 24 address lines, so the top
/// byte of every address is ignored (Inside Macintosh: Memory, "24-bit addressing"; apps keep flags there).
pub const MASK_24: u32 = 0x00FF_FFFF;

/// Guest RAM could not grow to `requested` bytes (the host refused the allocation); the memory is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRamExhausted { pub requested: usize }
impl fmt::Display for GuestRamExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "cannot grow guest RAM to {} bytes", self.requested) }
}
impl core::error::Error for GuestRamExhausted {}

/// Obtain `len` zero bytes from the global allocator's `alloc_zeroed` and
/// nothing else. The returned buffer's tail is never written by this crate;
/// on CoralOS, pages the guest has not touched are therefore never committed.
/// Returns `None` only when the allocator refuses or `len` has no valid layout.
fn zeroed_ram(len: usize) -> Option<Vec<u8>> {
    zeroed::<u8>(len)
}

/// `n` zeroed elements of a zero-valid plain integer type from `alloc_zeroed` (never a plain alloc, never a realloc).
fn zeroed<T: Copy>(n: usize) -> Option<Vec<T>> {
    if n == 0 {
        return Some(Vec::new());
    }
    let layout = Layout::array::<T>(n).ok()?;
    // SAFETY: `layout` is for `n` elements of `T` (only u8 / u64 here, for which all-zero bytes are a valid value).
    // `alloc_zeroed` returns either null or a valid, size-`layout.size()` block whose bytes are all zero by contract.
    let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` came from `alloc_zeroed(layout)` with the same layout; `n` elements are initialised (to zero) and
    // `n` == capacity, matching `Vec<T>`'s layout invariants so the deallocator uses the same layout.
    Some(unsafe { Vec::from_raw_parts(ptr.cast::<T>(), n, n) })
}

/// The pages a RAM of `len` bytes has, and the bitmap words that cover them.
fn watch_extent(len: usize) -> (usize, usize) {
    let pages = len.div_ceil(1 << CODE_PAGE_SHIFT);
    (pages, pages.div_ceil(64))
}

/// What the OWNER of guest RAM holds (a [`GuestMem::view`] holds none of it): the bytes, the watch arrays, and the two
/// `#[repr(C)]` records that describe them to cpu-ppc.library. Boxed, so the records' addresses never move while RAM grows.
struct Owned {
    ram: Vec<u8>,
    marked: Vec<u64>,
    gen: Vec<u64>,
    watch: CodeWatch,
    desc: FlatRam,
}

/// Checked big-endian guest RAM. The OWNER (the engine: [`GuestMem::new`]) allocates the bytes and the code watch and publishes
/// both as a [`FlatRam`] descriptor ([`GuestMem::flat_ram`]); a BORROWED VIEW of that descriptor ([`GuestMem::view`],
/// cpu-ppc.library during a run) reads and writes the same bytes and the same watch. Only the owner grows.
pub struct GuestMem {
    base: *mut u8,
    len: usize,
    mask: u32,
    hooks: watch::Hooks,
    owned: Option<alloc::boxed::Box<Owned>>,
}

// SAFETY: an owning GuestMem's raw pointers all point into its own Box<Owned> (moved with it, freed with it), so sending it to
// another thread is sending sole ownership. A view is made by cpu-ppc.library inside one run on the run's thread and never sent.
unsafe impl Send for GuestMem {}

impl GuestMem {
    pub fn new(size: usize) -> Self {
        let (pages, words) = watch_extent(size);
        let mut o = alloc::boxed::Box::new(Owned {
            ram: vec![0; size],
            marked: vec![0; words],
            gen: vec![0; pages],
            watch: CodeWatch { marked: core::ptr::null_mut(), marked_words: 0, gen_pages: 0, gen: core::ptr::null_mut(), bumps: 0, active: 0, reserved: 0 },
            desc: FlatRam { size: core::mem::size_of::<FlatRam>() as u32, flags: 0, base: core::ptr::null_mut(), len: 0, mask: MASK_32, epoch: 0, watch: core::ptr::null_mut() },
        });
        Self::publish(&mut o);
        let (base, watch) = (o.desc.base, o.desc.watch);
        GuestMem { base, len: size, mask: MASK_32, hooks: watch::Hooks::new(watch), owned: Some(o) }
    }

    /// Point the owner's records at its current arrays (after `new` or a growth).
    fn publish(o: &mut Owned) {
        o.watch.marked = o.marked.as_mut_ptr();
        o.watch.marked_words = o.marked.len() as u32;
        o.watch.gen = o.gen.as_mut_ptr();
        o.watch.gen_pages = o.gen.len() as u32;
        o.desc.base = o.ram.as_mut_ptr();
        o.desc.len = o.ram.len() as u64;
        o.desc.watch = &mut o.watch;
    }

    /// A BORROWED view of an engine's RAM: the same bytes, mask and code watch as the descriptor names; it owns nothing and
    /// cannot grow ([`GuestMem::try_grow_to`] refuses). cpu-ppc.library makes one per run and again after every host callback.
    ///
    /// # Safety
    /// `desc` must point at a live [`FlatRam`] whose `base` holds `len` readable and writable bytes and whose `watch` is a live
    /// [`CodeWatch`] with arrays of the lengths it states, and all of them must outlive the view and stay where they are while it
    /// is used (the owner grows only when no view is in use: between runs or inside a callback, after which the view is remade).
    pub unsafe fn view(desc: *const FlatRam) -> GuestMem {
        // SAFETY: the caller guarantees `desc` is a live descriptor.
        let d = unsafe { &*desc };
        GuestMem { base: d.base, len: d.len as usize, mask: d.mask, hooks: watch::Hooks::new(d.watch), owned: None }
    }

    /// The owner's descriptor of this RAM for cpu-ppc.library (stable while this GuestMem lives; its fields follow growth and
    /// [`set_addr_mask`](Self::set_addr_mask)). `None` for a view.
    pub fn flat_ram(&self) -> Option<*const FlatRam> { self.owned.as_ref().map(|o| &o.desc as *const FlatRam) }

    /// The bytes as a slice (empty RAM: an empty slice).
    fn ram(&self) -> &[u8] {
        if self.len == 0 { return &[]; }
        // SAFETY: `base` holds `len` initialised bytes (the owner's Vec, or the view's descriptor - GuestMem::view's contract).
        unsafe { core::slice::from_raw_parts(self.base, self.len) }
    }
    fn ram_mut(&mut self) -> &mut [u8] {
        if self.len == 0 { return &mut []; }
        // SAFETY: as in `ram`; `&mut self` is the only path to the bytes through this GuestMem.
        unsafe { core::slice::from_raw_parts_mut(self.base, self.len) }
    }

    /// Select the address bus width: every access (guest CPU and host handlers alike) drops the bits outside `mask`.
    pub fn set_addr_mask(&mut self, mask: u32) {
        self.mask = mask;
        if let Some(o) = self.owned.as_mut() { o.desc.mask = mask; }
    }
    pub fn addr_mask(&self) -> u32 { self.mask }
    /// `addr` as the bus sees it (StripAddress).
    pub fn strip(&self, addr: u32) -> u32 { addr & self.mask }
    pub fn len(&self) -> usize { self.len }
    /// Grow the address space to `new_len` bytes. New RAM is obtained from the
    /// global allocator's `alloc_zeroed`; only the previously-live bytes are
    /// copied into it, so on CoralOS untouched pages in the new tail are never
    /// committed. Never shrinks. Aborts on allocation failure (infallible).
    pub fn grow_to(&mut self, new_len: usize) {
        if self.try_grow_to(new_len).is_ok() {
            return;
        }
        // Mirror Vec's own failure mode: abort via handle_alloc_error so the
        // infallible contract of `grow_to` is preserved exactly.
        let layout = Layout::array::<u8>(new_len)
            .unwrap_or_else(|_| Layout::new::<u8>());
        alloc::alloc::handle_alloc_error(layout);
    }

    /// As [`GuestMem::grow_to`], but a host that cannot supply the memory is a
    /// named refusal (`GuestRamExhausted`) and nothing about `self` changes. A view cannot grow: refused the same way.
    /// The watch arrays grow first (so its coverage is never short of the RAM), then the bytes; the descriptor is republished and
    /// its `epoch` bumped only once both succeeded.
    pub fn try_grow_to(&mut self, new_len: usize) -> Result<(), GuestRamExhausted> {
        let len = self.len;
        if new_len <= len {
            return Ok(());
        }
        let refused = GuestRamExhausted { requested: new_len };
        let o = self.owned.as_mut().ok_or(refused)?;
        let (pages, words) = watch_extent(new_len);
        let mut gen = zeroed::<u64>(pages).ok_or(refused)?;
        let mut marked = zeroed::<u64>(words).ok_or(refused)?;
        let mut ram = zeroed_ram(new_len).ok_or(refused)?;
        gen[..o.gen.len()].copy_from_slice(&o.gen);
        marked[..o.marked.len()].copy_from_slice(&o.marked);
        ram[..len].copy_from_slice(&o.ram); // only old bytes are written
        (o.ram, o.gen, o.marked) = (ram, gen, marked); // old Vecs dropped; tail never touched
        Self::publish(o);
        o.desc.epoch = o.desc.epoch.wrapping_add(1);
        (self.base, self.len) = (o.desc.base, new_len);
        Ok(())
    }
    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn contains(&self, addr: u32, len: u32) -> bool {
        let addr = self.strip(addr);
        (addr as u64) + (len as u64) <= self.len as u64
    }
    pub fn check(&self, addr: u32, len: u32) -> Result<(), MemError> {
        if self.contains(addr, len) { Ok(()) } else { Err(MemError::OutOfRange { addr, len }) }
    }
    pub fn span(&self, addr: u32, len: u32) -> Result<Span, MemError> {
        self.check(addr, len).map(|_| Span { addr: self.strip(addr), len })
    }
    pub fn bytes(&self, s: Span) -> Result<&[u8], MemError> {
        self.check(s.addr, s.len)?;
        let a = self.strip(s.addr);
        Ok(&self.ram()[a as usize..(a + s.len) as usize])
    }
    pub fn write_bytes(&mut self, addr: u32, src: &[u8]) -> Result<(), MemError> {
        let len = u32::try_from(src.len()).map_err(|_| MemError::OutOfRange { addr, len: u32::MAX })?;
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.before_write(addr, len);
        self.ram_mut()[addr as usize..addr as usize + src.len()].copy_from_slice(src);
        Ok(())
    }
    pub fn zero(&mut self, addr: u32, len: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.before_write(addr, len);
        self.ram_mut()[addr as usize..(addr + len) as usize].fill(0);
        Ok(())
    }

    #[inline(always)]
    fn before_write(&mut self, addr: u32, len: u32) {
        // SAFETY: as in `ram`: `base` holds `len` initialised bytes; the hooks only read them (the journal's old bytes).
        let ram = if self.len == 0 { &[][..] } else { unsafe { core::slice::from_raw_parts(self.base, self.len) } };
        self.hooks.before_write(ram, addr, len);
    }

    fn read_be(&self, addr: u32, len: u32) -> Result<u32, MemError> {
        self.check(addr, len)?;
        let a = self.strip(addr) as usize;
        Ok(self.ram()[a..a + len as usize].iter().fold(0u32, |v, b| (v << 8) | *b as u32))
    }
    fn write_be(&mut self, addr: u32, len: u32, val: u32) -> Result<(), MemError> {
        self.check(addr, len)?;
        let addr = self.strip(addr);
        self.before_write(addr, len);
        let ram = self.ram_mut();
        for i in 0..len {
            ram[(addr + i) as usize] = (val >> (8 * (len - 1 - i))) as u8;
        }
        Ok(())
    }
    pub fn read_u8(&self, addr: u32) -> Result<u8, MemError> { self.read_be(addr, 1).map(|v| v as u8) }
    pub fn read_u16(&self, addr: u32) -> Result<u16, MemError> { self.read_be(addr, 2).map(|v| v as u16) }
    pub fn read_u32(&self, addr: u32) -> Result<u32, MemError> { self.read_be(addr, 4) }
    pub fn read_i16(&self, addr: u32) -> Result<i16, MemError> { self.read_u16(addr).map(|v| v as i16) }
    pub fn write_u8(&mut self, addr: u32, v: u8) -> Result<(), MemError> { self.write_be(addr, 1, v as u32) }
    pub fn write_u16(&mut self, addr: u32, v: u16) -> Result<(), MemError> { self.write_be(addr, 2, v as u32) }
    pub fn write_u32(&mut self, addr: u32, v: u32) -> Result<(), MemError> { self.write_be(addr, 4, v) }

    /// Read a NUL-terminated string (lossy UTF-8, at most [`MAX_CSTR`] bytes).
    pub fn read_cstr(&self, addr: u32) -> Result<String, MemError> {
        let mut s = Vec::new();
        for i in 0..MAX_CSTR as u32 {
            let a = addr.checked_add(i).ok_or(MemError::OutOfRange { addr, len: 1 })?;
            let b = self.read_u8(a).map_err(|_| MemError::OutOfRange { addr, len: 1 })?;
            if b == 0 { return Ok(String::from_utf8_lossy(&s).into_owned()); }
            s.push(b);
        }
        Err(MemError::Unterminated { addr, max: MAX_CSTR })
    }

    /// Like [`read_cstr`](Self::read_cstr) for TEXT a program owns (a label, a long About text, a Text object's contents): no 256-byte limit, the
    /// only bound is the end of guest RAM, so a string with no NUL before it is `Unterminated { max: <bytes to the end> }`, a named refusal.
    pub fn read_cstr_long(&self, addr: u32) -> Result<String, MemError> {
        let mut s = Vec::new();
        let mut a = addr;
        loop {
            let b = self.read_u8(a).map_err(|_| if s.is_empty() { MemError::OutOfRange { addr, len: 1 } } else { MemError::Unterminated { addr, max: s.len() } })?;
            if b == 0 { return Ok(String::from_utf8_lossy(&s).into_owned()); }
            s.push(b);
            a = a.checked_add(1).ok_or(MemError::Unterminated { addr, max: s.len() })?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_string_reads_whole_with_read_cstr_long_and_a_runaway_is_a_named_error() {
        let mut m = GuestMem::new(1024);
        let text = "x".repeat(700);
        m.write_bytes(10, text.as_bytes()).unwrap();
        m.write_u8(710, 0).unwrap();
        assert_eq!(m.read_cstr_long(10).unwrap(), text, "more than MAX_CSTR bytes");
        assert_eq!(m.read_cstr(10), Err(MemError::Unterminated { addr: 10, max: MAX_CSTR }), "the bounded read keeps its limit");
        m.write_bytes(900, &[b'y'; 124]).unwrap();
        assert_eq!(m.read_cstr_long(900), Err(MemError::Unterminated { addr: 900, max: 124 }), "NEGATIVE CONTROL: no NUL before the end of RAM");
    }

    /// CPU ABI v3: a view of the owner's descriptor is the SAME bytes and the SAME code watch; growth republishes the descriptor.
    #[test]
    fn a_view_shares_bytes_and_watch_with_its_owner_and_growth_republishes_the_descriptor() {
        let mut m = GuestMem::new(0x3000);
        let d = m.flat_ram().expect("the owner publishes a descriptor");
        // SAFETY: `d` is m's live descriptor and m is not grown while `v` is used.
        let mut v = unsafe { GuestMem::view(d) };
        assert!(v.flat_ram().is_none(), "a view owns nothing");
        v.write_u32(0x10, 0xA1B2_C3D4).unwrap();
        assert_eq!(m.read_u32(0x10), Ok(0xA1B2_C3D4), "the view's store is the owner's byte");
        let g = v.watch_code_page(0x1000);
        m.write_u8(0x1004, 9).unwrap();
        assert_eq!(v.code_gen(0x1000), g + 1, "an OWNER write invalidates code the VIEW decoded");
        assert_eq!(v.try_grow_to(0x10000).map_err(|e| e.requested), Err(0x10000), "NEGATIVE CONTROL: a view cannot grow");
        drop(v);
        // SAFETY: `d` is m's live descriptor.
        let (base0, epoch0) = unsafe { ((*d).base, (*d).epoch) };
        m.try_grow_to(0x40_0000).unwrap();
        // SAFETY: as above; the descriptor's address is stable (boxed) across growth.
        let (base1, len1, epoch1, pages) = unsafe { ((*d).base, (*d).len, (*d).epoch, (*(*d).watch).gen_pages) };
        assert_eq!((len1, epoch1, pages), (0x40_0000, epoch0 + 1, 0x400), "len, epoch and a watch covering every page follow the growth");
        assert_ne!(base1, base0, "the bytes moved (a library must reload base after a callback)");
        // SAFETY: as above.
        let mut v = unsafe { GuestMem::view(d) };
        assert_eq!(v.read_u32(0x10), Ok(0xA1B2_C3D4), "old bytes survive the move");
        let g2 = v.watch_code_page(0x3F_F000);
        v.write_u8(0x3F_F000, 1).unwrap();
        assert_eq!(m.code_gen(0x3F_F000), g2 + 1, "the new tail is watched too");
    }

    #[test]
    fn big_endian_round_trip() {
        let mut m = GuestMem::new(64);
        m.write_u32(4, 0x1122_3344).unwrap();
        assert_eq!(m.bytes(m.span(4, 4).unwrap()).unwrap(), &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(m.read_u16(4).unwrap(), 0x1122);
        assert_eq!(m.read_i16(6).unwrap(), 0x3344);
    }

    #[test]
    fn out_of_range_and_wrap_are_refused() {
        let mut m = GuestMem::new(64);
        assert_eq!(m.read_u32(62), Err(MemError::OutOfRange { addr: 62, len: 4 }));
        assert!(m.write_u8(64, 1).is_err());
        assert!(m.span(u32::MAX, 2).is_err(), "u32 wrap must not alias low RAM");
        assert!(m.read_cstr(0xFFFF_FFF0).is_err());
        assert_eq!(m.read_u8(63), Ok(0));
    }

    #[test]
    fn cstr_terminated_unterminated_and_edge() {
        let mut m = GuestMem::new(1024);
        m.write_bytes(10, b"Hello\0").unwrap();
        assert_eq!(m.read_cstr(10).unwrap(), "Hello");
        m.write_bytes(100, &[b'a'; 300]).unwrap();
        assert_eq!(m.read_cstr(100), Err(MemError::Unterminated { addr: 100, max: MAX_CSTR }));
        // string running off the end of RAM is an out-of-range refusal, not a panic
        let mut small = GuestMem::new(8);
        small.write_bytes(4, b"abcd").unwrap();
        assert!(matches!(small.read_cstr(4), Err(MemError::OutOfRange { .. })));
    }

    /// 24-bit bus: the top byte is ignored by loads, stores, spans and strings; 32-bit leaves it significant.
    #[test]
    fn a_24_bit_bus_ignores_the_top_byte_and_a_32_bit_bus_does_not() {
        let mut m = GuestMem::new(0x100);
        m.write_u32(0x10, 0xCAFE_F00D).unwrap();
        assert!(m.read_u32(0x8000_0010).is_err(), "32-bit: 0x80000010 is outside RAM");
        m.set_addr_mask(MASK_24);
        assert_eq!(m.read_u32(0x8000_0010), Ok(0xCAFE_F00D), "24-bit: the flag byte is ignored");
        m.write_u8(0xFF00_0020, 7).unwrap();
        assert_eq!(m.read_u8(0x20), Ok(7));
        assert_eq!(m.span(0x4000_0010, 4), Ok(Span { addr: 0x10, len: 4 }));
        assert_eq!(m.strip(0x8020_A9FA), 0x20_A9FA);
        assert!(m.read_u8(0x0000_0100).is_err(), "past RAM is still a refusal");
        m.set_addr_mask(MASK_32);
        assert!(m.write_u8(0xFF00_0020, 7).is_err());
    }

    #[test]
    fn refusal_text_is_stable() {
        let e = GuestMem::new(8).read_u32(0xFFFF_0000).unwrap_err();
        assert_eq!(e.to_string(), "pointer 0xffff0000 outside guest RAM");
    }

    #[test]
    fn an_impossible_growth_is_refused_and_changes_nothing() {
        let mut m = GuestMem::new(4096);
        assert_eq!(m.try_grow_to(usize::MAX / 2), Err(GuestRamExhausted { requested: usize::MAX / 2 }));
        assert_eq!(m.len(), 4096);
        assert_eq!(m.try_grow_to(8192), Ok(()));
        assert_eq!(m.len(), 8192);
    }
}
