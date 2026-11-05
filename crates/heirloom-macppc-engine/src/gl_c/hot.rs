//! The calls a game makes every frame, dispatched by their contract entry's ROUTE instead of by comparing the call's name with
//! every arm of `gl_call` (Warcraft III gameplay: ~5,200 GL calls a frame, ~4,500 of them the ones below). A route is resolved
//! once per entry from its name ([`hot_of`]); an entry no arm of `gl_call` serves is learnt as [`Hot::State`] the first time it
//! reaches the state calls. The bodies are the arms they replace, unchanged in effect.
use super::*;

/// How `gl_call` dispatches one contract entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hot {
    /// One of the other arms of `gl_call`'s name match (AGL, queries, fences, ...).
    Rest,
    /// No arm of `gl_call`: straight to `gl_state_call`.
    State,
    /// glClear, glMatrixMode, glEnable, glDisable, glActiveTextureARB: forwarded with their first argument.
    Forward,
    /// glViewport: forwarded with four arguments.
    Viewport,
    /// glFinish: forwarded with none, then the batch is flushed.
    Finish,
    LoadMatrixf,
    /// glEnableClientState (true) / glDisableClientState (false).
    ClientState(bool),
    ClientActiveTexture,
    /// glVertexPointer / glColorPointer / glTexCoordPointer / glNormalPointer: a descriptor of this role.
    Pointer(Role),
    DrawElements,
    /// glLockArraysEXT / glUnlockArraysEXT.
    LockArrays,
}

/// The route of the entry named `name` (never [`Hot::State`]: that one is learnt).
pub(super) fn hot_of(name: &str) -> Hot {
    match name {
        "glClear" | "glMatrixMode" | "glEnable" | "glDisable" | "glActiveTextureARB" => Hot::Forward,
        "glViewport" => Hot::Viewport,
        "glFinish" => Hot::Finish,
        "glLoadMatrixf" => Hot::LoadMatrixf,
        "glEnableClientState" => Hot::ClientState(true),
        "glDisableClientState" => Hot::ClientState(false),
        "glClientActiveTextureARB" => Hot::ClientActiveTexture,
        "glVertexPointer" => Hot::Pointer(Role::Vertex),
        "glColorPointer" => Hot::Pointer(Role::Color),
        "glTexCoordPointer" => Hot::Pointer(Role::TexCoord),
        "glNormalPointer" => Hot::Pointer(Role::Normal),
        "glDrawElements" => Hot::DrawElements,
        "glLockArraysEXT" | "glUnlockArraysEXT" => Hot::LockArrays,
        _ => Hot::Rest,
    }
}

impl<U: GuestUi> MacPpc<U> {
    /// The route of contract entry `entry`.
    pub(super) fn gl_route(&mut self, entry: u16) -> Hot { self.gl_disp(entry).route }

    /// `entry` reached the state calls through `gl_call`'s last arm: route it there directly from now on.
    pub(super) fn gl_route_state(&mut self, entry: u16) {
        if let Some(d) = self.gl.disp.get_mut(usize::from(entry)).filter(|d| d.route == Hot::Rest) { d.route = Hot::State; }
    }

