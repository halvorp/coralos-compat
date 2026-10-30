//! Guest OpenGL (`OpenGLLibrary`, `OpenGLMemory`) as ENCODING ONLY: this module turns the program's AGL/GL calls into
//! `heirloom-gl-abi` frames and hands them to a [`GlWorker`]. It links no Mesa and no GPU API (CORAL-GL-IS-ONE-MESA-WORKER,
//! HEIRLOOM-UI-HAS-ONE-SEAM): the worker is `heirloom-gl.engine`, behind the host's transport.
//!
//! Guarantees (each pinned by `tests/gl_triangle.rs`):
//! * An entry the contract table does not mark implemented is REFUSED BY NAME (`[HEIRLOOM-GL REFUSED call=glFoo]`, the call's
//!   outcome is `Refused`), never skipped.
//! * `gl*Pointer` and `gl{Enable,Disable}ClientState` only record descriptors here (GL reads client arrays at the draw);
//!   `glDrawElements` snapshots the referenced ranges of big-endian guest memory THEN, converted to the wire order, so a
//!   later guest write cannot change a queued draw.
//! * With no worker attached (no GPU acceleration) `aglChoosePixelFormat` returns no format and says
//!   `[HEIRLOOM-GL REFUSED reason=no-gpu-accel]`: the game shows its own "needs OpenGL" alert.
//! * `GL_VERSION` / `GL_EXTENSIONS` are answered HERE from `heirloom_gl_abi::derive` over the same table, never from the
//!   worker's own (Mesa) strings; every `glGetString` is also forwarded so the worker logs it on its ring
//!   (`[HEIRLOOM-GL-GETSTRING name=.. ]`): the first real run tells us which tokens the game asks for.
//! * `APPLE_fence` is real: a fence is complete only after the worker reports the batch it was set in retired.
//! * `APPLE_vertex_array_range` is accepted and validated; because arrays are snapshotted at draw time the hint changes
//!   nothing the guest can observe (NAMED LIMIT: no client-memory aliasing of GPU storage).
//!
//! NAMED LIMITS: one context current at a time; `aglSetDrawable` takes the port's `portRect` for the size; no `aglSetFullScreen` or
//! renderer queries. The GL 1.0-1.2 core is served in families: the regular calls (`gl_c/generic.rs`, from the generated
//! `heirloom_gl_abi::family_table`), the bulk-data calls (`bulk.rs`), client arrays and client state (`arrays.rs`, `client.rs`), client
//! images and bitmaps (`image.rs`), the glBegin/glEnd and glNewList/glEndList state machines (`begin_end.rs`) and the hand-written state
//! calls (`state.rs`). What the specification defines and the stack under the worker cannot render is refused by name
//! (`heirloom_gl_abi::gaps`); `heirloom_gl_abi::contract::core_tally` says how many entries are complete.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::fence::FenceBook;
use heirloom_gl_abi::snapshot::{Descriptor, ElemType, Role};
use heirloom_gl_abi::{admit_call, derive, extensions_string, find, refusal_marker, version_string, Command, DrawCommand, Frame, Token, ENTRIES};
use crate::cpu::Cpu;
use heirloom_gl_abi::pixel::Store;

mod arrays;
mod begin_end;
mod bulk;
mod client;
mod generic;
mod image;
mod state;

const GL_INVALID_ENUM: u32 = 0x0500;
const GL_INVALID_VALUE: u32 = 0x0501;
const GL_INVALID_OPERATION: u32 = 0x0502;
const AGL_BAD_CONTEXT: u32 = 10000;
const AGL_BAD_DRAWABLE: u32 = 10001;
const GL_VENDOR: u32 = 0x1F00;
const GL_RENDERER: u32 = 0x1F01;
const GL_VERSION: u32 = 0x1F02;
const GL_EXTENSIONS: u32 = 0x1F03;
const GL_VERTEX_ARRAY_STORAGE_HINT_APPLE: u32 = 0x851F;
const GL_STORAGE_CACHED_APPLE: u32 = 0x85BE;
const GL_STORAGE_SHARED_APPLE: u32 = 0x85BF;
const GL_VERTEX_ARRAY_RANGE_APPLE: u32 = 0x851D;

