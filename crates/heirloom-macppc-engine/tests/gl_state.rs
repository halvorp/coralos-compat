//! The GL state calls the contract marks implemented beyond the Phase 1 triangle (blend, depth, fog, lighting, materials,
//! texture names / parameters / images, pixel storage, queries, read-backs): REAL PowerPC programs (assembled with `asm`,
//! run by heirloom-cpu-ppc) call each entry with float arguments in FPRs, pointers into big-endian memory and a ninth
//! argument on the stack; a worker mock that obeys the ABI records the exact frames. What the REAL gl4es worker then does
//! with them is proven by `userland/coral-heirloom-gl` (the replayer + gl4es host tests), not here.
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{sig, Command, Frame, Token, TokenTable, ENTRIES};
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::fixture::{app_with, Lib, TOC_FIRST};
use heirloom_macppc_engine::{MacPpc, Step};
use heirloom_ui_abi::RecordingGuestUi;
use std::cell::Cell;
use std::sync::{Arc, Mutex};

type M = MacPpc<RecordingGuestUi>;

#[derive(Default)]
struct Seen {
    calls: Vec<(String, Vec<u64>, Vec<u8>)>,
}

struct Mock(TokenTable, Arc<Mutex<Seen>>);

impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let mut seen = self.1.lock().unwrap();
        let mut out = Vec::new();
        let none = Token { slot: 0, incarnation: 0 };
        let mut at = 0;
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            at += n;
            let reply = |out: &mut Vec<u8>, words: Vec<u64>| out.extend(Frame { token: none, command: Command::Reply { status: 0, words, text: String::new() } }.encode());
            match f.command {
                Command::ContextCreate => {
                    let t = self.0.create().unwrap();
                    reply(&mut out, vec![u64::from(t.slot), u64::from(t.incarnation)]);
                }
                Command::Call { entry, args } => {
                    let name = ENTRIES[usize::from(entry)].name;
                    if sig::shape(name).is_some() {
                        if let Err(e) = sig::check_words(name, &args) { panic!("{name}: {e}"); }
                    }
                    seen.calls.push((name.into(), args.clone(), Vec::new()));
                    match name {
                        "aglChoosePixelFormat" => reply(&mut out, vec![1]),
                        "glGetError" => reply(&mut out, vec![0]),
                        "glGenTextures" => reply(&mut out, (0..args[0]).map(|k| 100 + k).collect()),
                        "glIsEnabled" => reply(&mut out, vec![1]),
                        "glGetIntegerv" => reply(&mut out, (0..sig::integer_count(args[0] as u32).unwrap() as u64).map(|k| 0xFFFF_FFF0 + k).collect()),
                        "glReadPixels" => {
                            let l = heirloom_gl_abi::pixel::layout(args[4] as u32, args[5] as u32).unwrap();
                            let data: Vec<u8> = (0..(args[2] * args[3]) as usize * l.pixel).map(|b| b as u8 + 1).collect();
                            out.extend(Frame { token: none, command: Command::ReplyData { status: 0, words: vec![], data } }.encode());
                        }
                        _ => {}
                    }
                }
                Command::CallData { entry, args, data } => seen.calls.push((ENTRIES[usize::from(entry)].name.into(), args, data)),
                Command::BatchEnd { seq } => out.extend(Frame { token: none, command: Command::Retired { seq } }.encode()),
                _ => {}
            }
        }
        out
    }
}

