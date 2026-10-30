//! The memory layer's C-ABI records: the guest RAM an ENGINE allocates and publishes (heirloom-mem-flat32's `GuestMem` owns the
//! bytes and the watch arrays) and a CPU library borrows across its table (heirloom-cpu-abi::ppc, CPU ABI v3). The engine is the
//! only allocator: it sizes the watch to cover every page before publishing, grows both only between runs or inside a host
//! callback, and bumps `FlatRam::epoch` when it moves them. A borrower may flip watch bits and bump generations in place, never
//! reallocate. Plain `#[repr(C)]` data: no code, no dependencies, so the CPU layer may import it and the memory layer owns it.
#![no_std]

/// log2 of a watched code page (4 KiB, the PowerPC page).
pub const CODE_PAGE_SHIFT: u32 = 12;

/// The code watch over [`FlatRam`]: per 4 KiB page a generation and a "code was decoded from here" bit (heirloom-mem-flat32's watch).
/// ENGINE-ALLOCATED and sized to cover every page of RAM (`gen_pages >= ceil(len / 4096)`, `marked_words >= ceil(gen_pages / 64)`);
/// the library writes elements in place and never resizes.
#[repr(C)]
#[derive(Debug)]
pub struct CodeWatch {
    pub marked: *mut u64,
    pub marked_words: u32,
    pub gen_pages: u32,
    pub gen: *mut u64,
    /// Generation bumps so far (all pages): unchanged = no watched page was written.
    pub bumps: u64,
    /// 1 when any page is marked (the only thing a write tests when nothing is).
    pub active: u32,
    pub reserved: u32,
}

/// The engine's guest RAM: `len` big-endian bytes at `base`; every access drops the address bits outside `mask` first.
#[repr(C)]
#[derive(Debug)]
pub struct FlatRam {
    /// `size_of::<FlatRam>()`.
    pub size: u32,
    pub flags: u32,
    pub base: *mut u8,
    pub len: u64,
    pub mask: u32,
    /// Bumped by the engine each time it moves `base` or the watch arrays (growth).
    pub epoch: u32,
    pub watch: *mut CodeWatch,
}

