//! The engine's per-call GL path does not allocate: the hottest calls of a game frame (Warcraft III gameplay: glActiveTextureARB,
//! glEnable/glDisable, glMatrixMode + glLoadMatrixf, glBindTexture, glTexEnvi, glBlendFunc, glMaterialfv) encode their words into
//! one reused buffer and the batch, so the allocator is reached only when the batch itself grows (amortised, a handful of times
//! for thousands of calls). Counted with a counting global allocator around the calls alone.
use heirloom_gl_abi::command::peek_len;
use heirloom_gl_abi::{Command, Frame, Token, TokenTable};
use heirloom_macppc_engine::cpu::{Cpu, Host};
use heirloom_macppc_engine::MacPpc;
use heirloom_ui_abi::RecordingGuestUi;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
thread_local! { static ON: Cell<bool> = const { Cell::new(false) }; }

// SAFETY: forwards every call unchanged to the system allocator; the counter is a side effect only.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if ON.with(|o| o.get()) { ALLOCS.fetch_add(1, Ordering::Relaxed); }
        // SAFETY: the caller's contract for `alloc` is passed through to System.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: `p` came from System.alloc/realloc with layout `l`.
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ON.with(|o| o.get()) { ALLOCS.fetch_add(1, Ordering::Relaxed); }
        // SAFETY: `p` came from System with layout `l`; the caller's contract for `realloc` is passed through.
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

/// A worker that answers a context and retires batches, and keeps nothing.
struct Mock(TokenTable);
impl heirloom_macppc_engine::gl_c::GlWorker for Mock {
    fn transact(&mut self, req: &[u8]) -> Vec<u8> {
        let none = Token { slot: 0, incarnation: 0 };
        let (mut out, mut at) = (Vec::new(), 0);
        while at < req.len() {
            let n = peek_len(&req[at..]).unwrap().unwrap();
            let (f, _) = Frame::decode(&req[at..at + n]).unwrap();
            at += n;
            let words = match f.command {
                Command::ContextCreate => { let t = self.0.create().unwrap(); Some(vec![u64::from(t.slot), u64::from(t.incarnation)]) }
                Command::Call { entry, .. } if heirloom_gl_abi::ENTRIES[usize::from(entry)].name == "aglChoosePixelFormat" => Some(vec![1]),
                Command::BatchEnd { seq } => { out.extend(Frame { token: none, command: Command::Retired { seq } }.encode()); None }
                _ => None,
            };
            if let Some(words) = words { out.extend(Frame { token: none, command: Command::Reply { status: 0, words, text: String::new() } }.encode()); }
        }
        out
    }
}

/// Call the OpenGLLibrary export whose glue stub is `stub` the way JIT code does (`Host::native` with the guest's own CPU), `args`
/// in r3...
fn gl(m: &mut MacPpc<RecordingGuestUi>, cpu: &mut Cpu, stub: u32, args: &[u32]) -> u32 {
    cpu.gpr[3..3 + args.len()].copy_from_slice(args);
    cpu.pc = stub;
    assert!(m.native(cpu, stub));
    assert!(m.fault().is_none(), "{:?}", m.fault());
    cpu.gpr[3]
}

#[test]
fn the_hot_gl_calls_of_a_frame_do_not_allocate_per_call() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.gl_attach(Box::new(Mock(TokenTable::default())));
    let names = ["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "glActiveTextureARB", "glEnable", "glDisable", "glMatrixMode",
        "glLoadMatrixf", "glBindTexture", "glTexEnvi", "glBlendFunc", "glMaterialfv"];
    let k: Vec<u32> = names.iter().map(|n| {
        m.bind_from("OpenGLLibrary", n);
        let id = m.glue.intern(&mut m.mac, "OpenGLLibrary", n).unwrap();
        m.glue.entry(id).unwrap().stub
    }).collect();
    let mut cpu = Cpu::new(0x5000, 0x0004_0000);
    let pf = gl(&mut m, &mut cpu, k[0], &[0, 0, 0]);
    let ctx = gl(&mut m, &mut cpu, k[1], &[pf, 0]);
    assert_eq!(gl(&mut m, &mut cpu, k[2], &[ctx]), 1);
    const MATRIX: u32 = 0x0002_0000; // 16 floats (zero is a valid matrix) and a 4-float material colour
    let frame = |m: &mut MacPpc<RecordingGuestUi>, cpu: &mut Cpu| {
        gl(m, cpu, k[3], &[0x84C0]);
        gl(m, cpu, k[4], &[0x0DE1]);
        gl(m, cpu, k[5], &[0x0BE2]);
        gl(m, cpu, k[6], &[0x1700]);
        gl(m, cpu, k[7], &[MATRIX]);
        gl(m, cpu, k[8], &[0x0DE1, 7]);
        gl(m, cpu, k[9], &[0x2300, 0x2200, 0x2100]);
        gl(m, cpu, k[10], &[0x0302, 0x0303]);
        gl(m, cpu, k[11], &[0x0408, 0x1201, MATRIX]);
    };
    frame(&mut m, &mut cpu); // warm: the reused buffers exist
    ALLOCS.store(0, Ordering::Relaxed);
    ON.with(|o| o.set(true));
    for _ in 0..1000 { frame(&mut m, &mut cpu); }
    ON.with(|o| o.set(false));
    let n = ALLOCS.load(Ordering::Relaxed);
    // 9,000 calls; only the batch's own amortised growth may allocate (~log2 of its final size)
    assert!(n < 64, "{n} allocations for 9,000 GL calls");
}
