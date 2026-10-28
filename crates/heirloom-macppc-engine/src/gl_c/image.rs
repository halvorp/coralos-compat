//! The calls that carry client images or bitmaps: glTexImage1D, glTexSubImage1D, glTexImage3D, glTexSubImage3D, glDrawPixels,
//! glBitmap, glPolygonStipple, glGetPolygonStipple, glGetTexImage. The program's memory (alignment, row length, skips, byte order,
//! bit order) is converted to the tight little-endian rows / MSB-first bit rows of `heirloom_gl_abi::pixel` before it enters the ring
//! (`Command::CallData`), and a read-back (`Command::ReplyData`) is stored through the program's pack state.
//!
//! Wire layouts (scalar words, then the data): `glTexImage1D` = target level internalformat width border format type has_data;
//! `glTexSubImage1D` = target level xoffset width format type; `glTexImage3D` = target level internalformat width height depth border
//! format type has_data; `glTexSubImage3D` = target level xoffset yoffset zoffset width height depth format type; `glDrawPixels` = width
//! height format type; `glBitmap` = width height xorig yorig xmove ymove (the four floats as `f32` bits) + ceil(width/8) * height bytes;
//! `glPolygonStipple` = (none) + 128 bytes; `glGetPolygonStipple` = (none), answered by 128 bytes; `glGetTexImage` = target level format
//! type, answered by `[width, height, depth]` words (depth 1 below 3D) and the tight pixels.
use super::*;
use alloc::vec;
use heirloom_gl_abi::pixel;

/// `GL_TEXTURE_1D`, `GL_TEXTURE_2D`, `GL_TEXTURE_3D`.
const GL_TEXTURE_1D: u32 = 0x0DE0;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_3D: u32 = 0x806F;

impl<U: GuestUi> MacPpc<U> {
    fn gl_pixel_error(&mut self, e: pixel::PixelError) {
        self.gl_set_error(match e {
            pixel::PixelError::BadFormat(_) | pixel::PixelError::BadType(_) => GL_INVALID_ENUM,
            pixel::PixelError::Mismatch { .. } => GL_INVALID_OPERATION,
            _ => GL_INVALID_VALUE,
        });
    }

    /// A BITMAP of `w x h` bits at guest address `ptr` through the unpack state (a null pointer is no data).
    fn gl_bitmap_bits(&self, ptr: u32, w: i32, h: i32) -> Result<Vec<u8>, pixel::PixelError> {
        if ptr == 0 || w == 0 || h == 0 { return Ok(Vec::new()); }
        let len = self.mac.mem.len().min(u32::MAX as usize) as u32;
        let mem = self.mac.mem.span(0, len).and_then(|s| self.mac.mem.bytes(s)).map_err(|_| pixel::PixelError::OutOfRange)?;
        pixel::unpack_bitmap(mem, self.mac.mem.strip(ptr) as usize, w as u32, h as u32, &self.gl.unpack)
    }

    /// A `w x h x d` image at guest address `ptr` through the unpack state (a null pointer is no data).
    fn gl_pixels3d(&self, ptr: u32, w: i32, h: i32, d: i32, format: u32, ty: u32) -> Result<Vec<u8>, u32> {
        if ptr == 0 || w == 0 || h == 0 || d == 0 { return Ok(Vec::new()); }
        let len = self.mac.mem.len().min(u32::MAX as usize) as u32;
        let mem = self.mac.mem.span(0, len).and_then(|s| self.mac.mem.bytes(s)).map_err(|_| GL_INVALID_OPERATION)?;
        pixel::unpack3d(mem, self.mac.mem.strip(ptr) as usize, w as u32, h as u32, d as u32, format, ty, &self.gl.unpack).map_err(|e| match e {
            pixel::PixelError::BadFormat(_) | pixel::PixelError::BadType(_) => GL_INVALID_ENUM,
            pixel::PixelError::Mismatch { .. } => GL_INVALID_OPERATION,
            _ => GL_INVALID_VALUE,
        })
    }

