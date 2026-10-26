//! intuition.library Request / EndRequest / InitRequester: a `struct Requester` is a rectangle of a window with its own gadgets,
//! borders and text that blocks the window while it is up (rom/intuition/request.c, endrequest.c, requesters.c `render_requester`).
//!
//! What the guest can observe stays Intuition's: the rectangle (POINTREL centres it in the window's inner area plus RelLeft / RelTop,
//! otherwise LeftEdge / TopEdge are window-inner relative; both clipped to the inner area, request.c:60-120), `OlderRequest` /
//! `Window.FirstRequest` / `ReqCount`, `REQACTIVE`, `RWindow`, a non-NULL `ReqLayer` while it is up, and `IDCMP_REQCLEAR` at EndRequest.
//! A requester whose clipped rectangle is empty (a 0 x 0 "input blocker" requester) is not opened: FALSE, `ReqLayer` NULL, as AROS
//! answers (request.c:122-176). The requester's gadgets become native controls of the window through the same projection as AddGList
//! (positions relative to the requester, moved by its origin; `Amiga::req_gadgets`); its BackFill, ReqBorder and ReqText are the guest's
//! own content, drawn into the window's content plane by the graphics primitives (`SetRast`, DrawBorder, PrintIText). Refused by name:
//! `PREDRAWN` (ImageBMap) and `USEREQIMAGE` (ReqImage). Not modelled, and said so: the requester does not block the window's other
//! gadgets from the pointer (NOISYREQ is therefore moot).
use alloc::collections::BTreeMap;
use alloc::{format, string::String, vec::Vec};
use crate::{boopsi_glist, graphics::{drawinfo, layout}, idcmp, machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

// struct Requester (intuition/intuition.h, m68k -mno-align-int): OlderRequest 0, LeftEdge 4, TopEdge 6, Width 8, Height 10, RelLeft 12,
// RelTop 14, ReqGadget 16, ReqBorder 20, ReqText 24, Flags 28, BackFill 30, ReqLayer 32, ImageBMap 68, RWindow 72, ReqImage 76; 112 bytes.
const RQ_OLDER: u32 = 0;
const RQ_LEFT: u32 = 4;
const RQ_TOP: u32 = 6;
const RQ_WIDTH: u32 = 8;
const RQ_HEIGHT: u32 = 10;
const RQ_REL_LEFT: u32 = 12;
const RQ_REL_TOP: u32 = 14;
const RQ_GADGET: u32 = 16;
const RQ_BORDER: u32 = 20;
const RQ_TEXT: u32 = 24;
const RQ_FLAGS: u32 = 28;
const RQ_BACKFILL: u32 = 30;
const RQ_LAYER: u32 = 32;
const RQ_RWINDOW: u32 = 72;
const RQ_SIZE: u32 = 112;
const POINTREL: u16 = 1 << 0;
const PREDRAWN: u16 = 1 << 1;
const USEREQIMAGE: u16 = 1 << 5;
const NOREQBACKFILL: u16 = 1 << 6;
const REQACTIVE: u16 = 1 << 13;
// struct Window: LeftEdge 4, TopEdge 6, Width 8, Height 10, MouseY 12, MouseX 14, FirstRequest 36, DMRequest 40, ReqCount 44, RPort 50, BorderLeft 54.
const WD_LEFT: u32 = 4;
const WD_TOP: u32 = 6;
const WD_WIDTH: u32 = 8;
const WD_HEIGHT: u32 = 10;
const WD_FIRST_REQUEST: u32 = 36;
const WD_DM_REQUEST: u32 = 40;
const WD_REQ_COUNT: u32 = 44;
const WD_BORDER: u32 = 54;
const IDCMP_REQCLEAR: u32 = 0x1000;

/// An open requester: its window and the gadgets it added to that window's list.
pub struct ReqInfo { pub win: u32, pub gadgets: Vec<u32> }

/// Where a requester's gadget sits: the requester's origin in window coordinates and its size (the Rel* flags are relative to it).
#[derive(Clone, Copy)]
pub struct Origin { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

/// The requester state of the machine (open requesters, and each of their gadgets' origin).
#[derive(Default)]
pub struct Requesters { pub open: BTreeMap<u32, ReqInfo>, pub origin: BTreeMap<u32, Origin> }

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("Request", request::<U>).named("EndRequest", end_request::<U>).named("InitRequester", init_requester::<U>)
}

fn r16<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<i32, Refusal> { a.mem.read_i16(at).map(i32::from).map_err(Refusal::mem(what)) }
fn r32<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }
fn w32<U: GuestUi>(a: &mut Amiga<U>, at: u32, v: u32, what: &'static str) -> Result<(), Refusal> { a.mem.write_u32(at, v).map_err(Refusal::mem(what)) }

/// InitRequester(a0 = requester): zero the whole structure (intuition/initrequester.c).
fn init_requester<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let r = cpu.areg(0);
    a.mem.zero(r, RQ_SIZE).map_err(Refusal::mem("InitRequester"))?;
    Ok(format!("[intuition] InitRequester {r:#x}"))
}