/// The host's transport to `heirloom-gl.engine`: send request bytes (whole frames, ending in a `BatchEnd`), get back the
/// reply bytes (whole frames: `Reply`s and `Retired`). A worker that has died returns an empty vector for a request that
/// needs an answer, and the call is refused.
pub trait GlWorker {
    fn transact(&mut self, request: &[u8]) -> Vec<u8>;
}

/// The libraries whose exports are the contract table's names (OpenGL, its memory hooks, GLU).
pub fn provided(l: &str) -> bool { matches!(l, "OpenGLLibrary" | "OpenGLMemory" | "OpenGLUtility") }

/// An imported OpenGL name, by contract-table index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlFn(pub u16);

impl GlFn {
    /// Every name in the contract table is bound (so an unimplemented one is a NAMED refusal at the call, not an unbound import).
    pub fn by_name(name: &str) -> Option<GlFn> { find(ENTRIES, name).map(|i| GlFn(i as u16)) }
}

/// The (type, stride, enabled) of an index or edge-flag array.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct IndexArray {
    ty: u32,
    stride: u32,
    enabled: bool,
    /// The program's pointer (what `glGetPointerv` returns).
    ptr: u32,
}

#[derive(Default)]
pub struct GlState {
    worker: Option<Box<dyn GlWorker>>,
    pending: Vec<u8>,
    batch: u64,
    fences: FenceBook,
    /// fence name (what the program holds) -> the FenceBook id once set.
    fence_names: BTreeMap<u32, Option<u32>>,
    next_fence_name: u32,
    ctxs: BTreeMap<u32, Token>,
    next_ctx: u32,
    current: u32,
    arrays: BTreeMap<(u8, u8), Descriptor>,
    enabled: BTreeSet<(u8, u8)>,
    client_unit: u8,
    gl_error: u32,
    /// The context (its AGL handle) that is between a glBegin and its glEnd.
    begin_ctx: Option<u32>,
    /// `glPushClientAttrib` stack.
    client_stack: Vec<arrays::ClientSave>,
    /// The program's selection and feedback buffers (`glSelectBuffer` / `glFeedbackBuffer`): what `glGetPointerv` returns.
    select_ptr: u32,
    feedback_ptr: u32,
    /// The render mode the program is in (§5: `glRenderMode`); 0 before the first call = GL_RENDER. Leaving GL_SELECT or
    /// GL_FEEDBACK copies the worker's buffer into the program's.
    render_mode: u32,
    /// `glIndexPointer` / `glEdgeFlagPointer`: type and stride as the program set them, and whether the array is enabled.
    index_array: Option<IndexArray>,
    edge_array: Option<IndexArray>,
    /// The context (its AGL handle) that is compiling a display list (between glNewList and glEndList).
    list_ctx: Option<u32>,
    agl_error: u32,
    strings: BTreeMap<u32, (String, u32)>,
    /// `glGetString` queries in order, as `name=0x.. -> value` (the engine-side copy of the worker's ring lines).
    pub query_log: Vec<String>,
    /// `GL_UNPACK_*` / `GL_PACK_*` pixel-storage state as the program set it (the worker also receives every `glPixelStorei`).
    unpack: Store,
    pack: Store,
    /// Bytes handed to the worker so far (a recorder reads this for the ring image).
    pub sent_bytes: u64,
}

impl GlState {
    pub fn attach(&mut self, w: Box<dyn GlWorker>) { self.worker = Some(w); }
    pub fn has_worker(&self) -> bool { self.worker.is_some() }
}

fn role_of(cap: u32) -> Option<Role> {
    Some(match cap { 0x8074 => Role::Vertex, 0x8075 => Role::Normal, 0x8076 => Role::Color, 0x8078 => Role::TexCoord, _ => return None })
}

impl<U: GuestUi> MacPpc<U> {
    /// Attach the worker that executes the program's GL (the host's `heirloom-gl.engine` transport).
    pub fn gl_attach(&mut self, w: Box<dyn GlWorker>) { self.gl.attach(w); }

    fn gl_queue(&mut self, token: Token, command: Command) { self.gl.pending.extend(Frame { token, command }.encode()); }

