//! Deferred GL calls: the void state and client-array calls a game makes thousands of times a frame are QUEUED BY GUEST CODE
//! and served in a row at the program's next real GL call, instead of each leaving the guest's code for the engine.
//!
//! Why: on the Z83 a GL call that only sets state cost ~1 us of the guest thread (Warcraft III gameplay: ~6,000 GL calls a
//! frame, 6.3 ms), and ~60% of a light call was the generic way out of the guest and into the engine (the JIT's exit, the
//! glue dispatch, the safepoints, the stage clock), not the call's own work (callgrind, host harness, h86 glcap).
//!
//! How: a deferrable entry's TVector (`glue.rs`: `{code, toc}` at the cell) is pointed at a short PowerPC routine made here
//! ([`stub`]) instead of the cell's native `blr` stub. The routine appends one RECORD to a queue in guest memory - a header
//! word `(words after it) << 16 | contract entry`, then the GPR arguments r3.., then the FPR arguments f1.. as doubles (two
//! words each, `stfd`: the exact bits the native path reads), then the words a pointer argument points at, copied AT THE
//! CALL as GL itself copies them (`glLoadMatrixf`'s matrix) - and returns. It uses only r0, r11, r12, CR0 and (slow path)
//! CTR, all volatile in the PowerOpen ABI; r3 is left as the first argument (every deferred entry returns void). When the
//! queue is full the routine branches to the cell's native stub instead, which serves the call the ordinary way.
//!
//! Exactness: EVERY GL native ([`MacPpc::gl_call`]) first drains the queue, record by record, through the same code a native
//! call runs ([`MacPpc::gl_call_now`], registers rebuilt from the record), so the worker receives the same frames in the same
//! order and batches as without the queue, and a query (`glGetError`, `glGet*`, `glIsEnabled`) or a draw sees every call made
//! before it. The engine drains only inside a GL native, where no queue routine can be part-way through (one guest CPU; the
//! Thread Manager and MP tasks switch only at natives).
//!
//! NAMED LIMITS: a deferred call the engine REFUSES (a value the stack under the worker cannot render: a fault that stops the
//! program) faults at the next GL native rather than at itself, with the same message; the queue's records are counted as
//! that native's time (the frame ledger's `gl` is unchanged in meaning; `draw_us` has the drained time taken out).
use super::*;
use crate::asm::Asm;