/// Request(a0 = requester, a1 = window) -> TRUE when the requester opened.
fn request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (r, win) = (cpu.areg(0), cpu.areg(1));
    if !a.windows.contains_key(&win) { return Err(Refusal(format!("Request: {win:#x} is not an open window"))); }
    if a.req.open.contains_key(&r) { return Err(Refusal(format!("Request: requester {r:#x} is already open"))); }
    let flags = a.mem.read_u16(r + RQ_FLAGS).map_err(Refusal::mem("Requester.Flags"))?;
    if flags & PREDRAWN != 0 { return Err(Refusal(format!("Request {r:#x}: PREDRAWN (ImageBMap) is not modelled"))); }
    if flags & USEREQIMAGE != 0 { return Err(Refusal(format!("Request {r:#x}: USEREQIMAGE (ReqImage) is not modelled"))); }
    // request.c int_request: the rectangle, in screen coordinates, clipped to the window's inner area.
    let b = |i: u32| a.mem.read_u8(win + WD_BORDER + i).map(i32::from).map_err(Refusal::mem("Window.Border"));
    let (wl, wt) = (r16(a, win + WD_LEFT, "Window.LeftEdge")?, r16(a, win + WD_TOP, "Window.TopEdge")?);
    let (ww, wh) = (r16(a, win + WD_WIDTH, "Window.Width")?, r16(a, win + WD_HEIGHT, "Window.Height")?);
    let (bl, bt, br, bb) = (b(0)?, b(1)?, b(2)?, b(3)?);
    let (wleft, wtop, wright, wbottom) = (wl + bl, wt + bt, wl + ww - br - 1, wt + wh - bb - 1);
    let (rw, rh) = (r16(a, r + RQ_WIDTH, "Requester.Width")?, r16(a, r + RQ_HEIGHT, "Requester.Height")?);
    let (rl, rt) = (r16(a, r + RQ_LEFT, "Requester.LeftEdge")?, r16(a, r + RQ_TOP, "Requester.TopEdge")?);
    let (mut left, mut top) = if flags & POINTREL != 0 {
        let (rel_l, rel_t) = (r16(a, r + RQ_REL_LEFT, "Requester.RelLeft")?, r16(a, r + RQ_REL_TOP, "Requester.RelTop")?);
        if r == r32(a, win + WD_DM_REQUEST, "Window.DMRequest")? {
            let (mx, my) = (r16(a, win + idcmp::WD_MOUSE_X, "Window.MouseX")?, r16(a, win + idcmp::WD_MOUSE_Y, "Window.MouseY")?);
            let (mut l, mut t) = (wl + mx + rel_l, wt + my + rel_t);
            if l + rw - 1 > wright { l = wright - rw + 1; }
            if t + rh - 1 > wbottom { t = wbottom - rh + 1; }
            (l, t)
        } else { ((wleft + wright - rw) / 2 + rel_l, (wtop + wbottom - rh) / 2 + rel_t) }
    } else { (wleft + rl, wtop + rt) };
    left = left.clamp(wleft.min(wright), wright.max(wleft)); // `if (left < wleft) left = wleft; else if (left > wright) left = wright`
    top = top.clamp(wtop.min(wbottom), wbottom.max(wtop));
    let (right, bottom) = ((left + rw - 1).min(wright), (top + rh - 1).min(wbottom));
    w32(a, r + RQ_LAYER, 0, "Requester.ReqLayer")?;
    if right < left || bottom < top {
        cpu.set_d0(0);
        return Ok(format!("[intuition] Request {r:#x} on {win:#x}: an empty rectangle ({rw}x{rh}) -> FALSE"));
    }
    let (w_cl, h_cl) = (right - left + 1, bottom - top + 1);
    // Origin of the requester's layer in window coordinates (the window's outer top-left is 0,0).
    let (ox, oy) = (left - wl, top - wt);
    let first = r32(a, win + WD_FIRST_REQUEST, "Window.FirstRequest")?;
    let gadgets = r32(a, r + RQ_GADGET, "Requester.ReqGadget")?;
    let chain = boopsi_glist::chain(a, gadgets, -1)?;
    w32(a, r + RQ_LAYER, r, "Requester.ReqLayer")?; // a non-NULL token: the host composites the window, there is no separate layer
    w32(a, r + RQ_RWINDOW, win, "Requester.RWindow")?;
    a.mem.write_u16(r + RQ_LEFT, (left - wleft) as u16).map_err(Refusal::mem("Requester.LeftEdge"))?;
    a.mem.write_u16(r + RQ_TOP, (top - wtop) as u16).map_err(Refusal::mem("Requester.TopEdge"))?;
    w32(a, r + RQ_OLDER, first, "Requester.OlderRequest")?;
    w32(a, win + WD_FIRST_REQUEST, r, "Window.FirstRequest")?;
    let n = a.mem.read_u16(win + WD_REQ_COUNT).map_err(Refusal::mem("Window.ReqCount"))?;
    a.mem.write_u16(win + WD_REQ_COUNT, n.wrapping_add(1)).map_err(Refusal::mem("Window.ReqCount"))?;
    a.mem.write_u16(r + RQ_FLAGS, flags | REQACTIVE).map_err(Refusal::mem("Requester.Flags"))?;
    for &g in &chain { a.req.origin.insert(g, Origin { x: ox, y: oy, w: w_cl, h: h_cl }); }
    // AddGList(window, gadgets, 0, -1, requester): into the window's list, native controls through the shared projection.
    boopsi_glist::attach(a, win, gadgets, 0xFFFF, -1)?;
    a.req.open.insert(r, ReqInfo { win, gadgets: chain.clone() });
    // render_requester: BackFill, ReqBorder, (gadgets are native), ReqText, in the window's content at the requester's origin.
    let rp = r32(a, win + layout::WD_RPORT, "Window.RPort")?;
    let (bfill, border, text) = (a.mem.read_u8(r + RQ_BACKFILL).map_err(Refusal::mem("Requester.BackFill"))?, r32(a, r + RQ_BORDER, "Requester.ReqBorder")?, r32(a, r + RQ_TEXT, "Requester.ReqText")?);
    if flags & NOREQBACKFILL == 0 { drawinfo::fill_pen(a, rp, (ox, oy, w_cl, h_cl), bfill)?; }
    if border != 0 { drawinfo::draw_borders(a, rp, border, ox, oy)?; }
    if text != 0 { drawinfo::print_itexts(a, rp, text, ox, oy)?; }
    cpu.set_d0(1);
    Ok(format!("[intuition] Request {r:#x} on {win:#x}: {w_cl}x{h_cl} at {ox},{oy}, {} gadgets -> TRUE", chain.len()))
}

