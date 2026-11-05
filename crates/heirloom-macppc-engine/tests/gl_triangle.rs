//! The PEF triangle fixture: a REAL PowerPC program (assembled with `asm`, run by heirloom-cpu-ppc) imports OpenGLLibrary, makes a
//! context, draws one indexed, vertex-coloured triangle from BIG-ENDIAN guest arrays, mutates those arrays after the draw,
//! and swaps. The engine encodes everything into `heirloom-gl-abi` frames for a worker; here the worker is a mock that
//! obeys the ABI (token table, replies, retirement) and records the exact request bytes, which `RGL_RING_OUT=<file>`
//! writes as a ring image for the C worker's host test (tools/check-heirloom-gl-host.sh in the coralos tree).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::fixture::{app_with, Lib, TOC_FIRST};
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost, Step};
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{Command, Frame, OwnedRing, Token, TokenTable};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use std::sync::{Arc, Mutex};
mod wc3_drive;
mod wc3_heap;

#[global_allocator]
static HEAP: wc3_heap::Switch = wc3_heap::Switch;

type M = MacPpc<RecordingGuestUi>;

/// What the mock saw, shared with the test.
#[derive(Default)]
struct Seen {
    stream: Vec<u8>,
    kinds: Vec<String>,
    draws: Vec<heirloom_gl_abi::DrawCommand>,
    queries: Vec<u64>,
    bad_tokens: usize,
    /// Draws counted, not kept (a long game run).
    draws_seen: usize,
    /// Every aglSetDrawable's arguments, in order (width, height, the drawable's host window id).
    drawables: Vec<Vec<u64>>,
    /// `WC3_GL_ARGS`: (call with its arguments) -> count, for the texture- and state-relevant calls (a diagnostic census).
    args_hist: std::collections::BTreeMap<String, u32>,
    /// The last texture name handed out by glGenTextures (names are fresh and never 0, as GL makes them).
    names: u64,
    /// Swaps seen (a long run counts them here instead of rescanning `kinds`).
    swaps: usize,
}

struct Mock {
    tokens: TokenTable,
    seen: Arc<Mutex<Seen>>,
    gpu: bool,
    /// Keep every request byte (the triangle test's ring image); a long game run keeps none (its textures would fill the host).
    keep: bool,
}

impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> { wc3_heap::outside(|| self.transact_mock(req)) }
}

impl Mock {
    fn transact_mock(&mut self, req: &[u8]) -> Vec<u8> {
        let mut seen = self.seen.lock().unwrap();
        if self.keep { seen.stream.extend_from_slice(req); }
        let mut out = Vec::new();
        let reply = |out: &mut Vec<u8>, status: u32, words: Vec<u64>, text: &str| {
            out.extend(Frame { token: Token { slot: 0, incarnation: 0 }, command: Command::Reply { status, words, text: text.into() } }.encode());
        };
        let mut at = 0;
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            at += n;
            let live = f.token == Token { slot: 0, incarnation: 0 } || self.tokens.check(f.token).is_ok();
            if !live {
                seen.bad_tokens += 1;
            }
            match f.command {
                Command::ContextCreate => {
                    seen.kinds.push("ContextCreate".into());
                    let t = self.tokens.create().unwrap();
                    reply(&mut out, 0, vec![u64::from(t.slot), u64::from(t.incarnation)], "");
                }
                Command::ContextDestroy => {
                    seen.kinds.push("ContextDestroy".into());
                    let _ = self.tokens.destroy(f.token);
                }
                Command::MakeCurrent => seen.kinds.push("MakeCurrent".into()),
                Command::Call { entry, args } => {
                    let name = heirloom_gl_abi::ENTRIES[usize::from(entry)].name;
                    seen.kinds.push(format!("Call {name}"));
                    if name == "aglSetDrawable" {
                        seen.drawables.push(args.clone());
                    }
                    if std::env::var("WC3_GL_ARGS").is_ok() && matches!(name, "glBindTexture" | "glEnable" | "glDisable" | "glTexParameteri" | "glPixelStorei" | "glTexGeni" | "glBlendFunc" | "glAlphaFunc" | "glClearColor" | "glColorMaterial") {
                        *seen.args_hist.entry(format!("{name}{args:x?}")).or_insert(0) += 1;
                    }
                    match name {
                        "aglChoosePixelFormat" if self.gpu => reply(&mut out, 0, vec![1], ""),
                        "glGetString" => {
                            seen.queries.push(args[0]);
                            let renderer = std::env::var("WC3_RENDERER").unwrap_or_else(|_| String::from("Mock GPU"));
                            reply(&mut out, 0, vec![], if args[0] == 0x1F01 { renderer.as_str() } else { "" });
                        }
                        "glGetError" => reply(&mut out, 0, vec![0], ""),
                        // a GL's answer: GL_MAX_TEXTURE_UNITS_ARB is `WC3_TEX_UNITS` (default 8, Mesa's fixed-function units), every
                        // other query its value count of zeros
                        "glGetIntegerv" => {
                            seen.queries.push(args[0]);
                            let n = heirloom_gl_abi::sig::integer_count(args[0] as u32).unwrap_or(0);
                            let units: u64 = std::env::var("WC3_TEX_UNITS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
                            reply(&mut out, 0, if args[0] == 0x84E2 { vec![units] } else { vec![0; n] }, "");
                        }
                        "glGenTextures" => {
                            let first = seen.names + 1;
                            seen.names += args[0];
                            reply(&mut out, 0, (first..=seen.names).collect(), "");
                        }
                        _ => {}
                    }
                }
                Command::Draw(d) => {
                    seen.kinds.push("Draw".into());
                    if self.keep { seen.draws.push(d); } else { seen.draws_seen += 1; }
                }
                Command::CallData { entry, ref args, ref data } => {
                    let name = heirloom_gl_abi::ENTRIES[usize::from(entry)].name;
                    seen.kinds.push(format!("CallData {name}"));
                    if std::env::var("WC3_GL_ARGS").is_ok() {
                        *seen.args_hist.entry(format!("{name}{args:x?} bytes={}", data.len())).or_insert(0) += 1;
                    }
                }
                Command::FenceSet { .. } => seen.kinds.push("FenceSet".into()),
                Command::Swap { .. } => { seen.swaps += 1; seen.kinds.push("Swap".into()) }
                Command::BatchEnd { seq } => out.extend(Frame { token: Token { slot: 0, incarnation: 0 }, command: Command::Retired { seq } }.encode()),
                other => panic!("the engine sent a worker-to-engine frame: {other:?}"),
            }
        }
        out
    }
}

const IMPORTS: &[&str] = &[
    "aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglSetDrawable", "aglSwapBuffers", "aglDestroyContext", "aglDestroyPixelFormat",
    "glViewport", "glMatrixMode", "glLoadMatrixf", "glDisable", "glEnableClientState", "glVertexPointer", "glColorPointer", "glActiveTextureARB",
    "glClientActiveTextureARB", "glLockArraysEXT", "glUnlockArraysEXT", "glGenFencesAPPLE", "glDrawElements", "glSetFenceAPPLE", "glFinishFenceAPPLE",
    "glTestFenceAPPLE", "glVertexArrayRangeAPPLE", "glVertexArrayParameteriAPPLE", "glGetString", "glFinish", "glClear", "glDeleteFencesAPPLE",
];

/// Offsets (from r2) of the program's data after the TOC.
struct Data {
    bytes: Vec<u8>,
    base: i32,
}
impl Data {
    fn put(&mut self, b: &[u8]) -> i32 {
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        let at = self.base + self.bytes.len() as i32;
        self.bytes.extend_from_slice(b);
        at
    }
}

struct Layout {
    ident: i32,
    verts: i32,
    cols: i32,
    idx: i32,
    port: i32,
    fence: i32,
    fence_done: i32,
    ext_ptr: i32,
    ver_ptr: i32,
}

fn li32(a: &mut Asm, r: u32, v: u32) {
    if v < 0x8000 {
        a.li(r, v as i32);
    } else {
        a.lis(r, (v >> 16) as i32).ori(r, r, (v & 0xFFFF) as i32);
    }
}