/// A deferrable entry: `ints` GPR arguments (r3..), then `dbls` FPR arguments (f1..), then `copy` words read at the pointer
/// in the GPR after the ints (r3 + ints), copied into the record.
pub(super) struct Shape { name: &'static str, ints: u8, dbls: u8, copy: u8 }

impl Shape {
    /// Words of a record after its header.
    fn words(&self) -> u32 { u32::from(self.ints) + 2 * u32::from(self.dbls) + u32::from(self.copy) }
}

const fn s(name: &'static str, ints: u8, dbls: u8, copy: u8) -> Shape { Shape { name, ints, dbls, copy } }

/// The entry named `name`'s [`DEFERRED`] index + 1, 0 = not deferrable (the dispatch table's `shape`).
pub(super) fn shape_of(name: &str) -> u8 { DEFERRED.iter().position(|s| s.name == name).map_or(0, |i| i as u8 + 1) }

/// The deferred entries: void calls whose arguments are registers (or a fixed-length vector), and whose effect a later call
/// observes only through the worker or the engine's client-array state - which every GL native drains the queue before.
pub(super) const DEFERRED: &[Shape] = &[
    s("glActiveTextureARB", 1, 0, 0), s("glClientActiveTextureARB", 1, 0, 0),
    s("glEnable", 1, 0, 0), s("glDisable", 1, 0, 0), s("glEnableClientState", 1, 0, 0), s("glDisableClientState", 1, 0, 0),
    s("glMatrixMode", 1, 0, 0), s("glDepthMask", 1, 0, 0), s("glDepthFunc", 1, 0, 0), s("glClear", 1, 0, 0),
    s("glLockArraysEXT", 2, 0, 0), s("glUnlockArraysEXT", 0, 0, 0),
    s("glVertexPointer", 4, 0, 0), s("glColorPointer", 4, 0, 0), s("glTexCoordPointer", 4, 0, 0), s("glNormalPointer", 3, 0, 0),
    s("glBindTexture", 2, 0, 0), s("glBlendFunc", 2, 0, 0), s("glPixelStorei", 2, 0, 0),
    s("glTexEnvi", 3, 0, 0), s("glTexParameteri", 3, 0, 0),
    s("glViewport", 4, 0, 0), s("glScissor", 4, 0, 0),
    s("glAlphaFunc", 1, 1, 0), s("glFogf", 1, 1, 0), s("glDepthRange", 0, 2, 0), s("glClearColor", 0, 4, 0),
    s("glLoadMatrixf", 0, 0, 16), s("glNormal3fv", 0, 0, 3),
];

// the drain fills r3..r6 and f1..f4 at most (an unrolled copy, no `memcpy` call per record)
const _: () = { let mut i = 0; while i < DEFERRED.len() { assert!(DEFERRED[i].ints <= 4 && DEFERRED[i].dbls <= 4); i += 1; } };

/// The queue's record bytes, and the room the limit keeps free for the largest record (`glLoadMatrixf`: 17 words).
const QUEUE_BYTES: u32 = 16 * 1024;
const MAX_RECORD: u32 = 4 * 17;
/// The routines' code: its own pages (page-aligned inside a larger block), so the queue's stores never write a page the JIT
/// translated code from.
const PAGE: u32 = 4096;
const CODE_BYTES: u32 = 2 * PAGE;

#[derive(Default)]
pub(crate) struct DeferQ {
    /// No import is pointed at a queue routine (the reference behaviour, and the A/B switch).
    off: bool,
    /// Guest address of `{cursor, limit}`, the records following; 0 = not made yet.
    pub(super) ctl: u32,
    /// The next free byte of the code pages, and their end.
    code_at: u32,
    code_end: u32,
    /// Glue cells already pointed at their routine.
    bound: BTreeSet<usize>,
    /// The records being drained (host copy), and the register file they are served from: reused drain after drain.
    words: Vec<u32>,
    cpu: Option<Box<Cpu>>,
    /// Records served so far, and the stage-clock time the drains took (perf on).
    pub drained: u64,
    pub(crate) drain_time: u64,
}

/// The routine for `shape`: the queue's control block at `ctl`, the cell's native stub (the slow path) at `native`.
pub(super) fn stub(shape: &Shape, entry: u16, ctl: u32, native: u32) -> Vec<u8> {
    let hi = |v: u32| (v >> 16) as i32;
    let lo = |v: u32| (v & 0xFFFF) as i32;
    let n = shape.words();
    let head = n << 16 | u32::from(entry);
    let mut a = Asm::new();
    a.lis(12, hi(ctl)).ori(12, 12, lo(ctl)).lwz(11, 0, 12).lwz(0, 4, 12).cmplw(11, 0).bge("slow");
    a.lis(0, hi(head)).ori(0, 0, lo(head)).stw(0, 0, 11);
    let mut off = 4;
    for i in 0..u32::from(shape.ints) { a.stw(3 + i, off, 11); off += 4; }
    for j in 0..u32::from(shape.dbls) { a.stfd(1 + j, off, 11); off += 8; }
    let p = 3 + u32::from(shape.ints);
    for k in 0..i32::from(shape.copy) { a.lwz(0, 4 * k, p).stw(0, off, 11); off += 4; }
    a.addi(11, 11, off).stw(11, 0, 12).blr();
    a.label("slow").lis(12, hi(native)).ori(12, 12, lo(native)).mtctr(12).bctr();
    a.finish()
}

impl<U: GuestUi> MacPpc<U> {
    /// Defer the deferrable GL calls (on by default). Only imports bound after the switch follow it.
    pub fn gl_defer(&mut self, on: bool) { self.gl.defer.off = !on; }
    /// Deferred records served so far.
    pub fn gl_deferred(&self) -> u64 { self.gl.defer.drained }

    /// Glue cell `id` is bound to GL entry `f` and imported as a TVector: point it at the entry's queue routine when the entry
    /// is deferrable. A cell is pointed once.
    pub(crate) fn gl_defer_bind(&mut self, id: usize, f: GlFn) -> Result<(), String> {
        if self.gl.defer.off || self.gl.defer.bound.contains(&id) { return Ok(()); }
        let k = self.gl_disp(f.0).shape;
        if k == 0 { return Ok(()); }
        let Some(e) = self.glue.entry(id).cloned() else { return Ok(()) };
        if self.gl.defer.ctl == 0 { self.gl_defer_make()?; }
        let code = stub(&DEFERRED[usize::from(k) - 1], f.0, self.gl.defer.ctl, e.stub);
        let at = self.gl.defer.code_at;
        if at + code.len() as u32 > self.gl.defer.code_end { return Ok(()); } // the code pages are full: this entry stays a native
        self.put_bytes(at, &code, "a deferred GL call's routine")?;
        self.gl.defer.code_at = at + code.len() as u32;
        self.mac.mem.write_u32(e.tvector, at).map_err(|_| format!("{}: its TVector at {:#x} leaves guest RAM", e.symbol, e.tvector))?;
        self.gl.defer.bound.insert(id);
        Ok(())
    }

    /// The queue (control block + records) and the code pages, from the guest heap.
    fn gl_defer_make(&mut self) -> Result<(), String> {
        let m = &mut self.mac;
        let q = m.mm.new_ptr(&mut m.mem, &mut m.heap, 8 + QUEUE_BYTES, true).map_err(|e| format!("the deferred GL call queue: no memory ({e})"))?;
        let c = m.mm.new_ptr(&mut m.mem, &mut m.heap, CODE_BYTES + PAGE, true).map_err(|e| format!("the deferred GL calls' code: no memory ({e})"))?;
        let base = q + 8;
        m.mem.write_u32(q, base).and_then(|_| m.mem.write_u32(q + 4, base + QUEUE_BYTES - MAX_RECORD)).map_err(|_| String::from("the deferred GL call queue leaves guest RAM"))?;
        let start = (c + PAGE - 1) & !(PAGE - 1);
        (self.gl.defer.ctl, self.gl.defer.code_at, self.gl.defer.code_end) = (q, start, start + CODE_BYTES);
        Ok(())
    }

    /// Serve every queued record, oldest first, through [`MacPpc::gl_call_now`]: what each call would have done had it been a
    /// native call. The queue is empty afterwards (also when a record fails: its error is the caller's).
    pub(super) fn gl_defer_drain(&mut self) -> Result<(), String> {
        let ctl = self.gl.defer.ctl;
        let base = ctl + 8;
        let cur = self.mac.mem.read_u32(ctl).map_err(|_| String::from("the deferred GL call queue leaves guest RAM"))?;
        if cur == base { return Ok(()); }
        if cur < base || cur > base + QUEUE_BYTES || cur % 4 != 0 { return Err(format!("[HEIRLOOM-GL] the deferred GL call queue's cursor {cur:#x} is outside its queue at {base:#x}")); }
        let t0 = self.perf.as_ref().map(|p| p.now());
        self.mac.mem.write_u32(ctl, base).map_err(|_| String::from("the deferred GL call queue leaves guest RAM"))?;
        let mut words = core::mem::take(&mut self.gl.defer.words);
        words.clear();
        let bytes = self.mac.mem.span(base, cur - base).and_then(|s| self.mac.mem.bytes(s)).map_err(|_| String::from("the deferred GL call queue leaves guest RAM"))?;
        words.extend(bytes.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])));
        // boxed: taking it out and putting it back moves a pointer, not the register file (a `memcpy` call - on CoralOS three
        // microcoded string operations - per drain)
        let mut cpu = self.gl.defer.cpu.take().unwrap_or_else(|| Box::new(Cpu::new(0, 0)));
        let mut at = 0;
        let r = loop {
            let Some(&head) = words.get(at) else { break Ok(()) };
            let (entry, n) = ((head & 0xFFFF) as u16, (head >> 16) as usize);
            let d = self.gl_disp(entry);
            let shape = DEFERRED.get(usize::from(d.shape).wrapping_sub(1));
            let (Some(shape), Some(a)) = (shape.filter(|s| s.words() as usize == n), words.get(at + 1..at + 1 + n)) else {
                break Err(format!("[HEIRLOOM-GL] the deferred GL call queue holds a record no routine writes ({head:#x} at word {at})"));
            };
            let ints = usize::from(shape.ints);
            // only the shape's words, one store each (a copy of `ints` words would be a `memcpy` call per record)
            match *a {
                [w0, w1, w2, w3, ..] if ints == 4 => (cpu.gpr[3], cpu.gpr[4], cpu.gpr[5], cpu.gpr[6]) = (w0, w1, w2, w3),
                [w0, w1, w2, ..] if ints == 3 => (cpu.gpr[3], cpu.gpr[4], cpu.gpr[5]) = (w0, w1, w2),
                [w0, w1, ..] if ints == 2 => (cpu.gpr[3], cpu.gpr[4]) = (w0, w1),
                [w0, ..] if ints == 1 => cpu.gpr[3] = w0,
                _ => {}
            }
            for j in 0..usize::from(shape.dbls) { cpu.fpr[1 + j] = u64::from(a[ints + 2 * j]) << 32 | u64::from(a[ints + 2 * j + 1]); }
            if shape.copy != 0 { cpu.gpr[3 + ints] = base + 4 * (at + 1 + ints + 2 * usize::from(shape.dbls)) as u32; }
            if let Err(e) = self.gl_call_with(GlFn(entry), d, &mut cpu) { break Err(e); }
            self.gl.defer.drained += 1;
            at += 1 + n;
        };
        (self.gl.defer.words, self.gl.defer.cpu) = (words, Some(cpu));
        if let (Some(t0), Some(p)) = (t0, self.perf.as_mut()) { let dt = p.now().wrapping_sub(t0); self.gl.defer.drain_time += dt; }
        r
    }
}
