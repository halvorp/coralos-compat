//! intuition.library requesters as NATIVE alerts: EasyRequestArgs, BuildEasyRequestArgs, SysReqHandler, FreeSysRequest,
//! AutoRequest and BuildSysRequest. The EasyStruct (or the IntuiText chain) becomes one generic `GuestUi::request` (title, text,
//! buttons); the original OS drew a window with gadgets, the host shows its own alert. Everything the GUEST can observe stays
//! Intuition's (AROS rom/intuition easyrequestargs.c, buildeasyrequestargs.c, sysrequest_intern.c, autorequest.c, APL, read for
//! facts only):
//! - the buttons of an EasyStruct are es_GadgetFormat split at `|` (after RawDoFmt, with the arguments that follow the text's);
//!   gadget i has GadgetID i+1 except the LAST, which is 0, so a click returns 1..n-1 left to right and the rightmost returns 0;
//!   a lone button returns 0 as well. Return = the first button when there are two or more (Intuition's LCommand-V), Escape the last.
//! - AutoRequest is BuildSysRequest with the positive text first: TRUE for it, FALSE for the negative text.
//! - a window the requester was built over must be an open window (a title-less request borrows its Title); es_Title falls back to the
//!   reference window's title, then "System Request".
//! - the SYNCHRONOUS calls park only the calling TASK (`Waiting::Request`) until the host answers; `BuildEasyRequestArgs` returns
//!   a real `struct Window` with a UserPort whose signal fires when the answer arrives (an IDCMP_GADGETUP IntuiMessage whose
//!   IAddress is a gadget with that GadgetID), and `SysReqHandler` consumes it: -2 not yet, -1 an IDCMP class the caller listed
//!   closed it early (stored through the flags pointer), else the button number.
//! - IDCMP classes passed in (EasyRequestArgs' *IDCMP_ptr, BuildSysRequest's flags, AutoRequest's pFlag|nFlag) close the request
//!   early when one of them is reported with [`Amiga::notify_requesters`]; the engine itself raises none yet (it has no disk or
//!   preferences events), so only a host/driver that calls it can produce -1.
//! The alert's keyboard answers (Return / Escape) are the host's: `RequestModel::default` / `cancel` name the buttons.
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;
use crate::exec_wait::{self, Waiting};
use crate::idcmp::{self, IDCMP_GADGETUP, INTUIMESSAGE_SIZE};
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use guestui_abi::{GuestUi, GuestUiEvent, RequestModel, RequestRef};
use retro_m68k::Cpu;

// struct EasyStruct (intuition/intuition.h; m68k gcc -mno-align-int, 2026-10-19): es_Title 8, es_TextFormat 12, es_GadgetFormat 16.
const ES_TITLE: u32 = 8;
const ES_TEXT_FORMAT: u32 = 12;
const ES_GADGET_FORMAT: u32 = 16;
// struct IntuiText: LeftEdge 4, TopEdge 6, IText 12, NextText 16. struct Gadget: GadgetType 16, GadgetID 38, 44 bytes.
const IT_TOP: u32 = 6;
const IT_TEXT: u32 = 12;
const IT_NEXT: u32 = 16;
const GADGET_SIZE: u32 = 44;
const GA_NEXT: u32 = 0;
const GA_TYPE: u32 = 16;
const GA_ID: u32 = 38;
const GTYP_BOOLGADGET: u16 = 1;
// struct Window: Title 32, IDCMPFlags 82 (idcmp.rs), 136 bytes (window.rs).
const WD_TITLE: u32 = 32;
const IDCMP_RAWKEY: u32 = 0x400;
const IDCMP_VANILLAKEY: u32 = 0x0020_0000;
const SYSREQ_TITLE: &str = "System Request";
/// Keys of `State::pending` for a synchronous request live above guest RAM so they can never equal a Window address.
const SYNC_KEY_BASE: u32 = 0x8000_0000;
const MAX_STRING: usize = 8192;

/// What the guest called, which fixes how a click or an early close maps to its return value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind { Easy, Auto { pos_flag: u32 }, /** `MUI_RequestA` with an Application (mui/request.rs). */ Mui }