/// An integer argument: an immediate, or the address of data in the program's data area.
#[derive(Clone, Copy)]
enum I { V(u32), P(i32) }
/// One call: its integer arguments (r3.., then the stack) and its float arguments (f1..), and where to keep r3 after it.
struct Call { name: &'static str, ints: Vec<I>, fl: Vec<Fl>, keep: Option<i32> }
#[derive(Clone, Copy)]
enum Fl { S(i32), D(i32) }

fn li32(a: &mut Asm, r: u32, v: u32) {
    if v < 0x8000 { a.li(r, v as i32); } else { a.lis(r, (v >> 16) as i32).ori(r, r, (v & 0xFFFF) as i32); }
}

struct Data { bytes: Vec<u8>, base: i32 }
impl Data {
    fn put(&mut self, b: &[u8]) -> i32 {
        while self.bytes.len() % 8 != 0 { self.bytes.push(0); }
        let at = self.base + self.bytes.len() as i32;
        self.bytes.extend_from_slice(b);
        at
    }
    fn f32s(&mut self, v: &[f32]) -> i32 { self.put(&v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<_>>()) }
}

fn imports(calls: &[Call]) -> Vec<&'static str> {
    let mut v = vec!["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglDestroyContext", "glGetError"];
    for c in calls { if !v.contains(&c.name) { v.push(c.name); } }
    v
}

/// The PEF a PowerPC program making `calls` is (and the seen-by-the-worker record it will fill).
fn assemble(calls: &[Call], d: &Data, with_context: bool) -> (Vec<u8>, Arc<Mutex<Seen>>) {
    let names = imports(calls);
    let toc = |n: &str| (TOC_FIRST as i32) + 4 * names.iter().position(|x| *x == n).unwrap() as i32;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -160, 1).stw(31, 156, 1).stw(30, 152, 1);
    if with_context {
        a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(31, 3);
        a.mr(3, 31).li(4, 0).call_toc(toc("aglCreateContext")).mr(30, 3);
        a.mr(3, 30).call_toc(toc("aglSetCurrentContext"));
    }
    for c in calls {
        for (k, v) in c.ints.iter().enumerate() {
            let r = if k < 8 { 3 + k as u32 } else { 0 };
            match v { I::V(x) => li32(&mut a, r, *x), I::P(off) => { a.addi(r, 2, *off); } }
            if k >= 8 { a.stw(0, 24 + 4 * k as i32, 1); }
        }
        for (k, f) in c.fl.iter().enumerate() {
            match f { Fl::S(off) => a.lfs(1 + k as u32, *off, 2), Fl::D(off) => a.lfd(1 + k as u32, *off, 2) };
        }
        a.call_toc(toc(c.name));
        if let Some(off) = c.keep { a.stw(3, off, 2); }
    }
    if with_context { a.mr(3, 30).call_toc(toc("aglDestroyContext")); }
    a.li(3, 0).lwz(30, 152, 1).lwz(31, 156, 1).addi(1, 1, 160).lwz(0, 8, 1).mtlr(0).blr();
    let syms: Vec<(&str, SymbolClass, bool)> = names.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    let libs: [Lib; 1] = [("OpenGLLibrary", false, &syms)];
    (app_with(&libs, &a.finish(), &d.bytes), Arc::new(Mutex::new(Seen::default())))
}

/// Lay the data out for `build` (run twice: once to learn how many imports there are, which fixes where the data starts).
fn layout(build: &impl Fn(&mut Data) -> Vec<Call>) -> (Vec<Call>, Data) {
    let n_imports = imports(&build(&mut Data { bytes: Vec::new(), base: 0 })).len();
    let mut d = Data { bytes: Vec::new(), base: (TOC_FIRST + 4 * n_imports as u32) as i32 };
    let calls = build(&mut d);
    (calls, d)
}

/// Run the calls `build` makes in a context and return the machine, what the worker saw, and the data area's address.
fn run(build: impl Fn(&mut Data) -> Vec<Call>, with_context: bool) -> (M, Arc<Mutex<Seen>>, u32) {
    let (calls, d) = layout(&build);
    let (bytes, seen) = assemble(&calls, &d, with_context);
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.gl_attach(Box::new(Mock(TokenTable::default(), seen.clone())));
    let frag = m.instantiate("st", &Pef::parse(&bytes).unwrap()).unwrap();
    let data = frag.section_bases[1].unwrap();
    m.start(&frag).unwrap();
    assert_eq!(m.step(5_000_000).0, Step::Exited, "{:?}", m.fault());
    (m, seen, data)
}

