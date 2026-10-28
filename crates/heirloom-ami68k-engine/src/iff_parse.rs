//! iffparse.library, part 4: ParseIFF, the state machine of AROS parseiff.c / basicfuncs.c InvokeHandlers (APL, read for facts only).
//!
//! The states are AROS' (COMPOSITE -> PUSHCHUNK -> ATOMIC -> SCANEXIT -> EXIT -> POPCHUNK) and live in the handle (guest word at
//! +64), so a ParseIFF that returned to the client (IFFPARSE_STEP, a StopChunk, a handler's own IFF_RETURN2CLIENT) resumes exactly
//! where it stopped. The engine's own handlers (StopChunk, PropChunk, CollectionChunk, StopOnExit) run inline; a handler or a purge
//! hook of the PROGRAM is called as a guest subroutine (`guest_call`: A0 = hook, A2 = object, A1 = IFFCMD_ENTRY / IFFCMD_EXIT /
//! IFFCMD_PURGELCI) and the machine RESUMES from the hook's D0 in its continuation, so the guest can do anything inside a handler.
//! Modes: IFFPARSE_SCAN runs the whole file, STEP stops at every chunk entry and exit, RAWSTEP stops at every chunk without
//! running any handler. An atomic chunk the client did not read is skipped (SCAN) or left to the next call (STEP / RAWSTEP).
use alloc::boxed::Box;
use alloc::string::String;
use alloc::format;
use crate::guest_call::Next;
use crate::iff::*;
use crate::iff_lci::{self, IFFLCI_ENTRYHANDLER, IFFLCI_EXITHANDLER};
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const IFFPARSE_SCAN: i32 = 0;
const IFFPARSE_STEP: i32 = 1;
const IFFPARSE_RAWSTEP: i32 = 2;
const IFFCMD_ENTRY: u32 = 5;
const IFFCMD_EXIT: u32 = 6;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> { b.named("ParseIFF", parse_iff::<U>) }

enum Flow<U: GuestUi> { Go, Done(i32), Call(Next<U>) }

/// ParseIFF(a0 = iff, d0 = mode) -> 0 (returned to the client at a stop), IFFERR_EOF at the end of the file, or an IFFERR code.
fn parse_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, mode) = (cpu.areg(0), cpu.dreg(0) as i32);
    handle(a, "ParseIFF", iff)?;
    if !(IFFPARSE_SCAN..=IFFPARSE_RAWSTEP).contains(&mode) { return Err(Refusal(format!("ParseIFF: mode {mode} is not SCAN, STEP or RAWSTEP"))); }
    if flags(a, iff)? & IFFF_OPEN == 0 { cpu.set_d0(IFFERR_NOTIFF as u32); return Ok(format!("[iffparse] ParseIFF {iff:#x} (not open) -> NOTIFF")); }
    let next = run(a, cpu, iff, mode)?;
    crate::guest_call::start(a, cpu, next)?;
    Ok(format!("[iffparse] ParseIFF {iff:#x} mode={mode}"))
}

/// Run states until the parse stops or has to call the guest.
fn run<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, iff: u32, mode: i32) -> Result<Next<U>, Refusal> {
    loop {
        match step(a, cpu, iff, mode)? {
            Flow::Go => {}
            Flow::Done(e) => return Ok(Next::Return(e as u32)),
            Flow::Call(n) => return Ok(n),
        }
    }
}

/// What InvokeHandlers returned, read by the machine: 0 goes on, IFF_RETURN2CLIENT stops with 0, any other code stops with it.
fn after<U: GuestUi>(r: i32) -> Flow<U> { if r == 0 { Flow::Go } else { Flow::Done(if r == IFF_RETURN2CLIENT { 0 } else { r }) } }

/// The result of InvokeHandlers once the handler (if any) returned `err`.
fn invoked(mode: i32, ident: u32, err: i32) -> i32 {
    let stepping = if ident == IFFLCI_ENTRYHANDLER { IFF_RETURN2CLIENT } else { IFFERR_EOC };
    if err != 0 { err } else if mode == IFFPARSE_STEP { stepping } else { 0 }
}

