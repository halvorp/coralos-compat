//! The worker's GL state as this engine last SENT it, for the state Warcraft III-class games set again and again: a call that
//! would set a value the worker already holds is not sent, and `glActiveTextureARB` is sent only when a call that depends on
//! the active texture unit follows it.
//!
//! Why: in Warcraft III gameplay ~45% of the state calls on the wire changed nothing (every draw is preceded by
//! glDisable(GL_COLOR_MATERIAL) on a disabled cap and glActiveTextureARB(0); glActiveTextureARB(1) with nothing between):
//! each was encoded on the guest's thread and decoded and executed on the worker's (h86 glcap, request-stream census).
//!
//! Exactness (a draw sees the same GL state with and without the shadow):
//! * Only values GL 1.2 always accepts are shadowed (the caps of [`cap_scope`], units 0 and 1 - ARB_multitexture's minimum -,
//!   the matrix modes, the blend factors / comparison functions / texture filters and wraps of [`Shadow::call`]), so a call
//!   not sent is never one that would have set a GL error. Any other value is sent and forgets the slot it names.
//! * A shadowed slot is known only from a call this engine sent; every entry not classified here ([`Class::Forget`], the
//!   default: matrix stacks, attribute stacks, display lists, glBegin/glEnd, texture deletion, every AGL call but the swap)
//!   sends the pending unit and forgets everything, as does the start of glNewList/glBegin and a drawable resize.
//! * The pending unit is sent before any call that names the active unit's state (texture enables, bindings, environment,
//!   parameters, images, texture-matrix loads, queries): only calls known NOT to depend on it skip that ([`Class::Global`]).
//! `MacPpc::gl_shadow(false)` sends every call (the reference; harness `WC3_GL_SHADOW=0`).
use super::*;

const GL_TEXTURE_ENV: u32 = 0x2300;
const GL_TEXTURE_ENV_MODE: u32 = 0x2200;
const GL_TEXTURE: u32 = 0x1702;
const UNIT0: u32 = 0x84C0;

/// How a call meets the shadow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class { Unit, Mode, Enable(bool), Bind, Env, Param, Blend, Alpha, DepthFunc, DepthMask, Viewport, Scissor, DepthRange, ClearColor, LoadMatrix,
    /// Changes no shadowed slot and does not depend on the active texture unit.
    Global,
    /// Changes no shadowed slot; depends on the active unit (the pending unit is sent first).
    UnitUse,
    /// Anything else: the pending unit is sent, then everything is forgotten.
    Forget }

fn class_of(name: &str) -> Class {
    match name {
        "glActiveTextureARB" => Class::Unit,
        "glMatrixMode" => Class::Mode,
        "glEnable" => Class::Enable(true),
        "glDisable" => Class::Enable(false),
        "glBindTexture" => Class::Bind,
        "glTexEnvi" => Class::Env,
        "glTexParameteri" => Class::Param,
        "glBlendFunc" => Class::Blend,
        "glAlphaFunc" => Class::Alpha,
        "glDepthFunc" => Class::DepthFunc,
        "glDepthMask" => Class::DepthMask,
        "glViewport" => Class::Viewport,
        "glScissor" => Class::Scissor,
        "glDepthRange" => Class::DepthRange,
        "glClearColor" => Class::ClearColor,
        "glLoadMatrixf" => Class::LoadMatrix,
        // client-array state is the engine's own; draws read the enabled units, not the active one; per-vertex and lighting state is
        // not shadowed (draws and colour tracking change it)
        "glClientActiveTextureARB" | "glEnableClientState" | "glDisableClientState" | "glVertexPointer" | "glColorPointer" | "glTexCoordPointer"
        | "glNormalPointer" | "glLockArraysEXT" | "glUnlockArraysEXT" | "glDrawElements" | "glDrawRangeElements" | "glDrawArrays" | "glClear"
        | "glFinish" | "glFlush" | "glGetError" | "glPixelStorei" | "glNormal3fv" | "glMaterialfv" | "glLightfv" | "glLightf" | "glLightModelfv"
        | "glFogf" | "glFogfv" | "glFogi" | "glShadeModel" | "glCullFace" | "glFrontFace" | "glPolygonOffset" | "glColorMask" | "glClearDepth"
        | "glHint" | "glColorMaterial" | "glGenTextures" | "glGenFencesAPPLE" | "glSetFenceAPPLE" | "glTestFenceAPPLE" | "glFinishFenceAPPLE"
        | "glIsFenceAPPLE" | "glDeleteFencesAPPLE" | "glVertexArrayRangeAPPLE" | "glFlushVertexArrayRangeAPPLE" | "glVertexArrayParameteriAPPLE"
        | "aglSwapBuffers" | "aglGetError" | "glGetString" => Class::Global,
        "glTexImage2D" | "glTexSubImage2D" | "glTexImage1D" | "glTexSubImage1D" | "glCopyTexImage2D" | "glCopyTexSubImage2D" | "glGetIntegerv"
        | "glGetFloatv" | "glGetBooleanv" | "glGetDoublev" | "glIsEnabled" | "glGetTexEnviv" | "glGetTexEnvfv" | "glGetTexParameteriv"
        | "glGetTexParameterfv" | "glGetTexLevelParameteriv" | "glGetTexImage" => Class::UnitUse,
        _ => Class::Forget,
    }
}