fn ints(v: &[u32]) -> Vec<I> { v.iter().map(|x| I::V(*x)).collect() }
fn call(name: &'static str, ints: Vec<I>, fl: Vec<Fl>) -> Call { Call { name, ints, fl, keep: None } }
fn bits(f: f32) -> u64 { u64::from(f.to_bits()) }

/// What the worker saw of `name`, in order.
fn saw(seen: &Arc<Mutex<Seen>>, name: &str) -> Vec<(Vec<u64>, Vec<u8>)> {
    seen.lock().unwrap().calls.iter().filter(|c| c.0 == name).map(|c| (c.1.clone(), c.2.clone())).collect()
}

#[test]
fn scalar_and_float_state_calls_arrive_as_their_word_layouts() {
    let (_, seen, _) = run(|d| {
        let (c, dr, fogc) = (d.f32s(&[0.25, 0.5, 0.75, 1.0]), d.put(&[0x3F, 0xE0, 0, 0, 0, 0, 0, 0, 0x3F, 0xF0, 0, 0, 0, 0, 0, 0]), d.f32s(&[0.1, 0.2, 0.3, 0.4]));
        vec![
            call("glClearColor", vec![], vec![Fl::S(c), Fl::S(c + 4), Fl::S(c + 8), Fl::S(c + 12)]),
            call("glAlphaFunc", ints(&[0x0204]), vec![Fl::S(c + 4)]),
            call("glDepthRange", vec![], vec![Fl::D(dr), Fl::D(dr + 8)]),
            call("glPolygonOffset", vec![], vec![Fl::S(c + 12), Fl::S(c)]),
            call("glFogf", ints(&[0x0B62]), vec![Fl::S(c + 8)]),
            call("glFogi", ints(&[0x0B65, 0x0801]), vec![]),
            call("glFogfv", vec![I::V(0x0B66), I::P(fogc)], vec![]),
            call("glBlendFunc", ints(&[0x0302, 0x0303]), vec![]),
            call("glDepthFunc", ints(&[0x0203]), vec![]),
            call("glDepthMask", ints(&[0xFFFF_FF01]), vec![]),
            call("glScissor", ints(&[1, 2, 640, 480]), vec![]),
            call("glColorMaterial", ints(&[0x0408, 0x1602]), vec![]),
            call("glBindTexture", ints(&[0x0DE1, 7]), vec![]),
            call("glTexParameteri", ints(&[0x0DE1, 0x2801, 0x2601]), vec![]),
            call("glTexEnvi", ints(&[0x2300, 0x2200, 0x2100]), vec![]),
            call("glTexGeni", ints(&[0x2000, 0x2500, 0x2401]), vec![]),
        ]
    }, true);
    let w = |n: &str| saw(&seen, n)[0].0.clone();
    assert_eq!(w("glClearColor"), [bits(0.25), bits(0.5), bits(0.75), bits(1.0)]);
    assert_eq!(w("glAlphaFunc"), [0x0204, bits(0.5)]);
    assert_eq!(w("glDepthRange"), [0.5f64.to_bits(), 1.0f64.to_bits()], "a double is two registers' worth in one word");
    assert_eq!(w("glPolygonOffset"), [bits(1.0), bits(0.25)]);
    assert_eq!(w("glFogf"), [0x0B62, bits(0.75)]);
    assert_eq!(w("glFogi"), [0x0B65, 0x0801]);
    assert_eq!(w("glFogfv"), [0x0B66, bits(0.1), bits(0.2), bits(0.3), bits(0.4)], "GL_FOG_COLOR carries four floats");
    assert_eq!(w("glBlendFunc"), [0x0302, 0x0303]);
    assert_eq!(w("glDepthMask"), [1], "a GLboolean is its low byte");
    assert_eq!(w("glScissor"), [1, 2, 640, 480]);
    assert_eq!(w("glTexParameteri"), [0x0DE1, 0x2801, 0x2601]);
    assert_eq!(w("glTexGeni"), [0x2000, 0x2500, 0x2401]);
    for n in ["glColorMaterial", "glBindTexture", "glTexEnvi", "glDepthFunc"] {
        assert_eq!(saw(&seen, n).len(), 1, "{n}");
    }
}

#[test]
fn lighting_and_material_vectors_are_sized_by_their_pname() {
    let (_, seen, _) = run(|d| {
        let v = d.f32s(&[1.0, 2.0, 3.0, 4.0]);
        let l = d.f32s(&[0.5]);
        let cut = d.f32s(&[180.0]); // a spot cutoff other than 180 is refused by name (gaps::VALUE_GAPS)
        vec![
            call("glLightfv", vec![I::V(0x4001), I::V(0x1203), I::P(v)], vec![]), // GL_POSITION: 4
            call("glLightfv", vec![I::V(0x4001), I::V(0x1204), I::P(v)], vec![]), // GL_SPOT_DIRECTION: 3
            call("glLightfv", vec![I::V(0x4001), I::V(0x1206), I::P(cut)], vec![]), // GL_SPOT_CUTOFF: 1
            call("glLightf", ints(&[0x4000, 0x1207]), vec![Fl::S(l)]),
            call("glLightModelfv", vec![I::V(0x0B53), I::P(v)], vec![]),
            call("glMaterialfv", vec![I::V(0x0408), I::V(0x1601), I::P(l)], vec![]), // GL_SHININESS: 1
            call("glMaterialfv", vec![I::V(0x0408), I::V(0x1602), I::P(v)], vec![]), // GL_AMBIENT_AND_DIFFUSE: 4
            call("glNormal3fv", vec![I::P(v)], vec![]),
        ]
    }, true);
    let one = |n: &str, k: usize| saw(&seen, n)[k].0.clone();
    assert_eq!(one("glLightfv", 0), [0x4001, 0x1203, bits(1.0), bits(2.0), bits(3.0), bits(4.0)]);
    assert_eq!(one("glLightfv", 1), [0x4001, 0x1204, bits(1.0), bits(2.0), bits(3.0)]);
    assert_eq!(one("glLightfv", 2), [0x4001, 0x1206, bits(180.0)]);
    assert_eq!(one("glLightf", 0), [0x4000, 0x1207, bits(0.5)]);
    assert_eq!(one("glLightModelfv", 0), [0x0B53, bits(1.0), bits(2.0), bits(3.0), bits(4.0)]);
    assert_eq!(one("glMaterialfv", 0), [0x0408, 0x1601, bits(0.5)]);
    assert_eq!(one("glMaterialfv", 1).len(), 6);
    assert_eq!(one("glNormal3fv", 0), [bits(1.0), bits(2.0), bits(3.0)]);
}

fn keep(name: &'static str, ints: Vec<I>, at: i32) -> Call { Call { name, ints, fl: vec![], keep: Some(at) } }

#[test]
fn an_enum_the_encoder_cannot_size_is_invalid_enum_and_is_never_sent() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|d| {
        let v = d.f32s(&[1.0; 4]);
        at.set(d.put(&[0; 8]));
        vec![
            call("glLightfv", vec![I::V(0x4000), I::V(0xBEEF), I::P(v)], vec![]),
            call("glGetIntegerv", vec![I::V(0x1234), I::P(v)], vec![]),
            call("glPixelStorei", ints(&[0x0CF5, 3]), vec![]),
            keep("glGetError", vec![], at.get()),
            keep("glGetError", vec![], at.get() + 4),
        ]
    }, true);
    assert!(saw(&seen, "glLightfv").is_empty() && saw(&seen, "glGetIntegerv").is_empty() && saw(&seen, "glPixelStorei").is_empty(), "nothing invalid reached the worker");
    assert_eq!(m.mac.mem.read_u32(d + at.get() as u32).unwrap(), 0x0500, "GL_INVALID_ENUM (the first error recorded is the one reported)");
    assert_eq!(m.mac.mem.read_u32(d + at.get() as u32 + 4).unwrap(), 0, "and the flag is cleared by reading it");
}