fn invoke<U: GuestUi>(a: &mut Amiga<U>, iff: u32, mode: i32, ident: u32) -> Result<Flow<U>, Refusal> {
    let stepping = if ident == IFFLCI_ENTRYHANDLER { IFF_RETURN2CLIENT } else { IFFERR_EOC };
    if mode == IFFPARSE_RAWSTEP { return Ok(after(stepping)); }
    let cn = top(a, iff);
    let (ty, id) = (rd(a, cn + CN_TYPE)?, rd(a, cn + CN_ID)?);
    let Some(l) = iff_lci::find_local(a, iff, ty, id, ident)? else { return Ok(after(invoked(mode, ident, 0))) };
    let info = rd(a, l + 24)?; // the item's user data: a HandlerInfo { hook, object }
    let (hook, obj) = (rd(a, info)?, rd(a, info + 4)?);
    if hook == 0 { return Ok(after(IFFERR_NOHOOK)); }
    let native = match which_hook(a, hook) {
        Some(NativeHook::Stop) => Some(IFF_RETURN2CLIENT),
        Some(NativeHook::ExitContext) => Some(IFFERR_EOC),
        Some(NativeHook::Prop) => Some(iff_lci::prop_func(a, iff)?),
        Some(NativeHook::Collection) => Some(iff_lci::collection_func(a, iff)?),
        Some(NativeHook::Dos | NativeHook::Clip) => return Err(Refusal(format!("ParseIFF: a stream hook {hook:#x} was installed as a chunk handler"))),
        None => None,
    };
    if let Some(e) = native { return Ok(after(invoked(mode, ident, e))); }
    let entry = rd(a, hook + 8)?;
    if entry == 0 { return Err(Refusal(format!("ParseIFF: handler hook {hook:#x} has a NULL h_Entry"))); }
    let param = if ident == IFFLCI_ENTRYHANDLER { IFFCMD_ENTRY } else { IFFCMD_EXIT };
    Ok(Flow::Call(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, hook), (1, param), (2, obj)], then: Box::new(move |a, cpu, d0| {
        match after::<U>(invoked(mode, ident, d0 as i32)) {
            Flow::Done(e) => Ok(Next::Return(e as u32)),
            _ => run(a, cpu, iff, mode),
        }
    }) }))
}

fn step<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, iff: u32, mode: i32) -> Result<Flow<U>, Refusal> {
    match state(a, iff)? {
        PState::Composite => { set_state(a, iff, PState::PushChunk)?; invoke(a, iff, mode, IFFLCI_ENTRYHANDLER) }
        PState::PushChunk => {
            let e = get_chunk_header(a, iff)?;
            if e != 0 { return Ok(Flow::Done(e)); }
            let composite = a.iff.cns[&top(a, iff)].composite;
            set_state(a, iff, if composite { PState::Composite } else { PState::Atomic })?;
            Ok(Flow::Go)
        }
        PState::Atomic => { set_state(a, iff, PState::ScanExit)?; invoke(a, iff, mode, IFFLCI_ENTRYHANDLER) }
        PState::ScanExit => {
            set_state(a, iff, PState::Exit)?;
            if mode == IFFPARSE_SCAN {
                let cn = top(a, iff);
                let (size, scan) = (rd(a, cn + CN_SIZE_F)? as i32, rd(a, cn + CN_SCAN)? as i32);
                let toseek = size.wrapping_sub(scan) + (size & 1);
                if toseek != 0 {
                    let e = seek_stream(a, iff, toseek);
                    if e != 0 { return Ok(Flow::Done(e)); }
                    wr(a, cn + CN_SCAN, scan.wrapping_add(toseek) as u32)?;
                }
            }
            Ok(Flow::Go)
        }
        PState::Exit => { set_state(a, iff, PState::PopChunk)?; invoke(a, iff, mode, IFFLCI_EXITHANDLER) }
        PState::PopChunk => pop_state(a, cpu, iff, mode),
    }
}

fn pop_state<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, iff: u32, mode: i32) -> Result<Flow<U>, Refusal> {
    if depth(a, iff)? == 0 { return Ok(Flow::Done(IFFERR_EOF)); } // nothing left to pop: the file was parsed to its end already
    let cn = top(a, iff);
    let composite = a.iff.cns[&cn].composite;
    let (csize, scan) = (rd(a, cn + CN_SIZE_F)? as i32, rd(a, cn + CN_SCAN)? as i32);
    let mut size = csize + (csize & 1);
    if !composite && size - scan != 0 {
        let e = seek_stream(a, iff, size - scan);
        if e != 0 { return Ok(Flow::Done(e)); }
        wr(a, cn + CN_SCAN, size as u32)?;
    }
    size += if composite { 12 } else { 8 };
    let purge = pop_context(a, iff)?;
    let parent = top(a, iff);
    let pscan = (rd(a, parent + CN_SCAN)? as i32).wrapping_add(size);
    wr(a, parent + CN_SCAN, pscan as u32)?;
    let psize = rd(a, parent + CN_SIZE_F)? as i32;
    // 1. more chunks in the enclosing composite: read the next; 3. the outermost composite is used up: EOF; 2. else leave it.
    let end = if pscan < psize { set_state(a, iff, PState::PushChunk)?; None }
        else if depth(a, iff)? == 0 { Some(IFFERR_EOF) }
        else { set_state(a, iff, PState::Exit)?; None };
    if purge.is_empty() { return Ok(match end { Some(e) => Flow::Done(e), None => Flow::Go }); }
    let next = iff_lci::purge_chain(a, cpu, purge, Box::new(move |a, cpu, _| match end { Some(e) => Ok(Next::Return(e as u32)), None => run(a, cpu, iff, mode) }))?;
    Ok(Flow::Call(next))
}

