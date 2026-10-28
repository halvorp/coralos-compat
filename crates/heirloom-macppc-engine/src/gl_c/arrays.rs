//! Client-array calls (GL 1.2 §2.8) and client state: the pointers the program gives GL are descriptors until a draw reads them.
//! glIndexPointer / glEdgeFlagPointer, glInterleavedArrays, glDrawArrays, glDrawRangeElements, glArrayElement,
//! glPushClientAttrib / glPopClientAttrib, glPixelStoref and glGetPointerv. Every call is validated here with the error the
//! specification names and a draw is snapshotted out of guest memory at the draw, converted to the wire's byte order.
use super::*;
use alloc::vec;
use heirloom_gl_abi::family::Ty;
use heirloom_gl_abi::find;

/// `GL_CLIENT_PIXEL_STORE_BIT`, `GL_CLIENT_VERTEX_ARRAY_BIT`.
const CLIENT_PIXEL_STORE_BIT: u32 = 0x1;
const CLIENT_VERTEX_ARRAY_BIT: u32 = 0x2;

/// The client state `glPushClientAttrib` saves.
#[derive(Clone)]
pub(crate) struct ClientSave {
    mask: u32,
    unpack: Store,
    pack: Store,
    arrays: BTreeMap<(u8, u8), Descriptor>,
    enabled: BTreeSet<(u8, u8)>,
    client_unit: u8,
    index_array: Option<IndexArray>,
    edge_array: Option<IndexArray>,
}

/// GL 1.2 Table 2.5: one row of `glInterleavedArrays` - (format, tex size, colour size + type, normal, vertex size, offsets in floats, stride in floats).
struct Interleaved {
    format: u32,
    tex: u8,
    col: u8,
    col_ty: ElemType,
    normal: bool,
    vert: u8,
    pc: u32,
    pn: u32,
    pv: u32,
    stride: u32,
}

const F: u32 = 4; // sizeof(GLfloat); the colour of C4UB is four bytes = one float of room
const fn row(format: u32, tex: u8, col: u8, col_ty: ElemType, normal: bool, vert: u8, pc: u32, pn: u32, pv: u32, stride: u32) -> Interleaved {
    Interleaved { format, tex, col, col_ty, normal, vert, pc, pn, pv, stride }
}
static INTERLEAVED: &[Interleaved] = &[
    row(0x2A20, 0, 0, ElemType::F32, false, 2, 0, 0, 0, 2 * F),            // GL_V2F
    row(0x2A21, 0, 0, ElemType::F32, false, 3, 0, 0, 0, 3 * F),            // GL_V3F
    row(0x2A22, 0, 4, ElemType::U8, false, 2, 0, 0, F, 3 * F),             // GL_C4UB_V2F
    row(0x2A23, 0, 4, ElemType::U8, false, 3, 0, 0, F, 4 * F),             // GL_C4UB_V3F
    row(0x2A24, 0, 3, ElemType::F32, false, 3, 0, 0, 3 * F, 6 * F),        // GL_C3F_V3F
    row(0x2A25, 0, 0, ElemType::F32, true, 3, 0, 0, 3 * F, 6 * F),         // GL_N3F_V3F
    row(0x2A26, 0, 4, ElemType::F32, true, 3, 0, 4 * F, 7 * F, 10 * F),    // GL_C4F_N3F_V3F
    row(0x2A27, 2, 0, ElemType::F32, false, 3, 0, 0, 2 * F, 5 * F),        // GL_T2F_V3F
    row(0x2A28, 4, 0, ElemType::F32, false, 4, 0, 0, 4 * F, 8 * F),        // GL_T4F_V4F
    row(0x2A29, 2, 4, ElemType::U8, false, 3, 2 * F, 0, 3 * F, 4 * F + F), // GL_T2F_C4UB_V3F: c = 4 bytes
    row(0x2A2A, 2, 3, ElemType::F32, false, 3, 2 * F, 0, 5 * F, 8 * F),    // GL_T2F_C3F_V3F
    row(0x2A2B, 2, 0, ElemType::F32, true, 3, 0, 2 * F, 5 * F, 8 * F),     // GL_T2F_N3F_V3F
    row(0x2A2C, 2, 4, ElemType::F32, true, 3, 2 * F, 6 * F, 9 * F, 12 * F), // GL_T2F_C4F_N3F_V3F
    row(0x2A2D, 4, 4, ElemType::F32, true, 4, 4 * F, 8 * F, 11 * F, 15 * F), // GL_T4F_C4F_N3F_V4F
];