#[test]
fn without_a_current_context_a_state_call_is_invalid_operation_and_is_never_sent() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|d| {
        at.set(d.put(&[0; 8]));
        vec![call("glDepthFunc", ints(&[0x0203]), vec![]), call("glGenTextures", ints(&[1, 0]), vec![])]
    }, false);
    assert!(saw(&seen, "glDepthFunc").is_empty());
    let _ = (m, d);
}

#[test]
fn texture_names_queries_and_deletes_go_through_the_worker() {
    let (names, out, en, ints_at) = (Cell::new(0), Cell::new(0), Cell::new(0), Cell::new(0));
    let (m, seen, d) = run(|d| {
        names.set(d.put(&[0; 12]));
        out.set(d.put(&[0; 8]));
        en.set(d.put(&[0; 8]));
        ints_at.set(d.put(&[0; 64]));
        let del = d.put(&[0, 0, 0, 100, 0, 0, 0, 102]);
        vec![
            call("glGenTextures", vec![I::V(3), I::P(names.get())], vec![]),
            call("glDeleteTextures", vec![I::V(2), I::P(del)], vec![]),
            keep("glIsEnabled", ints(&[0x0DE1]), en.get()),
            call("glGetIntegerv", vec![I::V(0x0BA2), I::P(ints_at.get())], vec![]),
            call("glGetIntegerv", vec![I::V(0x0BA6), I::P(ints_at.get() + 16)], vec![]),
        ]
    }, true);
    assert_eq!(saw(&seen, "glGenTextures")[0].0, [3]);
    for k in 0..3 { assert_eq!(m.mac.mem.read_u32(d + names.get() as u32 + 4 * k).unwrap(), 100 + k, "the worker's names reach the guest big-endian"); }
    assert_eq!(saw(&seen, "glDeleteTextures")[0].0, [2, 100, 102], "names are read big-endian from guest memory");
    assert_eq!(m.mac.mem.read_u32(d + en.get() as u32).unwrap(), 1);
    assert_eq!(m.mac.mem.read_u32(d + ints_at.get() as u32).unwrap(), 0xFFFF_FFF0, "GL_VIEWPORT is four integers");
    assert_eq!(m.mac.mem.read_u32(d + ints_at.get() as u32 + 12).unwrap(), 0xFFFF_FFF3);
    assert_eq!(m.mac.mem.read_u32(d + ints_at.get() as u32 + 16 + 60).unwrap(), 0xFFFF_FFFF, "GL_MODELVIEW_MATRIX is sixteen");
    assert_eq!(m.mac.mem.read_u32(d + ints_at.get() as u32 + 16 + 64).unwrap(), 0, "and not a word more");
}

