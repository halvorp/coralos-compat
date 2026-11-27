//! Stateless `asl.library` adapter over CoralOS's native file requester (`GuestUi::open_file_request`, REUSE U9).
//!
//! `AllocAslRequest(d0 = type, a0 = tags)` allocates a real guest `struct FileRequester` (56 bytes;
//! `fr_File` 4, `fr_Drawer` 8, geometry 22..28, `fr_UserData` 40) or `ScreenModeRequester` (48 bytes).
//! `AslRequest(a0 = request, a1 = tags)` for `ASL_FileRequest` asks the host via `GuestUi::open_file_request`
//! (backed on CoralOS by `coral-filereq` + `gadget.library`) and parks the calling task on
//! `Waiting::FileRequest` until `GuestUiEvent::FileRequestAnswered` arrives:
//! - `Some(path)` -> splits `(drawer, file)`, writes NUL-terminated strings into guest heap at `fr_Drawer` /
//!   `fr_File`, and returns `1` (`TRUE`).
//! - `None` (Cancel) or `UiRefusal::Unsupported` -> returns `0` (`FALSE`), matching AmigaOS Cancel.
//! - `FreeAslRequest(a0)` frees the requester struct and any host-allocated `fr_Drawer`/`fr_File` C strings.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::cpu::Cpu;
use crate::exec_wait::{self, Waiting};
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::{tags, Refusal};
use heirloom_ui_abi::{FileRequestMode, FileRequestModel, GuestUi, GuestUiEvent, RequestRef, UiRefusal};

const ASL_TB: u32 = tags::TAG_USER + 0x80000;
const ASLFR_TITLE_TEXT: u32 = ASL_TB + 1;
const ASLFR_WINDOW: u32 = ASL_TB + 2;
const ASLFR_INITIAL_FILE: u32 = ASL_TB + 8;
const ASLFR_INITIAL_DRAWER: u32 = ASL_TB + 9;
const ASLFR_DO_SAVE_MODE: u32 = ASL_TB + 44;
const ASLFR_DRAWERS_ONLY: u32 = ASL_TB + 47;
const ASLFR_USER_DATA: u32 = ASL_TB + 52;

const FR_SIZE: u32 = 56;
const FR_FILE_OFF: u32 = 4;
const FR_DRAWER_OFF: u32 = 8;
const SM_SIZE: u32 = 48;
/// Default CyberGraphX 8-bit chunky RTG DisplayID returned by `ASL_ScreenModeRequest`.
pub const DEFAULT_RTG_DISPLAY_ID: u32 = 0x5000_1000;

const MAX_TAG_STRING: usize = 1024;

#[derive(Clone)]
struct FileReqMeta {
    title: String,
    initial_dir: String,
    initial_file: String,
    mode: FileRequestMode,
    window: u32,
    /// Guest heap allocations `(addr, len)` backing `fr_File` and `fr_Drawer` so we free them cleanly.
    file_buf: Option<(u32, u32)>,
    drawer_buf: Option<(u32, u32)>,
}

#[derive(Clone)]
enum ReqKind {
    File(FileReqMeta),
    ScreenMode,
}

struct PendingFileReq {
    ui: RequestRef,
    owner: u32,
    requester_ptr: u32,
    drawers_only: bool,
    answer: Option<Option<String>>,
}

#[derive(Default)]
pub struct State {
    requests: BTreeMap<u32, ReqKind>,
    pending: BTreeMap<u32, PendingFileReq>,
    next_key: u32,
}

impl State {
    pub fn has_pending_file_request(&self) -> bool {
        self.pending.values().any(|p| p.answer.is_none())
    }
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocAslRequest", alloc_request::<U>)
        .named("FreeAslRequest", free_request::<U>)
        .named("AslRequest", asl_request::<U>)
}

fn read_opt_tag_str<U: GuestUi>(a: &Amiga<U>, t: &[(u32, u32)], tag: u32) -> Result<Option<String>, Refusal> {
    let Some(ptr) = tags::get(t, tag).filter(|&v| v != 0) else { return Ok(None) };
    let bytes = crate::guest_str::read_bytes_z(&a.mem, ptr, MAX_TAG_STRING).map_err(Refusal::mem("ASL tag string"))?;
    Ok(Some(bytes.into_iter().map(|b| b as char).collect()))
}

fn apply_file_tags<U: GuestUi>(a: &Amiga<U>, meta: &mut FileReqMeta, t: &[(u32, u32)]) -> Result<(), Refusal> {
    if let Some(s) = read_opt_tag_str(a, t, ASLFR_TITLE_TEXT)? {
        meta.title = s;
    }
    if let Some(s) = read_opt_tag_str(a, t, ASLFR_INITIAL_DRAWER)? {
        meta.initial_dir = s;
    }
    if let Some(s) = read_opt_tag_str(a, t, ASLFR_INITIAL_FILE)? {
        meta.initial_file = s;
    }
    if let Some(w) = tags::get(t, ASLFR_WINDOW) {
        meta.window = w;
    }
    if let Some(v) = tags::get(t, ASLFR_DRAWERS_ONLY) {
        if v != 0 {
            meta.mode = FileRequestMode::DrawersOnly;
        }
    }
    if let Some(v) = tags::get(t, ASLFR_DO_SAVE_MODE) {
        if v != 0 && meta.mode != FileRequestMode::DrawersOnly {
            meta.mode = FileRequestMode::Save;
        } else if v == 0 && meta.mode == FileRequestMode::Save {
            meta.mode = FileRequestMode::Open;
        }
    }
    Ok(())
}