    /// Send what is queued as one batch (ending in `BatchEnd`) and return the replies. Retired batches advance the fences.
    fn gl_flush(&mut self) -> Vec<Command> {
        if self.gl.pending.is_empty() { return Vec::new(); }
        let seq = self.gl.batch;
        self.gl.batch += 1;
        self.gl_queue(Token { slot: 0, incarnation: 0 }, Command::BatchEnd { seq });
        let req = core::mem::take(&mut self.gl.pending);
        self.gl.sent_bytes += req.len() as u64;
        let Some(w) = self.gl.worker.as_mut() else { return Vec::new() };
        let reply = w.transact(&req);
        let mut out = Vec::new();
        let mut at = 0;
        while at < reply.len() {
            match peek_len(&reply[at..]) {
                Ok(Some(n)) if at + n <= reply.len() => match Frame::decode(&reply[at..at + n]) {
                    Ok((f, used)) => { at += used; if let Command::Retired { seq } = f.command { self.gl.fences.retire_through(seq); } else { out.push(f.command); } }
                    Err(e) => { self.mac.log.push(format!("[HEIRLOOM-GL] worker reply undecodable: {e:?}")); break; }
                },
                other => { self.mac.log.push(format!("[HEIRLOOM-GL] worker reply truncated or from another ABI: {other:?}")); break; }
            }
        }
        out
    }

    /// A request with an answer: queue it, flush, and take the first `Reply`.
    fn gl_ask(&mut self, token: Token, command: Command) -> Option<(u32, Vec<u64>, String)> {
        self.gl_queue(token, command);
        self.gl_flush().into_iter().find_map(|c| if let Command::Reply { status, words, text } = c { Some((status, words, text)) } else { None })
    }

    fn gl_ctx_token(&self) -> Option<Token> { self.gl.ctxs.get(&self.gl.current).copied() }

    /// Record a GL error the way GL does: the flag keeps the FIRST error until `glGetError` reads it (GL 1.2 §2.5).
    fn gl_set_error(&mut self, e: u32) { if self.gl.gl_error == 0 { self.gl.gl_error = e; } }

    fn gl_note(&mut self, line: String) { self.mac.log.push(line); }

