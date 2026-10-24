//! C library functions that call the PROGRAM back: `qsort`/`bsearch` (the comparison function), `atexit` handlers run by `exit`. Same machine as
//! the Toolbox callbacks ([`crate::callback`]): the native saves the interrupted call in a [`Frame`], enters the program's routine (a CFM function
//! pointer is a TVector: code, TOC) with LR = `$CallbackReturn`, and the frame's typed continuation decides what happens at the return. Nothing
//! re-enters the interpreter from inside a handler.
//!
//! * `qsort` is a binary-insertion sort done in place in guest memory: after element i is copied to a scratch buffer, each probe is one callback
//!   `cmp(scratch, &base[mid])`; equal elements go right, so it is STABLE (ISO C leaves the order of equal elements unspecified). O(n log n) calls.
//! * `bsearch` is the classic halving search, `cmp(key, &base[mid])`: negative = left half, positive = right half, zero = found.
//! * `exit(status)` runs the handlers last-registered-first, each as a callback, then ends the run. `ExitToShell` does not run them (as in C, where
//!   only `exit` and a return from `main` through the startup code do).
//! The comparison's result is the callee's r3 read as a signed 32-bit `int`.
use crate::callback::{Continuation, Frame, Saved};
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

#[derive(Debug, Clone)]
pub(crate) struct Sort { base: u32, n: u32, size: u32, cmp: u32, scratch: u32, i: u32, lo: u32, hi: u32, mid: u32 }
#[derive(Debug, Clone)]
pub(crate) struct Search { key: u32, base: u32, size: u32, cmp: u32, lo: u32, hi: u32, mid: u32 }

impl<U: GuestUi> MacPpc<U> {
    /// The frame for a native C call that will return to its caller (LR) with a result in r3.
    fn c_frame(&self, cpu: &Cpu, cont: Continuation) -> Frame { Frame { saved: Saved::of(cpu, cpu.lr & !3), cont } }

    fn check_cmp(&self, who: &str, cmp: u32) -> Result<(), String> {
        if cmp == 0 { return Err(format!("{who}: the comparison function pointer is NULL")); }
        self.mac.mem.read_u32(cmp).and_then(|_| self.mac.mem.read_u32(cmp + 4)).map(|_| ()).map_err(|e| format!("{who}: the comparison function pointer {cmp:#x} is not a TVector in guest RAM: {e}"))
    }