struct Pending {
    ui: RequestRef,
    owner: u32,
    buttons: u32,
    kind: Kind,
    /// Classes that close the request early (the caller's IDCMP flags, VANILLAKEY removed as Intuition does).
    idcmp: u32,
    /// Where SysReqHandler / EasyRequestArgs store the class that closed it early (0 = nowhere).
    out_ptr: u32,
    /// The requester window of an asynchronous request (its handle), with the gadgets IntuiMessages point at.
    window: Option<(u32, Vec<u32>)>,
    /// Set when answered: (the value the call returns, the IDCMP class if it was an early close).
    answer: Option<(i32, u32)>,
    /// The answer was handed to the guest already.
    taken: bool,
}

#[derive(Default)]
pub struct State { pending: BTreeMap<u32, Pending>, next_key: u32 }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("EasyRequestArgs", easy_request_args::<U>).named("BuildEasyRequestArgs", build_easy_request_args::<U>)
        .named("SysReqHandler", sys_req_handler::<U>).named("FreeSysRequest", free_sys_request::<U>)
        .named("AutoRequest", auto_request::<U>).named("BuildSysRequest", build_sys_request::<U>)
}

fn latin1(bytes: &[u8]) -> String { bytes.iter().map(|&b| b as char).collect() }

fn text_at<U: GuestUi>(a: &Amiga<U>, p: u32, what: &'static str) -> Result<Vec<u8>, Refusal> { crate::guest_str::read_bytes_z(&a.mem, p, MAX_STRING).map_err(Refusal::mem(what)) }

/// The value a click on button `i` of `n` returns (GadgetID of gadget i): 1..n-1 left to right, the last 0.
pub fn button_value(n: u32, i: u32) -> i32 { if i + 1 == n { 0 } else { (i + 1) as i32 } }

/// What `MUI_RequestA` returns for button `i` of `n` (Zune mui_request.c): button j sends ReturnID j+2 (the LAST sends 1, and a LONE button 2), the
/// call returns ID-1: 1..n-1 left to right, the rightmost 0, a lone button 1 (Intuition's EasyRequest would give it 0).
pub fn mui_value(n: u32, i: u32) -> i32 { if n == 1 { 1 } else { button_value(n, i) } }

/// Ask the host for `m` on behalf of the calling task and park it until the answer (what EasyRequestArgs does, for any requester kind).
pub(crate) fn ask_sync<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, ui_win: Option<guestui_abi::UiObjectRef>, m: RequestModel, kind: Kind, (idcmp, out_ptr): (u32, u32), what: String) -> Result<String, Refusal> {
    let (nb, ui) = (m.buttons.len() as u32, a.ask(ui_win, m)?);
    let k = SYNC_KEY_BASE + { a.requests.next_key += 1; a.requests.next_key };
    a.requests.pending.insert(k, Pending { ui, owner: a.sched.cur, buttons: nb, kind, idcmp, out_ptr, window: None, answer: None, taken: false });
    exec_wait::block_on(a, cpu, Waiting::Request(k), what)
}

fn ref_window<U: GuestUi>(a: &Amiga<U>, what: &str, w: u32) -> Result<(Option<guestui_abi::UiObjectRef>, Option<String>), Refusal> {
    if w == 0 { return Ok((None, None)); }
    let ui = *a.windows.get(&w).ok_or_else(|| Refusal(format!("{what}: reference window {w:#x} is not an open window")))?;
    let t = a.mem.read_u32(w + WD_TITLE).map_err(Refusal::mem("Window.Title"))?;
    Ok((Some(ui), if t == 0 { None } else { Some(latin1(&text_at(a, t, "Window.Title")?)) }))
}

/// The model of an EasyStruct with its arguments (`RawDoFmt` for the text, then for the buttons with the rest of the stream).
fn easy_model<U: GuestUi>(a: &Amiga<U>, win_title: Option<String>, es: u32, args: u32) -> Result<RequestModel, Refusal> {
    let m = Refusal::mem("EasyStruct");
    let (title, tf, gf) = (a.mem.read_u32(es + ES_TITLE).map_err(&m)?, a.mem.read_u32(es + ES_TEXT_FORMAT).map_err(&m)?, a.mem.read_u32(es + ES_GADGET_FORMAT).map_err(&m)?);
    let title = if title != 0 { latin1(&text_at(a, title, "es_Title")?) } else { win_title.unwrap_or_else(|| SYSREQ_TITLE.to_string()) };
    let text = crate::fmt::format(&a.mem, &text_at(a, tf, "es_TextFormat")?, args, false)?;
    let gtpl = text_at(a, gf, "es_GadgetFormat")?;
    let expect = gtpl.iter().filter(|&&c| c == b'|').count() + 1; // Intuition counts the pipes of the UNFORMATTED string
    let labels = crate::fmt::format(&a.mem, &gtpl, args + text.used, false)?;
    let parts: Vec<&[u8]> = labels.bytes.splitn(expect, |&c| c == b'|').collect();
    if parts.len() != expect { return Err(Refusal(format!("EasyStruct: es_GadgetFormat has {expect} buttons but its formatted text has fewer '|' (Intuition would read past the string)"))); }
    Ok(model(title, latin1(&text.bytes), parts.iter().map(|p| latin1(p)).collect()))
}