    /// The call `name` if it is one of this module's; `None` otherwise.
    pub(super) fn gl_image_call(&mut self, name: &str, entry: u16, cpu: &mut Cpu) -> Result<Option<u32>, String> {
        if !matches!(name, "glTexImage1D" | "glTexSubImage1D" | "glTexImage3D" | "glTexSubImage3D" | "glDrawPixels" | "glBitmap" | "glPolygonStipple" | "glGetPolygonStipple" | "glGetTexImage") { return Ok(None); }
        let Some(token) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); };
        let a = |m: &Self, k: usize| arg(m, cpu, k);
        match name {
            "glTexImage1D" | "glTexSubImage1D" => {
                let full = name == "glTexImage1D";
                // glTexImage1D(target, level, internalformat, width, border, format, type, data); glTexSubImage1D(target, level, xoffset, width, format, type, data)
                let v: Vec<u32> = (0..if full { 8 } else { 7 }).map(|k| a(self, k)).collect::<Result<_, _>>()?;
                let (target, w, format, ty, ptr) = if full { (v[0], v[3] as i32, v[5], v[6], v[7]) } else { (v[0], v[3] as i32, v[4], v[5], v[6]) };
                if target != GL_TEXTURE_1D { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if w < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                if !full && ptr == 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let data = match self.gl_pixels(ptr, w, 1, format, ty) { Ok(d) => d, Err(e) => { self.gl_set_error(e); return Ok(Some(0)); } };
                let mut args: Vec<u64> = v[..if full { 7 } else { 6 }].iter().copied().map(u64::from).collect();
                if full { args.push(u64::from(ptr != 0)); }
                self.gl_queue(token, Command::CallData { entry, args, data });
            }
            "glTexImage3D" | "glTexSubImage3D" => {
                let full = name == "glTexImage3D";
                // glTexImage3D(target, level, internalformat, width, height, depth, border, format, type, data); glTexSubImage3D(target, level, xoffset, yoffset, zoffset, width, height, depth, format, type, data)
                let v: Vec<u32> = (0..if full { 10 } else { 11 }).map(|k| a(self, k)).collect::<Result<_, _>>()?;
                let (target, w, h, d, format, ty, ptr) = if full { (v[0], v[3] as i32, v[4] as i32, v[5] as i32, v[7], v[8], v[9]) } else { (v[0], v[5] as i32, v[6] as i32, v[7] as i32, v[8], v[9], v[10]) };
                if target != GL_TEXTURE_3D { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if w < 0 || h < 0 || d < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                if !full && ptr == 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let data = match self.gl_pixels3d(ptr, w, h, d, format, ty) { Ok(d) => d, Err(e) => { self.gl_set_error(e); return Ok(Some(0)); } };
                let mut args: Vec<u64> = v[..if full { 9 } else { 10 }].iter().copied().map(u64::from).collect();
                if full { args.push(u64::from(ptr != 0)); }
                self.gl_queue(token, Command::CallData { entry, args, data });
            }
            "glDrawPixels" => {
                let (w, h, format, ty, ptr) = (a(self, 0)? as i32, a(self, 1)? as i32, a(self, 2)?, a(self, 3)?, a(self, 4)?);
                if w < 0 || h < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                // GL_STENCIL_INDEX: this visual has no stencil buffer (§4.3.1); depth and colour-index pixels are refused by name
                if format == pixel::GL_STENCIL_INDEX { self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); }
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(format)]) { self.gl_note(m.clone()); return Err(m); }
                let data = match self.gl_pixels(ptr, w, h, format, ty) { Ok(d) => d, Err(e) => { self.gl_set_error(e); return Ok(Some(0)); } };
                if ptr == 0 { return Ok(Some(0)); }
                self.gl_queue(token, Command::CallData { entry, args: vec![w as u64, h as u64, u64::from(format), u64::from(ty)], data });
            }
            "glBitmap" => {
                let (w, h) = (a(self, 0)? as i32, a(self, 1)? as i32);
                if w < 0 || h < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                // four floats in f1..f4 take parameter words 2..5, so the bitmap pointer is parameter word 6
                let fw = |cpu: &Cpu, k: usize| u64::from((f64::from_bits(cpu.fpr[1 + k]) as f32).to_bits());
                let ptr = a(self, 6)?;
                let bits = match self.gl_bitmap_bits(ptr, w, h) { Ok(b) => b, Err(e) => { self.gl_pixel_error(e); return Ok(Some(0)); } };
                let args = vec![w as u64, h as u64, fw(cpu, 0), fw(cpu, 1), fw(cpu, 2), fw(cpu, 3)];
                self.gl_queue(token, Command::CallData { entry, args, data: bits });
            }
            "glPolygonStipple" => {
                let ptr = a(self, 0)?;
                let bits = match self.gl_bitmap_bits(ptr, 32, 32) { Ok(b) if b.len() == 128 => b, Ok(_) => { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); } Err(e) => { self.gl_pixel_error(e); return Ok(Some(0)); } };
                self.gl_queue(token, Command::CallData { entry, args: Vec::new(), data: bits });
            }
            "glGetPolygonStipple" => {
                let ptr = a(self, 0)?;
                self.gl_queue(token, Command::Call { entry, args: Vec::new() });
                let Some((status, _, data)) = self.gl_flush().into_iter().find_map(|c| match c { Command::ReplyData { status, words, data } => Some((status, words, data)), _ => None }) else {
                    self.gl_note(String::from("[HEIRLOOM-GL REFUSED call=glGetPolygonStipple reason=worker-gone]"));
                    self.gl_set_error(GL_INVALID_OPERATION);
                    return Ok(Some(0));
                };
                if status != 0 || data.len() != 128 { self.gl_note(String::from_utf8_lossy(&data).into_owned()); self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); }
                let base = self.mac.mem.strip(ptr);
                let store = self.gl.pack;
                let mem = &mut self.mac.mem;
                let r = pixel::pack_bitmap(&data, 32, 32, &store, |at, mask, v| {
                    let Some(to) = u32::try_from(at).ok().and_then(|at| base.checked_add(at)) else { return false };
                    mem.read_u8(to).and_then(|old| mem.write_u8(to, old & !mask | v)).is_ok()
                });
                if let Err(e) = r { self.gl_note(format!("[HEIRLOOM-GL] glGetPolygonStipple result does not fit the guest buffer: {e:?}")); self.gl_set_error(GL_INVALID_OPERATION); }
            }
            "glGetTexImage" => {
                let (target, level, format, ty, ptr) = (a(self, 0)?, a(self, 1)? as i32, a(self, 2)?, a(self, 3)?, a(self, 4)?);
                if !matches!(target, GL_TEXTURE_1D | GL_TEXTURE_2D | GL_TEXTURE_3D) { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if level < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                if let Err(e) = pixel::layout(format, ty) { self.gl_pixel_error(e); return Ok(Some(0)); }
                let words = vec![u64::from(target), level as u64, u64::from(format), u64::from(ty)];
                self.gl_queue(token, Command::Call { entry, args: words });
                let Some((status, words, data)) = self.gl_flush().into_iter().find_map(|c| match c { Command::ReplyData { status, words, data } => Some((status, words, data)), _ => None }) else {
                    self.gl_note(String::from("[HEIRLOOM-GL REFUSED call=glGetTexImage reason=worker-gone]"));
                    self.gl_set_error(GL_INVALID_OPERATION);
                    return Ok(Some(0));
                };
                if status != 0 { self.gl_note(String::from_utf8_lossy(&data).into_owned()); self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); }
                let (w, h, d) = (words.first().copied().unwrap_or(0) as u32, words.get(1).copied().unwrap_or(0) as u32, words.get(2).copied().unwrap_or(1) as u32);
                let base = self.mac.mem.strip(ptr);
                let store = self.gl.pack;
                let mem = &mut self.mac.mem;
                let mut put = |at: usize, b: &[u8]| u32::try_from(at).ok().and_then(|at| base.checked_add(at)).is_some_and(|to| mem.write_bytes(to, b).is_ok());
                let r = if target == GL_TEXTURE_3D { pixel::pack3d(&data, w, h, d, format, ty, &store, &mut put) } else { pixel::pack(&data, w, h, format, ty, &store, &mut put) };
                if let Err(e) = r {
                    self.gl_note(format!("[HEIRLOOM-GL] glGetTexImage result does not fit the guest buffer: {e:?}"));
                    self.gl_set_error(GL_INVALID_OPERATION);
                }
            }
            _ => unreachable!(),
        }
        Ok(Some(0))
    }
}