#[test]
fn texture_images_carry_tight_little_endian_pixels_whatever_the_guests_storage_state() {
    let (m, seen, _) = run(|d| {
        // 3x2 RGB UNSIGNED_BYTE with the default alignment 4 (rows padded to 12), a 2x1 packed 8_8_8_8_REV BGRA image, and a 1x1 5_6_5
        let rgb = d.put(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 0, 0, 0, 11, 12, 13, 14, 15, 16, 17, 18, 19, 0, 0, 0]);
        let bgra = d.put(&[0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]);
        let r565 = d.put(&[0xF8, 0x1F, 0, 0]);
        vec![
            call("glPixelStorei", ints(&[0x0CF5, 4]), vec![]),
            call("glTexImage2D", vec![I::V(0x0DE1), I::V(0), I::V(0x1907), I::V(3), I::V(2), I::V(0), I::V(0x1907), I::V(0x1401), I::P(rgb)], vec![]),
            call("glTexImage2D", vec![I::V(0x0DE1), I::V(1), I::V(0x1908), I::V(2), I::V(1), I::V(0), I::V(0x80E1), I::V(0x8367), I::P(bgra)], vec![]),
            call("glTexSubImage2D", vec![I::V(0x0DE1), I::V(0), I::V(1), I::V(1), I::V(1), I::V(1), I::V(0x1907), I::V(0x8363), I::P(r565)], vec![]),
            call("glTexImage2D", vec![I::V(0x0DE1), I::V(2), I::V(0x1908), I::V(64), I::V(64), I::V(0), I::V(0x1908), I::V(0x1401), I::V(0)], vec![]),
        ]
    }, true);
    let t = saw(&seen, "glTexImage2D");
    assert_eq!(t[0].0, [0x0DE1, 0, 0x1907, 3, 2, 0, 0x1907, 0x1401, 1]);
    assert_eq!(t[0].1, [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18, 19], "padding removed, bytes untouched");
    assert_eq!(t[1].0[8], 1);
    assert_eq!(t[1].1, [0xDD, 0xCC, 0xBB, 0xAA, 0x44, 0x33, 0x22, 0x11], "each big-endian packed word becomes the little-endian word of the same value");
    assert_eq!((t[2].0[8], t[2].1.len()), (0, 0), "a NULL image is an allocation with no pixels");
    let s = saw(&seen, "glTexSubImage2D");
    assert_eq!(s[0].0, [0x0DE1, 0, 1, 1, 1, 1, 0x1907, 0x8363]);
    assert_eq!(s[0].1, [0x1F, 0xF8]);
    assert_eq!(saw(&seen, "glPixelStorei")[0].0, [0x0CF5, 4], "the storage state is forwarded so a state query agrees");
    let _ = m;
}

