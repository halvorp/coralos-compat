//! `gl_call` dispatches by each contract entry's route (resolved once, the state calls learnt at their first call) instead of by
//! name: the SECOND call of an entry - routed - sends the worker exactly the bytes and leaves exactly the state the FIRST did, for
//! the forwarded calls, the matrix and client-array calls, a draw, a state call (glBindTexture: learnt) and an entry served by
//! another arm (glGetError: never learnt as a state call, it would have no encoder there).
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{Command, Frame, Token, TokenTable};
use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;
use std::sync::{Arc, Mutex};

/// A worker that answers a context and retires batches, and keeps every request byte except the batch ends (their sequence
/// numbers differ by design).
struct Mock(TokenTable, Arc<Mutex<Vec<u8>>>);
impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let none = Token { slot: 0, incarnation: 0 };
        let (mut out, mut at) = (Vec::new(), 0);
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            if !matches!(f.command, Command::BatchEnd { .. }) { self.1.lock().unwrap().extend_from_slice(&req[at..at + n]); }
            at += n;
            let words = match f.command {
                Command::ContextCreate => { let t = self.0.create().unwrap(); Some(vec![u64::from(t.slot), u64::from(t.incarnation)]) }
                Command::Call { entry, .. } if matches!(heirloom_gl_abi::ENTRIES[usize::from(entry)].name, "aglChoosePixelFormat" | "glGetError") => {
                    Some(vec![if heirloom_gl_abi::ENTRIES[usize::from(entry)].name == "glGetError" { 0 } else { 1 }])
                }
                Command::BatchEnd { seq } => { out.extend(Frame { token: none, command: Command::Retired { seq } }.encode()); None }
                _ => None,
            };
            if let Some(words) = words { out.extend(Frame { token: none, command: Command::Reply { status: 0, words, text: String::new() } }.encode()); }
        }
        out
    }
}

#[test]
fn a_routed_call_sends_and_leaves_exactly_what_its_first_call_did() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    m.gl_attach(Box::new(Mock(TokenTable::default(), seen.clone())));
    let names = ["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglSwapBuffers", "glActiveTextureARB", "glEnable", "glViewport",
        "glMatrixMode", "glLoadMatrixf", "glClientActiveTextureARB", "glVertexPointer", "glEnableClientState", "glDrawElements", "glDisableClientState",
        "glLockArraysEXT", "glBindTexture", "glGetError"];
    let stub: Vec<u32> = names.iter().map(|n| {
        m.bind_from("OpenGLLibrary", n);
        let id = m.glue.intern(&mut m.mac, "OpenGLLibrary", n).unwrap();
        m.glue.entry(id).unwrap().stub
    }).collect();
    let mut cpu = Cpu::new(0x5000, 0x0004_0000);
    let mut gl = |m: &mut MacPpc<RecordingGuestUi>, i: usize, args: &[u32]| -> u32 {
        cpu.gpr[3..3 + args.len()].copy_from_slice(args);
        assert!(m.native(&mut cpu, stub[i]));
        assert!(m.fault().is_none(), "{}: {:?}", names[i], m.fault());
        cpu.gpr[3]
    };
    const MEM: u32 = 0x0002_0000; // a matrix, 3 vertices of 3 floats and 3 u16 indices (all big-endian)
    for (k, v) in [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0].iter().enumerate() { m.mac.mem.write_u32(MEM + 0x40 + 4 * k as u32, v.to_bits()).unwrap(); }
    for (k, i) in [0u16, 1, 2].iter().enumerate() { m.mac.mem.write_u16(MEM + 0x80 + 2 * k as u32, *i).unwrap(); }
    let pf = gl(&mut m, 0, &[0, 0, 0]);
    let ctx = gl(&mut m, 1, &[pf, 0]);
    assert_eq!(gl(&mut m, 2, &[ctx]), 1);
    gl(&mut m, 3, &[ctx]); // a swap: MakeCurrent goes out before the frames compared
    let mut frame = |m: &mut MacPpc<RecordingGuestUi>| -> (Vec<u8>, Vec<u32>) {
        seen.lock().unwrap().clear();
        let mut answers = vec![
            gl(m, 4, &[0x84C1]), gl(m, 5, &[0x0DE1]), gl(m, 6, &[0, 0, 640, 480]), gl(m, 7, &[0x1701]), gl(m, 8, &[MEM]),
            gl(m, 9, &[0x84C0]), gl(m, 10, &[3, 0x1406, 0, MEM + 0x40]), gl(m, 11, &[0x8074]), gl(m, 14, &[0, 3]),
            gl(m, 12, &[4, 3, 0x1403, MEM + 0x80]), gl(m, 13, &[0x8074]), gl(m, 15, &[0x0DE1, 9]),
        ];
        answers.push(gl(m, 16, &[])); // glGetError: answered by the engine/worker, not a state call
        gl(m, 12, &[4, 3, 0x1403, MEM + 0x80]); // a draw with the vertex array disabled again: no arrays
        gl(m, 3, &[ctx]); // swap: flushes the batch
        (seen.lock().unwrap().clone(), answers)
    };
    let first = frame(&mut m);
    assert!(first.0.len() > 400, "the frame reached the worker: {} bytes", first.0.len());
    let second = frame(&mut m);
    assert_eq!(first.1, second.1, "answers");
    assert_eq!(first.0, second.0, "the routed calls sent different bytes");
}
