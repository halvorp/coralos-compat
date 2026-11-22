//! asl.library: file-requester objects without a requester UI.
//!
//! AllocAslRequest(d0 = type, a0 = tags) -> a zeroed `struct FileRequester` (56 bytes; m68k-linux-gnu-gcc -m68020 -mno-align-int
//! over AROS libraries/asl.h, 2026-10-18: fr_File 4, fr_Drawer 8, fr_LeftEdge 22, fr_TopEdge 24, fr_Width 26, fr_Height 28,
//! fr_NumArgs 32, fr_ArgList 36, fr_UserData 40, fr_Pattern 52) with the initial window position/size from
//! ASLFR_InitialLeftEdge/TopEdge/Width/Height and ASLFR_UserData taken from the tags; FreeAslRequest(a0) releases it (a pointer
//! AllocAslRequest never returned is a named refusal).
//! AslRequest(a0 = request, a1 = tags) shows the requester; the engine has none, so it answers FALSE, the same answer as the
//! user pressing Cancel (logged as such). Font and screen-mode requesters (types 1, 2) are named refusals.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{tags, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const ASL_TB: u32 = tags::TAG_USER + 0x80000;
const FR_SIZE: u32 = 56;
const SM_SIZE: u32 = 48;
/// Default CyberGraphX 8-bit chunky RTG DisplayID returned by `ASL_ScreenModeRequest`.
pub const DEFAULT_RTG_DISPLAY_ID: u32 = 0x5000_1000;

#[derive(Clone, Copy)]
enum ReqKind { File, ScreenMode }

#[derive(Default)]
pub struct State { requests: BTreeMap<u32, ReqKind> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocAslRequest", alloc_request::<U>).named("FreeAslRequest", free_request::<U>).named("AslRequest", asl_request::<U>)
}

pub(crate) fn alloc_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, list) = (cpu.dreg(0), cpu.areg(0));
    match ty {
        0 => {
            let t = tags::walk(&a.mem, list)?;
            let Some(p) = a.heap.alloc(FR_SIZE) else { cpu.set_d0(0); return Ok("[asl] AllocAslRequest -> NULL (no memory)".into()) };
            a.mem.zero(p, FR_SIZE).map_err(Refusal::mem("FileRequester"))?;
            let m = Refusal::mem("FileRequester");
            for (tag, off) in [(ASL_TB + 3, 22u32), (ASL_TB + 4, 24), (ASL_TB + 5, 26), (ASL_TB + 6, 28)] {
                if let Some(v) = tags::get(&t, tag) { a.mem.write_u16(p + off, v as u16).map_err(&m)?; }
            }
            if let Some(v) = tags::get(&t, ASL_TB + 52) { a.mem.write_u32(p + 40, v).map_err(&m)?; } // ASLFR_UserData
            a.asl.requests.insert(p, ReqKind::File);
            cpu.set_d0(p);
            Ok(format!("[asl] AllocAslRequest ASL_FileRequest -> {p:#x}"))
        }
        2 => {
            let t = tags::walk(&a.mem, list)?;
            let Some(p) = a.heap.alloc(SM_SIZE) else { cpu.set_d0(0); return Ok("[asl] AllocAslRequest -> NULL (no memory)".into()) };
            a.mem.zero(p, SM_SIZE).map_err(Refusal::mem("ScreenModeRequester"))?;
            let m = Refusal::mem("ScreenModeRequester");
            let id = tags::get(&t, ASL_TB + 100).filter(|&v| v != 0).unwrap_or(DEFAULT_RTG_DISPLAY_ID);
            let w = tags::get(&t, ASL_TB + 101).filter(|&v| v != 0).unwrap_or(320);
            let h = tags::get(&t, ASL_TB + 102).filter(|&v| v != 0).unwrap_or(200);
            let d = tags::get(&t, ASL_TB + 103).filter(|&v| v != 0).unwrap_or(8);
            a.mem.write_u32(p, id).map_err(&m)?;
            a.mem.write_u32(p + 4, w).map_err(&m)?;
            a.mem.write_u32(p + 8, h).map_err(&m)?;
            a.mem.write_u16(p + 12, d as u16).map_err(&m)?;
            a.mem.write_u32(p + 18, w).map_err(&m)?;
            a.mem.write_u32(p + 22, h).map_err(&m)?;
            if let Some(v) = tags::get(&t, ASL_TB + 52) { a.mem.write_u32(p + 44, v).map_err(&m)?; }
            a.asl.requests.insert(p, ReqKind::ScreenMode);
            cpu.set_d0(p);
            Ok(format!("[asl] AllocAslRequest ASL_ScreenModeRequest -> {p:#x} ({w}x{h}x{d} id={id:#x})"))
        }
        _ => Err(Refusal(format!("AllocAslRequest type {ty}: only ASL_FileRequest (0) and ASL_ScreenModeRequest (2) are modelled (1 = font requester)"))),
    }
}

pub(crate) fn free_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if p == 0 { return Ok("[asl] FreeAslRequest NULL".into()); }
    let Some(kind) = a.asl.requests.remove(&p) else { return Err(Refusal(format!("FreeAslRequest: {p:#x} is not a request AllocAslRequest returned"))) };
    let sz = match kind { ReqKind::File => FR_SIZE, ReqKind::ScreenMode => SM_SIZE };
    a.heap.free(p, sz).map_err(|e| Refusal(format!("FreeAslRequest: {e}")))?;
    Ok(format!("[asl] FreeAslRequest {p:#x}"))
}

fn asl_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, list) = (cpu.areg(0), cpu.areg(1));
    let Some(&kind) = a.asl.requests.get(&p) else { return Err(Refusal(format!("AslRequest: {p:#x} is not a request AllocAslRequest returned"))) };
    match kind {
        ReqKind::File => {
            cpu.set_d0(0);
            Ok(format!("[asl] AslRequest {p:#x} -> FALSE (no requester UI: the same answer as Cancel)"))
        }
        ReqKind::ScreenMode => {
            let t = tags::walk(&a.mem, list)?;
            let m = Refusal::mem("ScreenModeRequester");
            if let Some(v) = tags::get(&t, ASL_TB + 100).filter(|&v| v != 0) { a.mem.write_u32(p, v).map_err(&m)?; }
            if let Some(v) = tags::get(&t, ASL_TB + 101).filter(|&v| v != 0) { a.mem.write_u32(p + 4, v).map_err(&m)?; }
            if let Some(v) = tags::get(&t, ASL_TB + 102).filter(|&v| v != 0) { a.mem.write_u32(p + 8, v).map_err(&m)?; }
            if let Some(v) = tags::get(&t, ASL_TB + 103).filter(|&v| v != 0) { a.mem.write_u16(p + 12, v as u16).map_err(&m)?; }
            cpu.set_d0(1);
            let id = a.mem.read_u32(p).unwrap_or(0);
            let w = a.mem.read_u32(p + 4).unwrap_or(0);
            let h = a.mem.read_u32(p + 8).unwrap_or(0);
            Ok(format!("[asl] AslRequest {p:#x} ScreenMode -> TRUE ({w}x{h} id={id:#x})"))
        }
    }
}