pub(crate) fn model(title: String, text: String, buttons: Vec<String>) -> RequestModel {
    let n = buttons.len() as u32;
    // Return = the first button (LCommand-V is ic_ReqTrue: gadget id 1 when there are two or more); Escape = the last (ic_ReqFalse: 0).
    RequestModel { title, text, buttons, default: Some(0), cancel: Some(n - 1) }
}

/// The text of an IntuiText chain: items on one row (same TopEdge) join, a lower TopEdge starts a new line.
fn intuitext_lines<U: GuestUi>(a: &Amiga<U>, mut it: u32) -> Result<String, Refusal> {
    let (mut out, mut top, mut guard) = (String::new(), None::<i16>, 0);
    while it != 0 {
        guard += 1;
        if guard > 64 { return Err(Refusal("IntuiText chain longer than 64 items (looped?)".into())); }
        let m = Refusal::mem("IntuiText");
        let (y, s, next) = (a.mem.read_u16(it + IT_TOP).map_err(&m)? as i16, a.mem.read_u32(it + IT_TEXT).map_err(&m)?, a.mem.read_u32(it + IT_NEXT).map_err(&m)?);
        if s != 0 {
            if top.is_some_and(|t| y > t) { out.push('\n'); }
            out.push_str(&latin1(&text_at(a, s, "IntuiText.IText")?));
            top = Some(y);
        }
        it = next;
    }
    Ok(out)
}

fn intuitext_label<U: GuestUi>(a: &Amiga<U>, it: u32) -> Result<String, Refusal> {
    let s = a.mem.read_u32(it + IT_TEXT).map_err(Refusal::mem("IntuiText"))?;
    Ok(if s == 0 { String::new() } else { latin1(&text_at(a, s, "IntuiText.IText")?) })
}

/// BuildSysRequest's model: body text, then [positive,] negative. The negative text and the body are required.
fn sys_model<U: GuestUi>(a: &Amiga<U>, win_title: Option<String>, body: u32, pos: u32, neg: u32) -> Result<RequestModel, Refusal> {
    let mut buttons = Vec::new();
    if pos != 0 { buttons.push(intuitext_label(a, pos)?); }
    buttons.push(intuitext_label(a, neg)?);
    Ok(model(win_title.unwrap_or_else(|| SYSREQ_TITLE.to_string()), intuitext_lines(a, body)?, buttons))
}

impl<U: GuestUi> Amiga<U> {
    /// True while a requester waits for its answer (the driver should then pump the host's events).
    pub fn requests_pending(&self) -> bool { self.requests.pending.values().any(|p| p.answer.is_none()) }

    /// Requesters not yet freed or consumed (answered asynchronous ones stay until FreeSysRequest).
    pub fn open_requesters(&self) -> usize { self.requests.pending.len() }

    /// Report an IDCMP class to every open requester: those that listed it close early (a return of -1 with the class stored).
    pub fn notify_requesters(&mut self, class: u32) -> Result<usize, Refusal> {
        let keys: Vec<u32> = self.requests.pending.iter().filter(|(_, p)| p.answer.is_none() && p.idcmp & class != 0).map(|(&k, _)| k).collect();
        for &k in &keys {
            let ui = self.requests.pending[&k].ui;
            let _ = self.ui.close_request(ui); // the host closes the alert; an unsupporting host has nothing to close
            self.settle(k, -1, class)?;
        }
        Ok(keys.len())
    }