fn program() -> (Vec<u8>, Vec<u8>, Layout) {
    let base = (TOC_FIRST + 4 * IMPORTS.len() as u32) as i32;
    let mut d = Data { bytes: Vec::new(), base };
    let mut ident = Vec::new();
    for i in 0..16 {
        ident.extend_from_slice(&(if i % 5 == 0 { 1.0f32 } else { 0.0 }).to_be_bytes());
    }
    let ident = d.put(&ident);
    let mut v = Vec::new();
    for x in [-0.9f32, -0.9, 0.0, 0.9, -0.9, 0.0, 0.0, 0.9, 0.0] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    let verts = d.put(&v);
    let cols = d.put(&[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    let mut ix = Vec::new();
    for i in [0u16, 1, 2] {
        ix.extend_from_slice(&i.to_be_bytes());
    }
    let idx = d.put(&ix);
    // a GrafPort whose portRect (+16) is 0,0,64,64
    let mut port = vec![0u8; 16];
    for v in [0i16, 0, 64, 64] {
        port.extend_from_slice(&v.to_be_bytes());
    }
    let port = d.put(&port);
    let (fence, fence_done, ext_ptr, ver_ptr) = (d.put(&[0; 4]), d.put(&[0; 4]), d.put(&[0; 4]), d.put(&[0; 4]));
    let l = Layout { ident, verts, cols, idx, port, fence, fence_done, ext_ptr, ver_ptr };

    let toc = |n: &str| (TOC_FIRST as i32) + 4 * IMPORTS.iter().position(|x| *x == n).unwrap() as i32;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1).stw(31, 60, 1).stw(30, 56, 1);
    a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(31, 3); // fmt
    a.mr(3, 31).li(4, 0).call_toc(toc("aglCreateContext")).mr(30, 3); // ctx
    a.mr(3, 30).call_toc(toc("aglSetCurrentContext"));
    a.mr(3, 30).addi(4, 2, l.port).call_toc(toc("aglSetDrawable"));
    a.li(3, 0).li(4, 0).li(5, 64).li(6, 64).call_toc(toc("glViewport"));
    a.li(3, 0x1701).call_toc(toc("glMatrixMode")).addi(3, 2, l.ident).call_toc(toc("glLoadMatrixf"));
    a.li(3, 0x1700).call_toc(toc("glMatrixMode")).addi(3, 2, l.ident).call_toc(toc("glLoadMatrixf"));
    a.li(3, 0x0B71).call_toc(toc("glDisable")).li(3, 0x0B44).call_toc(toc("glDisable"));
    li32(&mut a, 3, 0x8074);
    a.call_toc(toc("glEnableClientState"));
    li32(&mut a, 3, 0x8076);
    a.call_toc(toc("glEnableClientState"));
    a.li(3, 3).li(4, 0x1406).li(5, 0).addi(6, 2, l.verts).call_toc(toc("glVertexPointer"));
    a.li(3, 4).li(4, 0x1401).li(5, 0).addi(6, 2, l.cols).call_toc(toc("glColorPointer"));
    li32(&mut a, 3, 0x84C0);
    a.call_toc(toc("glActiveTextureARB"));
    li32(&mut a, 3, 0x84C0);
    a.call_toc(toc("glClientActiveTextureARB"));
    a.li(3, 0).li(4, 3).call_toc(toc("glLockArraysEXT"));
    a.li(3, 1).addi(4, 2, l.fence).call_toc(toc("glGenFencesAPPLE"));
    a.li(3, 4).li(4, 3).li(5, 0x1403).addi(6, 2, l.idx).call_toc(toc("glDrawElements"));
    a.lwz(3, l.fence, 2).call_toc(toc("glSetFenceAPPLE"));
    // the guest scribbles over every array AFTER the draw (nine vertex words and three colour words)
    a.li(0, -4370);
    for i in 0..9 {
        a.stw(0, l.verts + 4 * i, 2);
    }
    for i in 0..3 {
        a.stw(0, l.cols + 4 * i, 2);
    }
    a.lwz(3, l.fence, 2).call_toc(toc("glFinishFenceAPPLE"));
    a.lwz(3, l.fence, 2).call_toc(toc("glTestFenceAPPLE")).stw(3, l.fence_done, 2);
    a.li(3, 12).addi(4, 2, l.verts).call_toc(toc("glVertexArrayRangeAPPLE"));
    li32(&mut a, 3, 0x851F);
    li32(&mut a, 4, 0x85BE);
    a.call_toc(toc("glVertexArrayParameteriAPPLE"));
    a.li(3, 0).li(4, 3).call_toc(toc("glUnlockArraysEXT"));
    li32(&mut a, 3, 0x1F03);
    a.call_toc(toc("glGetString")).stw(3, l.ext_ptr, 2);
    li32(&mut a, 3, 0x1F02);
    a.call_toc(toc("glGetString")).stw(3, l.ver_ptr, 2);
    a.call_toc(toc("glFinish"));
    a.mr(3, 30).call_toc(toc("aglSwapBuffers"));
    a.li(3, 0).call_toc(toc("aglSetCurrentContext"));
    a.mr(3, 30).call_toc(toc("aglDestroyContext")).mr(3, 31).call_toc(toc("aglDestroyPixelFormat"));
    a.li(3, 0).lwz(30, 56, 1).lwz(31, 60, 1).addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    (a.finish(), d.bytes, l)
}

fn pef() -> (Vec<u8>, Layout) {
    let (code, tail, l) = program();
    let syms: Vec<(&str, SymbolClass, bool)> = IMPORTS.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    let libs: [Lib; 1] = [("OpenGLLibrary", false, &syms)];
    (app_with(&libs, &code, &tail), l)
}

/// The deferred-call program (gl_c/defer.rs): every record shape - GPR words, a float, two doubles, four floats, a 16-word and a
/// 3-word vector the guest SCRIBBLES right after the call - then 3,000 glEnable calls (more than the queue holds: its routine
/// falls back to the native call), a query and a swap.
const DEFER_IMPORTS: &[&str] = &[
    "aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglSetDrawable", "aglSwapBuffers", "aglDestroyContext", "aglDestroyPixelFormat",
    "glMatrixMode", "glLoadMatrixf", "glNormal3fv", "glAlphaFunc", "glDepthRange", "glClearColor", "glEnable", "glBindTexture", "glTexEnvi",
    "glViewport", "glGetError", "glFinish",
];

fn defer_pef() -> Vec<u8> {
    let base = (TOC_FIRST + 4 * DEFER_IMPORTS.len() as u32) as i32;
    let mut d = Data { bytes: Vec::new(), base };
    let words = |v: &[f32]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    let mat = d.put(&words(&(0..16).map(|i| i as f32 * 0.5).collect::<Vec<_>>()));
    let nrm = d.put(&words(&[0.25, -0.5, 0.75]));
    let fl = d.put(&words(&[0.125, 0.2, 0.4, 0.6, 0.8]));
    let dbl = d.put(&[0.1f64.to_be_bytes(), 0.9f64.to_be_bytes()].concat());
    let mut port = vec![0u8; 16];
    for v in [0i16, 0, 64, 64] { port.extend_from_slice(&v.to_be_bytes()); }
    let port = d.put(&port);
    let err = d.put(&[0xFF; 4]);
    let toc = |n: &str| (TOC_FIRST as i32) + 4 * DEFER_IMPORTS.iter().position(|x| *x == n).unwrap() as i32;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -64, 1).stw(31, 60, 1).stw(30, 56, 1);
    a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(31, 3);
    a.mr(3, 31).li(4, 0).call_toc(toc("aglCreateContext")).mr(30, 3);
    a.mr(3, 30).call_toc(toc("aglSetCurrentContext")).mr(3, 30).addi(4, 2, port).call_toc(toc("aglSetDrawable"));
    a.li(3, 0x1700).call_toc(toc("glMatrixMode")).addi(3, 2, mat).call_toc(toc("glLoadMatrixf"));
    a.addi(3, 2, nrm).call_toc(toc("glNormal3fv"));
    // the guest scribbles the matrix and the normal BEFORE any GL native runs: the records hold the values of the call
    a.li(0, -4370);
    for i in 0..16 { a.stw(0, mat + 4 * i, 2); }
    for i in 0..3 { a.stw(0, nrm + 4 * i, 2); }
    a.li(3, 0x0204).lfs(1, fl, 2).call_toc(toc("glAlphaFunc"));
    a.lfd(1, dbl, 2).lfd(2, dbl + 8, 2).call_toc(toc("glDepthRange"));
    a.lfs(1, fl + 4, 2).lfs(2, fl + 8, 2).lfs(3, fl + 12, 2).lfs(4, fl + 16, 2).call_toc(toc("glClearColor"));
    a.li(3, 0x0DE1).li(4, 7).call_toc(toc("glBindTexture"));
    a.li(3, 0x2300).li(4, 0x2200).li(5, 0x2100).call_toc(toc("glTexEnvi"));
    a.li(3, 1).li(4, 2).li(5, 3).li(6, 4).call_toc(toc("glViewport"));
    // 3,000 records: past the queue's capacity, so later ones take the routine's native path, in order
    a.li(31, 3000).label("more").li(3, 0x0BE2).call_toc(toc("glEnable")).addi(31, 31, -1).cmpwi(31, 0).bne("more");
    a.call_toc(toc("glGetError")).stw(3, err, 2);
    a.call_toc(toc("glFinish")).mr(3, 30).call_toc(toc("aglSwapBuffers"));
    a.li(3, 0).call_toc(toc("aglSetCurrentContext")).mr(3, 30).call_toc(toc("aglDestroyContext")).mr(3, 31).call_toc(toc("aglDestroyPixelFormat"));
    a.li(3, 0).lwz(30, 56, 1).lwz(31, 60, 1).addi(1, 1, 64).lwz(0, 8, 1).mtlr(0).blr();
    let syms: Vec<(&str, SymbolClass, bool)> = DEFER_IMPORTS.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    let libs: [Lib; 1] = [("OpenGLLibrary", false, &syms)];
    app_with(&libs, &a.finish(), &d.bytes)
}

