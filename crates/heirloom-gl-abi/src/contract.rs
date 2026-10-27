//! THE CONTRACT AS DATA: every GL/AGL entry point the four Warcraft III binaries import, plus the entries that make
//! each version/extension claim complete. Imports were read from the PEF import tables of
//! `roc-Warcraft_III`, `roc-Warcraft_III_World_Editor`, `tft-Warcraft_III_TFT` and `tft-Warcraft_III_World_Editor_TFT`
//! (libraries OpenGLLibrary, OpenGLUtility, OpenGLMemory; 73 distinct names) with `heirloom-fmt-pef` on 2026-10-25.
//!
//! `implemented` is the ONLY place a capability is switched on, and it is false until the worker backs the entry end to
//! end. [`crate::caps`] derives the advertised strings from it.
use crate::version::{Ext, Home, Version};

/// Who supplies the entry's behaviour in `heirloom-gl.engine`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// gl4es (pinned, unmodified) defines the symbol; checked against `libGL.a` at the audited revision.
    Gl4es,
    /// Nothing in gl4es defines it (AGL is Apple's API; the APPLE_* extensions are absent from gl4es): the Coral adapter.
    Adapter,
}

/// One entry point of the contract.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: &'static str,
    pub home: Home,
    /// Which binaries import it: bit0 RoC game, bit1 RoC World Editor, bit2 TFT game, bit3 TFT World Editor.
    /// Zero: not imported; listed so a version/extension claim is complete.
    pub imported_by: u8,
    pub owner: Owner,
    /// True only when the worker executes this entry end to end (an argument it cannot serve is refused by name; see `gap`).
    pub implemented: bool,
    /// `Some(what)`: the entry runs, but `what` (argument values the worker refuses by name) is not served, so a version
    /// or extension that lists the entry is NOT claimable until the gap is closed ([`Entry::complete`]).
    pub gap: Option<&'static str>,
}

impl Entry {
    /// The same entry, executed end to end by the worker (the only way a row is switched on).
    pub const fn on(self) -> Entry {
        Entry { implemented: true, ..self }
    }

    /// Executed by the worker, EXCEPT for what `gap` names (refused by name at the call): callable, never claimable.
    pub const fn partial(self, gap: &'static str) -> Entry {
        Entry { implemented: true, gap: Some(gap), ..self }
    }

    /// Whether a claim (a `GL_VERSION`, a `GL_EXTENSIONS` token) may rely on this entry: implemented with no gap.
    pub const fn complete(&self) -> bool {
        self.implemented && self.gap.is_none()
    }
}

const fn e(name: &'static str, home: Home, imported_by: u8, owner: Owner) -> Entry {
    Entry { name, home, imported_by, owner, implemented: false, gap: None }
}

