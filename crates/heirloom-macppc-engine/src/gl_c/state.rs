//! The state calls the worker executes on Mesa, encoded from the program's registers and big-endian memory:
//! blend / depth / scissor / fog / lighting / material / texture-parameter state, texture names, texture images and
//! read-backs. The argument-word layout of each is `heirloom_gl_abi::sig` (the worker decodes with the same table); pixels
//! are converted by `heirloom_gl_abi::pixel` to tight little-endian rows before they enter the ring, so the worker never
//! sees the guest's byte order, alignment or skips.
//!
//! A float argument is read from the floating-point registers in the order the floats appear (PowerPC: f1, f2, ...), a
//! double the same; an integer from r3.. then the parameter area. A call needs a current context (`GL_INVALID_OPERATION`
//! otherwise); an enum the encoder cannot size is `GL_INVALID_ENUM` and never sent, as GL 1.2 §2.5 says of an invalid call.
use super::*;
use alloc::vec;
use heirloom_gl_abi::{pixel, sig};

/// `glPixelStorei` names (GL 1.2 §3.6.1 and §4.3.2): each direction is the six consecutive enums SWAP_BYTES, LSB_FIRST, ROW_LENGTH, SKIP_ROWS, SKIP_PIXELS, ALIGNMENT.
const UNPACK_SWAP_BYTES: u32 = 0x0CF0;
const UNPACK_ALIGNMENT: u32 = 0x0CF5;
const PACK_SWAP_BYTES: u32 = 0x0D00;
const PACK_ALIGNMENT: u32 = 0x0D05;

impl<U: GuestUi> MacPpc<U> {
    /// The k-th float argument (an `f32` parameter arrives in an FPR as a double).
    fn fl(cpu: &Cpu, k: usize) -> u64 { u64::from((f64::from_bits(cpu.fpr[1 + k]) as f32).to_bits()) }

    /// `n` big-endian floats at guest `p`, as ring words (`f32` bits).
    fn gl_floats(&self, p: u32, n: usize, who: &str) -> Result<Vec<u64>, String> {
        (0..n).map(|i| self.mac.mem.read_u32(p.wrapping_add(4 * i as u32)).map(u64::from).map_err(|_| format!("{who}: the parameters at {p:#x} leave guest RAM"))).collect()
    }

    /// Record a `glPixelStorei` the way GL does; false = an invalid enum or value (the error is already set).
    pub(super) fn gl_pixel_store(&mut self, pname: u32, v: i32) -> bool {
        // GL 1.2's image parameters (§3.8.1): SKIP_IMAGES and IMAGE_HEIGHT lay out the images of a 3D texture in client memory
        if matches!(pname, 0x806B..=0x806E) {
            if v < 0 { self.gl_set_error(GL_INVALID_VALUE); return false; }
            let st = if pname >= 0x806D { &mut self.gl.unpack } else { &mut self.gl.pack };
            if pname & 1 == 1 { st.skip_images = v as u32 } else { st.image_height = v as u32 }
            return true;
        }
        let (st, which) = match pname { UNPACK_SWAP_BYTES..=UNPACK_ALIGNMENT => (&mut self.gl.unpack, pname - UNPACK_SWAP_BYTES), PACK_SWAP_BYTES..=PACK_ALIGNMENT => (&mut self.gl.pack, pname - PACK_SWAP_BYTES), _ => { self.gl_set_error(GL_INVALID_ENUM); return false; } };
        if v < 0 || (which == 5 && !matches!(v, 1 | 2 | 4 | 8)) { self.gl_set_error(GL_INVALID_VALUE); return false; }
        match which { 0 => st.swap_bytes = v != 0, 1 => st.lsb_first = v != 0, 2 => st.row_length = v as u32, 3 => st.skip_rows = v as u32, 4 => st.skip_pixels = v as u32, _ => st.alignment = v as u32 }
        true
    }

    /// The result of a call served by the worker, or the refusal it sent (noted, and `GL_INVALID_OPERATION` set).
    pub(super) fn gl_reply_words(&mut self, who: &str, r: Option<(u32, Vec<u64>, String)>) -> Option<Vec<u64>> {
        match r {
            Some((0, w, _)) => Some(w),
            Some((_, _, text)) => { self.gl_note(text); self.gl_set_error(GL_INVALID_OPERATION); None }
            None => { self.gl_note(format!("[HEIRLOOM-GL REFUSED call={who} reason=worker-gone]")); self.gl_set_error(GL_INVALID_OPERATION); None }
        }
    }