#[test]
fn deferred_gl_calls_reach_the_worker_as_the_same_frames_as_native_calls() {
    let go = |defer: bool| {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.gl_defer(defer);
        m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu: true, keep: true }));
        let frag = m.instantiate("defer", &Pef::parse(&defer_pef()).unwrap()).unwrap();
        m.start(&frag).unwrap();
        assert_eq!(m.step(10_000_000).0, Step::Exited, "{:?}", m.fault());
        let s = std::mem::take(&mut seen.lock().unwrap().stream);
        (m.gl_deferred(), s)
    };
    let ((n_off, off), (n_on, on)) = (go(false), go(true));
    assert_eq!(n_off, 0, "NEGATIVE CONTROL: with the queue off nothing is deferred");
    assert!(n_on > 2000, "the queue served the program's calls ({n_on})");
    assert!(n_on < 3009, "the queue filled and its routine took the native path ({n_on} of 3,009 deferrable calls)");
    assert_eq!(on, off, "the worker receives the same bytes either way");
    // the matrix and the normal are the values at the call, not the scribble that followed it
    let mut at = 0;
    let mut found = 0;
    while at < on.len() {
        let n = peek_len(&on[at..]).unwrap().unwrap();
        if let (Frame { command: Command::Call { entry, args }, .. }, _) = Frame::decode(&on[at..at + n]).unwrap() {
            match heirloom_gl_abi::ENTRIES[usize::from(entry)].name {
                "glLoadMatrixf" => { assert_eq!(args[3], u64::from(1.5f32.to_bits())); found += 1; }
                "glNormal3fv" => { assert_eq!(args, [0.25f32, -0.5, 0.75].map(|x| u64::from(x.to_bits()))); found += 1; }
                "glDepthRange" => { assert_eq!(args, [0.1f64.to_bits(), 0.9f64.to_bits()]); found += 1; }
                _ => {}
            }
        }
        at += n;
    }
    assert_eq!(found, 3);
}

fn run(gpu: bool) -> (M, Arc<Mutex<Seen>>, Layout, u32) {
    let (bytes, l) = pef();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    if gpu {
        m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu, keep: true }));
    }
    let frag = m.instantiate("tri", &Pef::parse(&bytes).unwrap()).unwrap();
    let data = frag.section_bases[1].unwrap();
    m.start(&frag).unwrap();
    assert_eq!(m.step(1_000_000).0, Step::Exited, "{:?}", m.fault());
    (m, seen, l, data)
}

#[test]
fn the_pef_triangle_program_draws_through_the_ring_and_the_snapshot_survives_the_guests_scribbling() {
    let (m, seen, l, data) = run(true);
    let s = seen.lock().unwrap();
    assert_eq!(s.bad_tokens, 0, "every frame carried a live token");
    // the order the worker saw: a context, the drawable, the state, ONE draw, the fence, the swap, teardown
    let k: Vec<&str> = s.kinds.iter().map(String::as_str).collect();
    let pos = |w: &str| k.iter().position(|x| *x == w).unwrap_or_else(|| panic!("{w} missing from {k:?}"));
    assert!(pos("Call aglChoosePixelFormat") < pos("ContextCreate") && pos("ContextCreate") < pos("MakeCurrent"));
    assert!(pos("MakeCurrent") < pos("Call aglSetDrawable") && pos("Call aglSetDrawable") < pos("Draw") && pos("Draw") < pos("FenceSet") && pos("FenceSet") < pos("Swap"));
    assert_eq!(k.iter().filter(|x| **x == "Draw").count(), 1);
    assert!(k.contains(&"Call glGetString") && k.contains(&"Call glActiveTextureARB") && k.contains(&"Call glMatrixMode"));
    assert!(pos("ContextDestroy") < pos("Call aglDestroyPixelFormat") && k.last().copied() == Some("Call aglDestroyPixelFormat"), "teardown is flushed in order");
    // the draw carries the PRE-scribble values, little-endian
    let d = &s.draws[0];
    assert_eq!((d.mode, d.indices.range, d.indices.data.clone()), (4, Some((0, 2)), vec![0, 0, 1, 0, 2, 0]));
    assert_eq!(d.arrays.len(), 2);
    assert_eq!(d.arrays[0].data[..4], (-0.9f32).to_le_bytes());
    assert_eq!(d.arrays[1].data, [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    // NEGATIVE CONTROL: the guest memory really was overwritten, so a late reader would have seen garbage
    assert_eq!(m.mac.mem.read_u32(data + l.verts as u32).unwrap(), 0xFFFF_EEEE);
    // the fence completed (the worker retired the batch), the extension string is the DERIVED one
    assert_eq!(m.mac.mem.read_u32(data + l.fence_done as u32).unwrap(), 1);
    let ext = m.mac.mem.read_u32(data + l.ext_ptr as u32).unwrap();
    assert_eq!(m.cstr(ext, "t").unwrap(), b"GL_ARB_multitexture GL_EXT_compiled_vertex_array GL_APPLE_fence GL_APPLE_vertex_array_range");
    let ver = m.mac.mem.read_u32(data + l.ver_ptr as u32).unwrap();
    assert!(String::from_utf8(m.cstr(ver, "t").unwrap()).unwrap().starts_with("1.2 "), "the table attests GL 1.2");
    // every glGetString query reached the worker (it logs them on its ring) and the engine
    assert_eq!(s.queries, vec![0x1F03, 0x1F02]);
    assert_eq!(m.gl.query_log.len(), 2);
    // hand the recorded ring to the C worker's host test
    if let Ok(path) = std::env::var("RGL_RING_OUT") {
        let ring = OwnedRing::new(s.stream.len().max(4096)).unwrap();
        assert_eq!(ring.ring().push(&s.stream), s.stream.len());
        std::fs::write(path, ring.image()).unwrap();
    }
}

#[test]
fn without_a_worker_there_is_no_pixel_format_and_the_refusal_is_named() {
    let (m, _, _, _) = run(false);
    assert!(m.mac.log.iter().any(|l| l == "[HEIRLOOM-GL REFUSED reason=no-gpu-accel]"), "{:?}", m.mac.log);
}

#[test]
fn an_unimplemented_entry_is_refused_by_name_never_skipped() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let mut regs = [0u32; 32];
    assert!(!matches!(m.bind("gluErrorString"), heirloom_macppc_engine::symbols::Binding::Unbound(_)), "bound, so the call is refused by name");
    match m.glue_call("gluErrorString", &mut regs) {
        GlueOutcome::Refused(s) => assert_eq!(s, "[HEIRLOOM-GL REFUSED call=gluErrorString]"),
        other => panic!("a skipped call: {other:?}"),
    }
    m.bind("glDrawElements");
    assert!(matches!(m.glue_call("glDrawElements", &mut regs), GlueOutcome::Return), "NEGATIVE control: an implemented row is not refused");
}

#[test]
fn a_destroyed_context_handle_is_bad_context_not_a_stale_use() {
    let (mut m, seen, _, _) = run(true);
    let mut regs = [0u32; 32];
    regs[3] = 1; // the handle aglCreateContext returned, destroyed by the program
    m.bind("aglSetCurrentContext");
    assert_eq!(m.glue_call("aglSetCurrentContext", &mut regs), GlueOutcome::Return);
    assert_eq!(regs[3], 0);
    let mut e = [0u32; 32];
    m.bind("aglGetError");
    m.glue_call("aglGetError", &mut e);
    assert_eq!(e[3], 10000, "AGL_BAD_CONTEXT");
    assert_eq!(seen.lock().unwrap().bad_tokens, 0, "and nothing reached the worker with a dead token");
}

/// The GL spinner (slice 5's on-screen witness, `fixture::gl_spinner_app`): a REAL PowerPC application opens a window, makes its
/// AGL drawable that window, and swaps a changing triangle frame after frame - each frame waits in WaitNextEvent, so the
/// frames come at the host's pace. `HEIRLOOM_GL_SPINNER_OUT=<file>` writes the MacBinary for a Z83 test bundle.
#[test]
fn the_gl_spinner_swaps_frame_after_frame_into_its_own_window() {
    use heirloom_common::GuestMachine;
    let app = heirloom_macppc_engine::fixture::gl_spinner_app();
    if let Ok(path) = std::env::var("HEIRLOOM_GL_SPINNER_OUT") {
        std::fs::write(path, &app).unwrap();
    }
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut m = MacPpc::for_program(RecordingGuestUi::new(), &app).unwrap();
    m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu: true, keep: true }));
    m.start_file(&app, "GL Spinner").unwrap();
    let mut t = 0u64;
    for _ in 0..12 {
        let (st, _) = GuestMachine::step(&mut m, 2_000_000);
        assert!(!matches!(st, Step::Fault(_) | Step::Exited), "{st:?} {:?}", m.fault());
        t += 20_000;
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
    }
    let s = seen.lock().unwrap();
    let swaps = s.kinds.iter().filter(|k| *k == "Swap").count();
    assert!(swaps >= 3, "frame after frame: {swaps} swaps, {:?}", &s.kinds[..s.kinds.len().min(40)]);
    assert!(s.kinds.iter().any(|k| k == "Call aglSetDrawable"), "the window is the drawable");
    // The drawable names its window: the third argument is the host window id of the port's window (ABI 3), so the window
    // owner attaches the picture to exactly that window.
    let natives: Vec<u64> = m.mac.windows.values().filter_map(|w| w.native).map(|n| u64::from(n.id)).collect();
    assert!(
        s.drawables.iter().any(|a| a.len() == 3 && a[2] != 0 && natives.contains(&a[2])),
        "aglSetDrawable carries its window's host id: {:?} (windows {natives:?})",
        s.drawables
    );
    assert!(s.draws.len() >= swaps, "every frame draws its triangle");
    let firsts: Vec<u8> = s.draws.iter().filter_map(|d| d.arrays.iter().find(|a| a.size == 4).map(|a| a.data[1])).collect();
    assert!(firsts.windows(2).any(|w| w[0] != w[1]), "the first vertex's colour changes from frame to frame: {firsts:?}");
}
/// Warcraft III's start-up AGL adapters (the 2026-11-01 game call map): ONE renderer described truthfully, aglSetFullScreen presented as
/// a window of that size (the games-windowed rule), the OpenGL Memory hooks recorded. Before this they were refused by name and stopped the game.
#[test]
fn the_renderer_info_fullscreen_as_window_and_memory_hooks_answer_instead_of_stopping_the_game() {
    use heirloom_macppc_engine::cpu::{Cpu, Host};
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu: true, keep: true }));
    let mut call = |m: &mut M, name: &str, args: &[u32]| -> u32 {
        m.bind(name);
        let id = m.glue.intern(&mut m.mac, "CFM", name).unwrap();
        let stub = m.glue.entry(id).unwrap().stub;
        let mut cpu = Cpu::new(0x5000, 0x0004_0000);
        cpu.lr = 0x0005_5554;
        cpu.gpr[3..3 + args.len()].copy_from_slice(args);
        assert!(m.native(&mut cpu, stub));
        assert!(m.fault().is_none(), "{name}: {:?}", m.fault());
        cpu.gpr[3]
    };
    let rend = call(&mut m, "aglQueryRendererInfo", &[0, 0]);
    assert_ne!(rend, 0, "one renderer");
    const OUT: u32 = 0x0003_9000;
    assert_eq!(call(&mut m, "aglDescribeRenderer", &[rend, 121, OUT]), 1, "AGL_TEXTURE_MEMORY");
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 64 << 20, "the worker's per-launch texture budget");
    assert_eq!(call(&mut m, "aglDescribeRenderer", &[rend, 73, OUT]), 1);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 1, "accelerated: a GPU worker is attached");
    // NEGATIVE CONTROL: a property with no true answer is GL_FALSE (never an invented value), the end of the list is NIL
    assert_eq!(call(&mut m, "aglDescribeRenderer", &[rend, 999, OUT]), 0);
    assert_eq!(call(&mut m, "aglNextRendererInfo", &[rend]), 0);
    call(&mut m, "glmSetMode", &[1]);
    call(&mut m, "glmSetFunc", &[1, 0x1234]);
    // aglSetFullScreen on no context is AGL_BAD_CONTEXT; on a context it is the drawable at that size
    assert_eq!(call(&mut m, "aglSetFullScreen", &[0x77, 800, 600, 0, 0]), 0);
    assert_eq!(call(&mut m, "aglGetError", &[]), 10000);
    let pf = call(&mut m, "aglChoosePixelFormat", &[0, 0, 0]);
    let ctx = call(&mut m, "aglCreateContext", &[pf, 0]);
    assert_eq!(call(&mut m, "aglSetFullScreen", &[ctx, 800, 600, 0, 0]), 1);
    assert!(m.mac.log.iter().any(|l| l.starts_with("[HEIRLOOM-GL FULLSCREEN-AS-WINDOW w=800 h=600]")), "named");
    // every AGL answer the engine gives is said, with the value an aglDescribeRenderer wrote (what the game was told)
    assert!(m.mac.log.iter().any(|l| l.starts_with("[HEIRLOOM-GL AGL aglDescribeRenderer(") && l.ends_with("-> 0x1 value=0x4000000]")), "the texture-memory answer is said: {:?}", m.mac.log);
    assert!(m.mac.log.iter().any(|l| l.starts_with("[HEIRLOOM-GL AGL aglSetFullScreen(")), "{:?}", m.mac.log);
    assert!(!m.mac.log.iter().any(|l| l.starts_with("[HEIRLOOM-GL AGL aglSwapBuffers(")), "per-frame calls are not said");
}