/// Whether `cap` is a GL 1.2 capability (never an enum error), and whether it is per texture unit.
fn cap_scope(cap: u32) -> Option<bool> {
    match cap {
        0x0DE0 | 0x0DE1 | 0x0C60..=0x0C63 => Some(true),
        0x0BC0 | 0x0D80 | 0x0BE2 | 0x3000..=0x3005 | 0x0BF2 | 0x0B57 | 0x0B44 | 0x0B71 | 0x0BD0 | 0x0B60 | 0x4000..=0x4007 | 0x0B50 | 0x0B20
        | 0x0B24 | 0x0BA1 | 0x0B10 | 0x8037 | 0x0B41 | 0x0B42 | 0x0C11 | 0x0B90 | 0x803A => Some(false),
        _ => None,
    }
}

fn blend_src(f: u32) -> bool { matches!(f, 0 | 1 | 0x0300..=0x0308) }
fn blend_dst(f: u32) -> bool { matches!(f, 0 | 1 | 0x0300..=0x0307) }
fn compare_fn(f: u32) -> bool { (0x0200..=0x0207).contains(&f) }
fn env_mode(m: u32) -> bool { matches!(m, 0x2100 | 0x2101 | 0x0BE2 | 0x1E01 | 0x0104 | 0x8570) }
fn tex_param(pname: u32, v: u32) -> bool {
    match pname { 0x2800 => matches!(v, 0x2600 | 0x2601), 0x2801 => matches!(v, 0x2600 | 0x2601 | 0x2700..=0x2703), 0x2802 | 0x2803 => matches!(v, 0x2900 | 0x2901 | 0x812F), _ => false }
}

#[derive(Default)]
pub(crate) struct Shadow {
    /// Sending every call (the reference behaviour).
    pub(crate) off: bool,
    class: Vec<Class>,
    /// The unit the worker has active, and the one the program selected last (sent when a call depends on it). None: unknown.
    sent_unit: Option<u32>,
    want_unit: Option<u32>,
    mode: Option<u32>,
    /// (cap, unit or 0) -> enabled; (unit, target) -> texture name; (unit, pname) -> env value; (target, name, pname) -> value.
    enabled: BTreeMap<(u32, u32), bool>,
    bound: BTreeMap<(u32, u32), u32>,
    env: BTreeMap<(u32, u32), u32>,
    params: BTreeMap<(u32, u32, u32), u32>,
    /// glBlendFunc, glAlphaFunc (func, ref bits), glDepthFunc, glDepthMask, glViewport, glScissor, glDepthRange, glClearColor.
    blend: Option<(u32, u32)>,
    alpha: Option<(u32, u64)>,
    depth_func: Option<u32>,
    depth_mask: Option<u32>,
    viewport: Option<[u32; 4]>,
    scissor: Option<[u32; 4]>,
    depth_range: Option<(u64, u64)>,
    clear_color: Option<[u64; 4]>,
    /// (mode, unit for GL_TEXTURE else 0) -> the matrix's words.
    matrices: BTreeMap<(u32, u32), [u32; 16]>,
    /// Calls not sent (a test and the census read it).
    pub(crate) dropped: u64,
}

