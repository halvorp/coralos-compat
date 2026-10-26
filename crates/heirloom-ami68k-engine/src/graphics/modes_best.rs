//! graphics.library `BestModeIDA(a0 = tags)`: the mode of the display database that best fits the request. The database holds the engine's ONE
//! mode ([`super::modes::WORKBENCH_MODE`]), so the answer is that mode or INVALID_ID; what is honest is WHEN: Zune's/AROS's rom/graphics/bestmodeida.c
//! (APL; behaviour only) filters on the monitor ID (three passes: exact, MONITOR_ID_MASK 0xFFFF1000, AROS_MONITOR_ID_MASK 0xFFFF0000), the
//! DIPF must-have / must-not-have flags (default must-not-have: DUALPF | PF2PRI | HAM | EXTRAHALFBRITE), the gun bits (default 4), the minimum
//! depth and the desired size against the mode's Nominal dimensions, so a request the one screen cannot meet (deeper than SCREEN_DEPTH, larger
//! than SCREEN_WIDTH x SCREEN_HEIGHT, a monitor it is not on) gets INVALID_ID.
use alloc::format;
use alloc::string::String;
use super::layout::{SCREEN_DEPTH, SCREEN_HEIGHT, SCREEN_WIDTH};
use super::modes::{record, INVALID_MODE, WORKBENCH_MODE};
use crate::{machine::Amiga, tags, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

const BIDTAG_DIPF_MUST_HAVE: u32 = 0x8000_0001;
const BIDTAG_DIPF_MUST_NOT_HAVE: u32 = 0x8000_0002;
const BIDTAG_VIEWPORT: u32 = 0x8000_0003;
const BIDTAG_NOMINAL_WIDTH: u32 = 0x8000_0004;
const BIDTAG_NOMINAL_HEIGHT: u32 = 0x8000_0005;
const BIDTAG_DESIRED_WIDTH: u32 = 0x8000_0006;
const BIDTAG_DESIRED_HEIGHT: u32 = 0x8000_0007;
const BIDTAG_DEPTH: u32 = 0x8000_0008;
const BIDTAG_MONITOR_ID: u32 = 0x8000_0009;
const BIDTAG_SOURCE_ID: u32 = 0x8000_000A;
const BIDTAG_RED_BITS: u32 = 0x8000_000B;
const BIDTAG_BLUE_BITS: u32 = 0x8000_000C;
const BIDTAG_GREEN_BITS: u32 = 0x8000_000D;
/// graphics/displayinfo.h DIPF_IS_DUALPF | DIPF_IS_PF2PRI | DIPF_IS_HAM | DIPF_IS_EXTRAHALFBRITE (modeid.h SPECIAL_FLAGS).
const SPECIAL_FLAGS: u32 = 1 << 1 | 1 << 2 | 1 << 3 | 1 << 12;
const MONITOR_ID_MASK: u32 = 0xFFFF_1000;
const AROS_MONITOR_ID_MASK: u32 = 0xFFFF_0000;
/// struct ViewPort: DWidth 24, DHeight 26, RasInfo 36; struct RasInfo: BitMap 4; struct BitMap: Depth 5 (graphics/view.h, gfx.h).
const VP_DWIDTH: u32 = 24;
const VP_RASINFO: u32 = 36;

/// What the request asks (bestmodeida.c's `struct MatchData` and the defaults it starts from).
struct Want { musthave: u32, mustnothave: u32, bits: [u32; 3], depth: u32, monitor: u32, nominal: (u32, u32), desired: (u32, u32) }

fn disp_flags() -> u32 { u32::from_be_bytes(record(0x8000_0000, WORKBENCH_MODE).expect("DTAG_DISP")[18..22].try_into().expect("4 bytes")) }

pub(super) fn best_mode_ida<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = tags::walk(&a.mem, cpu.areg(0))?;
    let mut w = Want { musthave: 0, mustnothave: SPECIAL_FLAGS, bits: [4; 3], depth: 1, monitor: INVALID_MODE, nominal: (0, 0), desired: (0, 0) };
    let mut source = INVALID_MODE;
    let mut vp = 0;
    for &(t, d) in &list {
        match t {
            BIDTAG_DIPF_MUST_HAVE => w.musthave = d,
            BIDTAG_DIPF_MUST_NOT_HAVE => w.mustnothave = d,
            BIDTAG_MONITOR_ID => w.monitor = d,
            BIDTAG_RED_BITS => w.bits[0] = d,
            BIDTAG_GREEN_BITS => w.bits[1] = d,
            BIDTAG_BLUE_BITS => w.bits[2] = d,
            BIDTAG_VIEWPORT => {
                vp = d;
                let m = Refusal::mem("ViewPort");
                w.nominal = (a.mem.read_u16(d + VP_DWIDTH).map_err(m)? as u32, a.mem.read_u16(d + VP_DWIDTH + 2).map_err(Refusal::mem("ViewPort"))? as u32);
                let ras = a.mem.read_u32(d + VP_RASINFO).map_err(Refusal::mem("ViewPort.RasInfo"))?;
                let bm = if ras != 0 { a.mem.read_u32(ras + 4).map_err(Refusal::mem("RasInfo.BitMap"))? } else { 0 };
                if bm == 0 { return Err(Refusal(format!("BestModeIDA: BIDTAG_ViewPort {d:#x} has no RasInfo/BitMap to take a depth from"))); }
                w.depth = a.mem.read_u8(bm + 5).map_err(Refusal::mem("BitMap.Depth"))? as u32;
                source = WORKBENCH_MODE; // GetVPModeID of a ViewPort
            }
            _ => {}
        }
    }
    source = tags::get(&list, BIDTAG_SOURCE_ID).unwrap_or(source);
    let mode_dims = (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    if source != INVALID_MODE {
        w.musthave |= disp_flags() & SPECIAL_FLAGS;
        if vp == 0 && source == WORKBENCH_MODE { w.nominal = mode_dims; } // the source mode's Nominal dimensions
    }
    w.nominal = (tags::get(&list, BIDTAG_NOMINAL_WIDTH).unwrap_or(w.nominal.0), tags::get(&list, BIDTAG_NOMINAL_HEIGHT).unwrap_or(w.nominal.1));
    w.desired = (tags::get(&list, BIDTAG_DESIRED_WIDTH).unwrap_or(w.nominal.0), tags::get(&list, BIDTAG_DESIRED_HEIGHT).unwrap_or(w.nominal.1));
    w.depth = tags::get(&list, BIDTAG_DEPTH).unwrap_or(w.depth);
    w.mustnothave &= !w.musthave;
    if w.nominal.0 == 0 {
        // No size asked for at all: the default monitor's nominal dimensions (here: the one screen's) and depth.
        w.nominal = mode_dims;
        w.depth = SCREEN_DEPTH as u32;
        if tags::get(&list, BIDTAG_DESIRED_WIDTH).is_none() { w.desired.0 = w.nominal.0; }
        if tags::get(&list, BIDTAG_DESIRED_HEIGHT).is_none() { w.desired.1 = w.nominal.1; }
    }
    let flags = disp_flags();
    let fits = flags & w.mustnothave == 0 && flags & w.musthave == w.musthave
        && w.bits.iter().all(|&b| b <= 8) && w.depth <= SCREEN_DEPTH as u32
        && w.desired.0 <= mode_dims.0 && w.desired.1 <= mode_dims.1;
    let monitor_ok = [!0u32, MONITOR_ID_MASK, AROS_MONITOR_ID_MASK].into_iter()
        .any(|mask| w.monitor == INVALID_MODE || w.monitor & mask == WORKBENCH_MODE & mask);
    let id = if fits && monitor_ok { WORKBENCH_MODE } else { INVALID_MODE };
    cpu.set_d0(id);
    Ok(format!("[graphics] BestModeIDA {}x{}x{} monitor {:#x} -> {id:#x}", w.desired.0, w.desired.1, w.depth, w.monitor))
}