/// A HOST RUN of Warcraft III (the owner's disc, never committed: /data/build/corpus-bin) WITH a GL worker, the way the Z83 runs it
/// since guest-GL slice 5: how far it gets, its last log lines and UI calls. Diagnostic, not a gate: `cargo test --features fixtures
/// --test gl_triangle wc3 -- --ignored --nocapture`.
#[test]
#[ignore]
fn wc3_host_run_with_a_gl_worker() {
    use heirloom_common::GuestMachine;
    let Ok(app) = std::fs::read("/data/build/corpus-bin/mac/warcraft3/apps/roc-Warcraft_III.bin") else {
        eprintln!("SKIPPED: the WC3 corpus file is absent");
        return;
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    // `WC3_NO_AUDIO=1`: a host with no audio service, as the CoralOS Heirloom host is today (SndNewChannel answers notEnoughHardware)
    // `WC3_HEAP=lla`: everything from here on allocates through linked_list_allocator, as a CoralOS process does (tests/wc3_heap)
    if std::env::var("WC3_HEAP").as_deref() == Ok("lla") { wc3_heap::enable(8 << 30); }
    let mut heap_at = wc3_heap::counts();
    let mut ui = RecordingGuestUi::new();
    ui.refuse_audio = std::env::var("WC3_NO_AUDIO").is_ok();
    let mut m = MacPpc::for_program(ui, &app).unwrap();
    // `RGL_RING_OUT=<file>`: keep the request stream up to the `WC3_RING_SWAPS`-th swap (default 128) and write it as a ring image
    // for the C worker's host replay (heirloom-gl-host --dump), the way the triangle test hands over its ring.
    let ring_out = std::env::var("RGL_RING_OUT").ok();
    let ring_swaps: usize = std::env::var("WC3_RING_SWAPS").ok().and_then(|s| s.parse().ok()).unwrap_or(128);
    m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu: true, keep: ring_out.is_some() }));
    // The INSTALLED tree the owner-key Installer run exported (`WC3_INSTALLED`, tests/installer.rs INST_EXPORT_DIR): staged as the volume
    // "Macintosh HD" the way the Z83 bundle stages it (every file a stream read on demand), launched from the folder holding War3.mpq.
    // `WC3_DROP_RSRC=1` leaves the resource forks and Finder info out, as a bundle volume does today.
    if let Ok(root) = std::env::var("WC3_INSTALLED") {
        stage_installed(&mut m, std::path::Path::new(&root));
    } else {
    // The installed game folder: War3.mpq (the disc's MacBinary file, its data fork) as a STREAM beside the application.
    let mpq = "/data/build/corpus-bin/mac/warcraft3/roc-fs/Support_War3.mpq.bin";
    if let Ok(f) = std::fs::File::open(mpq) {
        use std::io::Read;
        let mut hdr = [0u8; 128];
        (&f).read_exact(&mut hdr).unwrap();
        let len = u32::from_be_bytes(hdr[83..87].try_into().unwrap()) as u64;
        struct HostFile(std::fs::File);
        impl heirloom_volume::StreamSource for HostFile {
            fn read_at(&self, _locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
                if std::env::var("WC3_TRACE_READS").is_ok() { eprintln!("MPQ read {offset:#x} +{:#x}", dst.len()); }
                std::os::unix::fs::FileExt::read_at(&self.0, dst, 128 + offset).ok()
            }
        }
        let vol = GuestMachine::volume(&mut m);
        let game = vol.add_volume("Warcraft III", true);
        vol.set_stream_source(Box::new(HostFile(f)));
        vol.put_stream(game, "War3.mpq", len, "War3.mpq").unwrap();
        GuestMachine::set_home(&mut m, &["Warcraft III"]).unwrap();
        eprintln!("staged War3.mpq ({len} bytes) as a stream in Warcraft III:");
    }
    }
    // `WC3_GL_DEFER=0`: every GL call is a native call (the deferred-call queue off: the A/B reference, gl_c/defer.rs)
    if std::env::var("WC3_GL_DEFER").as_deref() == Ok("0") { m.gl_defer(false); }
    // `WC3_GL_SHADOW=0`: every GL call is sent, also one that sets what the worker already holds (gl_c/shadow.rs)
    if std::env::var("WC3_GL_SHADOW").as_deref() == Ok("0") { m.gl_shadow(false); }
    m.start_file(&app, "Warcraft III").unwrap();
    // `HEIRLOOM_PPC_TIER=0|1|lockstep|...`: the PowerPC execution tier (heirloom_cpu_ppc::Tier::parse), default tier 1
    // (tier 2, `jit` / `jitlockstep`, runs in this host's executable memory: heirloom_cpu_ppc::HostCodeMem)
    if let Some(t) = std::env::var("HEIRLOOM_PPC_TIER").ok().and_then(|s| heirloom_macppc_engine::cpu::Tier::parse(&s)) {
        m.exec().tier = t;
        // `WC3_KNOBS=<n>`: tier 2's ablation knobs (heirloom_cpu_abi::ppc::KNOB_*: 4 = no translated functions), the host A/B
        if let Some(k) = std::env::var("WC3_KNOBS").ok().and_then(|v| v.parse().ok()) { m.exec().set_jit_knobs(k); }
        // `WC3_CODEMEM_COST=<us per call>,<us per page>`: every publish / withdraw also costs this much (a busy wait), the
        // shape of CoralOS's EXEC_CODE_PROTECT - one syscall per call, one TLB shootdown round per page that changes state
        let mem: Box<dyn heirloom_cpu_ppc::CodeMem> = match std::env::var("WC3_CODEMEM_COST").ok().and_then(|v| v.split_once(',').map(|(a, b)| (a.parse::<u64>().ok(), b.parse::<u64>().ok()))) {
            Some((Some(call), Some(page))) => Box::new(CostedCodeMem::new(call, page)),
            _ => Box::new(heirloom_cpu_ppc::HostCodeMem::new()),
        };
        #[cfg(not(feature = "cpu-library"))]
        m.exec().set_code_mem(mem);
        // through cpu-ppc.library's table (`--no-default-features --features cpu-library,heirloom-cpu-ppc-client/bind-static`):
        // the same provider as CPU ABI v3's PpcCodeMem record, as macppc.engine hands it over on CoralOS
        #[cfg(feature = "cpu-library")]
        m.exec().set_code_mem(abi_code_mem(mem));
    }
    let jit_census = std::env::var("WC3_JIT_CENSUS").is_ok();
    // `WC3_JIT_FRAMES=<file>`: per guest frame (swap to swap, seen at slice ends) host wall ns, tier 2's translation ns and the
    // blocks it compiled - the data for a persistent translation cache's case (do first-translations land in p95/p99 frames?)
    fn host_ns() -> u64 { static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new(); T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64 }
    let mut jit_frames = std::env::var("WC3_JIT_FRAMES").ok().map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap()));
    #[cfg(not(feature = "cpu-library"))]
    if jit_frames.is_some() { m.exec().set_jit_clock(host_ns); }
    // `WC3_REGION_AFTER=<n>`: tier 2's region threshold (block entries before a hot head forms a region; 0 = no regions)
    #[cfg(not(feature = "cpu-library"))]
    if let Some(n) = std::env::var("WC3_REGION_AFTER").ok().and_then(|s| s.parse().ok()) { m.exec().set_jit_region_after(n); }
    // `WC3_COLD=<window insns>/<runs>`: tier 2's cold-tail rule (jit_cold; KNOB_COLD_TAIL sets 2500000/4)
    if let Some((w, n)) = std::env::var("WC3_COLD").ok().and_then(|v| v.split_once('/').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))) { m.exec().set_jit_cold(w, n); }
    // `WC3_REGION_CENSUS`: regions count their entries and internal edges (JitStats region_entries / region_edges)
    #[cfg(not(feature = "cpu-library"))]
    if std::env::var("WC3_REGION_CENSUS").is_ok() { m.exec().set_jit_region_census(true); }
    // `WC3_ARENA_TAGS=<file>` (with WC3_ARENA_DUMP): a tag per JIT code byte (cpu-ppc jit_census), dumped beside the arena
    #[cfg(not(feature = "cpu-library"))]
    if std::env::var("WC3_ARENA_TAGS").is_ok() { m.exec().set_jit_tag_census(true); }
    // `WC3_REGION_SHAPE=<nodes>/<insns>/<pages>/<follow calls 0|1>/<spend 0|1>` (or comma-separated): the region tier's shape (tuning)
    #[cfg(not(feature = "cpu-library"))]
    if let Ok(v) = std::env::var("WC3_REGION_SHAPE") {
        let f: Vec<usize> = v.split(['/', ',']).map(|x| x.trim().parse().unwrap()).collect();
        m.exec().set_jit_region_shape(heirloom_cpu_ppc::RegionShape { max_nodes: f[0], max_insns: f[1], max_pages: f[2], follow_calls: f[3] != 0, spend: f[4] != 0 });
    }
    let (mut jf_swaps, mut jf_t, mut jf_tr, mut jf_n) = (0usize, host_ns(), 0u64, 0u64);
    // `WC3_PROFILE_FRAMES=<file>` (with WC3_PROFILE): the guest-PC profile split by guest frame (swap to swap, at slice ends) -
    // per frame `F <swaps> <guest insns>`, then `S <fragment> <section> <offset> <insns> <ticks> <samples>` per function start
    // sampled in it - tools/ppc-profile-frames.py ranks the functions whose instructions grow in the heavy frames
    let mut prof_frames = std::env::var("WC3_PROFILE_FRAMES").ok().map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap()));
    // `WC3_KNOB_AB=<knobs>,<frames>`: tier 2's knobs alternate between 0 and <knobs> every <frames> swaps from gameplay (swap
    // 1216) on, each switch said as `[KNOB-AB] swap=<n> knobs=<k>` - an A/B inside ONE run (a census dumps at each line)
    let knob_ab: Option<(u32, usize)> = std::env::var("WC3_KNOB_AB").ok().and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))));
    let mut ab_at = 0usize;
    let (mut pf_swaps, mut pf_insns, mut pf_prev): (usize, u64, std::collections::BTreeMap<u32, heirloom_macppc_engine::profile::Acc>) = (0, 0, Default::default());
    // `WC3_PERF=<frames>`: the engine's stage clock (heirloom_macppc_engine::perf) on host nanoseconds, a line every <frames> swaps
    if let Ok(every) = std::env::var("WC3_PERF") {
        fn ns() -> u64 { static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new(); T0.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64 }
        m.perf_enable(ns);
        m.perf_report_every(every.parse().unwrap_or(64));
        if std::env::var("WC3_PERF_SITES").is_ok() { m.perf_census(); }
        // `WC3_PERF_DETAIL=1`: the capture probe, per-draw capture timing and per-run counters (off in the normal tier, perf.rs)
        if std::env::var("WC3_PERF_DETAIL").is_ok() { m.perf_detail(true); }
        // `WC3_PROFILE=1`: the guest-PC profile (heirloom_macppc_engine::profile, cpu-tier "jitprofile" on CoralOS)
        if std::env::var("WC3_PROFILE").is_ok() { m.perf_profile(ns); }
        // `WC3_ABLATE=1`: cpu-tier "jitablate"'s knob rotation (perf.rs ABLATIONS) with its [CPU-PPC-TIER] proof line per window
        if std::env::var("WC3_ABLATE").is_ok() { m.perf_ablate(ns); }
    }
    // `WC3_BUDGET=<insns>` per slice (default 5M) and `WC3_CLOCK_MIPS=<n>`: the virtual clock advances by the instructions the slice ran
    // at n million per second (a slower CPU's clock, e.g. the Z83's) instead of 20 ms per slice
    let budget: u64 = std::env::var("WC3_BUDGET").ok().and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let clock_mips: Option<u64> = std::env::var("WC3_CLOCK_MIPS").ok().and_then(|s| s.parse().ok());
    let mut t = 0u64;
    let slices: u32 = std::env::var("WC3_SLICES").ok().and_then(|s| s.parse().ok()).unwrap_or(400);
    use heirloom_ui_abi::UiCall;
    let mut drive = wc3_drive::Drive::from_env();
    // THE GAMEPLAY GATE: a census or a lockstep run is evidence about gameplay code only if it reached gameplay (h86 native,
    // 2026-11-05: at compat 9807006 the drive stalled in a menu at ~25 draws a frame while lockstep said divergence=None over
    // 10 G instructions). Gameplay = GAMEPLAY_RUN swaps in a row each with >= GAMEPLAY_DRAWS draws (WC3 gameplay draws ~265 a
    // frame; its menus 25-170, its loading screen 25); "[WC3-GAMEPLAY] reached" is said once, with the swap it began at.
    let mut gate = GameplayGate::default();
    let (mut churn_on, mut churn_from) = (false, 0usize);
    let (mut click_phase, mut click_kinds) = (0u8, 0usize);
    let mut ui_seen = 0usize;
    for i in 0..slices {
        // `WC3_JIT_CENSUS`: count the forms tier 2 hands to tier 1's handler from slice 40000 on (gameplay)
        #[cfg(not(feature = "cpu-library"))]
        if jit_census && i == 40_000 { m.exec().jit_helper_census(true); }
        let (st, stats) = GuestMachine::step(&mut m, budget);
        if matches!(st, Step::Fault(_) | Step::Exited) {
            eprintln!("slice {i}: {st:?} {:?}", m.fault());
            // `WC3_DUMP=<words>`: the code before the faulting call (LR - 4*words .. LR), for reading the caller
            if let (Ok(n), Some(cpu)) = (std::env::var("WC3_DUMP").map(|v| v.parse::<u32>().unwrap_or(32)), m.cpu()) {
                let from = cpu.lr.wrapping_sub(4 * n);
                let words: Vec<String> = (0..n + 2).map(|k| m.mac.mem.read_u32(from + 4 * k).map_or_else(|_| String::from("????????"), |w| format!("{w:08x}"))).collect();
                eprintln!("DUMP lr={:#x} from={from:#x}: {}", cpu.lr, words.join(" "));
            }
            break;
        }
        if std::env::var("WC3_STEPS").is_ok() && i % 50 == 0 { eprintln!("STEP {i}: {st:?} {stats:?} pc={:#x} lr={:#x} cb={}", m.cpu().map_or(0, |c| c.pc), m.cpu().map_or(0, |c| c.lr), m.callback_depth()); }
        // the stage clock's lines: per window the perf line, the capture probe's and the frame ledger's (the one output path)
        while let Some(l) = m.perf_take_line(1000) {
            if !l.starts_with("[HEIRLOOM-PERF ") { eprintln!("{l} (slice {i})"); continue; }
            let h = wc3_heap::counts();
            eprintln!("{l} (slice {i}) | heap: calls={} kcycles={}", h.0 - heap_at.0, (h.1 - heap_at.1) / 1000);
            heap_at = h;
            // `WC3_HEAP_SITES=<n>`: from gameplay (frame 1280) on, every n-th engine allocation's call site is recorded
            if let Some(every) = std::env::var("WC3_HEAP_SITES").ok().and_then(|v| v.parse().ok()).filter(|_| m.perf().is_some_and(|p| p.frames_total >= std::env::var("WC3_HEAP_SITES_FROM").ok().and_then(|v| v.parse().ok()).unwrap_or(1280))) { wc3_heap::sample(every); }
            if let Some(sites) = m.perf().and_then(|p| p.sites.as_ref()) {
                let mut v: Vec<_> = sites.iter().collect();
                v.sort_by(|a, b| b.1.cmp(a.1));
                for (s, n) in v.iter().take(6) { eprintln!("  POLL-SITE lr={:#x} {:#x} {:#x} insns={n}", s[0], s[1], s[2]); }
            }
        }
        pf_insns += stats.insns;
        if let Some((k, every)) = knob_ab {
            let swaps = seen.lock().unwrap().swaps;
            if swaps >= 1216 && swaps >= ab_at + every {
                let on = ((swaps - 1216) / every.max(1)) % 2 == 1;
                m.exec().set_jit_knobs(if on { k } else { 0 });
                eprintln!("[KNOB-AB] swap={swaps} knobs={:#x}", if on { k } else { 0 });
                ab_at = swaps;
            }
        }
        if let Some(w) = prof_frames.as_mut() {
            let swaps = seen.lock().unwrap().swaps;
            if swaps != pf_swaps {
                use std::io::Write;
                if let Some(pr) = m.perf().and_then(|p| p.profile()) {
                    // the profile restarts every PROFILE_WINDOWS windows (its sample count falls): count from zero then
                    if pr.samples < pf_prev.values().map(|a| a.samples).sum::<u64>() { pf_prev.clear(); }
                    writeln!(w, "F {swaps} {pf_insns}").unwrap();
                    for (&start, a) in &pr.by_fn {
                        let b = pf_prev.get(&start).copied().unwrap_or_default();
                        if a.samples == b.samples { continue; }
                        let at = m.code_map.locate(start).map_or_else(|| String::from("outside - 0"), |(f, sec, off)| format!("{f:?} {sec} {off:#x}"));
                        writeln!(w, "S {at} {} {} {}", a.insns - b.insns, a.ticks - b.ticks, a.samples - b.samples).unwrap();
                    }
                    pf_prev = pr.by_fn.clone();
                }
                (pf_swaps, pf_insns) = (swaps, 0);
            }
        }
        if let Some(w) = jit_frames.as_mut() {
            let swaps = seen.lock().unwrap().swaps;
            if swaps != jf_swaps {
                use std::io::Write;
                #[cfg(not(feature = "cpu-library"))]
                let (now, tr, n, fl, ev, js) = { let js = m.exec().jit_stats(); (host_ns(), js.translate_time, js.compiled, js.flushes, js.evictions, js) };
                #[cfg(feature = "cpu-library")]
                let (now, tr, n, fl, ev, js) = (host_ns(), 0u64, 0u64, 0u64, 0u64, heirloom_cpu_ppc::JitStats::default());
                // then the JIT's cumulative whole-code drops and segment evictions (a burst's cause), and the counters the Z83's
                // [HEIRLOOM-ENGINE-PERF] line shows: JIT insns, dispatcher entries, IBTC misses, publish / withdraw pages
                writeln!(w, "{swaps},{},{},{},{},{fl},{ev},{},{},{},{},{}", swaps - jf_swaps, now - jf_t, tr - jf_tr, n - jf_n, js.insns, js.entries, js.ibtc_misses, js.publish_pages, js.withdraw_pages).unwrap();
                (jf_swaps, jf_t, jf_tr, jf_n) = (swaps, now, tr, n);
            }
        }
        // a blocked guest waits for its deadline: the clock jumps there (the host's timer); else the slice's own time passes
        t += match (clock_mips, &st) {
            (Some(_), Step::Blocked) => GuestMachine::next_timer_deadline(&m).map_or(1000, |d| d.saturating_sub(t).max(1000)),
            (Some(mips), _) => (stats.insns / mips.max(1)).max(1),
            (None, _) => 20_000,
        };
        GuestMachine::advance_clock_to(&mut m, t).unwrap();
        // `WC3_DRIVE` (tests/wc3_drive): pointer and key steps to the window the game's last aglSetDrawable names (the GL picture's window,
        // as the Z83 host routes input on it), each once the game has swapped enough frames.
        if let Some(d) = drive.as_mut().filter(|d| !d.done()) {
            let swaps = seen.lock().unwrap().swaps;
            let host = seen.lock().unwrap().drawables.last().and_then(|d| d.get(2).copied()).unwrap_or(0) as u32;
            if let Some(win) = m.mac.windows.values().filter_map(|w| w.native).find(|n| n.id == host) {
                let evs = d.due(swaps, win, &mut |l| eprintln!("{l} (slice {i})"));
                if !evs.is_empty() && click_phase == 0 { click_kinds = seen.lock().unwrap().kinds.len(); click_phase = 1; }
                m.mac.ui.events.extend(evs);
            }
        }
        {
            let s = seen.lock().unwrap();
            if let Some(l) = gate.swap(s.swaps, s.draws.len() + s.draws_seen) { eprintln!("{l} (slice {i})"); }
            // `WC3_JIT_CHURN=<swap>`: classify every block tier 2 compiles by why (cpu-ppc jit_churn) - the census runs from the
            // start (a pc's history decides its cause), its counts from that swap on
            #[cfg(not(feature = "cpu-library"))]
            if let Some(from) = std::env::var("WC3_JIT_CHURN").ok().and_then(|v| v.parse::<usize>().ok()) {
                m.exec().set_jit_churn(true);
                if !churn_on && s.swaps >= from { churn_on = true; churn_from = s.swaps; m.exec().jit_churn_reset(); }
            }
        }
        if ring_out.is_some() && seen.lock().unwrap().swaps >= ring_swaps {
            eprintln!("slice {i}: {ring_swaps} swaps kept for the ring image");
            break;
        }
        let ui_calls = m.ui().calls.len();
        if m.ui().calls[ui_seen.min(ui_calls)..].iter().any(|c| format!("{c:?}").starts_with("Request")) {
            eprintln!("slice {i}: the program raised a request");
            break;
        }
        ui_seen = ui_calls;
    }
    // cpu-library: the library's read_prof record (where the instructions ran; tier 2's executable-memory calls)
    #[cfg(feature = "cpu-library")]
    eprintln!("PPC-TIERS {:?}", m.exec().tiers);
    #[cfg(not(feature = "cpu-library"))]
    {
        let e = m.exec();
        if let Some(c) = e.jit_helper_census(false) {
            let mut v: Vec<_> = c.iter().map(|(k, n)| (*n, *k)).collect();
            v.sort_unstable_by(|a, b| b.cmp(a));
            let total: u64 = v.iter().map(|x| x.0).sum();
            eprintln!("JIT-HELPER total={total} top: {}", v.iter().take(30).map(|(n, k)| format!("op{}/xo{}={n}", k >> 26, (k >> 1) & 0x3FF)).collect::<Vec<_>>().join(" "));
        }
        if let (Ok(path), Some((base, code))) = (std::env::var("WC3_ARENA_DUMP"), e.jit_arena_bytes()) {
            std::fs::write(&path, code).expect("WC3_ARENA_DUMP");
            eprintln!("ARENA-DUMP base={base:#x} len={} -> {path}", code.len());
        }
        if let (Ok(path), Some(tags)) = (std::env::var("WC3_ARENA_TAGS"), e.jit_arena_tags()) {
            std::fs::write(&path, tags.iter().flat_map(|t| t.to_le_bytes()).collect::<Vec<u8>>()).expect("WC3_ARENA_TAGS");
            eprintln!("ARENA-TAGS len={} -> {path}", tags.len());
            let words: Vec<u8> = e.jit_arena_words().iter().flat_map(|&(o, w, pc)| [(o as u32).to_le_bytes(), w.to_le_bytes(), pc.to_le_bytes()]).flatten().collect();
            std::fs::write(format!("{path}.words"), words).expect("WC3_ARENA_TAGS words");
            let units: Vec<u8> = e.jit_arena_units().iter().flat_map(|&(o, l, pc, k)| [(o as u32).to_le_bytes(), l.to_le_bytes(), pc.to_le_bytes(), (k as u32).to_le_bytes()]).flatten().collect();
            std::fs::write(format!("{path}.units"), units).expect("WC3_ARENA_TAGS units");
        }
        if churn_on {
            let (counts, log, drops, regions) = e.jit_churn();
            let again = regions.iter().filter(|r| r.1).count();
            eprintln!("JIT-CHURN regions formed: {} ({} at a head that had one before)", regions.len(), again);
            let mut rh: std::collections::BTreeMap<u32, u32> = Default::default();
            for (pc, _) in regions { *rh.entry(*pc).or_default() += 1; }
            let mut rv: Vec<_> = rh.into_iter().collect();
            rv.sort_by(|a, b| b.1.cmp(&a.1));
            for (pc, n) in rv.iter().take(15) { eprintln!("  JIT-CHURN region head={pc:#x} x{n}"); }
            for (w, clk) in drops { eprintln!("JIT-CHURN drop_code: {} at guest insn {clk}", heirloom_cpu_ppc::jit_churn::DROP_NAMES[*w as usize]); }
            let swaps = seen.lock().unwrap().swaps;
            let per: Vec<String> = heirloom_cpu_ppc::jit_churn::NAMES.iter().zip(counts).map(|(n, c)| format!("{n}={c}")).collect();
            eprintln!("JIT-CHURN swaps {churn_from}..{swaps}: {} compiles ({:.2} a frame): {}", log.len(), log.len() as f64 / (swaps - churn_from).max(1) as f64, per.join(" "));
            let mut by: std::collections::BTreeMap<(u32, u8), u32> = Default::default();
            for &(pc, c) in log { *by.entry((pc, c)).or_default() += 1; }
            let mut v: Vec<_> = by.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            for ((pc, c), n) in v.iter().take(40) { eprintln!("  JIT-CHURN pc={pc:#x} {} x{n}", heirloom_cpu_ppc::jit_churn::NAMES[*c as usize]); }
        }
        eprintln!("PPC-TIER {:?} prof={:?} jit={:?} cache={:?} lockstep={:?} divergence={:?}", e.tier, e.prof, e.jit_stats(), e.cache_stats(), e.lockstep_stats(), e.divergence());
        // the region compile-time tail (host ns, timed only with WC3_JIT_FRAMES): the council's p99 <= 4 ms proxy
        let mut rt = e.jit_region_times().to_vec();
        rt.sort_unstable();
        if !rt.is_empty() {
            let q = |p: usize| rt[(rt.len() * p / 100).min(rt.len() - 1)];
            eprintln!("JIT-REGION-COMPILE n={} p50_ns={} p95_ns={} p99_ns={} max_ns={}", rt.len(), q(50), q(95), q(99), rt[rt.len() - 1]);
        }
    }
    if let Some(cpu) = m.cpu() {
        eprintln!("END pc={:#x} lr={:#x} callbacks={:?}", cpu.pc, cpu.lr, m.callback_kinds());
        if let Ok(n) = std::env::var("WC3_DUMP").map(|v| v.parse::<u32>().unwrap_or(32)) {
            let from = cpu.pc.wrapping_sub(4 * n);
            let words: Vec<String> = (0..2 * n).map(|k| m.mac.mem.read_u32(from + 4 * k).map_or_else(|_| String::from("????????"), |w| format!("{w:08x}"))).collect();
            eprintln!("DUMP-END from={from:#x}: {}", words.join(" "));
            eprintln!("REGS r3..r12={:x?}", &cpu.gpr[3..13]);
        }
    }
    for (n, site) in wc3_heap::sites().iter().take(30) {
        eprintln!("HEAP-SITE {n:>6} {site}");
    }
    let log = &m.mac.log;
    let keep: usize = std::env::var("WC3_LOG_LINES").ok().and_then(|s| s.parse().ok()).unwrap_or(80);
    for l in log.iter().skip(log.len().saturating_sub(keep)) {
        eprintln!("LOG {l}");
    }
    for c in m.ui().calls.iter().rev().take(10) {
        eprintln!("UI {}", format!("{c:?}").chars().take(300).collect::<String>());
    }
    let s = seen.lock().unwrap();
    if click_phase > 0 {
        let mut after = std::collections::BTreeMap::new();
        for k in &s.kinds[click_kinds..] { *after.entry(k.as_str()).or_insert(0u32) += 1; }
        eprintln!("CLICK-AFTER GL kind counts since the first driven input: {after:?}");
    }
    eprintln!("GL kinds: {:?}", s.kinds.iter().take(60).collect::<Vec<_>>());
    let mut counts = std::collections::BTreeMap::new();
    for k in &s.kinds { *counts.entry(k.as_str()).or_insert(0u32) += 1; }
    eprintln!("GL kind counts ({} calls, {} draws): {counts:?}", s.kinds.len(), s.draws.len() + s.draws_seen);
    for (k, v) in &s.args_hist {
        eprintln!("ARGS {v:>6} {k}");
    }
    // `WC3_REQUIRE_GAMEPLAY=<swap>[/<frames>]`: the run FAILS as INCONCLUSIVE unless gameplay began by that swap and at least
    // <frames> gameplay frames (default 256) followed - a lockstep or census over a menu proves nothing about gameplay code.
    // A run that IS evidence about gameplay requires it by default (1216/256: the census window starts at swap 1216): a
    // driven JitLockstep run, an arena dump (the callgrind census), the guest-PC profile. `WC3_REQUIRE_GAMEPLAY=0` turns it off.
    let evidence = std::env::var("WC3_DRIVE").is_ok()
        && (std::env::var("HEIRLOOM_PPC_TIER").is_ok_and(|t| t.contains("lockstep")) || std::env::var("WC3_ARENA_DUMP").is_ok() || std::env::var("WC3_PROFILE").is_ok());
    let require = std::env::var("WC3_REQUIRE_GAMEPLAY").ok().or_else(|| evidence.then(|| "1216/256".into())).filter(|v| v != "0");
    if let Some((by, need)) = require.map(|v| {
        let mut f = v.split('/').map(|x| x.trim().parse::<usize>().unwrap());
        (f.next().unwrap(), f.next().unwrap_or(256))
    }) {
        let verdict = gate.verdict(by, need);
        eprintln!("{verdict}");
        assert!(verdict.starts_with("[WC3-GAMEPLAY] PROVEN"), "{verdict}");
    }
    if let Some(path) = &ring_out {
        let ring = OwnedRing::new(s.stream.len().max(4096)).unwrap();
        assert_eq!(ring.ring().push(&s.stream), s.stream.len());
        std::fs::write(path, ring.image()).unwrap();
        eprintln!("ring image: {} request bytes -> {path}", s.stream.len());
    }
}