impl Shadow {
    /// Forget every slot (the pending unit must have been sent first).
    pub(super) fn forget(&mut self) {
        let (off, class, dropped) = (self.off, core::mem::take(&mut self.class), self.dropped);
        *self = Shadow { off, class, dropped, ..Shadow::default() };
    }
}

/// `Some(true)`: `slot` already holds `v` (the call is not sent); `Some(false)`: it is set to `v`.
fn same<T: PartialEq + Copy>(slot: &mut Option<T>, v: T) -> bool { let s = *slot == Some(v); *slot = Some(v); s }
fn same_in<K: Ord, T: PartialEq + Copy>(m: &mut BTreeMap<K, T>, k: K, v: T) -> bool { m.insert(k, v) == Some(v) }

impl<U: GuestUi> MacPpc<U> {
    /// Send every GL call the program makes (off) or leave out the ones that change nothing (on, the default).
    pub fn gl_shadow(&mut self, on: bool) { self.gl.shadow.off = !on; self.gl.shadow.forget(); }
    /// Calls the shadow did not send.
    pub fn gl_shadowed(&self) -> u64 { self.gl.shadow.dropped }

    /// Send the selected texture unit if the worker has another one active.
    pub(super) fn gl_shadow_send_unit(&mut self) {
        let sh = &mut self.gl.shadow;
        let Some(u) = sh.want_unit.filter(|u| sh.sent_unit != Some(*u)) else { return };
        let (Some(t), Some(e)) = (self.gl.current_token, GlFn::by_name("glActiveTextureARB")) else { return };
        sh.sent_unit = Some(u);
        // the held call goes out after all: it is not one the shadow left out
        sh.dropped = sh.dropped.saturating_sub(1);
        let mut w = self.gl_words();
        w.push(u64::from(UNIT0 + u));
        self.gl_queue_call(t, e.0, w);
    }

