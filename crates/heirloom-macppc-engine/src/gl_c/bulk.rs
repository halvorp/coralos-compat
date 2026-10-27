//! The calls that carry data of a runtime-determined size (`heirloom_gl_abi::bulk_table`): evaluator maps, pixel maps, display-list
//! names, texture priorities. Each is validated HERE with the error GL 1.2 names (nothing invalid reaches the worker), and the
//! program's memory - strides, byte order, element types - is repacked into the tight words the table lays out.
use super::*;
use alloc::vec;
use heirloom_gl_abi::family::{self, Ty};

/// Components of one control point for an evaluator `target` (GL 1.2 Table 5.1): `MAP1_*` is 0x0D90.., `MAP2_*` 0x0DB0...
pub(super) fn map_components(target: u32) -> Option<usize> {
    Some(match target & !0x0020 {
        0x0D90 | 0x0D96 | 0x0D98 => 4, // COLOR_4, TEXTURE_COORD_4, VERTEX_4
        0x0D91 | 0x0D93 => 1,          // INDEX, TEXTURE_COORD_1
        0x0D94 => 2,                   // TEXTURE_COORD_2
        0x0D92 | 0x0D95 | 0x0D97 => 3, // NORMAL, TEXTURE_COORD_3, VERTEX_3
        _ => return None,
    })
}

impl<U: GuestUi> MacPpc<U> {
    /// `n` elements of type `t` read from guest address `base`, element `k` at byte `base + k * step`.
    fn gl_strided(&self, base: u32, step: usize, idx: impl Iterator<Item = usize>, t: Ty, who: &str) -> Result<Vec<u64>, String> {
        let mut out = Vec::new();
        for k in idx {
            let at = u32::try_from(k.checked_mul(step).ok_or_else(|| format!("{who}: the control points overflow"))?).map_err(|_| format!("{who}: the control points overflow"))?;
            let b = self.get_bytes(self.mac.mem.strip(base).wrapping_add(at), t.guest_bytes() as u32, who)?;
            out.push(t.from_guest(&b));
        }
        Ok(out)
    }