/// Draws a swap must reach to count as a gameplay frame, and the run of them that says gameplay began.
const GAMEPLAY_DRAWS: usize = 220;
const GAMEPLAY_RUN: usize = 32;

/// The harness's gameplay detector (see `gate` in wc3_host_run_with_a_gl_worker).
#[derive(Default)]
struct GameplayGate { last_swaps: usize, last_draws: usize, run: usize, began: Option<usize>, frames: usize, swaps: usize }

impl GameplayGate {
    /// The swap count and the draws so far, after a slice: the line to say when gameplay is first seen.
    fn swap(&mut self, swaps: usize, draws: usize) -> Option<String> {
        if swaps == self.last_swaps { return None; }
        // draws per swap since the last look (a slice may span several swaps)
        let per = (draws - self.last_draws) / (swaps - self.last_swaps);
        let n = swaps - self.last_swaps;
        (self.last_swaps, self.last_draws, self.swaps) = (swaps, draws, swaps);
        if per >= GAMEPLAY_DRAWS {
            self.run += n;
            if self.began.is_some() { self.frames += n; }
        } else {
            self.run = 0;
        }
        if self.began.is_none() && self.run >= GAMEPLAY_RUN {
            self.began = Some(swaps - self.run);
            self.frames = self.run;
            return Some(format!("[WC3-GAMEPLAY] reached at swap {} ({per} draws a frame)", swaps - self.run));
        }
        None
    }
    fn verdict(&self, by: usize, need: usize) -> String {
        match self.began {
            Some(b) if b <= by && self.frames >= need => format!("[WC3-GAMEPLAY] PROVEN: began at swap {b} (required by {by}), {} gameplay frames of {} swaps", self.frames, self.swaps),
            Some(b) => format!("[WC3-GAMEPLAY] INCONCLUSIVE: began at swap {b} (required by {by}), {} gameplay frames (need {need}) of {} swaps", self.frames, self.swaps),
            None => format!("[WC3-GAMEPLAY] INCONCLUSIVE: never reached gameplay (>= {GAMEPLAY_DRAWS} draws a frame for {GAMEPLAY_RUN} swaps) in {} swaps", self.swaps),
        }
    }
}

