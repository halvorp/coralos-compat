//! The state shadow (gl_c/shadow.rs): a call that sets what the worker already holds is not sent; glActiveTextureARB is sent only
//! before a call that depends on the active unit; anything the shadow does not model makes it forget; a value outside the sets
//! GL 1.2 always accepts is always sent. Negative control: with the shadow off every call is sent.
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{Command, Frame, Token, TokenTable};
use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;
use std::sync::{Arc, Mutex};

/// A worker that answers a context and retires batches, and keeps every Call and Draw it receives as (name, args).
struct Mock(TokenTable, Arc<Mutex<Vec<(String, Vec<u64>)>>>);
impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let none = Token { slot: 0, incarnation: 0 };
        let (mut out, mut at) = (Vec::new(), 0);
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            at += n;
            match f.command {
                Command::ContextCreate => {
                    let t = self.0.create().unwrap();
                    out.extend(Frame { token: none, command: Command::Reply { status: 0, words: vec![u64::from(t.slot), u64::from(t.incarnation)], text: String::new() } }.encode());
                }
                Command::Call { entry, args } => {
                    let name = heirloom_gl_abi::ENTRIES[usize::from(entry)].name;
                    if name == "aglChoosePixelFormat" { out.extend(Frame { token: none, command: Command::Reply { status: 0, words: vec![1], text: String::new() } }.encode()); }
                    self.1.lock().unwrap().push((name.into(), args));
                }
                Command::Draw(_) => self.1.lock().unwrap().push(("Draw".into(), vec![])),
                Command::BatchEnd { seq } => out.extend(Frame { token: none, command: Command::Retired { seq } }.encode()),
                _ => {}
            }
        }
        out
    }
}

const NAMES: &[&str] = &["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglSwapBuffers", "glActiveTextureARB", "glEnable", "glDisable",
    "glBindTexture", "glTexEnvi", "glTexParameteri", "glBlendFunc", "glAlphaFunc", "glMatrixMode", "glLoadMatrixf", "glViewport", "glPushMatrix",
    "glDeleteTextures", "glClear"];

struct Gl { m: MacPpc<RecordingGuestUi>, stubs: Vec<u32>, cpu: Cpu, seen: Arc<Mutex<Vec<(String, Vec<u64>)>>>, ctx: u32 }

impl Gl {
    fn new(shadow: bool) -> Gl {
        let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
        m.gl_shadow(shadow);
        let seen = Arc::new(Mutex::new(Vec::new()));
        m.gl_attach(Box::new(Mock(TokenTable::default(), seen.clone())));
        let stubs = NAMES.iter().map(|n| { m.bind_from("OpenGLLibrary", n); let id = m.glue.intern(&mut m.mac, "OpenGLLibrary", n).unwrap(); m.glue.entry(id).unwrap().stub }).collect();
        let mut g = Gl { m, stubs, cpu: Cpu::new(0x5000, 0x0004_0000), seen, ctx: 0 };
        let pf = g.call("aglChoosePixelFormat", &[0, 0, 0]);
        g.ctx = g.call("aglCreateContext", &[pf, 0]);
        g.call("aglSetCurrentContext", &[g.ctx]);
        g
    }
    fn call(&mut self, name: &str, args: &[u32]) -> u32 {
        let i = NAMES.iter().position(|n| *n == name).unwrap();
        self.cpu.gpr[3..3 + args.len()].copy_from_slice(args);
        assert!(self.m.native(&mut self.cpu, self.stubs[i]));
        assert!(self.m.fault().is_none(), "{name}: {:?}", self.m.fault());
        self.cpu.gpr[3]
    }
    /// The calls the worker received since the last look (a swap flushes the batch first), swaps left out.
    fn sent(&mut self) -> Vec<String> {
        let ctx = self.ctx;
        self.call("aglSwapBuffers", &[ctx]);
        let v = std::mem::take(&mut *self.seen.lock().unwrap());
        v.into_iter().filter(|(n, _)| !n.starts_with("agl")).map(|(n, a)| format!("{n}{}", a.iter().map(|x| format!(" {x:#x}")).collect::<String>())).collect()
    }
}

const MAT: u32 = 0x0002_0000;

