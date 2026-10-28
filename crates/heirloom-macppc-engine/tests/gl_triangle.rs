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

type M = MacPpc<RecordingGuestUi>;

/// What the mock saw, shared with the test.
#[derive(Default)]
struct Seen {
    stream: Vec<u8>,
    kinds: Vec<String>,
    draws: Vec<heirloom_gl_abi::DrawCommand>,
    queries: Vec<u64>,
    bad_tokens: usize,
}

struct Mock {
    tokens: TokenTable,
    seen: Arc<Mutex<Seen>>,
    gpu: bool,
}

impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let mut seen = self.seen.lock().unwrap();
        seen.stream.extend_from_slice(req);
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
                    match name {
                        "aglChoosePixelFormat" if self.gpu => reply(&mut out, 0, vec![1], ""),
                        "glGetString" => {
                            seen.queries.push(args[0]);
                            reply(&mut out, 0, vec![], if args[0] == 0x1F01 { "Mock GPU" } else { "" });
                        }
                        "glGetError" => reply(&mut out, 0, vec![0], ""),
                        _ => {}
                    }
                }
                Command::Draw(d) => {
                    seen.kinds.push("Draw".into());
                    seen.draws.push(d);
                }
                Command::FenceSet { .. } => seen.kinds.push("FenceSet".into()),
                Command::Swap { .. } => seen.kinds.push("Swap".into()),
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

fn run(gpu: bool) -> (M, Arc<Mutex<Seen>>, Layout, u32) {
    let (bytes, l) = pef();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    if gpu {
        m.gl_attach(Box::new(Mock { tokens: TokenTable::default(), seen: seen.clone(), gpu }));
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
    assert!(!matches!(m.bind("aglSetFullScreen"), heirloom_macppc_engine::symbols::Binding::Unbound(_)), "bound, so the call is refused by name");
    match m.glue_call("aglSetFullScreen", &mut regs) {
        GlueOutcome::Refused(s) => assert_eq!(s, "[HEIRLOOM-GL REFUSED call=aglSetFullScreen]"),
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
