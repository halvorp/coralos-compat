//! asl.library: file-requester objects without a requester UI.
//!
//! AllocAslRequest(d0 = type, a0 = tags) -> a zeroed `struct FileRequester` (56 bytes; m68k-linux-gnu-gcc -m68020 -mno-align-int
//! over AROS libraries/asl.h, 2026-10-18: fr_File 4, fr_Drawer 8, fr_LeftEdge 22, fr_TopEdge 24, fr_Width 26, fr_Height 28,
//! fr_NumArgs 32, fr_ArgList 36, fr_UserData 40, fr_Pattern 52) with the initial window position/size from
//! ASLFR_InitialLeftEdge/TopEdge/Width/Height and ASLFR_UserData taken from the tags; FreeAslRequest(a0) releases it (a pointer
//! AllocAslRequest never returned is a named refusal).
//! AslRequest(a0 = request, a1 = tags) shows the requester; the engine has none, so it answers FALSE, the same answer as the
//! user pressing Cancel (logged as such). Font and screen-mode requesters (types 1, 2) are named refusals.
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{tags, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const ASL_TB: u32 = tags::TAG_USER + 0x80000;
const FR_SIZE: u32 = 56;

#[derive(Default)]
pub struct State { requests: BTreeSet<u32> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocAslRequest", alloc_request::<U>).named("FreeAslRequest", free_request::<U>).named("AslRequest", asl_request::<U>)
}

pub(crate) fn alloc_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, list) = (cpu.dreg(0), cpu.areg(0));
    if ty != 0 { return Err(Refusal(format!("AllocAslRequest type {ty}: only ASL_FileRequest (0) is modelled (1 = font, 2 = screen mode requester)"))); }
    let t = tags::walk(&a.mem, list)?;
    let Some(p) = a.heap.alloc(FR_SIZE) else { cpu.set_d0(0); return Ok("[asl] AllocAslRequest -> NULL (no memory)".into()) };
    a.mem.zero(p, FR_SIZE).map_err(Refusal::mem("FileRequester"))?;
    let m = Refusal::mem("FileRequester");
    for (tag, off) in [(ASL_TB + 3, 22u32), (ASL_TB + 4, 24), (ASL_TB + 5, 26), (ASL_TB + 6, 28)] {
        if let Some(v) = tags::get(&t, tag) { a.mem.write_u16(p + off, v as u16).map_err(&m)?; }
    }
    if let Some(v) = tags::get(&t, ASL_TB + 52) { a.mem.write_u32(p + 40, v).map_err(&m)?; } // ASLFR_UserData
    a.asl.requests.insert(p);
    cpu.set_d0(p);
    Ok(format!("[asl] AllocAslRequest ASL_FileRequest -> {p:#x}"))
}

pub(crate) fn free_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if p == 0 { return Ok("[asl] FreeAslRequest NULL".into()); }
    if !a.asl.requests.remove(&p) { return Err(Refusal(format!("FreeAslRequest: {p:#x} is not a request AllocAslRequest returned"))); }
    a.heap.free(p, FR_SIZE).map_err(|e| Refusal(format!("FreeAslRequest: {e}")))?;
    Ok(format!("[asl] FreeAslRequest {p:#x}"))
}

fn asl_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if !a.asl.requests.contains(&p) { return Err(Refusal(format!("AslRequest: {p:#x} is not a request AllocAslRequest returned"))); }
    cpu.set_d0(0);
    Ok(format!("[asl] AslRequest {p:#x} -> FALSE (no requester UI: the same answer as Cancel)"))
}