    pub(super) fn gl_bulk_call(&mut self, name: &str, entry: u16, cpu: &mut Cpu) -> Result<u32, String> {
        let Some(token) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(0); };
        match name {
            "glMap1f" | "glMap1d" | "glMap2f" | "glMap2d" => self.gl_map(name, entry, token, cpu),
            "glGetMapfv" | "glGetMapdv" | "glGetMapiv" | "glGetPixelMapfv" | "glGetPixelMapuiv" | "glGetPixelMapusv" => self.gl_get_out_any(name, entry, token, cpu),
            "glPixelMapfv" | "glPixelMapuiv" | "glPixelMapusv" => self.gl_pixel_map(name, entry, token, cpu),
            "glCallLists" => self.gl_call_lists(entry, token, cpu),
            "glPrioritizeTextures" => self.gl_prioritize(entry, token, cpu),
            "glAreTexturesResident" => self.gl_are_resident(entry, token, cpu),
            "glSelectBuffer" | "glFeedbackBuffer" => self.gl_select_feedback_buffer(name, entry, token, cpu),
            "glRenderMode" => {
                // §5.2: GL_RENDER is the only mode this engine serves (selection and feedback are refused by name): leaving it
                // returns 0, and any other value is GL_INVALID_ENUM
                let mode = arg(self, cpu, 0)?;
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(mode)]) { self.gl_note(m.clone()); return Err(m); }
                if mode != 0x1C00 { self.gl_set_error(0x0500); }
                Ok(0)
            }
            other => Err(format!("{other}: marked implemented but has no bulk encoder in this engine (a contract/engine skew)")),
        }
    }

    /// glMap1{f,d} / glMap2{f,d} (§5.1): validate, repack the control points tight.
    fn gl_map(&mut self, name: &str, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let dbl = name.ends_with('d');
        let two = name.starts_with("glMap2");
        let ty = if dbl { Ty::Double } else { Ty::Float };
        let fl = |cpu: &Cpu, k: usize| ty.float_word(f64::from_bits(cpu.fpr[1 + k]));
        let arg_i = |m: &Self, cpu: &Cpu, s: usize| arg(m, cpu, s);
        let w = usize::from(dbl) + 1; // parameter words of one float / double
        let target = arg_i(self, cpu, 0)?;
        let (u1, u2) = (fl(cpu, 0), fl(cpu, 1));
        let mut s = 1 + 2 * w;
        let (ustride, uorder) = (arg_i(self, cpu, s)? as i32, arg_i(self, cpu, s + 1)? as i32);
        s += 2;
        let (v1, v2, vstride, vorder) = if two {
            let (a, b) = (fl(cpu, 2), fl(cpu, 3));
            s += 2 * w;
            let (vs, vo) = (arg_i(self, cpu, s)? as i32, arg_i(self, cpu, s + 1)? as i32);
            s += 2;
            (a, b, vs, vo)
        } else { (0, 0, 0, 0) };
        let ptr = arg_i(self, cpu, s)?;
        let Some(k) = map_components(target).filter(|_| (target >= 0x0DB0) == two) else { self.gl_set_error(0x0500); return Ok(0); };
        let dom_empty = |a: u64, b: u64| if dbl { f64::from_bits(a) == f64::from_bits(b) } else { f32::from_bits(a as u32) == f32::from_bits(b as u32) };
        if uorder < 1 || ustride < k as i32 || dom_empty(u1, u2) || (two && (vorder < 1 || vstride < k as i32 || dom_empty(v1, v2))) { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        let (uo, vo) = (uorder as usize, if two { vorder as usize } else { 1 });
        let idx = (0..uo).flat_map(|i| (0..vo).flat_map(move |j| (0..k).map(move |c| (i, j, c))));
        let pts = self.gl_strided_idx(ptr, ty, ustride as usize, if two { vstride as usize } else { 0 }, idx, name)?;
        let mut words = vec![u64::from(target), u1, u2, uo as u64];
        if two { words.extend([v1, v2, vo as u64]); }
        words.push(k as u64);
        words.extend(pts);
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }

    /// Elements `(i, j, c)` at element offset `i * ustride + j * vstride + c` (strides in ELEMENTS, as GL's are).
    fn gl_strided_idx(&self, base: u32, t: Ty, ustride: usize, vstride: usize, idx: impl Iterator<Item = (usize, usize, usize)>, who: &str) -> Result<Vec<u64>, String> {
        let b = self.mac.mem.strip(base);
        let mut out = Vec::new();
        for (i, j, c) in idx {
            let el = i.checked_mul(ustride).and_then(|a| j.checked_mul(vstride).and_then(|b| a.checked_add(b))).and_then(|a| a.checked_add(c)).and_then(|e| e.checked_mul(t.guest_bytes())).and_then(|e| u32::try_from(e).ok());
            let at = el.and_then(|e| b.checked_add(e)).ok_or_else(|| format!("{who}: the control points leave the address space"))?;
            let bytes = self.get_bytes(at, t.guest_bytes() as u32, who)?;
            out.push(t.from_guest(&bytes));
        }
        Ok(out)
    }

    /// `glGetMap*`, `glGetPixelMap*`: the worker's reply (value first when the call returns one) is stored as the program's type.
    fn gl_get_out_any(&mut self, name: &str, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let sig = family::lookup_bulk(name).ok_or_else(|| format!("{name}: no signature"))?;
        let nscalar = sig.params.iter().filter(|p| matches!(p, family::P::S(_))).count();
        let mut words: Vec<u64> = Vec::new();
        for k in 0..nscalar { words.push(u64::from(arg(self, cpu, k)?)); }
        let ptr = arg(self, cpu, nscalar)?;
        let (_, t) = (0, sig.params.iter().find_map(|p| if let family::P::OutAny(t) = p { Some(*t) } else { None }).unwrap_or(Ty::Float));
        let r = self.gl_ask(token, Command::Call { entry, args: words });
        let Some(reply) = self.gl_reply_words(name, r) else { return Ok(0); };
        let mut bytes = Vec::new();
        for w in &reply { t.to_guest(*w, &mut bytes); }
        if !bytes.is_empty() { self.put_bytes(self.mac.mem.strip(ptr), &bytes, name)?; }
        Ok(0)
    }

    /// glSelectBuffer(size, buffer) (§5.2) and glFeedbackBuffer(size, type, buffer) (§5.3): the pointer stays HERE (it is what
    /// glGetPointerv returns), the size and type go to the worker (which keeps them as state).
    fn gl_select_feedback_buffer(&mut self, name: &str, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let feedback = name == "glFeedbackBuffer";
        let (size, ty, ptr) = if feedback { (arg(self, cpu, 0)? as i32, arg(self, cpu, 1)?, arg(self, cpu, 2)?) } else { (arg(self, cpu, 0)? as i32, 0, arg(self, cpu, 1)?) };
        if size < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        if feedback && !(0x0600..=0x0604).contains(&ty) { self.gl_set_error(0x0500); return Ok(0); }
        if feedback { self.gl.feedback_ptr = ptr; } else { self.gl.select_ptr = ptr; }
        let mut words = vec![size as u64];
        if feedback { words.push(u64::from(ty)); }
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }

    /// glPixelMap{fv,uiv,usv}(map, mapsize, values) (§3.6.3).
    fn gl_pixel_map(&mut self, name: &str, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let t = match name { "glPixelMapfv" => Ty::Float, "glPixelMapuiv" => Ty::Uint, _ => Ty::Ushort };
        let (map, n, ptr) = (arg(self, cpu, 0)?, arg(self, cpu, 1)? as i32, arg(self, cpu, 2)?);
        if !(0x0C70..=0x0C79).contains(&map) { self.gl_set_error(0x0500); return Ok(0); }
        if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        let vals = self.gl_strided(ptr, t.guest_bytes(), 0..n as usize, t, name)?;
        let mut words = vec![u64::from(map), n as u64];
        words.extend(vals);
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }

    /// glCallLists(n, type, lists) (§5.4): the names are converted from `type` to unsigned integers here (a signed type sign-extends,
    /// GL_2_BYTES / 3_BYTES / 4_BYTES are big-endian groups of unsigned bytes, GL_FLOAT is truncated); the worker adds the list base.
    fn gl_call_lists(&mut self, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let (n, ty, ptr) = (arg(self, cpu, 0)? as i32, arg(self, cpu, 1)?, arg(self, cpu, 2)?);
        if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        let size = match ty { 0x1400 | 0x1401 => 1, 0x1402 | 0x1403 | 0x1407 => 2, 0x1404 | 0x1405 | 0x1406 | 0x1409 => 4, 0x1408 => 3, _ => { self.gl_set_error(0x0500); return Ok(0); } };
        // GL_2_BYTES 0x1407, GL_3_BYTES 0x1408, GL_4_BYTES 0x1409
        let b = self.mac.mem.strip(ptr);
        let mut words = vec![n as u64];
        for k in 0..n as u32 {
            let raw = self.get_bytes(b.wrapping_add(k * size as u32), size as u32, "glCallLists")?;
            let v: u32 = match ty {
                0x1400 => i32::from(raw[0] as i8) as u32,
                0x1401 => u32::from(raw[0]),
                0x1402 => i32::from(i16::from_be_bytes([raw[0], raw[1]])) as u32,
                0x1403 | 0x1407 => u32::from(u16::from_be_bytes([raw[0], raw[1]])),
                0x1408 => u32::from_be_bytes([0, raw[0], raw[1], raw[2]]),
                0x1406 => f32::from_bits(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]])) as u32,
                _ => u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]),
            };
            words.push(u64::from(v));
        }
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }

    /// glPrioritizeTextures(n, textures, priorities) (§3.8.12).
    fn gl_prioritize(&mut self, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let (n, names, prios) = (arg(self, cpu, 0)? as i32, arg(self, cpu, 1)?, arg(self, cpu, 2)?);
        if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        let nm = self.gl_strided(names, 4, 0..n as usize, Ty::Uint, "glPrioritizeTextures")?;
        let pr = self.gl_strided(prios, 4, 0..n as usize, Ty::Float, "glPrioritizeTextures")?;
        let mut words = vec![n as u64];
        words.extend(nm);
        words.extend(pr);
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }

    /// glAreTexturesResident(n, textures, residences) (§3.8.12): the returned boolean, and one byte per name.
    fn gl_are_resident(&mut self, entry: u16, token: Token, cpu: &mut Cpu) -> Result<u32, String> {
        let (n, names, out) = (arg(self, cpu, 0)? as i32, arg(self, cpu, 1)?, arg(self, cpu, 2)?);
        if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
        let nm = self.gl_strided(names, 4, 0..n as usize, Ty::Uint, "glAreTexturesResident")?;
        let mut words = vec![n as u64];
        words.extend(nm);
        let r = self.gl_ask(token, Command::Call { entry, args: words });
        let Some(reply) = self.gl_reply_words("glAreTexturesResident", r) else { return Ok(0); };
        let Some((ret, rest)) = reply.split_first() else { return Err(String::from("glAreTexturesResident: the worker answered nothing")) };
        let bytes: Vec<u8> = rest.iter().map(|w| u8::from(*w != 0)).collect();
        if !bytes.is_empty() { self.put_bytes(self.mac.mem.strip(out), &bytes, "glAreTexturesResident")?; }
        Ok(u32::from(*ret != 0))
    }
}