    /// Record the answer of request `k`, post its message if asynchronous, and wake its task.
    fn settle(&mut self, k: u32, value: i32, class: u32) -> Result<(), Refusal> {
        let (owner, window, kind) = { let p = &self.requests.pending[&k]; (p.owner, p.window.as_ref().map(|(w, g)| (*w, g.clone())), p.kind) };
        let ret = match (kind, class) { (Kind::Auto { pos_flag }, c) if c != 0 => (pos_flag & c != 0) as i32, _ => value };
        if let Some((w, gadgets)) = window {
            // The message Intuition would post: GADGETUP with the clicked gadget in IAddress (an early close posts the class instead).
            let msg = idcmp::reserve(self)?;
            let gadget = gadgets.iter().copied().find(|&g| self.mem.read_u16(g + GA_ID).map(|id| id as i32 == value).unwrap_or(false)).unwrap_or(0);
            idcmp::queue(self, w, msg, if class != 0 { class } else { IDCMP_GADGETUP }, 0, gadget)?;
        }
        self.requests.pending.get_mut(&k).expect("pending").answer = Some((ret, class));
        self.wake(owner)
    }

    /// The host answered: `button` of request `req`.
    pub(crate) fn request_answered(&mut self, req: RequestRef, button: u32) -> Result<(), Refusal> {
        let Some((&k, p)) = self.requests.pending.iter().find(|(_, p)| p.ui == req) else { return Err(Refusal(format!("RequestAnswered: {req:?} is not an open request (stale or never asked)"))) };
        if p.answer.is_some() { return Err(Refusal(format!("RequestAnswered: {req:?} was answered already"))); }
        if button >= p.buttons { return Err(Refusal(format!("RequestAnswered: button {button} of a request with {} buttons", p.buttons))); }
        let v = match p.kind { Kind::Mui => mui_value(p.buttons, button), _ => button_value(p.buttons, button) };
        self.settle(k, v, 0)
    }

    /// The scheduler's check for a task parked on a request: Some(return value) once answered; `commit` consumes it.
    pub(crate) fn request_done(&mut self, k: u32, commit: bool) -> Result<Option<u32>, Refusal> {
        let Some(p) = self.requests.pending.get(&k) else { return Err(Refusal(format!("a task waits on request {k:#x}, which no longer exists"))) };
        let Some((v, class)) = p.answer else { return Ok(None) };
        if commit { self.take_answer(k, class)?; }
        Ok(Some(v as u32))
    }

    /// The guest receives the answer: the early-close class goes to the caller's pointer, an asynchronous request's message leaves its port,
    /// a synchronous request disappears.
    fn take_answer(&mut self, k: u32, class: u32) -> Result<(), Refusal> {
        let (out, window, sync) = { let p = &self.requests.pending[&k]; (p.out_ptr, p.window.as_ref().map(|(w, _)| *w), p.window.is_none()) };
        if class != 0 && out != 0 { self.mem.write_u32(out, class).map_err(Refusal::mem("IDCMP flags pointer"))?; }
        if let Some(w) = window {
            let port = self.mem.read_u32(w + idcmp::WD_USERPORT).map_err(Refusal::mem("Window.UserPort"))?;
            let msg = exec_wait::rem_head(self, port + 20)?;
            if msg != 0 && self.engine_msgs.remove(&msg).is_some() { self.heap.free(msg, INTUIMESSAGE_SIZE).map_err(|e| Refusal(format!("SysReqHandler: {e}")))?; }
        }
        if sync { self.requests.pending.remove(&k); } else { self.requests.pending.get_mut(&k).expect("pending").taken = true; }
        Ok(())
    }

    /// Ask the host; on refusal nothing is left behind.
    fn ask(&mut self, win: Option<guestui_abi::UiObjectRef>, m: RequestModel) -> Result<RequestRef, Refusal> { self.ui.request(win, m).map_err(Refusal::ui) }
}

/// The driver's pump: while a request waits, hand the host's queued events to the engine. True if any was delivered.
pub fn pump<U: GuestUi>(a: &mut Amiga<U>) -> Result<bool, Refusal> {
    if !a.requests_pending() { return Ok(false); }
    let mut any = false;
    while let Some(ev) = a.ui.poll_event() { a.deliver(ev)?; any = true; }
    Ok(any)
}

/// EasyRequestArgs(a0 = window, a1 = EasyStruct, a2 = IDCMP flags pointer, a3 = argument stream) -> button number, -1 (IDCMP) or 0.
fn easy_request_args<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, es, idcmp_ptr, args) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if es == 0 { cpu.set_d0(0); return Ok("[intuition] EasyRequestArgs NULL EasyStruct -> 0".into()); }
    let (ui_win, win_title) = ref_window(a, "EasyRequestArgs", win)?;
    let mask = if idcmp_ptr == 0 { 0 } else { a.mem.read_u32(idcmp_ptr).map_err(Refusal::mem("IDCMP flags pointer"))? };
    let m = easy_model(a, win_title, es, args)?;
    let what = format!("[intuition] EasyRequestArgs {:?} {:?} {:?}", m.title, m.text, m.buttons);
    ask_sync(a, cpu, ui_win, m, Kind::Easy, (mask & !IDCMP_VANILLAKEY, idcmp_ptr), what)
}