/// Stage an exported installed tree (`manifest.tsv`: path, type, creator, flags, data bytes, resource bytes; data forks at their paths,
/// `<name>.rsrc` beside) the way the Z83 bundle stages it (coralos tools/z83-heirloom-stage.sh stage_export): ONLY the game's own
/// folder (the top folder holding War3.mpq) becomes the read-only volume "Warcraft III", every file a stream by its host path, and the
/// folder holding War3.mpq is home. The export's System Folder is the machine's own (CarbonLib, OpenGL, Game Sprockets: the engine
/// serves them), so it is not staged.
fn stage_installed(m: &mut M, root: &std::path::Path) {
    use heirloom_common::GuestMachine;
    struct Raw;
    impl heirloom_volume::StreamSource for Raw {
        fn read_at(&self, locator: &str, offset: u64, dst: &mut [u8]) -> Option<usize> {
            std::os::unix::fs::FileExt::read_at(&std::fs::File::open(locator).ok()?, dst, offset).ok()
        }
    }
    let drop_rsrc = std::env::var("WC3_DROP_RSRC").is_ok();
    let manifest = std::fs::read_to_string(root.join("manifest.tsv")).expect("the export's manifest.tsv");
    let rows: Vec<Vec<&str>> = manifest.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).map(|l| l.split('\t').collect()).collect();
    let anchor = rows.iter().find(|f| f[0].rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case("War3.mpq"))).expect("War3.mpq in the exported tree")[0];
    let top = anchor.split('/').next().unwrap().to_string();
    let vol = GuestMachine::volume(m);
    vol.enable_streams();
    vol.set_stream_source(Box::new(Raw));
    let game = vol.add_volume("Warcraft III", false);
    for f in rows.iter().filter(|f| f[0].starts_with(&format!("{top}/"))) {
        let (path, ty, cr, flags) = (f[0], f[1], f[2], u16::from_str_radix(f[3].trim_start_matches("0x"), 16).unwrap());
        let mut parts: Vec<&str> = path[top.len() + 1..].split('/').collect();
        let leaf = parts.pop().unwrap();
        let dir = vol.mkdirs(game, parts.iter().copied()).unwrap();
        let host = root.join(path);
        let len = std::fs::metadata(&host).unwrap().len();
        let id = vol.put_stream(dir, leaf, len, host.to_str().unwrap()).unwrap_or_else(|e| panic!("staging {path}: {e:?}"));
        if drop_rsrc { continue }
        let fourcc = |t: &str| { let mut b = [b' '; 4]; b[..t.len().min(4)].copy_from_slice(&t.as_bytes()[..t.len().min(4)]); b };
        let n = vol.get_mut(id);
        n.finfo.file_type = fourcc(ty);
        n.finfo.creator = fourcc(cr);
        n.finfo.flags = flags;
        if let Ok(r) = std::fs::read(root.join(format!("{path}.rsrc"))) { n.rsrc = r; }
    }
    let mut at = vec!["Warcraft III"];
    at.extend(anchor[top.len() + 1..].split('/').rev().skip(1).collect::<Vec<_>>().into_iter().rev());
    GuestMachine::set_home(m, &at).unwrap();
    eprintln!("staged {top} of {} as the volume Warcraft III, home {at:?} (resource forks {})", root.display(), if drop_rsrc { "DROPPED" } else { "kept" });
}