/// Split a chosen CoralOS or Amiga path into `(drawer, file)`.
pub fn split_drawer_and_file(path: &str, drawers_only: bool) -> (String, String) {
    if drawers_only {
        return (String::from(path), String::new());
    }
    if let Some(i) = path.rfind('/') {
        let dir = &path[..i];
        let file = &path[i + 1..];
        (String::from(dir), String::from(file))
    } else if let Some(i) = path.rfind(':') {
        (String::from(&path[..=i]), String::from(&path[i + 1..]))
    } else {
        (String::new(), String::from(path))
    }
}

fn alloc_cstr<U: GuestUi>(a: &mut Amiga<U>, s: &str) -> Result<Option<(u32, u32)>, Refusal> {
    let mut bytes: Vec<u8> = s.chars().map(|c| if (c as u32) <= 0xFF { c as u8 } else { b'?' }).collect();
    bytes.push(0);
    let len = bytes.len() as u32;
    let Some(ptr) = a.heap.alloc(len) else { return Ok(None) };
    a.mem.write_bytes(ptr, &bytes).map_err(Refusal::mem("FileRequester string"))?;
    Ok(Some((ptr, len)))
}

fn free_meta_bufs<U: GuestUi>(a: &mut Amiga<U>, meta: &mut FileReqMeta) -> Result<(), Refusal> {
    if let Some((addr, len)) = meta.file_buf.take() {
        a.heap.free(addr, len).map_err(|e| Refusal(format!("ASL fr_File free: {e}")))?;
    }
    if let Some((addr, len)) = meta.drawer_buf.take() {
        a.heap.free(addr, len).map_err(|e| Refusal(format!("ASL fr_Drawer free: {e}")))?;
    }
    Ok(())
}

pub(crate) fn alloc_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, list) = (cpu.dreg(0), cpu.areg(0));
    match ty {
        0 => {
            let t = tags::walk(&a.mem, list)?;
            let Some(p) = a.heap.alloc(FR_SIZE) else {
                cpu.set_d0(0);
                return Ok("[asl] AllocAslRequest -> NULL (no memory)".into());
            };
            a.mem.zero(p, FR_SIZE).map_err(Refusal::mem("FileRequester"))?;
            let m = Refusal::mem("FileRequester");
            for (tag, off) in [(ASL_TB + 3, 22u32), (ASL_TB + 4, 24), (ASL_TB + 5, 26), (ASL_TB + 6, 28)] {
                if let Some(v) = tags::get(&t, tag) {
                    a.mem.write_u16(p + off, v as u16).map_err(&m)?;
                }
            }
            if let Some(v) = tags::get(&t, ASLFR_USER_DATA) {
                a.mem.write_u32(p + 40, v).map_err(&m)?;
            }
            let mut meta = FileReqMeta {
                title: String::new(),
                initial_dir: String::new(),
                initial_file: String::new(),
                mode: FileRequestMode::Open,
                window: 0,
                file_buf: None,
                drawer_buf: None,
            };
            apply_file_tags(a, &mut meta, &t)?;
            a.asl.requests.insert(p, ReqKind::File(meta));
            cpu.set_d0(p);
            Ok(format!("[asl] AllocAslRequest ASL_FileRequest -> {p:#x}"))
        }
        2 => {
            let t = tags::walk(&a.mem, list)?;
            let Some(p) = a.heap.alloc(SM_SIZE) else {
                cpu.set_d0(0);
                return Ok("[asl] AllocAslRequest -> NULL (no memory)".into());
            };
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
            if let Some(v) = tags::get(&t, ASLFR_USER_DATA) {
                a.mem.write_u32(p + 44, v).map_err(&m)?;
            }
            a.asl.requests.insert(p, ReqKind::ScreenMode);
            cpu.set_d0(p);
            Ok(format!("[asl] AllocAslRequest ASL_ScreenModeRequest -> {p:#x} ({w}x{h}x{d} id={id:#x})"))
        }
        _ => Err(Refusal(format!("AllocAslRequest type {ty}: only ASL_FileRequest (0) and ASL_ScreenModeRequest (2) are modelled (1 = font requester)"))),
    }
}

pub(crate) fn free_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    if p == 0 {
        return Ok("[asl] FreeAslRequest NULL".into());
    }
    let Some(kind) = a.asl.requests.remove(&p) else {
        return Err(Refusal(format!("FreeAslRequest: {p:#x} is not a request AllocAslRequest returned")));
    };
    let sz = match kind {
        ReqKind::File(mut meta) => {
            free_meta_bufs(a, &mut meta)?;
            FR_SIZE
        }
        ReqKind::ScreenMode => SM_SIZE,
    };
    a.heap.free(p, sz).map_err(|e| Refusal(format!("FreeAslRequest: {e}")))?;
    Ok(format!("[asl] FreeAslRequest {p:#x}"))
}