/// EndRequest(a0 = requester, a1 = window): unlink it (a requester that is not the window's is ignored, as endrequest.c does), take its
/// gadgets back out of the window, and post IDCMP_REQCLEAR (IAddress = the window).
fn end_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (r, win) = (cpu.areg(0), cpu.areg(1));
    if !a.windows.contains_key(&win) { return Err(Refusal(format!("EndRequest: {win:#x} is not an open window"))); }
    let mut p = r32(a, win + WD_FIRST_REQUEST, "Window.FirstRequest")?;
    let mut linked = false;
    if p == r { let older = r32(a, r + RQ_OLDER, "Requester.OlderRequest")?; w32(a, win + WD_FIRST_REQUEST, older, "Window.FirstRequest")?; linked = true; }
    else {
        let mut guard = 0;
        while p != 0 && guard < 4096 && r32(a, p + RQ_OLDER, "Requester.OlderRequest")? != r { p = r32(a, p + RQ_OLDER, "Requester.OlderRequest")?; guard += 1; }
        if p != 0 && guard < 4096 { let older = r32(a, r + RQ_OLDER, "Requester.OlderRequest")?; w32(a, p + RQ_OLDER, older, "Requester.OlderRequest")?; linked = true; }
    }
    if !linked { return Ok(format!("[intuition] EndRequest {r:#x}: not one of {win:#x}'s requesters (nothing to do)")); }
    let n = a.mem.read_u16(win + WD_REQ_COUNT).map_err(Refusal::mem("Window.ReqCount"))?;
    a.mem.write_u16(win + WD_REQ_COUNT, n.wrapping_sub(1)).map_err(Refusal::mem("Window.ReqCount"))?;
    let flags = a.mem.read_u16(r + RQ_FLAGS).map_err(Refusal::mem("Requester.Flags"))?;
    a.mem.write_u16(r + RQ_FLAGS, flags & !REQACTIVE).map_err(Refusal::mem("Requester.Flags"))?;
    w32(a, r + RQ_OLDER, 0, "Requester.OlderRequest")?;
    w32(a, r + RQ_LAYER, 0, "Requester.ReqLayer")?;
    if let Some(info) = a.req.open.remove(&r) {
        if let Some(&first) = info.gadgets.first() { boopsi_glist::detach(a, win, first, info.gadgets.len() as i32)?; }
        for g in &info.gadgets { a.req.origin.remove(g); }
        w32(a, r + RQ_GADGET, info.gadgets.first().copied().unwrap_or(0), "Requester.ReqGadget")?;
    }
    let d = idcmp::post(a, win, IDCMP_REQCLEAR, 0, win)?;
    Ok(format!("[intuition] EndRequest {r:#x} on {win:#x} (REQCLEAR {d:?})"))
}

/// The window closed: its requesters' bookkeeping goes with it.
pub fn forget_window<U: GuestUi>(a: &mut Amiga<U>, win: u32) {
    let gone: Vec<u32> = a.req.open.iter().filter(|(_, i)| i.win == win).map(|(r, _)| *r).collect();
    for r in gone { if let Some(i) = a.req.open.remove(&r) { for g in i.gadgets { a.req.origin.remove(&g); } } }
}
