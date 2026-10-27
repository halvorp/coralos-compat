//! Every regular GL call family (`heirloom_gl_abi::family`, generated from the Khronos registry) driven by REAL PowerPC
//! programs: each generated call is made with distinctive arguments placed exactly where the PowerOpen calling convention
//! puts them (integer-class and pointer arguments at their PARAMETER WORD, floats and doubles in f1.., a double taking two
//! words), and the frame the worker receives is compared with the words an INDEPENDENT model of the argument types says it
//! must be. OUT vectors and returned values come back through a worker mock and are checked in guest memory (big-endian)
//! and in r3. One loop over the whole table: a new generated call is covered the moment it exists, and a table entry whose
//! contract row is not switched on fails `every_generated_call_is_switched_on_in_the_contract`.
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::family::{self, Sig, Ty, P, R};
use heirloom_gl_abi::family_table::GENERATED;
use heirloom_gl_abi::sig::VecKind;
use heirloom_gl_abi::{find, Command, Frame, Token, TokenTable, ENTRIES};
use heirloom_macppc_engine::asm::Asm;
use heirloom_macppc_engine::fixture::{app_with, Lib, TOC_FIRST};
use heirloom_macppc_engine::{MacPpc, Step};
use heirloom_ui_abi::RecordingGuestUi;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    calls: Vec<(String, Vec<u64>)>,
}

/// A worker that records every call and answers the families' asks with typed, distinctive values.
struct Mock(TokenTable, Arc<Mutex<Seen>>);

/// The distinctive value the mock answers for element `k` of an OUT vector or a returned value of type `t`.
fn answer(t: Ty, k: usize) -> u64 {
    match t {
        Ty::Float => u64::from((k as f32 + 0.5).to_bits()),
        Ty::Double => (k as f64 + 0.5).to_bits(),
        Ty::Bool => (k % 2) as u64,
        _ => t.int_word(0x8000_0080 + k as u32),
    }
}

impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let none = Token { slot: 0, incarnation: 0 };
        let mut at = 0;
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            at += n;
            let mut reply = |words: Vec<u64>| out.extend(Frame { token: none, command: Command::Reply { status: 0, words, text: String::new() } }.encode());
            match f.command {
                Command::ContextCreate => {
                    let t = self.0.create().unwrap();
                    reply(vec![u64::from(t.slot), u64::from(t.incarnation)]);
                }
                Command::Call { entry, args } => {
                    let name = ENTRIES[usize::from(entry)].name;
                    self.1.lock().unwrap().calls.push((name.into(), args.clone()));
                    if name == "aglChoosePixelFormat" {
                        reply(vec![1]);
                    } else if let Some(sig) = family::lookup(name) {
                        family::check_words(sig, &args).unwrap_or_else(|e| panic!("{name}: the engine sent a frame that does not fit the layout: {e}"));
                        if let Some((t, kind)) = sig.out() {
                            let pname = args.last().copied().unwrap_or(0) as u32;
                            reply((0..family::var_len(kind, pname).unwrap()).map(|k| answer(t, k)).collect());
                        } else if sig.answers() {
                            reply(vec![match sig.ret { R::Bool => 1, _ => 0x5150 }]);
                        }
                    }
                }
                Command::BatchEnd { seq } => out.extend(Frame { token: none, command: Command::Retired { seq } }.encode()),
                _ => {}
            }
        }
        out
    }
}

/// A valid `pname` (or other sizing word) for the vector kind: its length in `want`.
fn pname_for(kind: VecKind) -> (u32, usize) {
    match kind {
        VecKind::Fog => (0x0B66, 4),
        VecKind::LightModel => (0x0B53, 4),
        VecKind::Light => (0x1203, 4),
        VecKind::Material => (0x1602, 4),
        VecKind::MaterialGet => (0x1603, 3),
        VecKind::TexEnv => (0x2201, 4),
        VecKind::TexGen => (0x2501, 4),
        VecKind::TexParam => (0x1004, 4),
        VecKind::TexParamGet => (0x8067, 1),
        VecKind::TexLevel => (0x1003, 1),
        VecKind::Four => (0x3000, 4),
        VecKind::State => (0x0BA2, 4),
    }
}

