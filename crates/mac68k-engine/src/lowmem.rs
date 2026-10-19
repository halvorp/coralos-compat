//! The low-memory globals the engine owns: fixed addresses below 0x1000 that a classic Mac program (and the startup
//! code the linker puts in front of it) reads and writes by absolute address.
//!
//! Addresses and sizes are interface facts, read from metamage_1's `68k/libs/v68k-lowmem/lowmem/lowmem.cc` table (its
//! `globals[]` array) and cross-checked against executor's `docs/subsystems/low-memory-globals.md` for the six it
//! lists; no code from either is used. [`MAP`] is the complete list of what [`init`] may write: the test
//! `init_writes_only_mapped_globals` fails if the engine ever touches a byte outside it.
use crate::machine::{HEAP_END, STACK_TOP};
use retro_guest_mem::DEFAULT_SIZE;
use crate::Refusal;
use retro_guest_mem::GuestMem;

pub const SCREEN_ROW: u32 = 0x106; // short: rowBytes of the main screen
pub const MEM_TOP: u32 = 0x108; // Ptr: top of RAM
pub const BUF_PTR: u32 = 0x10C; // Ptr
pub const HEAP_END_G: u32 = 0x114; // Ptr
pub const THE_ZONE: u32 = 0x118; // THz: the current heap zone
pub const APPL_LIMIT: u32 = 0x130; // Ptr: heap growth limit; StackSpace = SP - ApplLimit
pub const TIME: u32 = 0x20C; // long: seconds since 1904-01-01 (the clock)
pub const TICKS: u32 = 0x16A; // long: 60 Hz tick count
pub const MB_STATE: u32 = 0x172; // byte: mouse button state, bit 7 SET = button up
pub const LO_3_BYTES: u32 = 0x31A; // long: $00FFFFFF, the mask 24-bit code applies to a pointer or block size
pub const MEM_ERR: u32 = 0x220; // short: the last Memory Manager result
pub const SYS_ZONE: u32 = 0x2A6; // THz
pub const APPL_ZONE: u32 = 0x2AA; // THz
pub const SCRN_BASE: u32 = 0x824; // Ptr: main screen bitmap
pub const RAW_MOUSE: u32 = 0x82C; // Point: v, h (global)
pub const MOUSE: u32 = 0x830; // Point: v, h (global), the processed mouse
pub const CRSR_PIN: u32 = 0x834; // Rect: the cursor's bound = screen bounds
pub const CUR_AP_REF_NUM: u32 = 0x900; // short: the application's resource-file refNum
pub const CURRENT_A5: u32 = 0x904; // Ptr: the application's A5
pub const CUR_STACK_BASE: u32 = 0x908; // Ptr: top of the application's stack
pub const CUR_AP_NAME: u32 = 0x910; // Str31 (the table gives 32 bytes)
pub const CUR_JT_OFFSET: u32 = 0x934; // short: the jump table's offset from A5
pub const WINDOW_LIST: u32 = 0x9D6; // WindowPeek: front window
pub const MENU_LIST: u32 = 0xA1C; // Handle: the menu bar's list
pub const RES_LOAD: u32 = 0xA5E; // short: SetResLoad flag (the high byte is the Boolean)
pub const RES_ERR: u32 = 0xA60; // short: the last Resource Manager result
pub const MBAR_HEIGHT: u32 = 0xBAA; // short
pub const MMU_32_BIT: u32 = 0xCB2; // byte: the Memory Manager's mode, true (1) = 32-bit addressing (Inside Macintosh: Memory)