/// The full table (the import census sorted by name, then the completeness entries).
pub static ENTRIES: &[Entry] = &[
    e("aglChoosePixelFormat", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglCreateContext", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglDescribeRenderer", Home::Agl, 0b0101, Owner::Adapter),
    e("aglDestroyContext", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglDestroyPixelFormat", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglDestroyRendererInfo", Home::Agl, 0b0101, Owner::Adapter),
    e("aglGetError", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglNextRendererInfo", Home::Agl, 0b0101, Owner::Adapter),
    e("aglQueryRendererInfo", Home::Agl, 0b0101, Owner::Adapter),
    e("aglSetCurrentContext", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglSetDrawable", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("aglSetFullScreen", Home::Agl, 0b1111, Owner::Adapter),
    e("aglSwapBuffers", Home::Agl, 0b1111, Owner::Adapter).on(),
    e("glActiveTextureARB", Home::Ext(Ext::ArbMultitexture), 0b1111, Owner::Gl4es).on(),
    e("glAlphaFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glBindTexture", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glBlendFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glClear", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glClearColor", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glClientActiveTextureARB", Home::Ext(Ext::ArbMultitexture), 0b1111, Owner::Gl4es).on(),
    e("glColorMaterial", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glColorPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glDeleteFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glDeleteTextures", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glDepthFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glDepthMask", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glDepthRange", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glDisable", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glDisableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glDrawBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("a front buffer is refused by name (gaps::VALUE_GAPS): the drawable is one image; while GL_NONE is the draw buffer a display list that calls glColorMask re-enables writes (the list is gl4es's)"),
    e("glDrawElements", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glEnable", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("GL_COLOR_LOGIC_OP, GL_POINT_SMOOTH, GL_LINE_SMOOTH, GL_POLYGON_SMOOTH, GL_POLYGON_STIPPLE, GL_POLYGON_OFFSET_POINT, GL_POLYGON_OFFSET_LINE and GL_TEXTURE_3D are refused by name (gaps::VALUE_GAPS): gl4es over GLES 2 cannot render them"),
    e("glEnableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).partial("GL_EDGE_FLAG_ARRAY is refused by name (gaps::VALUE_GAPS): a draw does not carry per-vertex edge flags"),
    e("glFinish", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFinishFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glFlushVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glFogf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFogfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFogi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glGenFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glGenTextures", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glGetError", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es).on(),
    e("glGetIntegerv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("GL_CURRENT_RASTER_* is refused by name when the raster position was set with lighting or texture coordinate generation enabled, or by a display list: the lit colour and the generators are not evaluated"),
    e("glGetString", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glIsEnabled", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es).on(),
    e("glLightModelfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLightf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("GL_SPOT_CUTOFF other than 180 is refused by name (gaps::VALUE_GAPS): gl4es never updates its cosine-cutoff uniform"),
    e("glLightfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("GL_SPOT_CUTOFF other than 180 is refused by name (gaps::VALUE_GAPS): gl4es never updates its cosine-cutoff uniform"),
    e("glLoadMatrixf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Gl4es).on(),
    e("glMaterialfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glMatrixMode", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glNormal3fv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glNormalPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glPixelStorei", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glPolygonOffset", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glReadBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("a front buffer is refused by name (gaps::VALUE_GAPS): the drawable is one image; while GL_NONE is the draw buffer a display list that calls glColorMask re-enables writes (the list is gl4es's)"),
    e("glReadPixels", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glScissor", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glSetFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glTexCoordPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glTexEnvi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("GL_TEXTURE_ENV_MODE = GL_BLEND is refused by name (gaps::VALUE_GAPS): gl4es applies the blend twice"),
    e("glTexGeni", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glTexImage2D", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glTexParameteri", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glTexSubImage2D", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glUnlockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Gl4es).on(),
    e("glVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glVertexPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glViewport", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glmSetFunc", Home::Glm, 0b1111, Owner::Adapter),
    e("glmSetMode", Home::Glm, 0b1111, Owner::Adapter),
    e("gluErrorString", Home::Glu, 0b1100, Owner::Adapter),
    e("glDrawRangeElements", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es).on(),
    e("glTexImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glCopyTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glIsFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glFinishObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glVertexArrayParameteriAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b0000, Owner::Adapter).on(),
    // ---- the rest of core GL 1.0 / 1.1 / 1.2 (completeness rows, appended so no existing entry index moves) ----
    e("glAccum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glBegin", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glBitmap", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glCallList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glCallLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glClearAccum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glClearDepth", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glClearIndex", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glClearStencil", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3ub", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3ubv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3ui", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3uiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3us", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor3usv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4ub", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4ubv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4ui", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4uiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4us", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColor4usv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glColorMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glCopyPixels", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_DEPTH copies are refused by name (gaps::VALUE_GAPS): a GLES 2 context cannot read the depth buffer; inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glCullFace", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glDeleteLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("a list deleted while another list that calls it is alive is still run by the caller: gl4es binds a nested list when the caller is compiled"),
    e("glDrawPixels", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_COLOR_INDEX and GL_DEPTH_COMPONENT pixels are refused by name (gaps::VALUE_GAPS): gl4es draws colour pixels only; inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glEdgeFlag", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glEdgeFlagv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glEnd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glEndList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glEvalCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalMesh1", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalMesh2", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalPoint1", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glEvalPoint2", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("evaluation inside a display list being compiled uses the maps in effect at compile time (gl4es records the vertex calls an evaluation makes), where GL evaluates when the list runs"),
    e("glFeedbackBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glFlush", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glFogiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glFrontFace", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glFrustum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGenLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetBooleanv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_CURRENT_RASTER_* is refused by name when the raster position was set with lighting or texture coordinate generation enabled, or by a display list: the lit colour and the generators are not evaluated"),
    e("glGetClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("a plane set inside a display list is not tracked, so it is not what this returns after the list runs"),
    e("glGetDoublev", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_CURRENT_RASTER_* is refused by name when the raster position was set with lighting or texture coordinate generation enabled, or by a display list: the lit colour and the generators are not evaluated"),
    e("glGetFloatv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_CURRENT_RASTER_* is refused by name when the raster position was set with lighting or texture coordinate generation enabled, or by a display list: the lit colour and the generators are not evaluated"),
    e("glGetLightfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetMapdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetMapiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetMaterialfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexImage", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("a 1D texture or a level other than 0 is refused by name (gaps::VALUE_GAPS): gl4es reads only level 0 of 2D textures back"),
    e("glGetTexLevelParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexLevelParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glGetTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glHint", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexi", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexs", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIndexsv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glInitNames", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glIsList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLightModelf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLightModeli", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLightModeliv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLighti", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_SPOT_CUTOFF other than 180 is refused by name (gaps::VALUE_GAPS): gl4es never updates its cosine-cutoff uniform"),
    e("glLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_SPOT_CUTOFF other than 180 is refused by name (gaps::VALUE_GAPS): gl4es never updates its cosine-cutoff uniform"),
    e("glLineStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLineWidth", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glListBase", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLoadIdentity", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLoadMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLoadName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glLogicOp", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMap1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMap1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMap2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMap2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMapGrid1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMapGrid1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMapGrid2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMapGrid2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMaterialf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMateriali", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMultMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glMultMatrixf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNewList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glNormal3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glOrtho", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPassThrough", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glPixelStoref", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPixelTransferf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPixelTransferi", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPixelZoom", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPointSize", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPolygonMode", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glPopAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPopMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPopName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPushAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPushMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glPushName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRasterPos4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRecti", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRects", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRectsv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRenderMode", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_SELECT and GL_FEEDBACK are refused by name (gaps::VALUE_GAPS): the primitives are gl4es's and it has no conforming selection or feedback"),
    e("glRotated", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glRotatef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glScaled", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glScalef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glSelectBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glShadeModel", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_FLAT is refused by name (gaps::VALUE_GAPS): gl4es stores the shade model but its GLES 2 shaders always interpolate"),
    e("glStencilFunc", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glStencilMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glStencilOp", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord1sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexCoord4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexEnvf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_TEXTURE_ENV_MODE = GL_BLEND is refused by name (gaps::VALUE_GAPS): gl4es applies the blend twice"),
    e("glTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_TEXTURE_ENV_MODE = GL_BLEND is refused by name (gaps::VALUE_GAPS): gl4es applies the blend twice"),
    e("glTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).partial("GL_TEXTURE_ENV_MODE = GL_BLEND is refused by name (gaps::VALUE_GAPS): gl4es applies the blend twice"),
    e("glTexGend", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexGenf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexImage1D", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexParameterf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTranslated", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glTranslatef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glVertex4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es).on(),
    e("glAreTexturesResident", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glArrayElement", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glCopyTexImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glCopyTexImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glCopyTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glCopyTexSubImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glDrawArrays", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glEdgeFlagPointer", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glGetPointerv", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glIndexPointer", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glIndexub", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glIndexubv", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glInterleavedArrays", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glIsTexture", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glPopClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glPrioritizeTextures", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).partial("inside a display list being compiled it is refused by name (gaps::IN_LIST_GAPS)"),
    e("glPushClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
    e("glTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es).on(),
];

/// FNV-1a 64 over every name and a NUL, in table order: the identity of the entry numbering.
pub fn entries_digest(entries: &[Entry]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for e in entries {
        for b in e.name.bytes().chain(core::iter::once(0)) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// [`entries_digest`] of [`ENTRIES`] (the numbering the worker's `rgl_entries.h` was generated from).
pub const ENTRIES_DIGEST: u64 = 0x65f5_0765_5386_fb4f;

/// How many core entries a version lists and how many of them the worker executes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Tally {
    pub version: Version,
    pub listed: usize,
    /// Entries the worker runs (some may refuse argument values: see [`Entry::gap`]).
    pub implemented: usize,
    /// Entries a claim may rely on: implemented with no gap. The version waits on THIS number reaching `listed`.
    pub complete: usize,
}

impl Tally {
    /// Listed entries the worker still refuses by name altogether.
    pub const fn refused(&self) -> usize {
        self.listed - self.implemented
    }
}

/// Per-version tally of the core entries IN that version (not cumulative): the number a claim of that version waits on.
pub fn core_tally(entries: &[Entry]) -> [Tally; 3] {
    Version::ALL.map(|v| {
        let own = || entries.iter().filter(move |e| e.home == Home::Core(v));
        Tally { version: v, listed: own().count(), implemented: own().filter(|e| e.implemented).count(), complete: own().filter(|e| e.complete()).count() }
    })
}

/// Look an entry up by its exact name.
pub fn find(entries: &[Entry], name: &str) -> Option<usize> {
    entries.iter().position(|x| x.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeSet;

    #[test]
    fn names_are_unique_and_sized_as_audited() {
        let set: BTreeSet<_> = ENTRIES.iter().map(|x| x.name).collect();
        assert_eq!(set.len(), ENTRIES.len(), "duplicate entry name");
        assert_eq!(ENTRIES.iter().filter(|x| x.imported_by != 0).count(), 73);
    }

    #[test]
    fn every_extension_and_version_has_entries() {
        for x in Ext::ALL {
            assert!(ENTRIES.iter().any(|e| e.home == Home::Ext(x)), "{x:?} has no entry");
        }
        for v in Version::ALL {
            assert!(ENTRIES.iter().any(|e| e.home == Home::Core(v)), "{v:?} has no entry");
        }
    }

    /// The per-version CORE lists are COMPLETE: the Khronos registry (gl.xml, features GL_VERSION_1_0 / 1_1 / 1_2, read
    /// 2026-10-27) lists 306 / 30 / 4 commands, and gl4es (the pinned revision's `gl4es_*` exports, `nm` of the build of
    /// `tools/heirloom-gl4es-build.sh host`) defines every one of them, so every row names `Owner::Gl4es`. GL 1.2's
    /// imaging subset (`glBlendColor`, `glBlendEquation`, ...) is OPTIONAL in 1.2 (spec Appendix / `ARB_imaging`) and is
    /// not part of the three features' core lists, so it is not a hole.
    #[test]
    fn the_core_lists_are_the_registrys() {
        let t = core_tally(ENTRIES);
        assert_eq!([t[0].listed, t[1].listed, t[2].listed], [306, 30, 4]);
        assert!(ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_))).all(|e| e.owner == Owner::Gl4es));
    }

    /// The tally the claim waits on (updated in the same change that switches a row on).
    #[test]
    fn the_tally_is_what_the_implemented_flags_say() {
        let t = core_tally(ENTRIES);
        assert_eq!([t[0].implemented, t[1].implemented, t[2].implemented], [306, 30, 1], "GL 1.2's 3D texture calls are refused by name: gl4es treats a 3D texture as a 2D one");
        assert_eq!([t[0].complete, t[1].complete, t[2].complete], [268, 28, 1], "the rest run with a named gap");
        let mut gaps: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| e.gap.is_some()).map(|e| e.name).collect();
        gaps.sort_unstable();
        assert_eq!(gaps.len(), 40);
        let refused: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_)) && !e.implemented).map(|e| e.name).collect();
        assert_eq!(refused, ["glTexImage3D", "glTexSubImage3D", "glCopyTexSubImage3D"]);
        assert_eq!(t[0].refused() + t[1].refused() + t[2].refused(), refused.len());
        // every gap says what is missing, and every VALUE_GAPS row and IN_LIST_GAPS row is one of them
        assert!(ENTRIES.iter().filter(|e| e.gap.is_some()).all(|e| e.gap.is_some_and(|g| g.len() > 20)));
        for g in crate::gaps::VALUE_GAPS {
            assert!(gaps.contains(&g.call), "{} has a refused value but no gap text", g.call);
        }
        for (c, _) in crate::gaps::IN_LIST_GAPS {
            assert!(gaps.contains(c), "{c} is refused inside a display list but has no gap text");
        }
    }

    /// The set of rows switched on is a reviewed list, not an accident: the hand-written rows below (each has an engine encoder, a worker
    /// replay and a test that ran it end to end), plus EVERY generated family call (`family_table`, one tested mechanism) and every
    /// bulk-data call (`bulk_table`). Adding a hand-written row means adding it here in the same change.
    #[test]
    fn the_implemented_set_is_exactly_the_reviewed_list() {
        let hand = [
            "aglChoosePixelFormat", "aglCreateContext", "aglDestroyContext", "aglDestroyPixelFormat", "aglGetError", "aglSetCurrentContext",
            "aglSetDrawable", "aglSwapBuffers", "glActiveTextureARB", "glAlphaFunc", "glArrayElement", "glBindTexture", "glBitmap", "glBlendFunc",
            "glClear", "glClearColor", "glClientActiveTextureARB", "glColorMaterial", "glColorPointer", "glDeleteFencesAPPLE", "glDeleteTextures",
            "glDepthFunc", "glDepthMask", "glDepthRange", "glDisable", "glDisableClientState", "glDrawArrays", "glDrawElements", "glDrawPixels",
            "glDrawRangeElements", "glEdgeFlagPointer", "glEnable", "glEnableClientState", "glFinish", "glFinishFenceAPPLE", "glFinishObjectAPPLE",
            "glFlushVertexArrayRangeAPPLE", "glFogf", "glFogfv", "glFogi", "glGenFencesAPPLE", "glGenTextures", "glGetError", "glGetPointerv",
            "glGetPolygonStipple", "glGetString", "glGetTexImage", "glIndexPointer", "glInterleavedArrays", "glIsFenceAPPLE", "glLightModelfv",
            "glLightf", "glLightfv", "glLoadMatrixf", "glLockArraysEXT", "glMaterialfv", "glMatrixMode", "glNormal3fv", "glNormalPointer",
            "glPixelStoref", "glPixelStorei", "glPolygonOffset", "glPolygonStipple", "glPopClientAttrib", "glPushClientAttrib", "glReadPixels",
            "glScissor", "glSetFenceAPPLE", "glTestFenceAPPLE", "glTestObjectAPPLE", "glTexCoordPointer", "glTexEnvi", "glTexGeni", "glTexImage1D",
            "glTexImage2D", "glTexParameteri", "glTexSubImage1D", "glTexSubImage2D", "glUnlockArraysEXT", "glVertexArrayParameteriAPPLE",
            "glVertexArrayRangeAPPLE", "glVertexPointer", "glViewport",
        ];
        let mut want: alloc::vec::Vec<&str> = hand.iter().copied().chain(crate::family_table::GENERATED.iter().map(|s| s.name)).chain(crate::bulk_table::BULK.iter().map(|s| s.name)).collect();
        want.sort_unstable();
        let mut have: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| e.implemented).map(|e| e.name).collect();
        have.sort_unstable();
        assert_eq!(have, want);
    }

    /// The entry INDEX is ABI (a Call frame names an entry by it), so a change to the table's order or contents must be a
    /// conscious one: this digest (FNV-1a over the names in order) changes with it, and so must the worker's generated
    /// header (`userland/coral-heirloom-gl/rgl_entries.h`).
    #[test]
    fn the_entry_order_is_pinned() {
        assert_eq!(entries_digest(ENTRIES), ENTRIES_DIGEST);
    }
}