/// AutoRequest(a0 = window, a1 = body, a2 = positive, a3 = negative, d0 = pFlag, d1 = nFlag, d2/d3 = size) -> TRUE (positive) / FALSE.
fn auto_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, body, pos, neg, pf, nf) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3), cpu.dreg(0), cpu.dreg(1));
    if neg == 0 || body == 0 { cpu.set_d0(0); return Ok("[intuition] AutoRequest without body or negative text -> FALSE (BuildSysRequest returns NULL)".into()); }
    let (ui_win, win_title) = ref_window(a, "AutoRequest", win)?;
    let m = sys_model(a, win_title, body, pos, neg)?;
    let what = format!("[intuition] AutoRequest {:?} {:?}", m.text, m.buttons);
    let (nb, ui) = (m.buttons.len() as u32, a.ask(ui_win, m)?);
    let k = SYNC_KEY_BASE + { a.requests.next_key += 1; a.requests.next_key };
    a.requests.pending.insert(k, Pending { ui, owner: a.sched.cur, buttons: nb, kind: Kind::Auto { pos_flag: pf }, idcmp: (pf | nf) & !IDCMP_VANILLAKEY, out_ptr: 0, window: None, answer: None, taken: false });
    exec_wait::block_on(a, cpu, Waiting::Request(k), what)
}

/// The requester `struct Window` of an asynchronous request: a UserPort of the calling task and one gadget per button.
fn open_requester_window<U: GuestUi>(a: &mut Amiga<U>, buttons: u32, idcmp_flags: u32) -> Result<(u32, Vec<u32>), Refusal> {
    let win = crate::memory::alloc_zeroed(a, crate::window::WINDOW_SIZE, "requester Window")?;
    let m = Refusal::mem("requester Window");
    a.mem.write_u32(win + idcmp::WD_IDCMP, IDCMP_GADGETUP | IDCMP_RAWKEY | (idcmp_flags & !IDCMP_VANILLAKEY)).map_err(&m)?;
    a.mem.write_u32(win + WD_TITLE, 0).map_err(&m)?;
    idcmp::create_window_port(a, win)?;
    idcmp::create_user_port(a, win)?;
    let mut gadgets = Vec::new();
    for i in 0..buttons {
        let g = crate::memory::alloc_zeroed(a, GADGET_SIZE, "requester gadget")?;
        a.mem.write_u16(g + GA_TYPE, GTYP_BOOLGADGET).map_err(&m)?;
        a.mem.write_u16(g + GA_ID, button_value(buttons, i) as u16).map_err(&m)?;
        if let Some(&prev) = gadgets.last() { a.mem.write_u32(prev + GA_NEXT, g).map_err(&m)?; }
        gadgets.push(g);
    }
    Ok((win, gadgets))
}

fn start_async<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, ui_win: Option<guestui_abi::UiObjectRef>, m: RequestModel, idcmp_flags: u32, what: String) -> Result<String, Refusal> {
    let nb = m.buttons.len() as u32;
    let ui = a.ask(ui_win, m)?;
    let (win, gadgets) = match open_requester_window(a, nb, idcmp_flags) {
        Ok(w) => w,
        Err(e) => { let _ = a.ui.close_request(ui); return Err(e); }
    };
    a.requests.pending.insert(win, Pending { ui, owner: a.sched.cur, buttons: nb, kind: Kind::Easy, idcmp: idcmp_flags & !IDCMP_VANILLAKEY, out_ptr: 0, window: Some((win, gadgets)), answer: None, taken: false });
    cpu.set_d0(win);
    Ok(format!("{what} -> requester {win:#x}"))
}

/// BuildEasyRequestArgs(a0 = window, a1 = EasyStruct, d0 = IDCMP flags, a3 = arguments) -> the requester window, or 0.
fn build_easy_request_args<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, es, flags, args) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0), cpu.areg(3));
    if es == 0 { cpu.set_d0(0); return Ok("[intuition] BuildEasyRequestArgs NULL EasyStruct -> 0".into()); }
    let (ui_win, win_title) = ref_window(a, "BuildEasyRequestArgs", win)?;
    let m = easy_model(a, win_title, es, args)?;
    let what = format!("[intuition] BuildEasyRequestArgs {:?} {:?}", m.text, m.buttons);
    start_async(a, cpu, ui_win, m, flags, what)
}