/// HostCodeMem plus a simulated provider cost (WC3_CODEMEM_COST): `.1` us per call and `.2` us per page whose state
/// CHANGES (CoralOS's exec_code_protect skips a leaf already in the asked state: no shootdown for it).
struct CostedCodeMem(heirloom_cpu_ppc::HostCodeMem, u64, u64, usize, std::collections::BTreeSet<usize>);
impl CostedCodeMem {
    fn new(call: u64, page: u64) -> Self { CostedCodeMem(heirloom_cpu_ppc::HostCodeMem::new(), call, page, 0, Default::default()) }
    fn cost(&mut self, ptr: *mut u8, len: usize, rx: bool) {
        let first = (ptr as usize - self.3) / 4096;
        let changed = (first..first + len / 4096).filter(|p| if rx { self.4.insert(*p) } else { self.4.remove(p) }).count() as u64;
        let t = std::time::Instant::now();
        let us = self.1 + self.2 * changed;
        // a busy wait, not a sleep: the modelled kernel path holds this CPU (IF off, waiting for acks) - host test only
        while (t.elapsed().as_micros() as u64) < us { std::hint::spin_loop(); }
    }
}
impl heirloom_cpu_ppc::CodeMem for CostedCodeMem {
    fn reserve(&mut self, len: usize) -> Option<*mut u8> { let p = self.0.reserve(len)?; self.3 = p as usize; Some(p) }
    fn publish(&mut self, ptr: *mut u8, len: usize) -> bool { self.cost(ptr, len, true); self.0.publish(ptr, len) }
    fn reset(&mut self, ptr: *mut u8, len: usize) -> bool { self.cost(ptr, len, false); self.0.reset(ptr, len) }
}