    /// `qsort(base, nmemb, size, compar)`.
    pub(crate) fn start_qsort(&mut self, cpu: &mut Cpu, base: u32, n: u32, size: u32, cmp: u32) -> Result<(), String> {
        if n < 2 { cpu.gpr[3] = 0; return Ok(()); }
        if size == 0 { return Err(String::from("qsort: element size 0")); }
        self.check_cmp("qsort", cmp)?;
        let total = n.checked_mul(size).ok_or_else(|| String::from("qsort: nmemb * size overflows"))?;
        self.mac.mem.span(base, total).map_err(|_| format!("qsort: the {total}-byte array at {base:#x} leaves guest RAM"))?;
        let scratch = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, size, false).map_err(|e| format!("qsort: no memory for the scratch element (memory error {e})"))?;
        let st = Sort { base, n, size, cmp, scratch, i: 0, lo: 0, hi: 0, mid: 0 };
        let frame = self.c_frame(cpu, Continuation::Qsort(st));
        self.sort_advance(cpu, frame, None)
    }

    pub(crate) fn sort_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let r = cpu.gpr[3] as i32;
        self.sort_advance(cpu, frame, Some(r))
    }

    /// Apply the comparison result (if any) and run to the next probe, or finish.
    fn sort_advance(&mut self, cpu: &mut Cpu, mut frame: Frame, result: Option<i32>) -> Result<(), String> {
        let Continuation::Qsort(st) = &mut frame.cont else { return Err(String::from("qsort step on another continuation")) };
        if let Some(r) = result { if r < 0 { st.hi = st.mid; } else { st.lo = st.mid + 1; } }
        loop {
            if st.i == 0 { st.i = 1; self.sort_load(st)?; }
            if st.lo < st.hi {
                st.mid = st.lo + (st.hi - st.lo) / 2;
                let (key, elem, cmp, sp) = (st.scratch, st.base + st.mid * st.size, st.cmp, frame.saved.gpr[1]);
                self.enter_tvector(cpu, cmp, &[key, elem], sp).map_err(|e| format!("qsort: the comparison function: {e}"))?;
                self.rt.callbacks.push(frame);
                return Ok(());
            }
            // insert the scratch element at lo: shift [lo, i) up by one slot
            let (at, from, n) = (st.base + st.lo * st.size, st.i, st.size);
            let bytes = self.get_bytes(at, (from - st.lo) * n, "qsort")?;
            self.put_bytes(at + n, &bytes, "qsort")?;
            let el = self.get_bytes(st.scratch, n, "qsort")?;
            self.put_bytes(at, &el, "qsort")?;
            st.i += 1;
            if st.i >= st.n {
                let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, st.scratch);
                frame.saved.restore(cpu);
                cpu.gpr[3] = 0;
                return Ok(());
            }
            self.sort_load(st)?;
        }
    }

    /// Copy element `i` to the scratch buffer and open its search window [0, i).
    fn sort_load(&mut self, st: &mut Sort) -> Result<(), String> {
        let el = self.get_bytes(st.base + st.i * st.size, st.size, "qsort")?;
        self.put_bytes(st.scratch, &el, "qsort")?;
        st.lo = 0;
        st.hi = st.i;
        Ok(())
    }

    /// `bsearch(key, base, nmemb, size, compar)`.
    pub(crate) fn start_bsearch(&mut self, cpu: &mut Cpu, key: u32, base: u32, n: u32, size: u32, cmp: u32) -> Result<(), String> {
        if n == 0 { cpu.gpr[3] = 0; return Ok(()); }
        if size == 0 { return Err(String::from("bsearch: element size 0")); }
        self.check_cmp("bsearch", cmp)?;
        let st = Search { key, base, size, cmp, lo: 0, hi: n, mid: 0 };
        let frame = self.c_frame(cpu, Continuation::Bsearch(st));
        self.search_advance(cpu, frame, None)
    }

    pub(crate) fn search_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        let r = cpu.gpr[3] as i32;
        self.search_advance(cpu, frame, Some(r))
    }

    fn search_advance(&mut self, cpu: &mut Cpu, mut frame: Frame, result: Option<i32>) -> Result<(), String> {
        let Continuation::Bsearch(st) = &mut frame.cont else { return Err(String::from("bsearch step on another continuation")) };
        if let Some(r) = result {
            let found = st.base + st.mid * st.size;
            if r == 0 { frame.saved.restore(cpu); cpu.gpr[3] = found; return Ok(()); }
            if r < 0 { st.hi = st.mid; } else { st.lo = st.mid + 1; }
        }
        if st.lo >= st.hi { frame.saved.restore(cpu); cpu.gpr[3] = 0; return Ok(()); }
        st.mid = st.lo + (st.hi - st.lo) / 2;
        let (key, elem, cmp, sp) = (st.key, st.base + st.mid * st.size, st.cmp, frame.saved.gpr[1]);
        self.enter_tvector(cpu, cmp, &[key, elem], sp).map_err(|e| format!("bsearch: the comparison function: {e}"))?;
        self.rt.callbacks.push(frame);
        Ok(())
    }

    /// `atexit(func)`: remember the handler (a TVector). Returns 0.
    pub(crate) fn register_atexit(&mut self, tv: u32) -> Result<(), String> {
        self.check_cmp("atexit", tv)?;
        self.rt.atexit.push(tv);
        Ok(())
    }

    /// `exit(status)`: run the handlers, then end the run. With none registered the run ends at once.
    pub(crate) fn begin_exit(&mut self, cpu: &mut Cpu) {
        if self.rt.atexit.is_empty() { cpu.halt(); return; }
        let frame = self.c_frame(cpu, Continuation::AtExit);
        if let Err(why) = self.next_exit_handler(cpu, frame) { self.rt.fault = Some(why); cpu.halt(); }
    }

    fn next_exit_handler(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> {
        match self.rt.atexit.pop() {
            Some(tv) => {
                self.mac.log.push(format!("[stdclib] exit: atexit handler {tv:#x} runs"));
                let sp = frame.saved.gpr[1];
                self.enter_tvector(cpu, tv, &[], sp).map_err(|e| format!("exit: atexit handler: {e}"))?;
                self.rt.callbacks.push(frame);
                Ok(())
            }
            None => { cpu.halt(); Ok(()) }
        }
    }

    pub(crate) fn exit_handler_returned(&mut self, cpu: &mut Cpu, frame: Frame) -> Result<(), String> { self.next_exit_handler(cpu, frame) }
}
