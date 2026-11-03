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
//! NAMED LIMITS: one context current at a time; `aglSetDrawable` takes the port's `portRect` for the size; `aglSetFullScreen` is the
//! games-windowed rule (the context's drawable becomes a window of the asked size, the display mode is never changed); the renderer
//! queries describe ONE renderer, the Heirloom GPU renderer (properties without a true answer are GL_FALSE); `glmSetMode` / `glmSetFunc`
//! are recorded and have no effect (the GL's memory is heirloom-gl.engine's own). The GL 1.0-1.2 core is served in families: the regular calls (`gl_c/generic.rs`, from the generated
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
mod hot;
mod image;
mod state;

const GL_INVALID_ENUM: u32 = 0x0500;
const GL_INVALID_VALUE: u32 = 0x0501;
const GL_INVALID_OPERATION: u32 = 0x0502;
const AGL_BAD_CONTEXT: u32 = 10000;
const AGL_BAD_DRAWABLE: u32 = 10001;
/// AGL.h renderer properties this engine answers (`aglDescribeRenderer`): AGL_RENDERER_ID 70, AGL_ACCELERATED 73, AGL_WINDOW 80,
/// AGL_VIDEO_MEMORY 120, AGL_TEXTURE_MEMORY 121, AGL_RENDERER_COUNT 128 (Apple's AGL.h numbering).
const AGL_RENDERER_ID: u32 = 70; const AGL_ACCELERATED: u32 = 73; const AGL_WINDOW: u32 = 80; const AGL_FULLSCREEN: u32 = 54;
const AGL_VIDEO_MEMORY: u32 = 120; const AGL_TEXTURE_MEMORY: u32 = 121; const AGL_RENDERER_COUNT: u32 = 128;
/// The one renderer's info handle (opaque, odd: never a guest pointer).
const RENDERER_INFO: u32 = 0x0053_0001;
/// The Heirloom GPU renderer's id: no Apple-assigned id names it, so it is one OUTSIDE Apple's table (whose hardware ids are 0x0002xxxx).
pub const HEIRLOOM_RENDERER_ID: u32 = 0x0007_0001;
/// The GL memory the renderer reports as its video and texture memory: heirloom-gl.engine's per-launch texture budget, 64 MiB.
pub const GL_TEXTURE_BUDGET: u32 = 64 << 20;
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
    /// `ctxs[current]`, kept by `gl_ctx_changed` (the per-call path reads it instead of the map).
    current_token: Option<Token>,
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
    /// Per contract entry: its generated family signature (`heirloom_gl_abi::family::lookup` of its name), made once at the
    /// first call - the per-call path indexes it instead of scanning the 259-row table by name.
    sigs: Vec<Option<&'static heirloom_gl_abi::family::Sig>>,
    /// Per contract entry: how `gl_call` dispatches it (`hot::Hot`), made once at the first call.
    routes: Vec<hot::Hot>,
    /// Per contract entry: [`MacPpc::gl_plain`], made once at the first call.
    plain: Vec<bool>,
    /// The draw being captured, the array snapshots it does not use now, and its enabled arrays' descriptors: buffers reused draw
    /// after draw (`gl_queue_draw`), so a frame's draws allocate nothing once warm.
    draw: DrawCommand,
    spare: Vec<heirloom_gl_abi::snapshot::ArraySnapshot>,
    descs: Vec<Descriptor>,
    /// The argument words of the call being encoded: one buffer reused call after call ([`MacPpc::gl_words`]), not a `Vec` each.
    words: Vec<u64>,
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

    fn gl_queue(&mut self, token: Token, command: Command) { Frame { token, command }.encode_into(&mut self.gl.pending); }

    /// The (empty) argument buffer for one call: fill it, then hand it back through [`MacPpc::gl_queue_call`] or [`MacPpc::gl_words_back`].
    pub(super) fn gl_words(&mut self) -> Vec<u64> { let mut w = core::mem::take(&mut self.gl.words); w.clear(); w }
    pub(super) fn gl_words_back(&mut self, w: Vec<u64>) { self.gl.words = w; }

    /// Queue `Command::Call { entry, args: w }` without building the command, and keep `w`'s buffer for the next call.
    pub(super) fn gl_queue_call(&mut self, token: Token, entry: u16, w: Vec<u64>) {
        Frame::encode_call_into(&mut self.gl.pending, token, entry, &w);
        self.gl.words = w;
    }

    /// Send what is queued as one batch (ending in `BatchEnd`) and return the replies. Retired batches advance the fences.
    fn gl_flush(&mut self) -> Vec<Command> {
        if self.gl.pending.is_empty() { return Vec::new(); }
        let seq = self.gl.batch;
        self.gl.batch += 1;
        self.gl_queue(Token { slot: 0, incarnation: 0 }, Command::BatchEnd { seq });
        let mut req = core::mem::take(&mut self.gl.pending);
        self.gl.sent_bytes += req.len() as u64;
        let Some(w) = self.gl.worker.as_mut() else { return Vec::new() };
        let t0 = self.perf.as_ref().map(|p| p.now());
        let reply = w.transact(&req);
        // the next batch reuses this one's capacity (a gameplay frame is ~470 KB: no regrowth from zero every flush)
        req.clear();
        self.gl.pending = req;
        if let (Some(t0), Some(p)) = (t0, self.perf.as_mut()) { let dt = p.now().wrapping_sub(t0); p.add_gl_ring(dt); }
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

    /// Whether `entry` is PLAIN: implemented (admit_call admits it), a `gl*` call, and not one of the calls the glBegin/glEnd and
    /// display-list state machines act on (glBegin, glEnd, glNewList, glEndList). Outside glBegin/glEnd and list compilation such a
    /// call's begin/end gate always passes, so `gl_call` routes it without the gate's name matching (made once per entry).
    fn gl_plain(&mut self, entry: u16) -> bool {
        if self.gl.plain.is_empty() {
            self.gl.plain = ENTRIES.iter().map(|e| e.implemented && e.name.starts_with("gl") && !matches!(e.name, "glBegin" | "glEnd" | "glNewList" | "glEndList")).collect();
        }
        self.gl.plain.get(usize::from(entry)).copied().unwrap_or(false)
    }

    /// The generated family signature of contract entry `entry`, if it is a family call.
    fn gl_sig(&mut self, entry: u16) -> Option<&'static heirloom_gl_abi::family::Sig> {
        if self.gl.sigs.is_empty() { self.gl.sigs = ENTRIES.iter().map(|e| heirloom_gl_abi::family::lookup(e.name)).collect(); }
        self.gl.sigs.get(usize::from(entry)).copied().flatten()
    }

    /// The current context's token (kept in step with `ctxs` and `current` by [`MacPpc::gl_ctx_changed`]: every GL call asks).
    fn gl_ctx_token(&self) -> Option<Token> { self.gl.current_token }

    /// `ctxs` or `current` changed: the current context's token again.
    fn gl_ctx_changed(&mut self) { self.gl.current_token = self.gl.ctxs.get(&self.gl.current).copied(); }

    /// Record a GL error the way GL does: the flag keeps the FIRST error until `glGetError` reads it (GL 1.2 §2.5).
    fn gl_set_error(&mut self, e: u32) { if self.gl.gl_error == 0 { self.gl.gl_error = e; } }

    fn gl_note(&mut self, line: String) { self.mac.log.push(line); }

    pub(crate) fn gl_call(&mut self, f: GlFn, cpu: &mut Cpu) -> Result<(), String> {
        // the per-frame calls of a plain entry, outside glBegin/glEnd and display-list compilation: straight to their route
        if self.gl_plain(f.0) {
            let inside = self.gl.current_token.is_some() && (self.gl.begin_ctx == Some(self.gl.current) || self.gl.list_ctx == Some(self.gl.current));
            if !inside {
                match self.gl_route(f.0) {
                    hot::Hot::Rest => {}
                    hot::Hot::State => {
                        let r = self.gl_state_call(ENTRIES[usize::from(f.0)].name, f.0, cpu)?;
                        return self.gl_finish(cpu, r);
                    }
                    h => {
                        self.gl_hot_call(h, f.0, ENTRIES[usize::from(f.0)].name, [cpu.gpr[3], cpu.gpr[4], cpu.gpr[5], cpu.gpr[6]])?;
                        return self.gl_finish(cpu, 0);
                    }
                }
            }
        }
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
        // the per-frame calls are dispatched by their entry's route, resolved once - not by comparing the name with every arm below
        match self.gl_route(f.0) {
            hot::Hot::Rest => {}
            hot::Hot::State => { let r = self.gl_state_call(name, f.0, cpu)?; return self.gl_finish(cpu, r); }
            h => { self.gl_hot_call(h, f.0, name, [a0, a1, a2, a3])?; return self.gl_finish(cpu, 0); }
        }
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
                        self.gl_ctx_changed();
                        ret = h;
                    }
                }
            }
            "aglDestroyContext" => match self.gl.ctxs.remove(&a0) {
                Some(t) => { self.gl_queue(t, Command::ContextDestroy); self.gl_flush(); if self.gl.current == a0 { self.gl.current = 0; } self.gl_ctx_changed(); ret = 1; }
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
            },
            "aglSetCurrentContext" => {
                if a0 == 0 { self.gl_queue(none, Command::MakeCurrent); self.gl_flush(); self.gl.current = 0; self.gl_ctx_changed(); ret = 1; }
                else if let Some(t) = self.gl.ctxs.get(&a0).copied() { self.gl_queue(t, Command::MakeCurrent); self.gl.current = a0; self.gl_ctx_changed(); ret = 1; }
                else { self.gl.agl_error = AGL_BAD_CONTEXT; }
            }
            "aglSetDrawable" => match self.gl.ctxs.get(&a0).copied() {
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
                Some(t) if a1 == 0 => { self.gl_queue(t, Command::Call { entry: f.0, args: alloc::vec![0, 0, 0] }); ret = 1; }
                Some(t) => {
                    // CGrafPort / GrafPort: portRect (top, left, bottom, right) at +16
                    let r: Result<[i16; 4], _> = (|| Ok::<_, heirloom_mem_flat32::MemError>([self.mac.mem.read_i16(a1 + 16)?, self.mac.mem.read_i16(a1 + 18)?, self.mac.mem.read_i16(a1 + 20)?, self.mac.mem.read_i16(a1 + 22)?]))();
                    match r {
                        Ok([t0, l, b, rr]) if b > t0 && rr > l => {
                            // ABI 3: the drawable's host window id (the port IS the window here: GetWindowPort returns the window),
                            // so the window owner attaches the picture to exactly that window; 0 = a port that is no shown window.
                            let win = self.mac.windows.get(&a1).and_then(|w| w.native).map_or(0, |n| u64::from(n.id));
                            self.gl_queue(t, Command::Call { entry: f.0, args: alloc::vec![u64::from((rr - l) as u32), u64::from((b - t0) as u32), win] });
                            ret = 1;
                        }
                        _ => self.gl.agl_error = AGL_BAD_DRAWABLE,
                    }
                }
            },
            "aglSwapBuffers" => match self.gl.ctxs.get(&a0).copied() {
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
                Some(t) => { self.gl_queue(t, Command::Swap { drawable: t }); self.gl_flush(); if let Some(p) = self.perf.as_mut() { p.swap(); } }
            },
            "aglSetFullScreen" => match self.gl.ctxs.get(&a0).copied() {
                // aglSetFullScreen(ctx, width, height, freq, device): the games-windowed rule - the drawable becomes a window of that size
                None => self.gl.agl_error = AGL_BAD_CONTEXT,
                Some(_) if a1 == 0 || a2 == 0 || a1 > 8192 || a2 > 8192 => self.gl.agl_error = AGL_BAD_DRAWABLE,
                Some(t) => {
                    let set = GlFn::by_name("aglSetDrawable").ok_or_else(|| String::from("aglSetFullScreen: the contract has no aglSetDrawable"))?;
                    // The window the games-windowed rule shows it in: the DrawSprocket context's window when one is reserved, else none
                    // (0: the window owner attaches nothing and says so).
                    let win = self.mac.windows.get(&self.dsp.window()).and_then(|w| w.native).map_or(0, |n| u64::from(n.id));
                    self.gl_queue(t, Command::Call { entry: set.0, args: alloc::vec![u64::from(a1), u64::from(a2), win] });
                    self.gl_note(format!("[HEIRLOOM-GL FULLSCREEN-AS-WINDOW w={a1} h={a2}] the games-windowed rule: the display mode is not changed"));
                    ret = 1;
                }
            },
            "aglQueryRendererInfo" => { ret = RENDERER_INFO; }
            "aglNextRendererInfo" => { ret = 0; }
            "aglDestroyRendererInfo" => { ret = 0; }
            "aglDescribeRenderer" => {
                // aglDescribeRenderer(rend, prop, GLint *value) -> GLboolean
                if a0 != RENDERER_INFO || a2 == 0 { self.gl.agl_error = AGL_BAD_DRAWABLE; } else {
                    let v = match a1 {
                        AGL_RENDERER_ID => Some(HEIRLOOM_RENDERER_ID),
                        AGL_ACCELERATED | AGL_WINDOW => Some(u32::from(self.gl.has_worker())),
                        AGL_FULLSCREEN => Some(0),
                        AGL_VIDEO_MEMORY | AGL_TEXTURE_MEMORY => Some(GL_TEXTURE_BUDGET),
                        AGL_RENDERER_COUNT => Some(1),
                        _ => None,
                    };
                    match v {
                        Some(v) => { self.mac.mem.write_u32(a2, v).map_err(|_| format!("aglDescribeRenderer: the value at {a2:#x} leaves guest RAM"))?; ret = 1; }
                        None => self.gl_note(format!("[HEIRLOOM-GL RENDERER-PROPERTY {a1} unanswered] no true answer: GL_FALSE")),
                    }
                }
            }
            "glmSetMode" | "glmSetFunc" => self.gl_note(format!("[HEIRLOOM-GL GLM {name}({a0:#x}, {a1:#x})] recorded, no effect: the GL's memory is heirloom-gl.engine's own")),
            "aglGetError" => { ret = core::mem::take(&mut self.gl.agl_error); }
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
            other => {
                // no arm above serves this entry (a pure function of the name: the one guarded arm falls through to another arm), so
                // its later calls go straight to the state calls
                self.gl_route_state(f.0);
                ret = self.gl_state_call(other, f.0, cpu)?;
            }
        }
        // Every AGL answer the engine gives (renderer info, pixel formats, contexts, errors) is said: a game decides its renderer
        // path from these, and nothing else shows what it was told. Swaps and context switches are per frame: not said.
        if name.starts_with("agl") && !matches!(name, "aglSwapBuffers" | "aglSetCurrentContext") {
            let v = if name == "aglDescribeRenderer" && ret == 1 { self.mac.mem.read_u32(a2).map_or(String::new(), |v| format!(" value={v:#x}")) } else { String::new() };
            self.gl_note(format!("[HEIRLOOM-GL AGL {name}({a0:#x}, {a1:#x}, {a2:#x}) -> {ret:#x}{v}]"));
        }
        self.gl_finish(cpu, ret)
    }

    fn gl_finish(&mut self, cpu: &mut Cpu, ret: u32) -> Result<(), String> { cpu.gpr[3] = ret; Ok(()) }
}

/// The string `name` would show in a log (kept for the census).
pub fn describe(f: GlFn) -> String { ENTRIES.get(usize::from(f.0)).map_or_else(|| "?".to_string(), |e| format!("OpenGL {} ({:?})", e.name, e.home)) }