    pub(crate) fn gl_call(&mut self, f: GlFn, cpu: &mut Cpu) -> Result<(), String> {
        if let Err(r) = admit_call(ENTRIES, f.0) {
            let m = refusal_marker(&r);
            self.gl_note(m.clone());
            return Err(m);
        }
        let name = ENTRIES[usize::from(f.0)].name;
        let a = |m: &Self, i: usize| arg(m, cpu, i);
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        let none = Token { slot: 0, incarnation: 0 };
        let mut ret: u32 = 0;
        if self.gl_ctx_token().is_some() && !self.gl_begin_end_gate(name, a0 <= 9, a0, a1)? { return self.gl_finish(cpu, 0); }
        match name {
            // ---- AGL ------------------------------------------------------------------------------------------------
            "aglChoosePixelFormat" => {
                ret = if !self.gl.has_worker() {
                    self.gl_note(String::from("[HEIRLOOM-GL REFUSED reason=no-gpu-accel]"));
                    0
                } else {
                    match self.gl_ask(none, Command::Call { entry: f.0, args: Vec::new() }) {
                        Some((0, w, _)) if !w.is_empty() => w[0] as u32,
                        Some((_, _, text)) => { self.gl_note(text); 0 }
                        None => { self.gl_note(String::from("[HEIRLOOM-GL REFUSED reason=worker-gone]")); 0 }
                    }
                };
            }
            "aglDestroyPixelFormat" => { self.gl_queue(none, Command::Call { entry: f.0, args: alloc::vec![u64::from(a0)] }); self.gl_flush(); }
            "aglCreateContext" => {
                if a0 == 0 { self.gl.agl_error = AGL_BAD_CONTEXT; } else if let Some((0, w, _)) = self.gl_ask(none, Command::ContextCreate) {
                    if let [slot, inc] = w[..] {
                        self.gl.next_ctx += 1;
                        let h = self.gl.next_ctx;
                        self.gl.ctxs.insert(h, Token { slot: slot as u32, incarnation: inc as u32 });
                        ret = h;
                    }
                }
            }
            "aglDestroyContext" => match self.gl.ctxs.remove(&a0) {
                Some(t) => { self.gl_queue(t, Command::ContextDestroy); self.gl_flush(); if self.gl.current == a0 { self.gl.current = 0; } ret = 1; }
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
            },
            "aglSetCurrentContext" => {
                if a0 == 0 { self.gl_queue(none, Command::MakeCurrent); self.gl_flush(); self.gl.current = 0; ret = 1; }
                else if let Some(t) = self.gl.ctxs.get(&a0).copied() { self.gl_queue(t, Command::MakeCurrent); self.gl.current = a0; ret = 1; }
                else { self.gl.agl_error = AGL_BAD_CONTEXT; }
            }
            "aglSetDrawable" => match self.gl.ctxs.get(&a0).copied() {
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
                Some(t) if a1 == 0 => { self.gl_queue(t, Command::Call { entry: f.0, args: alloc::vec![0, 0] }); ret = 1; }
                Some(t) => {
                    // CGrafPort / GrafPort: portRect (top, left, bottom, right) at +16
                    let r: Result<[i16; 4], _> = (|| Ok::<_, heirloom_mem_flat32::MemError>([self.mac.mem.read_i16(a1 + 16)?, self.mac.mem.read_i16(a1 + 18)?, self.mac.mem.read_i16(a1 + 20)?, self.mac.mem.read_i16(a1 + 22)?]))();
                    match r {
                        Ok([t0, l, b, rr]) if b > t0 && rr > l => { self.gl_queue(t, Command::Call { entry: f.0, args: alloc::vec![u64::from((rr - l) as u32), u64::from((b - t0) as u32)] }); ret = 1; }
                        _ => self.gl.agl_error = AGL_BAD_DRAWABLE,
                    }
                }
            },
            "aglSwapBuffers" => match self.gl.ctxs.get(&a0).copied() {
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
                Some(t) => { self.gl_queue(t, Command::Swap { drawable: t }); self.gl_flush(); }
            },
            "aglGetError" => { ret = core::mem::take(&mut self.gl.agl_error); }
            // ---- GL state forwarded as calls ------------------------------------------------------------------------
            "glViewport" | "glClear" | "glMatrixMode" | "glEnable" | "glDisable" | "glActiveTextureARB" | "glFinish" => {
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(a0)]) { self.gl_note(m.clone()); return Err(m); }
                let args: Vec<u64> = match name { "glViewport" => alloc::vec![u64::from(a0), u64::from(a1), u64::from(a2), u64::from(a3)], "glFinish" => Vec::new(), _ => alloc::vec![u64::from(a0)] };
                match self.gl_ctx_token() {
                    Some(t) => { self.gl_queue(t, Command::Call { entry: f.0, args }); if name == "glFinish" { self.gl_flush(); } }
                    None => self.gl_set_error(GL_INVALID_OPERATION),
                }
            }
            "glLoadMatrixf" => {
                let mut w = Vec::with_capacity(16);
                for i in 0..16 { w.push(u64::from(self.mac.mem.read_u32(a0.wrapping_add(4 * i)).map_err(|_| format!("glLoadMatrixf: the matrix at {a0:#x} leaves guest RAM"))?)); }
                match self.gl_ctx_token() { Some(t) => self.gl_queue(t, Command::Call { entry: f.0, args: w }), None => self.gl_set_error(GL_INVALID_OPERATION) }
            }
            // ---- client arrays: descriptors only, read at the draw ----------------------------------------------------
            "glEnableClientState" | "glDisableClientState" if matches!(a0, 0x8077 | 0x8079) => {
                let enable = name == "glEnableClientState";
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(a0)]).filter(|_| enable) { self.gl_note(m.clone()); return Err(m); }
                let slot = if a0 == 0x8077 { &mut self.gl.index_array } else { &mut self.gl.edge_array };
                slot.get_or_insert(IndexArray { ty: 0x1406, stride: 0, enabled: false, ptr: 0 }).enabled = enable;
            }
            "glEnableClientState" | "glDisableClientState" => match role_of(a0) {
                Some(r) => {
                    let key = (r as u8, if r == Role::TexCoord { self.gl.client_unit } else { 0 });
                    if name == "glEnableClientState" { self.gl.enabled.insert(key); } else { self.gl.enabled.remove(&key); }
                }
                None => return Err(format!("{name}: client state {a0:#x} is not one of the vertex/normal/colour/texcoord arrays this engine serves")),
            },
            "glClientActiveTextureARB" => { self.gl.client_unit = (a0.wrapping_sub(0x84C0)).min(255) as u8; if a0 < 0x84C0 || a0 > 0x84DF { self.gl_set_error(GL_INVALID_ENUM); self.gl.client_unit = 0; } }
            "glVertexPointer" | "glColorPointer" | "glTexCoordPointer" | "glNormalPointer" => {
                let (role, size, ty, stride, ptr) = match name {
                    "glVertexPointer" => (Role::Vertex, a0, a1, a2, a3),
                    "glColorPointer" => (Role::Color, a0, a1, a2, a3),
                    "glTexCoordPointer" => (Role::TexCoord, a0, a1, a2, a3),
                    _ => (Role::Normal, 3, a0, a1, a2),
                };
                let ok_size = match role { Role::Vertex => (2..=4).contains(&size), Role::Color => size == 3 || size == 4, Role::TexCoord => (1..=4).contains(&size), Role::Normal => true, Role::EdgeFlag => false };
                match ElemType::from_gl(ty).filter(|_| arrays::type_ok(role, ty)) {
                    Some(t) if ok_size && (stride as i32) >= 0 => {
                        let unit = if role == Role::TexCoord { self.gl.client_unit } else { 0 };
                        self.gl.arrays.insert((role as u8, unit), Descriptor { role, unit, offset: self.mac.mem.strip(ptr) as usize, size: size as u8, ty: t, stride });
                    }
                    Some(_) => self.gl_set_error(GL_INVALID_VALUE),
                    None => self.gl_set_error(GL_INVALID_ENUM),
                }
            }
            "glDrawElements" => {
                let (mode, count, ty, ptr) = (a0, a1, a2, a3);
                let Some(t) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return self.gl_finish(cpu, 0); };
                if mode > 9 || (count as i32) < 0 { self.gl_set_error(if mode > 9 { GL_INVALID_ENUM } else { GL_INVALID_VALUE }); return self.gl_finish(cpu, 0); }
                let it = match ElemType::from_gl(ty) { Some(t @ (ElemType::U8 | ElemType::U16 | ElemType::U32)) => t, _ => { self.gl_set_error(GL_INVALID_ENUM); return self.gl_finish(cpu, 0); } };
                let descs: Vec<Descriptor> = self.gl.enabled.iter().filter_map(|k| self.gl.arrays.get(k).copied()).collect();
                if descs.len() != self.gl.enabled.len() { self.gl_set_error(GL_INVALID_OPERATION); return self.gl_finish(cpu, 0); } // enabled with no pointer set
                let off = self.mac.mem.strip(ptr) as usize;
                self.gl_queue_draw(t, "glDrawElements", |mem| DrawCommand::capture(mem, mode, off, count, it, &descs))?;
            }
            "glLockArraysEXT" | "glUnlockArraysEXT" => { /* arrays are read at the draw; the lock's promise (no change) is honoured trivially */ }
            // ---- queries --------------------------------------------------------------------------------------------
            "glGetError" => {
                let engine = core::mem::take(&mut self.gl.gl_error);
                ret = if engine != 0 { engine } else if let Some(t) = self.gl_ctx_token() {
                    self.gl_ask(t, Command::Call { entry: f.0, args: Vec::new() }).and_then(|(_, w, _)| w.first().copied()).unwrap_or(0) as u32
                } else { 0 };
            }
            "glGetString" => {
                let Some(t) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return self.gl_finish(cpu, 0); };
                let worker_text = self.gl_ask(t, Command::Call { entry: f.0, args: alloc::vec![u64::from(a0)] }).map(|x| x.2);
                let caps = derive(ENTRIES, heirloom_gl_abi::caps::ATTESTED_THROUGH);
                let text = match a0 {
                    GL_VERSION => Some(version_string(&caps).unwrap_or_else(|_| String::from("0.0 CoralOS Heirloom GL (no GL version attested)"))),
                    GL_EXTENSIONS => Some(extensions_string(&caps)),
                    GL_VENDOR | GL_RENDERER => worker_text,
                    _ => None,
                };
                match text {
                    None => self.gl_set_error(GL_INVALID_ENUM),
                    Some(s) => {
                        self.gl.query_log.push(format!("glGetString({a0:#x}) -> {s}"));
                        let p = match self.gl.strings.get(&a0) { Some((old, p)) if *old == s => *p, _ => {
                            let mut b = s.clone().into_bytes();
                            b.push(0);
                            let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, b.len() as u32, false).unwrap_or(0);
                            if p != 0 { self.put_bytes(p, &b, "glGetString")?; }
                            self.gl.strings.insert(a0, (s, p));
                            p
                        } };
                        ret = p;
                    }
                }
            }
            // ---- APPLE_fence -----------------------------------------------------------------------------------------
            "glGenFencesAPPLE" => {
                for i in 0..a0 {
                    self.gl.next_fence_name += 1;
                    let n = self.gl.next_fence_name;
                    self.gl.fence_names.insert(n, None);
                    self.mac.mem.write_u32(a1.wrapping_add(4 * i), n).map_err(|_| String::from("glGenFencesAPPLE: the name array leaves guest RAM"))?;
                }
            }
            "glDeleteFencesAPPLE" => {
                for i in 0..a0 {
                    let n = self.mac.mem.read_u32(a1.wrapping_add(4 * i)).map_err(|_| String::from("glDeleteFencesAPPLE: the name array leaves guest RAM"))?;
                    if let Some(Some(id)) = self.gl.fence_names.remove(&n) { let _ = self.gl.fences.delete(id); }
                }
            }
            "glSetFenceAPPLE" => match self.gl.fence_names.get(&a0).copied() {
                None => self.gl_set_error(GL_INVALID_OPERATION),
                Some(old) => {
                    if let Some(o) = old { let _ = self.gl.fences.delete(o); }
                    match self.gl.fences.set(self.gl.batch) {
                        Ok(id) => { self.gl.fence_names.insert(a0, Some(id)); if let Some(t) = self.gl_ctx_token() { self.gl_queue(t, Command::FenceSet { id }); } }
                        Err(_) => return Err(String::from("glSetFenceAPPLE: fence ids are exhausted")),
                    }
                }
            },
            "glIsFenceAPPLE" => { ret = u32::from(self.gl.fence_names.contains_key(&a0)); }
            "glTestFenceAPPLE" | "glFinishFenceAPPLE" | "glTestObjectAPPLE" | "glFinishObjectAPPLE" => {
                let (object, fname) = if name.ends_with("ObjectAPPLE") { (a0, a1) } else { (0, a0) };
                if name.ends_with("ObjectAPPLE") && object != GL_VERTEX_ARRAY_RANGE_APPLE { self.gl_set_error(GL_INVALID_ENUM); return self.gl_finish(cpu, 0); }
                let finish = name.starts_with("glFinish");
                if finish { self.gl_flush(); }
                match self.gl.fence_names.get(&fname).copied() {
                    Some(Some(id)) => ret = u32::from(self.gl.fences.is_complete(id).unwrap_or(false)),
                    _ => { self.gl_set_error(GL_INVALID_OPERATION); }
                }
                if name.ends_with("ObjectAPPLE") { /* the "object" is the vertex array range: its fence is the stream's */ }
            }
            // ---- APPLE_vertex_array_range ------------------------------------------------------------------------------
            "glVertexArrayRangeAPPLE" | "glFlushVertexArrayRangeAPPLE" => { /* validated hint: nothing observable (see the module doc) */ }
            "glVertexArrayParameteriAPPLE" => {
                if a0 != GL_VERTEX_ARRAY_STORAGE_HINT_APPLE || !(a1 == GL_STORAGE_CACHED_APPLE || a1 == GL_STORAGE_SHARED_APPLE || a1 == 0x85BC /* GL_STORAGE_PRIVATE_APPLE */) { self.gl_set_error(GL_INVALID_ENUM); }
            }
            other => ret = self.gl_state_call(other, f.0, cpu)?,
        }
        self.gl_finish(cpu, ret)
    }

    fn gl_finish(&mut self, cpu: &mut Cpu, ret: u32) -> Result<(), String> { cpu.gpr[3] = ret; Ok(()) }
}

/// The string `name` would show in a log (kept for the census).
pub fn describe(f: GlFn) -> String { ENTRIES.get(usize::from(f.0)).map_or_else(|| "?".to_string(), |e| format!("OpenGL {} ({:?})", e.name, e.home)) }