    /// Meet call `entry` (its arguments in `cpu`) with the shadow: true = it changes nothing the worker holds and is not sent.
    /// `inside`: the current context is between glBegin/glEnd or compiling a display list (nothing is shadowed there).
    pub(super) fn gl_shadow_meet(&mut self, entry: u16, cpu: &Cpu, inside: bool) -> Result<bool, String> {
        if self.gl.shadow.off { return Ok(false); }
        if self.gl.shadow.class.is_empty() { self.gl.shadow.class = ENTRIES.iter().map(|e| class_of(e.name)).collect(); }
        let class = self.gl.shadow.class.get(usize::from(entry)).copied().unwrap_or(Class::Forget);
        if self.gl.current_token.is_none() || inside {
            // no context (the call is an error) or a glBegin / display list in progress: send everything, know nothing
            if class != Class::Global { self.gl_shadow_send_unit(); self.gl.shadow.forget(); }
            return Ok(false);
        }
        let g = |k: usize| cpu.gpr[3 + k];
        let unit_dep = |sh: &Shadow| sh.want_unit;
        let sh = &mut self.gl.shadow;
        let drop = match class {
            Class::Global => return Ok(false),
            Class::UnitUse => { self.gl_shadow_send_unit(); return Ok(false); }
            Class::Forget => { self.gl_shadow_send_unit(); self.gl.shadow.forget(); return Ok(false); }
            Class::Unit => match g(0).checked_sub(UNIT0).filter(|u| *u < 2) {
                // held: sent before the next call that depends on it
                Some(u) => { sh.want_unit = Some(u); true }
                None => { sh.want_unit = None; sh.sent_unit = None; sh.forget_unit_state(); false }
            },
            Class::Mode => if matches!(g(0), 0x1700..=0x1702) { same(&mut sh.mode, g(0)) } else { sh.mode = None; false },
            Class::Enable(on) => match cap_scope(g(0)) {
                Some(false) => same_in(&mut sh.enabled, (g(0), 0), on),
                Some(true) => match unit_dep(sh) {
                    Some(u) => { if same_in(&mut sh.enabled, (g(0), u + 1), on) { true } else { self.gl_shadow_send_unit(); false } }
                    None => { self.gl_shadow_send_unit(); false }
                },
                None => { self.gl_shadow_send_unit(); false }
            },
            Class::Bind => match (unit_dep(sh), g(0)) {
                (Some(u), t @ (0x0DE0 | 0x0DE1)) => { if same_in(&mut sh.bound, (u, t), g(1)) { true } else { self.gl_shadow_send_unit(); false } }
                _ => { self.gl_shadow_send_unit(); self.gl.shadow.bound.clear(); self.gl.shadow.params.clear(); false }
            },
            Class::Env => match unit_dep(sh) {
                Some(u) if g(0) == GL_TEXTURE_ENV && g(1) == GL_TEXTURE_ENV_MODE && env_mode(g(2)) => { if same_in(&mut sh.env, (u, g(1)), g(2)) { true } else { self.gl_shadow_send_unit(); false } }
                _ => { self.gl_shadow_send_unit(); self.gl.shadow.env.clear(); false }
            },
            Class::Param => match unit_dep(sh).and_then(|u| sh.bound.get(&(u, g(0))).copied()) {
                Some(name) if tex_param(g(1), g(2)) => { if same_in(&mut sh.params, (g(0), name, g(1)), g(2)) { true } else { self.gl_shadow_send_unit(); false } }
                _ => { self.gl_shadow_send_unit(); self.gl.shadow.params.clear(); false }
            },
            Class::Blend => if blend_src(g(0)) && blend_dst(g(1)) { same(&mut sh.blend, (g(0), g(1))) } else { sh.blend = None; false },
            Class::Alpha => if compare_fn(g(0)) { same(&mut sh.alpha, (g(0), cpu.fpr[1])) } else { sh.alpha = None; false },
            Class::DepthFunc => if compare_fn(g(0)) { same(&mut sh.depth_func, g(0)) } else { sh.depth_func = None; false },
            Class::DepthMask => same(&mut sh.depth_mask, g(0) & 0xFF),
            Class::Viewport | Class::Scissor => {
                let v = [g(0), g(1), g(2), g(3)];
                let slot = if class == Class::Viewport { &mut sh.viewport } else { &mut sh.scissor };
                if (v[2] as i32) >= 0 && (v[3] as i32) >= 0 { same(slot, v) } else { *slot = None; false }
            }
            Class::DepthRange => same(&mut sh.depth_range, (cpu.fpr[1], cpu.fpr[2])),
            Class::ClearColor => same(&mut sh.clear_color, [cpu.fpr[1], cpu.fpr[2], cpu.fpr[3], cpu.fpr[4]]),
            Class::LoadMatrix => {
                let key = match (sh.mode, unit_dep(sh)) { (Some(GL_TEXTURE), Some(u)) => Some((GL_TEXTURE, u)), (Some(GL_TEXTURE), None) | (None, _) => None, (Some(m), _) => Some((m, 0)) };
                let p = self.mac.mem.strip(g(0));
                let mut m = [0u32; 16];
                for (i, w) in m.iter_mut().enumerate() { *w = self.mac.mem.read_u32(p.wrapping_add(4 * i as u32)).map_err(|_| format!("glLoadMatrixf: the matrix at {p:#x} leaves guest RAM"))?; }
                let sh = &mut self.gl.shadow;
                let drop = match key { Some(k) => same_in(&mut sh.matrices, k, m), None => { sh.matrices.clear(); false } };
                if !drop && key.is_none_or(|k| k.0 == GL_TEXTURE) { self.gl_shadow_send_unit(); }
                drop
            }
        };
        if drop { self.gl.shadow.dropped += 1; }
        Ok(drop)
    }
}

impl Shadow {
    /// The active unit became unknown: what was known per unit cannot be addressed any more.
    fn forget_unit_state(&mut self) {
        self.enabled.retain(|(_, u), _| *u == 0);
        self.bound.clear();
        self.env.clear();
        self.params.clear();
        self.matrices.retain(|(m, _), _| *m != GL_TEXTURE);
    }
}