/// One entry of the map: address, size in bytes, name.
pub type Global = (u32, u32, &'static str);

/// Every global the engine initialises or maintains, ascending and non-overlapping.
pub static MAP: &[Global] = &[
    (SCREEN_ROW, 2, "ScreenRow"), (MEM_TOP, 4, "MemTop"), (BUF_PTR, 4, "BufPtr"), (HEAP_END_G, 4, "HeapEnd"),
    (THE_ZONE, 4, "TheZone"), (APPL_LIMIT, 4, "ApplLimit"), (TICKS, 4, "Ticks"), (MB_STATE, 1, "MBState"), (TIME, 4, "Time"), (MEM_ERR, 2, "MemErr"),
    (SYS_ZONE, 4, "SysZone"), (APPL_ZONE, 4, "ApplZone"), (LO_3_BYTES, 4, "Lo3Bytes"), (SCRN_BASE, 4, "ScrnBase"), (RAW_MOUSE, 4, "RawMouse"), (MOUSE, 4, "Mouse"), (CRSR_PIN, 8, "CrsrPin"),
    (CUR_AP_REF_NUM, 2, "CurApRefNum"), (CURRENT_A5, 4, "CurrentA5"), (CUR_STACK_BASE, 4, "CurStackBase"),
    (CUR_AP_NAME, 32, "CurApName"), (CUR_JT_OFFSET, 2, "CurJTOffset"), (WINDOW_LIST, 4, "WindowList"),
    (MENU_LIST, 4, "MenuList"), (RES_LOAD, 2, "ResLoad"), (RES_ERR, 2, "ResErr"), (MBAR_HEIGHT, 2, "MBarHeight"), (MMU_32_BIT, 1, "MMU32Bit"),
];

/// The emulated screen (a 1-bit Mac Plus-sized display: what a 1984-era application expects to find), until a real
/// display size is bridged from the host.
pub const SCREEN_W: i16 = 512;
pub const SCREEN_H: i16 = 342;
pub const MENU_BAR_HEIGHT: i16 = 20;
/// ApplLimit: the engine's stack lies between here and `STACK_TOP` (the heap is above the stack, the reverse of a real
/// Mac, so StackSpace = SP - ApplLimit is the stack headroom).
pub const APPL_LIMIT_ADDR: u32 = 0x2_0000;
/// Where the 52-byte stub Zone record (plus one 8-byte free-block header after it) lives (inside low RAM, below `CODE_BASE`).
pub const ZONE_STUB: u32 = 0xC00;
/// The screen's pretend bitmap address (the engine draws nothing to it: QuickDraw output is a later step).
pub const SCREEN_BASE: u32 = DEFAULT_SIZE as u32 - 0x5580; // 342 rows x 64 bytes, at the very top of RAM; the heap ends below it

/// Initialise the globals a program expects at launch.
pub fn init(mem: &mut GuestMem) -> Result<(), Refusal> {
    let e = Refusal::mem("low-memory globals");
    mem.write_u16(SCREEN_ROW, 64).map_err(&e)?; // 512 pixels / 8
    mem.write_u32(MEM_TOP, DEFAULT_SIZE as u32).map_err(&e)?;
    mem.write_u32(BUF_PTR, SCREEN_BASE).map_err(&e)?;
    mem.write_u32(HEAP_END_G, HEAP_END).map_err(&e)?;
    for z in [THE_ZONE, SYS_ZONE, APPL_ZONE] { mem.write_u32(z, ZONE_STUB).map_err(&e)?; }
    // The stub Zone: bkLim = the end of the walkable block list, moreMast = 64 (the master-pointer block size). A program that
    // WALKS its heap (`zone + 52`, block after block by the 24-bit size in each header until it reaches bkLim: MacWrite's
    // free-space count) finds ONE free block, the whole application heap. The zone's individual allocations are not
    // represented as blocks in guest memory (the Memory Manager keeps them host-side), so the walk sees no detail.
    // Every other field is zero: this is a name for ApplZone to point at, not a zone the guest may modify.
    mem.zero(ZONE_STUB, 52 + 8).map_err(&e)?;
    let heap_bytes = HEAP_END - crate::machine::HEAP_START;
    mem.write_u32(ZONE_STUB, ZONE_STUB + 52 + heap_bytes).map_err(&e)?;
    mem.write_u16(ZONE_STUB + 20, 64).map_err(&e)?;
    mem.write_u32(ZONE_STUB + 52, heap_bytes).map_err(&e)?; // block header: tag 0 (free), 24-bit size
    mem.write_u32(APPL_LIMIT, APPL_LIMIT_ADDR).map_err(&e)?;
    mem.write_u32(CUR_STACK_BASE, STACK_TOP).map_err(&e)?;
    mem.write_u32(SCRN_BASE, SCREEN_BASE).map_err(&e)?;
    for (i, v) in [0, 0, SCREEN_H, SCREEN_W].into_iter().enumerate() { mem.write_u16(CRSR_PIN + 2 * i as u32, v as u16).map_err(&e)?; }
    mem.write_u8(MB_STATE, 0x80).map_err(&e)?; // button up
    mem.write_u32(TIME, crate::clock::DEFAULT_BOOT_SECS).map_err(&e)?;
    mem.write_u32(LO_3_BYTES, 0x00FF_FFFF).map_err(&e)?;
    mem.write_u16(RES_LOAD, 0x0100).map_err(&e)?; // ResLoad = TRUE (Boolean in the high byte)
    mem.write_u16(MBAR_HEIGHT, MENU_BAR_HEIGHT as u16).map_err(&e)
}

/// Name `app` in CurApName (a Str31) and record the launch state (`launch` calls this).
pub fn set_app(mem: &mut GuestMem, name: &str, a5: u32, jt_offset: u16, ref_num: i16) -> Result<(), Refusal> {
    let e = Refusal::mem("low-memory globals");
    let n = name.len().min(31);
    mem.write_u8(CUR_AP_NAME, n as u8).map_err(&e)?;
    mem.write_bytes(CUR_AP_NAME + 1, &name.as_bytes()[..n]).map_err(&e)?;
    mem.write_u32(CURRENT_A5, a5).map_err(&e)?;
    mem.write_u16(CUR_JT_OFFSET, jt_offset).map_err(&e)?;
    mem.write_u16(CUR_AP_REF_NUM, ref_num as u16).map_err(&e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_is_ascending_and_disjoint() {
        for w in MAP.windows(2) { assert!(w[0].0 + w[0].1 <= w[1].0, "{:?} overlaps {:?}", w[0], w[1]); }
    }

    #[test]
    fn init_writes_only_mapped_globals() {
        let mut m = GuestMem::new(0x2000);
        init(&mut m).unwrap();
        set_app(&mut m, "Test", 0x44444, 32, 2).unwrap();
        for a in 0..0x1000u32 {
            if m.read_u8(a).unwrap() != 0 {
                assert!(MAP.iter().any(|&(g, n, _)| (g..g + n).contains(&a)) || (ZONE_STUB..ZONE_STUB + 60).contains(&a),
                    "byte {a:#x} written outside the documented map");
            }
        }
        // spot values: the facts a startup sequence reads
        assert_eq!(m.read_u32(CURRENT_A5).unwrap(), 0x44444);
        assert_eq!(m.read_u16(CUR_JT_OFFSET).unwrap(), 32);
        assert_eq!(m.read_u32(APPL_ZONE).unwrap(), ZONE_STUB);
        assert_eq!(m.read_u16(MBAR_HEIGHT).unwrap(), 20);
        assert_eq!(m.read_u16(CRSR_PIN + 6).unwrap(), 512);
        assert_eq!(m.read_u8(CUR_AP_NAME).unwrap(), 4);
        // NEGATIVE control for the scan itself: a stray write is detected by the same scan
        m.write_u8(0x500, 1).unwrap();
        assert!(!MAP.iter().any(|&(g, n, _)| (g..g + n).contains(&0x500)), "0x500 (the exit stub) is not a documented global");
    }
}