/// The raw 32-bit register (or memory element) the program passes for scalar `k` of type `t`: narrow types carry junk in the
/// bits above their width, as a real caller's register may.
fn raw(t: Ty, k: usize) -> u32 {
    let k = k as u32;
    match t {
        Ty::Byte => 0x0000_FF00 | 0x80 | (k & 0x7F), // the low byte is negative; the bits above it are junk
        Ty::Ubyte => 0x0100_00C0 + k,
        Ty::Short => 0x0001_8000 | (k & 0x7FFF), // the low half is negative
        Ty::Ushort => 0x0002_9001 + k,
        Ty::Bool => if k % 2 == 0 { 0x0000_0102 } else { 0x0000_0100 }, // low byte 2 -> true; 0 -> false
        Ty::Int => 0xFFFF_FF00 + k, // negative as an int; GL itself judges the value
        Ty::Sizei => 0x40 + k,      // a size is never negative (the engine raises GL_INVALID_VALUE for one: tests/gl_begin_end.rs)
        Ty::Uint => 0xDEAD_BE00 + k,
        Ty::Enum => 0x0DE1 + k,
        Ty::Bitfield => 0x4100 + k,
        _ => unreachable!(),
    }
}

/// Independent model of the wire word of a raw integer (not `Ty::int_word`): what each GL type means.
fn model_int(t: Ty, raw: u32) -> u64 {
    match t {
        Ty::Byte => i64::from(raw as u8 as i8) as u32 as u64,
        Ty::Ubyte => u64::from(raw as u8),
        Ty::Short => i64::from(raw as u16 as i16) as u32 as u64,
        Ty::Ushort => u64::from(raw as u16),
        Ty::Bool => u64::from(raw as u8 != 0),
        _ => u64::from(raw),
    }
}

fn float_value(k: usize) -> f64 {
    0.25 * (k as f64 + 1.0) + if k % 2 == 1 { -8.0 } else { 0.0 }
}

/// One call's arguments, placed for the program and the words the worker must see.
struct Planned {
    /// (parameter word, value) for integer-class args; the parameter word decides r3.. or the stack
    ints: Vec<(usize, IntArg)>,
    /// f1.. in order: a float or a double
    floats: Vec<(bool, i32)>,
    expect: Vec<u64>,
    /// data-area offset and byte length of the OUT buffer, and the type/element count it must hold afterwards
    out: Option<(i32, Ty, usize)>,
}

#[derive(Clone, Copy)]
enum IntArg {
    V(u32),
    Data(i32),
}

struct Data {
    bytes: Vec<u8>,
    base: i32,
}

impl Data {
    fn put(&mut self, b: &[u8]) -> i32 {
        while self.bytes.len() % 8 != 0 {
            self.bytes.push(0);
        }
        let at = self.base + self.bytes.len() as i32;
        self.bytes.extend_from_slice(b);
        at
    }
}

fn plan(sig: &Sig, salt: usize, d: &mut Data) -> Planned {
    let mut p = Planned { ints: Vec::new(), floats: Vec::new(), expect: Vec::new(), out: None };
    let mut slot = 0usize;
    let mut last_pname = 0u32;
    for (i, prm) in sig.params.iter().enumerate() {
        let k = salt + i;
        match *prm {
            P::S(t) if t.is_float() => {
                let v = float_value(k);
                let off = if t == Ty::Double { d.put(&v.to_be_bytes()) } else { d.put(&(v as f32).to_be_bytes()) };
                p.floats.push((t == Ty::Double, off));
                p.expect.push(if t == Ty::Double { v.to_bits() } else { u64::from((v as f32).to_bits()) });
                slot += t.slots();
            }
            P::S(t) => {
                // a scalar directly before a pname-sized vector IS the pname
                let sized = matches!(sig.params.get(i + 1), Some(P::In(_, _) | P::Out(_, _)));
                let domain = heirloom_gl_abi::checks::ENUM_ARGS.iter().find(|c| c.call == sig.name && c.arg == i);
                let r = if sized {
                    let kind = match sig.params[i + 1] { P::In(_, kd) | P::Out(_, kd) => kd, _ => unreachable!() };
                    pname_for(kind).0
                } else if let Some(c) = domain {
                    // an argument the engine checks: a value inside its domain (and not one refused by name)
                    c.allowed.iter().copied().find(|v| !c.absent.contains(v) && heirloom_gl_abi::gaps::refused_value(sig.name, &[u64::from(*v)]).is_none()).expect("an accepted value")
                } else if heirloom_gl_abi::checks::POSITIVE_ARGS.iter().any(|(c, idx)| *c == sig.name && idx.contains(&i)) {
                    3 + k as u32 // a division count the engine checks is at least 1
                } else {
                    raw(t, k)
                };
                last_pname = r;
                p.ints.push((slot, IntArg::V(r)));
                p.expect.push(model_int(t, r));
                slot += 1;
            }
            P::V(t, _) | P::In(t, _) => {
                let n_el = match *prm {
                    P::V(_, n) => usize::from(n),
                    P::In(_, kd) => {
                        assert_eq!(family::var_len(kd, last_pname), Some(pname_for(kd).1));
                        pname_for(kd).1
                    }
                    _ => unreachable!(),
                };
                let mut mem = Vec::new();
                for e in 0..n_el {
                    match t {
                        Ty::Float => { let v = float_value(k + e) as f32; mem.extend_from_slice(&v.to_be_bytes()); p.expect.push(u64::from(v.to_bits())); }
                        Ty::Double => { let v = float_value(k + e); mem.extend_from_slice(&v.to_be_bytes()); p.expect.push(v.to_bits()); }
                        _ => {
                            let r = raw(t, k + e);
                            match t.guest_bytes() { 1 => mem.push(r as u8), 2 => mem.extend_from_slice(&(r as u16).to_be_bytes()), _ => mem.extend_from_slice(&r.to_be_bytes()) }
                            p.expect.push(model_int(t, r));
                        }
                    }
                }
                p.ints.push((slot, IntArg::Data(d.put(&mem))));
                slot += 1;
            }
            P::Data(..) | P::OutAny(_) => unreachable!("bulk calls are graded by tests/gl_bulk.rs"),
            P::Out(t, kind) => {
                let n = family::var_len(kind, last_pname).unwrap();
                let off = d.put(&vec![0xEE; t.guest_bytes() * n + 8]);
                p.ints.push((slot, IntArg::Data(off)));
                p.out = Some((off, t, n));
                slot += 1;
            }
        }
    }
    p
}

