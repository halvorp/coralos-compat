//! Shared harness for the engine's GL tests: REAL PowerPC programs (assembled with `asm`) call GL entries with integer arguments
//! in r3.. (then the stack) and float arguments in f1.., against a worker mock that records every frame it is sent.
#![allow(dead_code)]
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{sig, Command, Frame, Token, TokenTable, ENTRIES};
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::fixture::{app_with, Lib, TOC_FIRST};
use heirloom_macppc_engine::{MacPpc, Step};
use heirloom_ui_abi::RecordingGuestUi;
use std::sync::{Arc, Mutex};

pub type M = MacPpc<RecordingGuestUi>;

#[derive(Default)]
pub struct Seen {
    pub calls: Vec<(String, Vec<u64>, Vec<u8>)>,
    pub draws: Vec<heirloom_gl_abi::DrawCommand>,
}

pub struct Mock(pub TokenTable, pub Arc<Mutex<Seen>>);

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
                        // bulk-data answers: three doubles / floats / ints, four pixel-map values, residence (true, true, false)
                        "glGetMapdv" => reply(&mut out, (0..3).map(|k| (k as f64 + 0.5).to_bits()).collect()),
                        "glGetMapfv" => reply(&mut out, (0..3).map(|k| u64::from((k as f32 + 0.5).to_bits())).collect()),
                        "glGetMapiv" => reply(&mut out, vec![7, 0xFFFF_FFFF, 9]),
                        "glGetPixelMapfv" => reply(&mut out, (0..4).map(|k| u64::from((k as f32 * 0.25).to_bits())).collect()),
                        "glGetPixelMapuiv" => reply(&mut out, vec![1, 2, 3, 0xFFFF_FFFF]),
                        "glGetPixelMapusv" => reply(&mut out, vec![1, 2, 3, 0xFFFF]),
                        "glAreTexturesResident" => reply(&mut out, vec![0, 1, 1, 0]),
                        "glGetIntegerv" => reply(&mut out, (0..sig::integer_count(args[0] as u32).unwrap() as u64).map(|k| 0xFFFF_FFF0 + k).collect()),
                        "glReadPixels" => {
                            let l = heirloom_gl_abi::pixel::layout(args[4] as u32, args[5] as u32).unwrap();
                            let data: Vec<u8> = (0..(args[2] * args[3]) as usize * l.pixel).map(|b| b as u8 + 1).collect();
                            out.extend(Frame { token: none, command: Command::ReplyData { status: 0, words: vec![], data } }.encode());
                        }
                        _ => {}
                    }
                }
                Command::Draw(d) => seen.draws.push(d),
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
pub enum I { V(u32), P(i32) }
/// One call: its integer arguments (r3.., then the stack) and its float arguments (f1..), and where to keep r3 after it.
pub struct Call { pub name: &'static str, pub ints: Vec<I>, pub fl: Vec<Fl>, pub keep: Option<i32> }
#[derive(Clone, Copy)]
pub enum Fl { S(i32), D(i32) }

pub fn li32(a: &mut Asm, r: u32, v: u32) {
    if v < 0x8000 { a.li(r, v as i32); } else { a.lis(r, (v >> 16) as i32).ori(r, r, (v & 0xFFFF) as i32); }
}

pub struct Data { pub bytes: Vec<u8>, pub base: i32 }
impl Data {
    pub fn put(&mut self, b: &[u8]) -> i32 {
        while self.bytes.len() % 8 != 0 { self.bytes.push(0); }
        let at = self.base + self.bytes.len() as i32;
        self.bytes.extend_from_slice(b);
        at
    }
    pub fn f32s(&mut self, v: &[f32]) -> i32 { self.put(&v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<_>>()) }
}

pub fn imports(calls: &[Call]) -> Vec<&'static str> {
    let mut v = vec!["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglDestroyContext", "glGetError"];
    for c in calls { if !v.contains(&c.name) { v.push(c.name); } }
    v
}

/// The PEF a PowerPC program making `calls` is (and the seen-by-the-worker record it will fill).
pub fn assemble(calls: &[Call], d: &Data, with_context: bool) -> (Vec<u8>, Arc<Mutex<Seen>>) {
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
pub fn layout(build: &impl Fn(&mut Data) -> Vec<Call>) -> (Vec<Call>, Data) {
    let n_imports = imports(&build(&mut Data { bytes: Vec::new(), base: 0 })).len();
    let mut d = Data { bytes: Vec::new(), base: (TOC_FIRST + 4 * n_imports as u32) as i32 };
    let calls = build(&mut d);
    (calls, d)
}

/// Run the calls `build` makes in a context and return the machine, what the worker saw, and the data area's address.
pub fn run(build: impl Fn(&mut Data) -> Vec<Call>, with_context: bool) -> (M, Arc<Mutex<Seen>>, u32) {
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

pub fn ints(v: &[u32]) -> Vec<I> { v.iter().map(|x| I::V(*x)).collect() }
pub fn call(name: &'static str, ints: Vec<I>, fl: Vec<Fl>) -> Call { Call { name, ints, fl, keep: None } }
pub fn bits(f: f32) -> u64 { u64::from(f.to_bits()) }

/// What the worker saw of `name`, in order.
pub fn saw(seen: &Arc<Mutex<Seen>>, name: &str) -> Vec<(Vec<u64>, Vec<u8>)> {
    seen.lock().unwrap().calls.iter().filter(|c| c.0 == name).map(|c| (c.1.clone(), c.2.clone())).collect()
}