pub(crate) fn asl_request<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (p, list) = (cpu.areg(0), cpu.areg(1));
    let Some(kind_clone) = a.asl.requests.get(&p).cloned() else {
        return Err(Refusal(format!("AslRequest: {p:#x} is not a request AllocAslRequest returned")));
    };
    match kind_clone {
        ReqKind::File(mut meta) => {
            let t = tags::walk(&a.mem, list)?;
            apply_file_tags(a, &mut meta, &t)?;
            let ui_win = if meta.window != 0 { a.windows.get(&meta.window).copied() } else { None };
            let model = FileRequestModel {
                title: meta.title.clone(),
                mode: meta.mode,
                initial_dir: meta.initial_dir.clone(),
                initial_file: meta.initial_file.clone(),
            };
            let drawers_only = meta.mode == FileRequestMode::DrawersOnly;
            // Save updated tags on the handle.
            if let Some(ReqKind::File(slot)) = a.asl.requests.get_mut(&p) {
                slot.title = meta.title;
                slot.initial_dir = meta.initial_dir;
                slot.initial_file = meta.initial_file;
                slot.mode = meta.mode;
                slot.window = meta.window;
            }
            match a.ui.open_file_request(ui_win, model.clone()) {
                Ok(ui_req) => {
                    a.asl.next_key += 1;
                    let k = a.asl.next_key;
                    a.asl.pending.insert(
                        k,
                        PendingFileReq {
                            ui: ui_req,
                            owner: a.sched.cur,
                            requester_ptr: p,
                            drawers_only,
                            answer: None,
                        },
                    );
                    exec_wait::block_on(
                        a,
                        cpu,
                        Waiting::FileRequest(k),
                        format!("[asl] AslRequest {p:#x} {:?} {:?}", model.mode, model.title),
                    )
                }
                Err(UiRefusal::Unsupported(_)) => {
                    cpu.set_d0(0);
                    Ok(format!("[asl] AslRequest {p:#x} -> FALSE (no requester UI: the same answer as Cancel)"))
                }
                Err(other) => Err(Refusal::ui(other)),
            }
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

/// Deliver `GuestUiEvent::FileRequestAnswered` to the parked ASL file request.
pub fn deliver<U: GuestUi>(a: &mut Amiga<U>, ev: &GuestUiEvent) -> Result<bool, Refusal> {
    let GuestUiEvent::FileRequestAnswered { req, path } = ev else { return Ok(false) };
    let Some((&k, p)) = a.asl.pending.iter().find(|(_, p)| p.ui == *req) else {
        return Err(Refusal(format!("FileRequestAnswered: {req:?} is not an open file request (stale or never asked)")));
    };
    if p.answer.is_some() {
        return Err(Refusal(format!("FileRequestAnswered: {req:?} was answered already")));
    }
    let owner = p.owner;
    a.asl.pending.get_mut(&k).expect("pending").answer = Some(path.clone());
    a.wake(owner)?;
    Ok(true)
}

impl<U: GuestUi> Amiga<U> {
    /// Scheduler check for `Waiting::FileRequest(k)`: writes `fr_Drawer` / `fr_File` on `commit` and returns `1` (OK) or `0` (Cancel).
    pub(crate) fn asl_file_request_done(&mut self, k: u32, commit: bool) -> Result<Option<u32>, Refusal> {
        let Some(p) = self.asl.pending.get(&k) else {
            return Err(Refusal(format!("a task waits on file request {k:#x}, which no longer exists")));
        };
        let Some(ans) = p.answer.clone() else { return Ok(None) };
        let req_ptr = p.requester_ptr;
        let drawers_only = p.drawers_only;
        if !commit {
            return Ok(Some(u32::from(ans.is_some())));
        }
        self.asl.pending.remove(&k);
        let Some(chosen) = ans else {
            return Ok(Some(0));
        };
        let (drawer, file) = split_drawer_and_file(&chosen, drawers_only);
        // Free any previous result strings on this FileRequester handle before allocating new ones.
        if let Some(ReqKind::File(mut meta)) = self.asl.requests.remove(&req_ptr) {
            free_meta_bufs(self, &mut meta)?;
            let d_buf = alloc_cstr(self, &drawer)?;
            let f_buf = alloc_cstr(self, &file)?;
            let d_ptr = d_buf.map_or(0, |(addr, _)| addr);
            let f_ptr = f_buf.map_or(0, |(addr, _)| addr);
            self.mem.write_u32(req_ptr + FR_DRAWER_OFF, d_ptr).map_err(Refusal::mem("fr_Drawer"))?;
            self.mem.write_u32(req_ptr + FR_FILE_OFF, f_ptr).map_err(Refusal::mem("fr_File"))?;
            meta.drawer_buf = d_buf;
            meta.file_buf = f_buf;
            meta.initial_dir = drawer;
            meta.initial_file = file;
            self.asl.requests.insert(req_ptr, ReqKind::File(meta));
        }
        Ok(Some(1))
    }
}
