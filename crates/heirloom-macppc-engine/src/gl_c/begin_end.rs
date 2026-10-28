//! The glBegin / glEnd (GL 1.2 §2.6.1) and glNewList / glEndList (§5.4) state machines, kept by the ENGINE because every GL call passes through it before it
//! reaches the worker: between a glBegin and its glEnd the only commands allowed are the ones that specify vertex data (Vertex,
//! Color, Index, Normal, TexCoord, EvalCoord, EvalPoint, ArrayElement), Material, EdgeFlag and the display-list invocations
//! (CallList, CallLists); any other GL command, a second glBegin, and a glEnd with no glBegin all raise GL_INVALID_OPERATION and
//! do nothing. glNewList inside another glNewList and glEndList with no glNewList are GL_INVALID_OPERATION too; list name 0 is
//! GL_INVALID_VALUE. (glGetError stays callable, as in every implementation, so a program can find the error.)
use super::*;

/// May `name` be executed between a glBegin and its glEnd.
pub(super) fn allowed_inside_begin(name: &str) -> bool {
    const PREFIXES: &[&str] = &["glVertex", "glColor", "glIndex", "glNormal", "glTexCoord", "glEvalCoord", "glEvalPoint", "glMaterial", "glEdgeFlag"];
    matches!(name, "glCallList" | "glCallLists" | "glArrayElement" | "glGetError")
        || PREFIXES.iter().any(|p| name.starts_with(p) && !name.ends_with("Pointer") && name != "glColorMaterial" && name != "glIndexMask")
}

impl<U: GuestUi> MacPpc<U> {
    /// Whether a GL (not AGL) call may proceed given the Begin/End state; if not, the error is already raised.
    /// `mode_ok`: for glBegin, whether its argument is a valid primitive type.
    pub(super) fn gl_begin_end_gate(&mut self, name: &str, mode_ok: bool, a0: u32, a1: u32) -> Result<bool, String> {
        if !name.starts_with("gl") { return Ok(true); }
        // a call the worker cannot record into a display list is refused by name while one is compiled (gaps::IN_LIST_GAPS, empty on Mesa)
        if self.gl.list_ctx == Some(self.gl.current) {
            if let Some(m) = heirloom_gl_abi::gaps::refused_in_list(name) { self.gl_note(m.clone()); return Err(m); }
        }
        let inside = self.gl.begin_ctx == Some(self.gl.current);
        Ok(self.gl_begin_end_state(name, mode_ok, a0, a1, inside))
    }

    fn gl_begin_end_state(&mut self, name: &str, mode_ok: bool, a0: u32, a1: u32, inside: bool) -> bool {
        match name {
            "glBegin" if inside => { self.gl_set_error(GL_INVALID_OPERATION); false }
            "glBegin" if !mode_ok => { self.gl_set_error(0x0500); false }
            "glBegin" => { self.gl.begin_ctx = Some(self.gl.current); true }
            "glEnd" if !inside => { self.gl_set_error(GL_INVALID_OPERATION); false }
            "glEnd" => { self.gl.begin_ctx = None; true }
            "glNewList" if !inside && self.gl.list_ctx == Some(self.gl.current) => { self.gl_set_error(GL_INVALID_OPERATION); false }
            "glNewList" if !inside && a0 == 0 => { self.gl_set_error(GL_INVALID_VALUE); false }
            "glNewList" if !inside && !matches!(a1, 0x1300 | 0x1301) => { self.gl_set_error(0x0500); false }
            "glNewList" if !inside => { self.gl.list_ctx = Some(self.gl.current); true }
            "glEndList" if !inside && self.gl.list_ctx != Some(self.gl.current) => { self.gl_set_error(GL_INVALID_OPERATION); false }
            "glEndList" if !inside => { self.gl.list_ctx = None; true }
            other if inside && !allowed_inside_begin(other) => { self.gl_set_error(GL_INVALID_OPERATION); false }
            _ => true,
        }
    }
}