    /// One routed call (`hot` is neither `Rest` nor `State`); the result register is 0 (none of these calls returns a value).
    pub(super) fn gl_hot_call(&mut self, hot: Hot, entry: u16, name: &str, [a0, a1, a2, a3]: [u32; 4]) -> Result<(), String> {
        match hot {
            Hot::Forward | Hot::Viewport | Hot::Finish => {
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(a0)]) { self.gl_note(m.clone()); return Err(m); }
                let mut args = self.gl_words();
                match hot { Hot::Viewport => args.extend([a0, a1, a2, a3].map(u64::from)), Hot::Finish => {}, _ => args.push(u64::from(a0)) }
                match self.gl_ctx_token() {
                    Some(t) => { self.gl_queue_call(t, entry, args); if hot == Hot::Finish { self.gl_flush(); } }
                    None => self.gl_set_error(GL_INVALID_OPERATION),
                }
            }
            Hot::LoadMatrixf => {
                let mut w = self.gl_words();
                for i in 0..16 { w.push(u64::from(self.mac.mem.read_u32(a0.wrapping_add(4 * i)).map_err(|_| format!("glLoadMatrixf: the matrix at {a0:#x} leaves guest RAM"))?)); }
                match self.gl_ctx_token() { Some(t) => self.gl_queue_call(t, entry, w), None => { self.gl_words_back(w); self.gl_set_error(GL_INVALID_OPERATION) } }
            }
            // ---- client arrays: descriptors only, read at the draw ----------------------------------------------------
            Hot::ClientState(enable) if matches!(a0, 0x8077 | 0x8079) => {
                if let Some(m) = heirloom_gl_abi::gaps::refused_value(name, &[u64::from(a0)]).filter(|_| enable) { self.gl_note(m.clone()); return Err(m); }
                let slot = if a0 == 0x8077 { &mut self.gl.index_array } else { &mut self.gl.edge_array };
                slot.get_or_insert(IndexArray { ty: 0x1406, stride: 0, enabled: false, ptr: 0 }).enabled = enable;
            }
            Hot::ClientState(enable) => match role_of(a0) {
                Some(r) => {
                    let key = (r as u8, if r == Role::TexCoord { self.gl.client_unit } else { 0 });
                    if enable { self.gl.enabled.insert(key); } else { self.gl.enabled.remove(&key); }
                }
                None => return Err(format!("{name}: client state {a0:#x} is not one of the vertex/normal/colour/texcoord arrays this engine serves")),
            },
            Hot::ClientActiveTexture => { self.gl.client_unit = (a0.wrapping_sub(0x84C0)).min(255) as u8; if a0 < 0x84C0 || a0 > 0x84DF { self.gl_set_error(GL_INVALID_ENUM); self.gl.client_unit = 0; } }
            Hot::Pointer(role) => {
                let (size, ty, stride, ptr) = if role == Role::Normal { (3, a0, a1, a2) } else { (a0, a1, a2, a3) };
                let ok_size = match role { Role::Vertex => (2..=4).contains(&size), Role::Color => size == 3 || size == 4, Role::TexCoord => (1..=4).contains(&size), Role::Normal => true, Role::EdgeFlag => false };
                match ElemType::from_gl(ty).filter(|_| arrays::type_ok(role, ty)) {
                    Some(t) if ok_size && (stride as i32) >= 0 => {
                        let unit = if role == Role::TexCoord { self.gl.client_unit } else { 0 };
                        self.gl.arrays.insert((role as u8, unit), Descriptor { role, unit, offset: self.mac.mem.strip(ptr) as usize, size: size as u8, ty: t, stride });
                    }
                    Some(_) => self.gl_set_error(GL_INVALID_VALUE),
                    None => self.gl_set_error(GL_INVALID_ENUM),
                }
            }
            Hot::DrawElements => {
                let (mode, count, ty, ptr) = (a0, a1, a2, a3);
                let Some(t) = self.gl_ctx_token() else { self.gl_set_error(GL_INVALID_OPERATION); return Ok(()); };
                if mode > 9 || (count as i32) < 0 { self.gl_set_error(if mode > 9 { GL_INVALID_ENUM } else { GL_INVALID_VALUE }); return Ok(()); }
                let it = match ElemType::from_gl(ty) { Some(t @ (ElemType::U8 | ElemType::U16 | ElemType::U32)) => t, _ => { self.gl_set_error(GL_INVALID_ENUM); return Ok(()); } };
                let mut descs = core::mem::take(&mut self.gl.descs);
                descs.clear();
                descs.extend(self.gl.enabled.iter().filter_map(|k| self.gl.arrays.get(k).copied()));
                if descs.len() != self.gl.enabled.len() { self.gl.descs = descs; self.gl_set_error(GL_INVALID_OPERATION); return Ok(()); } // enabled with no pointer set
                let off = self.mac.mem.strip(ptr) as usize;
                let r = self.gl_queue_draw_elements(t, "glDrawElements", mode, off, count, it, &descs);
                self.gl.descs = descs;
                r?;
            }
            Hot::LockArrays => { /* arrays are read at the draw; the lock's promise (no change) is honoured trivially */ }
            Hot::Rest | Hot::State => return Err(format!("{name}: routed to the per-frame calls by mistake ({hot:?})")),
        }
        Ok(())
    }
}