#[test]
fn unpack_state_changes_what_is_read_from_guest_memory() {
    let (_, seen, _) = run(|d| {
        // an 8-wide one-byte image, read as a 3x2 window at row 1 / pixel 2, alignment 1
        let img = d.put(&(0u8..64).collect::<Vec<_>>());
        vec![
            call("glPixelStorei", ints(&[0x0CF5, 1]), vec![]),
            call("glPixelStorei", ints(&[0x0CF2, 8]), vec![]),
            call("glPixelStorei", ints(&[0x0CF3, 1]), vec![]),
            call("glPixelStorei", ints(&[0x0CF4, 2]), vec![]),
            call("glTexImage2D", vec![I::V(0x0DE1), I::V(0), I::V(0x1909), I::V(3), I::V(2), I::V(0), I::V(0x1909), I::V(0x1401), I::P(img)], vec![]),
            call("glPixelStorei", ints(&[0x0CF0, 1]), vec![]),
            call("glTexImage2D", vec![I::V(0x0DE1), I::V(0), I::V(0x1909), I::V(1), I::V(1), I::V(0), I::V(0x1909), I::V(0x1403), I::P(img)], vec![]),
        ]
    }, true);
    let t = saw(&seen, "glTexImage2D");
    assert_eq!(t[0].1, [10, 11, 12, 18, 19, 20]);
    assert_eq!(t[1].1, [20, 21], "SWAP_BYTES: the 16-bit pixel at row 1, pixel 2 of a 16-byte-stride image is already little-endian, so its bytes are untouched")
}

#[test]
fn read_pixels_lands_in_guest_memory_in_the_guests_layout() {
    let at = Cell::new(0);
    let (m, seen, d) = run(|d| {
        at.set(d.put(&[0xEE; 32]));
        vec![
            call("glPixelStorei", ints(&[0x0D05, 1]), vec![]),
            call("glReadPixels", vec![I::V(0), I::V(0), I::V(2), I::V(1), I::V(0x1908), I::V(0x8035), I::P(at.get())], vec![]),
        ]
    }, true);
    assert_eq!(saw(&seen, "glReadPixels")[0].0, [0, 0, 2, 1, 0x1908, 0x8035]);
    // the worker answered bytes 1..=8 (two little-endian packed words): the guest sees them as big-endian words
    let g: Vec<u8> = (0..9).map(|k| m.mac.mem.read_u8(d + at.get() as u32 + k).unwrap()).collect();
    assert_eq!(g, [4, 3, 2, 1, 8, 7, 6, 5, 0xEE], "converted to big-endian, and the byte after the image is untouched");
}