fn li32(a: &mut Asm, r: u32, v: u32) {
    if v < 0x8000 { a.li(r, v as i32); } else { a.lis(r, (v >> 16) as i32).ori(r, r, (v & 0xFFFF) as i32); }
}

/// Build and run a program making every call of `sigs` in a context; returns the machine, what the worker saw, the data
/// base address, the plans, and where r3 of each call was kept.
fn run(sigs: &[&'static Sig]) -> (MacPpc<RecordingGuestUi>, Arc<Mutex<Seen>>, u32, Vec<Planned>, Vec<i32>) {
    let mut names: Vec<&str> = vec!["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglDestroyContext"];
    for s in sigs { if !names.contains(&s.name) { names.push(s.name); } }
    let base = (TOC_FIRST as usize + 4 * names.len()) as i32;
    let mut d = Data { bytes: Vec::new(), base };
    let plans: Vec<Planned> = sigs.iter().enumerate().map(|(n, s)| plan(s, 3 * n + 1, &mut d)).collect();
    let keeps: Vec<i32> = sigs.iter().map(|_| d.put(&[0xEE; 8])).collect();
    assert!(d.bytes.len() < 0x7000, "data area must stay addressable by a 16-bit offset");
    let toc = |n: &str| (TOC_FIRST as i32) + 4 * names.iter().position(|x| *x == n).unwrap() as i32;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -160, 1).stw(31, 156, 1).stw(30, 152, 1);
    a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(31, 3);
    a.mr(3, 31).li(4, 0).call_toc(toc("aglCreateContext")).mr(30, 3);
    a.mr(3, 30).call_toc(toc("aglSetCurrentContext"));
    for (n, (s, p)) in sigs.iter().zip(&plans).enumerate() {
        for (slot, v) in &p.ints {
            let r = if *slot < 8 { 3 + *slot as u32 } else { 0 };
            match v { IntArg::V(x) => li32(&mut a, r, *x), IntArg::Data(off) => { a.addi(r, 2, *off); } }
            if *slot >= 8 { a.stw(0, 24 + 4 * *slot as i32, 1); }
        }
        for (k, (dbl, off)) in p.floats.iter().enumerate() {
            if *dbl { a.lfd(1 + k as u32, *off, 2); } else { a.lfs(1 + k as u32, *off, 2); }
        }
        a.call_toc(toc(s.name));
        a.stw(3, keeps[n], 2);
    }
    a.mr(3, 30).call_toc(toc("aglDestroyContext"));
    a.li(3, 0).lwz(30, 152, 1).lwz(31, 156, 1).addi(1, 1, 160).lwz(0, 8, 1).mtlr(0).blr();
    let syms: Vec<(&str, SymbolClass, bool)> = names.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    let libs: [Lib; 1] = [("OpenGLLibrary", false, &syms)];
    let bytes = app_with(&libs, &a.finish(), &d.bytes);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.gl_attach(Box::new(Mock(TokenTable::default(), seen.clone())));
    let frag = m.instantiate("fam", &Pef::parse(&bytes).unwrap()).unwrap();
    let data = frag.section_bases[1].unwrap();
    m.start(&frag).unwrap();
    assert_eq!(m.step(20_000_000).0, Step::Exited, "{:?}", m.fault());
    (m, seen, data, plans, keeps)
}