/// BuildSysRequest(a0 = window, a1 = body, a2 = positive, a3 = negative, d0 = IDCMP flags, d2/d3 = size) -> the requester window, or 0.
fn build_sys_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, body, pos, neg, flags) = (cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3), cpu.dreg(0));
    if neg == 0 || body == 0 { cpu.set_d0(0); return Ok("[intuition] BuildSysRequest without body or negative text -> NULL".into()); }
    let (ui_win, win_title) = ref_window(a, "BuildSysRequest", win)?;
    let m = sys_model(a, win_title, body, pos, neg)?;
    let what = format!("[intuition] BuildSysRequest {:?} {:?}", m.text, m.buttons);
    start_async(a, cpu, ui_win, m, flags, what)
}

/// SysReqHandler(a0 = requester, a1 = IDCMP flags pointer, d0 = WaitInput) -> -2 pending, -1 IDCMP, 0 rightmost, n the n-th button.
/// A NULL requester answers 0 and (struct Window *)1 answers 1 (BuildSysRequest's "no window could be opened" results).
fn sys_req_handler<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (w, out, wait) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) != 0);
    if w == 0 || w == 1 { cpu.set_d0(w); return Ok(format!("[intuition] SysReqHandler {w:#x} -> {w}")); }
    let Some(p) = a.requests.pending.get_mut(&w) else { return Err(Refusal(format!("SysReqHandler: {w:#x} is not a requester BuildEasyRequestArgs / BuildSysRequest returned"))) };
    if p.window.is_none() { return Err(Refusal(format!("SysReqHandler: {w:#x} is a synchronous request"))); }
    p.out_ptr = out;
    let (taken, answered) = (p.taken, p.answer.is_some());
    let what = format!("[intuition] SysReqHandler {w:#x}");
    if answered && !taken { let v = a.request_done(w, true)?.expect("answered"); cpu.set_d0(v); return Ok(format!("{what} -> {}", v as i32)); }
    if !wait { cpu.set_d0(-2i32 as u32); return Ok(format!("{what} -> -2 (not answered yet)")); }
    if taken { return Err(Refusal(format!("SysReqHandler: waiting on requester {w:#x}, which delivered its answer already (it could never wake)"))); }
    exec_wait::block_on(a, cpu, Waiting::Request(w), what)
}

/// FreeSysRequest(a0 = requester): closes it (an unanswered one is withdrawn from the host) and gives its window back.
fn free_sys_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let w = cpu.areg(0);
    let _ = cpu;
    if w == 0 || w == 1 { return Ok(format!("[intuition] FreeSysRequest {w:#x}")); }
    let Some(p) = a.requests.pending.get(&w) else { return Err(Refusal(format!("FreeSysRequest: {w:#x} is not a requester BuildEasyRequestArgs / BuildSysRequest returned"))) };
    let (ui, open, gadgets) = (p.ui, p.answer.is_none(), p.window.as_ref().map(|(_, g)| g.clone()).unwrap_or_default());
    if open { let _ = a.ui.close_request(ui); }
    let port = a.mem.read_u32(w + idcmp::WD_USERPORT).map_err(Refusal::mem("Window.UserPort"))?;
    a.drop_window_msgs(w, port)?;
    idcmp::free_user_port(a, w)?;
    idcmp::free_window_port(a, w)?;
    for g in gadgets { a.heap.free(g, GADGET_SIZE).map_err(|e| Refusal(format!("FreeSysRequest: {e}")))?; }
    a.heap.free(w, crate::window::WINDOW_SIZE).map_err(|e| Refusal(format!("FreeSysRequest: {e}")))?;
    a.requests.pending.remove(&w);
    Ok(format!("[intuition] FreeSysRequest {w:#x}{}", if open { " (withdrawn before it was answered)" } else { "" }))
}

/// Deliver a `RequestAnswered` host event.
pub(crate) fn deliver<U: GuestUi>(a: &mut Amiga<U>, ev: &GuestUiEvent) -> Result<bool, Refusal> {
    match ev {
        GuestUiEvent::RequestAnswered { req, button } => { a.request_answered(*req, *button)?; Ok(true) }
        _ => Ok(false),
    }
}