    /// The pixels `[x y w h]` of a texture image: `ptr` null means no client data (an allocation only).
    pub(super) fn gl_pixels(&self, ptr: u32, w: i32, h: i32, format: u32, ty: u32) -> Result<Vec<u8>, u32> {
        if ptr == 0 { return Ok(Vec::new()); }
        let len = self.mac.mem.len().min(u32::MAX as usize) as u32;
        let mem = self.mac.mem.span(0, len).and_then(|s| self.mac.mem.bytes(s)).map_err(|_| GL_INVALID_OPERATION)?;
        pixel::unpack(mem, self.mac.mem.strip(ptr) as usize, w as u32, h as u32, format, ty, &self.gl.unpack).map_err(|e| match e {
            pixel::PixelError::BadFormat(_) | pixel::PixelError::BadType(_) | pixel::PixelError::Mismatch { .. } => GL_INVALID_ENUM,
            _ => GL_INVALID_VALUE,
        })
    }

    pub(super) fn gl_state_call(&mut self, name: &str, entry: u16, cpu: &mut Cpu) -> Result<u32, String> {
        if let Some(sig) = heirloom_gl_abi::family::lookup(name) { return self.gl_generic_call(sig, entry, cpu); }
        if heirloom_gl_abi::family::lookup_bulk(name).is_some() { return self.gl_bulk_call(name, entry, cpu); }
        if let Some(r) = self.gl_array_call(name, entry, cpu)? { return Ok(r); }
        if let Some(r) = self.gl_image_call(name, entry, cpu)? { return Ok(r); }
        let i = |m: &Self, k: usize| arg(m, cpu, k);
        if name == "glPixelStorei" {
            let (p, v) = (i(self, 0)?, i(self, 1)? as i32);
            if !self.gl_pixel_store(p, v) { return Ok(0); }
        }
        let Some(token) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(0); };
        let words: Vec<u64> = match name {
            "glAlphaFunc" => vec![u64::from(i(self, 0)?), Self::fl(cpu, 0)],
            "glBindTexture" | "glBlendFunc" | "glColorMaterial" | "glFogi" | "glPixelStorei" => vec![u64::from(i(self, 0)?), u64::from(i(self, 1)?)],
            "glClearColor" => (0..4).map(|k| Self::fl(cpu, k)).collect(),
            "glDepthFunc" => vec![u64::from(i(self, 0)?)],
            "glDepthMask" => vec![u64::from(i(self, 0)? & 0xFF)],
            "glDepthRange" => vec![cpu.fpr[1], cpu.fpr[2]],
            "glFogf" => vec![u64::from(i(self, 0)?), Self::fl(cpu, 0)],
            "glPolygonOffset" => vec![Self::fl(cpu, 0), Self::fl(cpu, 1)],
            "glScissor" => (0..4).map(|k| i(self, k).map(u64::from)).collect::<Result<_, _>>()?,
            "glTexEnvi" | "glTexGeni" | "glTexParameteri" => (0..3).map(|k| i(self, k).map(u64::from)).collect::<Result<_, _>>()?,
            "glLightf" => vec![u64::from(i(self, 0)?), u64::from(i(self, 1)?), Self::fl(cpu, 0)],
            "glNormal3fv" => self.gl_floats(i(self, 0)?, 3, name)?,
            "glFogfv" | "glLightModelfv" | "glLightfv" | "glMaterialfv" => {
                let (lead, kind) = match name { "glFogfv" => (1, sig::VecKind::Fog), "glLightModelfv" => (1, sig::VecKind::LightModel), "glLightfv" => (2, sig::VecKind::Light), _ => (2, sig::VecKind::Material) };
                let mut w: Vec<u64> = (0..lead).map(|k| i(self, k).map(u64::from)).collect::<Result<_, _>>()?;
                let Some(n) = sig::vec_len(kind, w[lead - 1] as u32) else { self.gl_set_error(GL_INVALID_ENUM); return Ok(0); };
                w.extend(self.gl_floats(i(self, lead)?, n, name)?);
                w
            }
            "glDeleteTextures" => {
                let n = i(self, 0)? as i32;
                if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
                let mut w = vec![n as u64];
                let p = i(self, 1)?;
                for k in 0..n as u32 { w.push(u64::from(self.mac.mem.read_u32(p.wrapping_add(4 * k)).map_err(|_| String::from("glDeleteTextures: the names leave guest RAM"))?)); }
                w
            }
            "glGenTextures" => {
                let n = i(self, 0)? as i32;
                if n < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
                if n == 0 { return Ok(0); }
                let Some(names) = self.gl_ask(token, Command::Call { entry, args: vec![n as u64] }).and_then(|r| self.gl_reply_words(name, Some(r))) else { return Ok(0); };
                if names.len() != n as usize { return Err(format!("glGenTextures: the worker answered {} names for {n}", names.len())); }
                let p = i(self, 1)?;
                for (k, v) in names.iter().enumerate() { self.mac.mem.write_u32(p.wrapping_add(4 * k as u32), *v as u32).map_err(|_| String::from("glGenTextures: the name array leaves guest RAM"))?; }
                return Ok(0);
            }
            "glTexImage2D" | "glTexSubImage2D" => {
                let full = name == "glTexImage2D";
                let a: Vec<u32> = (0..9).map(|k| i(self, k)).collect::<Result<_, _>>()?;
                // glTexImage2D(target, level, internalformat, w, h, border, format, type, data); glTexSubImage2D(target, level, x, y, w, h, format, type, data)
                let (w, h, format, ty, ptr) = if full { (a[3] as i32, a[4] as i32, a[6], a[7], a[8]) } else { (a[4] as i32, a[5] as i32, a[6], a[7], a[8]) };
                if w < 0 || h < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
                if !full && ptr == 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
                let data = match self.gl_pixels(ptr, w, h, format, ty) { Ok(d) => d, Err(e) => { self.gl_set_error(e); return Ok(0); } };
                let mut args: Vec<u64> = a[..8].iter().copied().map(u64::from).collect();
                if full { args.push(u64::from(ptr != 0)); } else { args.truncate(8); }
                self.gl_queue(token, Command::CallData { entry, args, data });
                return Ok(0);
            }
            "glReadPixels" => {
                let a: Vec<u32> = (0..7).map(|k| i(self, k)).collect::<Result<_, _>>()?;
                let (w, h) = (a[2] as i32, a[3] as i32);
                if w < 0 || h < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(0); }
                if let Err(e) = pixel::layout(a[4], a[5]) { self.gl_set_error(if matches!(e, pixel::PixelError::Mismatch { .. }) { GL_INVALID_OPERATION } else { GL_INVALID_ENUM }); return Ok(0); }
                self.gl_queue(token, Command::Call { entry, args: a[..6].iter().copied().map(u64::from).collect() });
                let Some((status, _, data)) = self.gl_flush().into_iter().find_map(|c| match c {
                    Command::ReplyData { status, words, data } => Some((status, words, data)),
                    Command::Reply { status, words, text } => Some((status, words, text.into_bytes())),
                    _ => None,
                }) else {
                    self.gl_note(String::from("[HEIRLOOM-GL REFUSED call=glReadPixels reason=worker-gone]"));
                    self.gl_set_error(GL_INVALID_OPERATION);
                    return Ok(0);
                };
                if status != 0 { self.gl_note(String::from_utf8_lossy(&data).into_owned()); self.gl_set_error(GL_INVALID_OPERATION); return Ok(0); }
                let base = self.mac.mem.strip(a[6]);
                let store = self.gl.pack;
                let mem = &mut self.mac.mem;
                if let Err(e) = pixel::pack(&data, w as u32, h as u32, a[4], a[5], &store, |at, b| u32::try_from(at).ok().and_then(|at| base.checked_add(at)).is_some_and(|to| mem.write_bytes(to, b).is_ok())) {
                    self.gl_note(format!("[HEIRLOOM-GL] glReadPixels result does not fit the guest buffer: {e:?}"));
                    self.gl_set_error(GL_INVALID_OPERATION);
                }
                return Ok(0);
            }
            other => return Err(format!("{other}: marked implemented but has no encoder in this engine (a contract/engine skew)")),
        };
        if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &words) {
            self.gl_note(m.clone());
            return Err(m);
        }
        self.gl_queue(token, Command::Call { entry, args: words });
        Ok(0)
    }
}