/// The generated calls the contract marks implemented (every one, once the families are switched on).
fn implemented() -> Vec<&'static Sig> {
    // glBegin/glEnd and glNewList/glEndList open a state in which later calls are errors: tests/gl_begin_end.rs grades them
    // glAccum is always GL_INVALID_OPERATION on this visual (no accumulation buffer): `checks` and tests/gl_begin_end.rs grade it
    const STATEFUL: &[&str] = &["glBegin", "glEnd", "glNewList", "glEndList", "glAccum"];
    GENERATED.iter().filter(|s| !STATEFUL.contains(&s.name) && find(ENTRIES, s.name).is_some_and(|i| ENTRIES[i].implemented)).collect()
}

fn check(sigs: &[&'static Sig]) {
    let (m, seen, data, plans, keeps) = run(sigs);
    let seen = seen.lock().unwrap();
    // every call, in order (a refused or unsent one would shift the record and fail by name)
    let calls: Vec<_> = seen.calls.iter().filter(|c| !c.0.starts_with("agl")).collect();
    assert_eq!(calls.len(), sigs.len(), "every call reached the worker once");
    for (n, sig) in sigs.iter().enumerate() {
        assert_eq!(calls[n].0, sig.name);
        assert_eq!(calls[n].1, plans[n].expect, "{}: the words the worker received", sig.name);
        if let Some((off, t, cnt)) = plans[n].out {
            let at = data + off as u32;
            let mut want = Vec::new();
            for k in 0..cnt {
                match t.guest_bytes() {
                    1 => want.push(answer(t, k) as u8),
                    2 => want.extend_from_slice(&(answer(t, k) as u16).to_be_bytes()),
                    4 => want.extend_from_slice(&(answer(t, k) as u32).to_be_bytes()),
                    _ => want.extend_from_slice(&answer(t, k).to_be_bytes()),
                }
            }
            let got: Vec<u8> = (0..want.len() as u32 + 1).map(|b| m.mac.mem.read_u8(at + b).unwrap()).collect();
            assert_eq!(&got[..want.len()], &want[..], "{}: the OUT vector in guest memory (big-endian)", sig.name);
            assert_eq!(got[want.len()], 0xEE, "{}: not a byte past the vector is written", sig.name);
        } else if sig.ret != R::Void {
            let r3 = m.mac.mem.read_u32(data + keeps[n] as u32).unwrap();
            assert_eq!(r3, if sig.ret == R::Bool { 1 } else { 0x5150 }, "{}: the returned value reaches r3", sig.name);
        }
    }
}

#[test]
fn every_generated_call_is_switched_on_in_the_contract() {
    let off: Vec<_> = GENERATED.iter().filter(|s| find(ENTRIES, s.name).is_none_or(|i| !ENTRIES[i].implemented)).map(|s| s.name).collect();
    assert!(off.is_empty(), "generated and tested, but not switched on in the contract: {off:?}");
}

#[test]
fn every_implemented_family_call_arrives_as_the_words_its_types_say_and_answers_land_big_endian() {
    let all = implemented();
    assert!(all.len() > 100, "the families are switched on ({} are)", all.len());
    for chunk in all.chunks(32) {
        check(chunk);
    }
}

#[test]
fn an_int_after_floats_is_read_at_its_parameter_word_not_as_the_next_integer() {
    // glMapGrid2f(un, u1, u2, vn, v1, v2): vn is parameter word 3 (r6); glMapGrid2d's vn is word 5 (r8): a double takes two words
    let sigs: Vec<&Sig> = ["glMapGrid2f", "glMapGrid2d", "glRectd"].iter().map(|n| family::lookup(n).unwrap()).collect();
    assert!(sigs.iter().all(|s| find(ENTRIES, s.name).is_some_and(|i| ENTRIES[i].implemented)), "these three are switched on");
    check(&sigs);
}