fn script(g: &mut Gl) -> Vec<Vec<String>> {
    for k in 0..16 { g.m.mac.mem.write_u32(MAT + 4 * k, (k as f32).to_bits()).unwrap(); }
    let mut out = vec![g.sent()];
    // a frame of state, then the same frame again
    for _ in 0..2 {
        g.call("glEnable", &[0x0BE2]); g.call("glBlendFunc", &[0x302, 0x303]); g.cpu.fpr[1] = 0.5f64.to_bits(); g.call("glAlphaFunc", &[0x206]);
        g.call("glActiveTextureARB", &[0x84C0]); g.call("glEnable", &[0x0DE1]); g.call("glBindTexture", &[0x0DE1, 7]);
        g.call("glTexEnvi", &[0x2300, 0x2200, 0x2100]); g.call("glTexParameteri", &[0x0DE1, 0x2801, 0x2601]);
        g.call("glMatrixMode", &[0x1700]); g.call("glLoadMatrixf", &[MAT]); g.call("glViewport", &[0, 0, 64, 64]);
        out.push(g.sent());
    }
    // unit switches with nothing between that depends on the unit: nothing; then a unit-dependent call: the unit first
    g.call("glActiveTextureARB", &[0x84C1]); g.call("glActiveTextureARB", &[0x84C0]); g.call("glActiveTextureARB", &[0x84C1]); g.call("glClear", &[0x4000]);
    out.push(g.sent());
    g.call("glDisable", &[0x0DE1]);
    out.push(g.sent());
    // a call the shadow does not model (glPushMatrix): it forgets, so the same matrix is sent again
    g.call("glPushMatrix", &[]); g.call("glLoadMatrixf", &[MAT]); g.call("glEnable", &[0x0BE2]);
    out.push(g.sent());
    // deleting a texture forgets (its bindings among all else); a cap and a unit outside the shadowed sets are always sent
    g.m.mac.mem.write_u32(MAT + 0x100, 7).unwrap();
    g.call("glDeleteTextures", &[1, MAT + 0x100]); g.call("glBindTexture", &[0x0DE1, 7]); g.call("glBindTexture", &[0x0DE1, 7]);
    g.call("glEnable", &[0x1234]); g.call("glEnable", &[0x1234]); g.call("glActiveTextureARB", &[0x84C3]); g.call("glActiveTextureARB", &[0x84C3]);
    out.push(g.sent());
    out
}

#[test]
fn a_call_that_sets_what_the_worker_holds_is_not_sent_and_the_active_unit_is_sent_when_used() {
    let mut g = Gl::new(true);
    let f = script(&mut g);
    assert_eq!(f[1].len(), 11, "the first frame: every call (the unit as it is used) {:?}", f[1]);
    assert_eq!(f[1][3], "glActiveTextureARB 0x84c0", "the unit goes out before the texture enable: {:?}", f[1]);
    assert_eq!(f[2], Vec::<String>::new(), "the same frame again sends nothing");
    assert_eq!(f[3], vec!["glClear 0x4000"], "unit switches with nothing using them send nothing");
    assert_eq!(f[4], vec!["glActiveTextureARB 0x84c1", "glDisable 0xde1"], "the selected unit, then the call that uses it");
    assert_eq!(f[5], vec!["glPushMatrix", "glLoadMatrixf 0x0 0x3f800000 0x40000000 0x40400000 0x40800000 0x40a00000 0x40c00000 0x40e00000 0x41000000 0x41100000 0x41200000 0x41300000 0x41400000 0x41500000 0x41600000 0x41700000", "glEnable 0xbe2"]);
    // after a call it does not model the shadow knows no active unit either (glPopAttrib could have changed it): both bindings go
    assert_eq!(f[6], vec!["glDeleteTextures 0x1 0x7", "glBindTexture 0xde1 0x7", "glBindTexture 0xde1 0x7", "glEnable 0x1234", "glEnable 0x1234", "glActiveTextureARB 0x84c3", "glActiveTextureARB 0x84c3"]);
    assert_eq!(g.m.gl_shadowed(), 13, "11 repeated + 3 unit switches - 1 sent when used");
}

#[test]
fn with_the_shadow_off_every_call_is_sent() {
    let mut g = Gl::new(false);
    let f = script(&mut g);
    assert_eq!(f[1].len(), 11);
    assert_eq!(f[2], f[1], "NEGATIVE CONTROL: the repeated frame is sent again");
    assert_eq!(f[3].len(), 4);
    assert_eq!(g.m.gl_shadowed(), 0);
}
