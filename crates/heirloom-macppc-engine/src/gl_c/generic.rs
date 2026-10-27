//! The regular GL call families (`heirloom_gl_abi::family`): every per-vertex call, matrix and state setter, parameter vector
//! and parameter query is encoded by this ONE routine from the generated signature table, so a parameter's type (not a
//! hand-written case) decides where its value is read.
//!
//! PowerPC calling convention (the Mac OS runtime's PowerOpen layout): each argument takes its place in the PARAMETER AREA in
//! order - one word for an integer, pointer or float, two for a double - and the first eight words travel in r3..r10, the rest
//! on the stack; a float or double argument is ALSO loaded into the next floating-point register (f1, f2, ...), and its words
//! in the general registers are left unused. So the k-th integer-class argument is read at the PARAMETER WORD where it falls,
//! not at "the k-th integer".
//!
//! An enum the encoder cannot size (a `pname` the call has no parameter for) is `GL_INVALID_ENUM` and never sent; a call with
//! no current context is `GL_INVALID_OPERATION` (GL 1.2 §2.5).
use super::*;
use heirloom_gl_abi::family::{self, Sig, Ty, P, R};

impl<U: GuestUi> MacPpc<U> {
    /// The `n` elements of type `t` at guest address `p`, as wire words.
    pub(super) fn gl_elements(&self, p: u32, n: usize, t: Ty, who: &str) -> Result<Vec<u64>, String> {
        let bytes = n.checked_mul(t.guest_bytes()).and_then(|b| u32::try_from(b).ok()).ok_or_else(|| format!("{who}: a vector of {n} elements is not addressable"))?;
        if n == 0 { return Ok(Vec::new()); }
        let raw = self.get_bytes(self.mac.mem.strip(p), bytes, who)?;
        Ok(raw.chunks_exact(t.guest_bytes()).map(|c| t.from_guest(c)).collect())
    }

    /// One generated call: the guest's arguments to ring words, the answer (if any) back into the guest.
    pub(super) fn gl_generic_call(&mut self, sig: &Sig, entry: u16, cpu: &mut Cpu) -> Result<u32, String> {
        let Some(token) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(0); };
        let who = sig.name;
        let (mut slot, mut fp) = (0usize, 0usize);
        let mut words: Vec<u64> = Vec::new();
        let mut out: Option<(u32, Ty, usize)> = None;
        for p in sig.params {
            match *p {
                P::S(t) if t.is_float() => {
                    words.push(t.float_word(f64::from_bits(cpu.fpr[1 + fp])));
                    fp += 1;
                    slot += t.slots();
                }
                P::S(t) => {
                    words.push(t.int_word(arg(self, cpu, slot)?));
                    slot += 1;
                }
                P::V(t, n) => {
                    let ptr = arg(self, cpu, slot)?;
                    slot += 1;
                    words.extend(self.gl_elements(ptr, usize::from(n), t, who)?);
                }
                P::Data(..) | P::OutAny(_) => return Err(format!("{who}: a bulk-data call routed to the generic encoder (a table skew)")),
                P::In(t, kind) | P::Out(t, kind) => {
                    let ptr = arg(self, cpu, slot)?;
                    slot += 1;
                    let Some(n) = words.last().and_then(|w| u32::try_from(*w).ok()).and_then(|pn| family::var_len(kind, pn)) else {
                        self.gl_set_error(GL_INVALID_ENUM);
                        return Ok(0);
                    };
                    if matches!(p, P::In(..)) { words.extend(self.gl_elements(ptr, n, t, who)?); } else { out = Some((ptr, t, n)); }
                }
            }
        }
        if let Some((err, why)) = heirloom_gl_abi::checks::violation(who, &words) {
            self.gl_note(format!("[HEIRLOOM-GL] {why}"));
            self.gl_set_error(err);
            return Ok(0);
        }
        if let Some(m) = heirloom_gl_abi::gaps::refused_value(who, &words) {
            self.gl_note(m.clone());
            return Err(m);
        }
        if !sig.answers() {
            self.gl_queue(token, Command::Call { entry, args: words });
            return Ok(0);
        }
        // client-array state is the engine's: it answers those queries itself
        if let Some(pname) = words.first().and_then(|w| u32::try_from(*w).ok()).filter(|p| heirloom_gl_abi::sig::engine_answers(*p)).filter(|_| matches!(who, "glIsEnabled" | "glGetBooleanv" | "glGetIntegerv" | "glGetFloatv" | "glGetDoublev")) {
            let t = if who == "glIsEnabled" { Ty::Bool } else { sig.out().map_or(Ty::Int, |o| o.0) };
            let Some(w) = self.gl_client_words(pname, t) else { return Err(format!("{who}: {pname:#x} is not client-array state")); };
            return match out {
                Some((ptr, t, _)) => {
                    let mut bytes = Vec::new();
                    for x in &w { t.to_guest(*x, &mut bytes); }
                    self.put_bytes(self.mac.mem.strip(ptr), &bytes, who)?;
                    Ok(0)
                }
                None => Ok(w[0] as u32),
            };
        }
        let r = self.gl_ask(token, Command::Call { entry, args: words });
        let Some(reply) = self.gl_reply_words(who, r) else { return Ok(0); };
        if let Some((ptr, t, n)) = out {
            if reply.len() != n { return Err(format!("{who}: the worker answered {} values, the contract says {n}", reply.len())); }
            let mut bytes = Vec::with_capacity(n * t.guest_bytes());
            for w in &reply { t.to_guest(*w, &mut bytes); }
            self.put_bytes(self.mac.mem.strip(ptr), &bytes, who)?;
            return Ok(0);
        }
        let v = reply.first().copied().ok_or_else(|| format!("{who}: the worker answered no value"))?;
        Ok(match sig.ret { R::Bool => u32::from(v != 0), _ => v as u32 })
    }
}
