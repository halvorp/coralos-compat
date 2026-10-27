//! `UserHal` (e32hal.h): what the device reports about itself, answered from this engine's own model, nothing invented.
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// `TMemoryInfoV1` (e32hal.h:421): iTotalRamInBytes 0, iTotalRomInBytes 4, iMaxFreeRamInBytes 8, iFreeRamInBytes 12,
/// iInternalDiskRamInBytes 16, iRomIsReprogrammable 20: 24 bytes written into the guest's `TMemoryInfoV1Buf` (a `TPckgBuf`: a TBuf8).
const MEMORY_INFO_BYTES: usize = 24;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    Some(match name {
        // TInt UserHal::MemoryInfo(TDes8& aInfo): the RAM is the guest heap (its capacity, and what is not allocated); there is no RAM
        // disk and the ROM is not reprogrammable. The ROM's size is not known to the engine (the ROM is read file by file), so it is 0.
        // DELETE WHEN the ROM source reports its size (then iTotalRomInBytes).
        "_ZN7UserHal10MemoryInfoER5TDes8" => {
            let Some(h) = p.heap.as_ref() else { return Some(NativeOutcome::Refused(String::from("NEXT-NEEDED euser UserHal::MemoryInfo before heap setup"))) };
            let (total, free) = (h.capacity(), h.capacity() - h.live_bytes());
            let words = [total, 0, free, free, 0, 0];
            let bytes: alloc::vec::Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            debug_assert_eq!(bytes.len(), MEMORY_INFO_BYTES);
            let r0 = p.cpu.regs[0];
            match crate::desc::write8(p, r0, &bytes) { Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return } Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")) }
        }
        _ => return None,
    })
}