/// The valid element types of each array (GL 1.2 §2.8).
pub(super) fn type_ok(role: Role, ty: u32) -> bool {
    match role {
        Role::Vertex | Role::TexCoord => matches!(ty, 0x1402 | 0x1404 | 0x1406 | 0x140A),
        Role::Normal => matches!(ty, 0x1400 | 0x1402 | 0x1404 | 0x1406 | 0x140A),
        Role::Color => matches!(ty, 0x1400..=0x1406 | 0x140A),
        Role::EdgeFlag => ty == 0x1401,
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// Snapshot the enabled arrays' descriptors (an enabled array with no pointer set is GL_INVALID_OPERATION).
    fn gl_enabled_descriptors(&mut self) -> Option<Vec<Descriptor>> {
        let mut descs: Vec<Descriptor> = self.gl.enabled.iter().filter_map(|k| self.gl.arrays.get(k).copied()).collect();
        if descs.len() != self.gl.enabled.len() { self.gl_set_error(GL_INVALID_OPERATION); return None; }
        // the edge-flag array (§2.8): one GLboolean per vertex, carried with the draw like the other arrays
        if let Some(x) = self.gl.edge_array.filter(|x| x.enabled) {
            descs.push(Descriptor { role: Role::EdgeFlag, unit: 0, offset: self.mac.mem.strip(x.ptr) as usize, size: 1, ty: ElemType::U8, stride: x.stride });
        }
        Some(descs)
    }

    /// Queue a draw command captured from guest memory (without copying the memory).
    pub(super) fn gl_queue_draw(&mut self, token: Token, who: &str, cap: impl FnOnce(&[u8]) -> Result<DrawCommand, heirloom_gl_abi::snapshot::SnapshotError>) -> Result<(), String> {
        let len = self.mac.mem.len().min(u32::MAX as usize) as u32;
        let r = {
            let mem = self.mac.mem.span(0, len).and_then(|s| self.mac.mem.bytes(s)).map_err(|_| format!("{who}: guest RAM is not addressable"))?;
            cap(mem)
        };
        match r {
            Ok(d) => self.gl_queue(token, Command::Draw(d)),
            Err(e) => { self.gl_set_error(GL_INVALID_VALUE); self.gl_note(format!("[HEIRLOOM-GL] {who} refused: {e:?}")); }
        }
        Ok(())
    }

    /// The call `name` if it is one of this module's; `None` otherwise.
    pub(super) fn gl_array_call(&mut self, name: &str, entry: u16, cpu: &mut Cpu) -> Result<Option<u32>, String> {
        let a = |m: &Self, k: usize| arg(m, cpu, k);
        match name {
            "glIndexPointer" | "glEdgeFlagPointer" | "glInterleavedArrays" | "glDrawArrays" | "glDrawRangeElements" | "glArrayElement" | "glPushClientAttrib" | "glPopClientAttrib" | "glPixelStoref" | "glGetPointerv" => {}
            _ => return Ok(None),
        }
        let (a0, a1, a2, a3) = (a(self, 0)?, a(self, 1).unwrap_or(0), a(self, 2).unwrap_or(0), a(self, 3).unwrap_or(0));
        match name {
            "glIndexPointer" => {
                if !matches!(a0, 0x1401 | 0x1402 | 0x1404 | 0x1406 | 0x140A) { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if (a1 as i32) < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let en = self.gl.index_array.is_some_and(|x| x.enabled);
                self.gl.index_array = Some(IndexArray { ty: a0, stride: a1, enabled: en, ptr: a2 });
            }
            "glEdgeFlagPointer" => {
                if (a0 as i32) < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let en = self.gl.edge_array.is_some_and(|x| x.enabled);
                self.gl.edge_array = Some(IndexArray { ty: 0x1401, stride: a0, enabled: en, ptr: a1 });
            }
            "glInterleavedArrays" => {
                let Some(r) = INTERLEAVED.iter().find(|r| r.format == a0) else { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); };
                if (a1 as i32) < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let (stride, base) = (if a1 == 0 { r.stride } else { a1 }, self.mac.mem.strip(a2) as usize);
                // §2.8: the edge-flag and index arrays are disabled; the others are enabled exactly when the format has them
                if let Some(x) = self.gl.index_array.as_mut() { x.enabled = false; }
                if let Some(x) = self.gl.edge_array.as_mut() { x.enabled = false; }
                let unit = self.gl.client_unit;
                self.gl.enabled.retain(|k| !matches!(Role::from_u8(k.0), Some(Role::Vertex | Role::Normal | Role::Color)) && !(k.0 == Role::TexCoord as u8 && k.1 == unit));
                let set = |m: &mut Self, role: Role, u: u8, size: u8, ty: ElemType, off: u32| {
                    m.gl.arrays.insert((role as u8, u), Descriptor { role, unit: u, offset: base + off as usize, size, ty, stride });
                    m.gl.enabled.insert((role as u8, u));
                };
                if r.tex > 0 { set(self, Role::TexCoord, unit, r.tex, ElemType::F32, 0); }
                if r.col > 0 { set(self, Role::Color, 0, r.col, r.col_ty, r.pc); }
                if r.normal { set(self, Role::Normal, 0, 3, ElemType::F32, r.pn); }
                set(self, Role::Vertex, 0, r.vert, ElemType::F32, r.pv);
            }
            "glDrawArrays" => {
                let Some(t) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); };
                if a0 > 9 { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if (a2 as i32) < 0 || (a1 as i32) < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let Some(descs) = self.gl_enabled_descriptors() else { return Ok(Some(0)); };
                self.gl_queue_draw(t, name, |mem| DrawCommand::capture_arrays(mem, a0, a1, a2, &descs))?;
            }
            "glDrawRangeElements" => {
                // glDrawRangeElements(mode, start, end, count, type, indices) = glDrawElements for indices within [start, end] (§2.8)
                let Some(t) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(Some(0)); };
                let (count, ty, ptr) = (a3, a(self, 4)?, a(self, 5)?);
                if a0 > 9 { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                if (count as i32) < 0 || a2 < a1 { self.gl_set_error(GL_INVALID_VALUE); return Ok(Some(0)); }
                let it = match ElemType::from_gl(ty) { Some(t @ (ElemType::U8 | ElemType::U16 | ElemType::U32)) => t, _ => { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); } };
                let Some(descs) = self.gl_enabled_descriptors() else { return Ok(Some(0)); };
                let off = self.mac.mem.strip(ptr) as usize;
                self.gl_queue_draw(t, name, |mem| DrawCommand::capture(mem, a0, off, count, it, &descs))?;
            }
            "glArrayElement" => return self.gl_array_element(a0 as i32).map(|()| Some(0)),
            "glPushClientAttrib" => {
                let save = ClientSave { mask: a0, unpack: self.gl.unpack, pack: self.gl.pack, arrays: self.gl.arrays.clone(), enabled: self.gl.enabled.clone(), client_unit: self.gl.client_unit, index_array: self.gl.index_array, edge_array: self.gl.edge_array };
                self.gl.client_stack.push(save);
            }
            "glPopClientAttrib" => {
                let Some(s) = self.gl.client_stack.pop() else { self.gl_set_error(0x0504); return Ok(Some(0)); };
                if s.mask & CLIENT_VERTEX_ARRAY_BIT != 0 {
                    self.gl.arrays = s.arrays;
                    self.gl.enabled = s.enabled;
                    self.gl.client_unit = s.client_unit;
                    self.gl.index_array = s.index_array;
                    self.gl.edge_array = s.edge_array;
                }
                if s.mask & CLIENT_PIXEL_STORE_BIT != 0 {
                    self.gl.unpack = s.unpack;
                    self.gl.pack = s.pack;
                    // the worker's own pixel-store state follows
                    if let (Some(t), Some(e)) = (self.gl_ctx_token(), find(ENTRIES, "glPixelStorei")) {
                        for (base, st) in [(0x0CF0u64, s.unpack), (0x0D00, s.pack)] {
                            for (k, v) in [u32::from(st.swap_bytes), 0, st.row_length, st.skip_rows, st.skip_pixels, st.alignment].into_iter().enumerate() {
                                if k != 1 { self.gl_queue(t, Command::Call { entry: e as u16, args: vec![base + k as u64, u64::from(v)] }); }
                            }
                        }
                    }
                }
            }
            "glPixelStoref" => {
                // §3.6.1: a float parameter of an integer-valued mode is rounded to the nearest integer (a boolean mode is nonzero = true)
                let v = f64::from_bits(cpu.fpr[1]) as f32;
                let r = if v >= 0.0 { v + 0.5 } else { v - 0.5 } as i32;
                if !self.gl_pixel_store(a0, r) { return Ok(Some(0)); }
                if let (Some(t), Some(e)) = (self.gl_ctx_token(), find(ENTRIES, "glPixelStorei")) {
                    self.gl_queue(t, Command::Call { entry: e as u16, args: vec![u64::from(a0), u64::from(r as u32)] });
                }
            }
            "glGetPointerv" => {
                // §6.1: the pointer a program set, as the program set it (the client unit's texture-coordinate pointer)
                let p = match a0 {
                    0x808E | 0x808F | 0x8090 | 0x8092 => {
                        let (role, unit) = match a0 { 0x808E => (Role::Vertex, 0), 0x808F => (Role::Normal, 0), 0x8090 => (Role::Color, 0), _ => (Role::TexCoord, self.gl.client_unit) };
                        self.gl.arrays.get(&(role as u8, unit)).map_or(0, |d| d.offset as u32)
                    }
                    0x8091 => self.gl.index_array.map_or(0, |x| x.ptr),
                    0x8093 => self.gl.edge_array.map_or(0, |x| x.ptr),
                    0x0DF0 => self.gl.feedback_ptr,
                    0x0DF3 => self.gl.select_ptr,
                    _ => { self.gl_set_error(GL_INVALID_ENUM); return Ok(Some(0)); }
                };
                self.mac.mem.write_u32(a1, p).map_err(|_| String::from("glGetPointerv: the result leaves guest RAM"))?;
            }
            _ => unreachable!(),
        }
        let _ = (entry, a2, a3);
        Ok(Some(0))
    }

    /// glArrayElement(i) (§2.8): the enabled arrays' element `i` as the vertex, normal, colour and texture-coordinate calls it is
    /// equivalent to, in the order GL 1.2 gives (texture coordinate, colour, index, normal, vertex), sent as ordinary calls.
    fn gl_array_element(&mut self, i: i32) -> Result<(), String> {
        let Some(token) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(()); };
        if i < 0 { self.gl_set_error(GL_INVALID_VALUE); return Ok(()); }
        let unit = self.gl.client_unit;
        let mut calls: Vec<(String, Vec<u64>)> = Vec::new();
        let order = [(Role::TexCoord, unit), (Role::Color, 0), (Role::Normal, 0), (Role::Vertex, 0)];
        for (role, u) in order {
            if !self.gl.enabled.contains(&(role as u8, u)) { continue; }
            let Some(d) = self.gl.arrays.get(&(role as u8, u)).copied() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(()); };
            let (ty, sfx) = match d.ty {
                ElemType::I8 => (Ty::Byte, "b"), ElemType::U8 => (Ty::Ubyte, "ub"), ElemType::I16 => (Ty::Short, "s"), ElemType::U16 => (Ty::Ushort, "us"),
                ElemType::I32 => (Ty::Int, "i"), ElemType::U32 => (Ty::Uint, "ui"), ElemType::F32 => (Ty::Float, "f"), ElemType::F64 => (Ty::Double, "d"),
            };
            let stride = if d.stride == 0 { d.size as usize * ty.guest_bytes() } else { d.stride as usize };
            let at = u32::try_from((i as usize).checked_mul(stride).and_then(|o| o.checked_add(d.offset)).ok_or_else(|| String::from("glArrayElement: the element address overflows"))?).map_err(|_| String::from("glArrayElement: the element address overflows"))?;
            let words: Vec<u64> = self.gl_elements(at, d.size as usize, ty, "glArrayElement")?;
            let name = match role {
                Role::TexCoord => format!("glTexCoord{}{sfx}v", d.size),
                Role::Color => format!("glColor{}{sfx}v", d.size),
                Role::Normal => format!("glNormal3{sfx}v"),
                Role::Vertex => format!("glVertex{}{sfx}v", d.size),
                Role::EdgeFlag => String::from("glEdgeFlagv"), // not in `order`: the edge-flag array is read below
            };
            calls.push((name, words));
        }
        if let Some(x) = self.gl.edge_array.filter(|x| x.enabled) {
            let stride = if x.stride == 0 { 1 } else { x.stride };
            let at = self.mac.mem.strip(x.ptr).wrapping_add((i as u32).wrapping_mul(stride));
            calls.push((String::from("glEdgeFlagv"), self.gl_elements(at, 1, Ty::Bool, "glArrayElement")?));
        }
        if self.gl.index_array.is_some_and(|x| x.enabled) {
            let x = self.gl.index_array.unwrap();
            let (ty, sfx) = match x.ty { 0x1401 => (Ty::Ubyte, "ub"), 0x1402 => (Ty::Short, "s"), 0x1404 => (Ty::Int, "i"), 0x1406 => (Ty::Float, "f"), _ => (Ty::Double, "d") };
            let stride = if x.stride == 0 { ty.guest_bytes() } else { x.stride as usize };
            let at = self.mac.mem.strip(x.ptr).wrapping_add((i as u32).wrapping_mul(stride as u32));
            calls.push((format!("glIndex{sfx}v"), self.gl_elements(at, 1, ty, "glArrayElement")?));
        }
        for (name, words) in calls {
            let Some(e) = find(ENTRIES, &name) else { return Err(format!("glArrayElement: no contract entry {name}")); };
            self.gl_queue(token, Command::Call { entry: e as u16, args: words });
        }
        Ok(())
    }
}