/// A tier-2 provider as CPU ABI v3's PpcCodeMem record (cpu-library builds): `ctx` owns the boxed provider (leaked: the
/// harness's machine lives to the process's end).
#[cfg(feature = "cpu-library")]
fn abi_code_mem(mem: Box<dyn heirloom_cpu_ppc::CodeMem>) -> heirloom_cpu_ppc_client::PpcCodeMem {
    use core::ffi::c_void;
    type M = Box<dyn heirloom_cpu_ppc::CodeMem>;
    // SAFETY (all three): `ctx` is the leaked `*mut M` below, never freed, used by one CPU instance at a time.
    extern "C" fn reserve(ctx: *mut c_void, len: usize, out: *mut *mut u8) -> i32 {
        match unsafe { &mut *(ctx as *mut M) }.reserve(len) { Some(p) => { unsafe { *out = p }; 0 } None => -1 }
    }
    extern "C" fn publish(ctx: *mut c_void, ptr: *mut u8, len: usize) -> i32 { if unsafe { &mut *(ctx as *mut M) }.publish(ptr, len) { 0 } else { -1 } }
    extern "C" fn reset(ctx: *mut c_void, ptr: *mut u8, len: usize) -> i32 { if unsafe { &mut *(ctx as *mut M) }.reset(ptr, len) { 0 } else { -1 } }
    heirloom_cpu_ppc_client::PpcCodeMem { size: core::mem::size_of::<heirloom_cpu_ppc_client::PpcCodeMem>() as u32, flags: 0, ctx: Box::into_raw(Box::new(mem)) as *mut c_void, reserve, publish, reset }
}
