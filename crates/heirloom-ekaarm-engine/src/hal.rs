//! `UserHal` (e32hal.h): what the device reports about itself, answered from this engine's own model, nothing invented.
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// `TMemoryInfoV1` (e32hal.h:421): iTotalRamInBytes 0, iTotalRomInBytes 4, iMaxFreeRamInBytes 8, iFreeRamInBytes 12,
/// iInternalDiskRamInBytes 16, iRomIsReprogrammable 20: 24 bytes written into the guest's `TMemoryInfoV1Buf` (a `TPckgBuf`: a TBuf8).
const MEMORY_INFO_BYTES: usize = 24;

const KERR_NOT_SUPPORTED: u32 = -5i32 as u32;

fn hal_attr_value(p: &Process, attr: u32) -> Option<i32> {
    let (sw, sh) = p.screen_size();
    let (total, free) = p.heap.as_ref().map(|h| (h.capacity() as i32, (h.capacity() - h.live_bytes()) as i32)).unwrap_or((64 * 1024 * 1024, 48 * 1024 * 1024));
    match attr {
        5 => Some(0x2000_2d7f),               // EMachineUid (S60 5th Ed device)
        11 => Some(434_000),                  // ECPUSpeed (kHz)
        14 => Some(15_625),                   // ESystemTickPeriod (us, 64 Hz)
        15 => Some(total),                    // EMemoryRAM
        16 => Some(free),                     // EMemoryRAMFree
        31 => Some(sw),                       // EDisplayXPixels
        32 => Some(sh),                       // EDisplayYPixels
        33 => Some(2235),                     // EDisplayXTwips
        34 => Some(3973),                     // EDisplayYTwips
        35 => Some(1 << 24),                  // EDisplayColors (16M)
        41 => Some(1),                        // EPen (touch screen present)
        42 => Some(sw),                       // EPenX
        43 => Some(sh),                       // EPenY
        76 => Some(32),                       // EDisplayBitsPerPixel
        77 => Some(1),                        // EDisplayNumModes
        79 => Some(0),                        // EDisplayOffsetToFirstPixel
        80 => Some(sw * 4),                   // EDisplayOffsetBetweenLines
        91 => Some(1),                        // EDisplayNumberOfScreens
        92 => Some(1_000),                    // ENanoTickPeriod (us, 1 kHz)
        _ => None,
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    Some(match name {
        // TInt HAL::Get(HALData::TAttribute aAttribute, TInt& aValue)
        "_ZN3HAL3GetEN7HALData10TAttributeERi" => {
            let (attr, out) = (p.cpu.regs[0], p.cpu.regs[1]);
            match hal_attr_value(p, attr) {
                Some(v) => match p.wr(out, v as u32) {
                    Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return }
                    Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED hal {name}: {m}")),
                },
                None => {
                    p.cpu.regs[0] = KERR_NOT_SUPPORTED;
                    NativeOutcome::Return
                }
            }
        }
        // TInt HAL::Get(TInt aDeviceNumber, HALData::TAttribute aAttribute, TInt& aValue)
        "_ZN3HAL3GetEiN7HALData10TAttributeERi" => {
            let (attr, out) = (p.cpu.regs[1], p.cpu.regs[2]);
            match hal_attr_value(p, attr) {
                Some(v) => match p.wr(out, v as u32) {
                    Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return }
                    Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED hal {name}: {m}")),
                },
                None => {
                    p.cpu.regs[0] = KERR_NOT_SUPPORTED;
                    NativeOutcome::Return
                }
            }
        }
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
