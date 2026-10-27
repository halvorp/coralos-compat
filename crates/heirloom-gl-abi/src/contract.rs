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
    e("glDrawBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es), // gl4es defines it as a no-op STUB (glstub.c): refused until the worker serves the back/front/none buffers itself
    e("glDrawElements", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glEnable", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glEnableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glFinish", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFinishFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glFlushVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glFogf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFogfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFogi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glGenFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glGenTextures", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glGetError", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es).on(),
    e("glGetIntegerv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).partial("62 of the 249 GL 1.0-1.2 state queries are refused by name (sig::INTEGER_PNAMES_UNANSWERED): gl4es keeps no such state (index mode, accumulation, evaluators, selection and feedback, client-array details)"),
    e("glGetString", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glIsEnabled", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es).partial("6 capabilities are refused by name (sig::CAPABILITIES_UNANSWERED): GL_INDEX_LOGIC_OP, GL_POLYGON_OFFSET_POINT, GL_POLYGON_OFFSET_LINE, GL_POLYGON_SMOOTH, GL_INDEX_ARRAY, GL_EDGE_FLAG_ARRAY"),
    e("glLightModelfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLightf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLightfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLoadMatrixf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Gl4es).on(),
    e("glMaterialfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glMatrixMode", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glNormal3fv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glNormalPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glPixelStorei", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glPolygonOffset", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glReadBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es), // gl4es defines it as a no-op STUB (glstub.c): refused until the worker serves the back/front/none buffers itself
    e("glReadPixels", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glScissor", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glSetFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glTexCoordPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glTexEnvi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
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
    e("glDrawRangeElements", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glTexImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glCopyTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Gl4es),
    e("glIsFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glFinishObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glVertexArrayParameteriAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b0000, Owner::Adapter).on(),
    // ---- the rest of core GL 1.0 / 1.1 / 1.2 (completeness rows, appended so no existing entry index moves) ----
    e("glAccum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glBegin", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glBitmap", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glCallList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glCallLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glClearAccum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glClearDepth", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glClearIndex", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glClearStencil", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3ub", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3ubv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3ui", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3uiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3us", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor3usv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4ub", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4ubv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4ui", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4uiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4us", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColor4usv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glColorMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glCopyPixels", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glCullFace", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glDeleteLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glDrawPixels", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEdgeFlag", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEdgeFlagv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEnd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEndList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalMesh1", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalMesh2", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalPoint1", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glEvalPoint2", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glFeedbackBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glFlush", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glFogiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glFrontFace", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glFrustum", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGenLists", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetBooleanv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetDoublev", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetFloatv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetLightfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetMapdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetMapiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetMaterialfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexImage", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexLevelParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexLevelParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glGetTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glHint", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexi", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexs", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIndexsv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glInitNames", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glIsList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLightModelf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLightModeli", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLightModeliv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLighti", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLineStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLineWidth", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glListBase", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLoadIdentity", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLoadMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLoadName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glLogicOp", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMap1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMap1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMap2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMap2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMapGrid1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMapGrid1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMapGrid2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMapGrid2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMaterialf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMateriali", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMultMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glMultMatrixf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNewList", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3b", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3bv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glNormal3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glOrtho", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPassThrough", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelStoref", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelTransferf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelTransferi", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPixelZoom", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPointSize", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPolygonMode", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPopAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPopMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPopName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPushAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPushMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glPushName", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRasterPos4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectd", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectdv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRecti", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectiv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRects", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRectsv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRenderMode", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRotated", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glRotatef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glScaled", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glScalef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glSelectBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glShadeModel", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glStencilFunc", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glStencilMask", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glStencilOp", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord1sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexCoord4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexEnvf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexGend", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexGenf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexImage1D", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexParameterf", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTranslated", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glTranslatef", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex2sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex3sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4d", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4dv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4f", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4fv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4i", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4iv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4s", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glVertex4sv", Home::Core(Version::V1_0), 0b0000, Owner::Gl4es),
    e("glAreTexturesResident", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glArrayElement", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glCopyTexImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glCopyTexImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glCopyTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glCopyTexSubImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glDrawArrays", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glEdgeFlagPointer", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glGetPointerv", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glIndexPointer", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glIndexub", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glIndexubv", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glInterleavedArrays", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glIsTexture", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glPopClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glPrioritizeTextures", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glPushClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
    e("glTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Gl4es),
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
        assert_eq!([t[0].implemented, t[1].implemented, t[2].implemented], [33, 12, 0]);
        assert_eq!([t[0].complete, t[1].complete, t[2].complete], [31, 12, 0], "glGetIntegerv and glIsEnabled run but have gaps");
        let gaps: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| e.gap.is_some()).map(|e| e.name).collect();
        assert_eq!(gaps, ["glGetIntegerv", "glIsEnabled"]);
        assert_eq!(t[0].refused() + t[1].refused() + t[2].refused(), 340 - (t[0].implemented + t[1].implemented + t[2].implemented));
    }

    /// The set of rows switched on is a reviewed list, not an accident: each name here has an engine encoder, a worker
    /// replay and a test that ran it end to end (docs: PHASE1-REPORT). Adding a row means adding it here in the same change.
    #[test]
    fn the_implemented_set_is_exactly_the_reviewed_list() {
        let want = [
            "aglChoosePixelFormat", "aglCreateContext", "aglDestroyContext", "aglDestroyPixelFormat", "aglGetError",
            "aglSetCurrentContext", "aglSetDrawable", "aglSwapBuffers", "glActiveTextureARB", "glAlphaFunc", "glBindTexture",
            "glBlendFunc", "glClear", "glClearColor", "glClientActiveTextureARB", "glColorMaterial", "glColorPointer",
            "glDeleteFencesAPPLE", "glDeleteTextures", "glDepthFunc", "glDepthMask", "glDepthRange", "glDisable",
            "glDisableClientState", "glDrawElements", "glEnable", "glEnableClientState", "glFinish",
            "glFinishFenceAPPLE", "glFinishObjectAPPLE", "glFlushVertexArrayRangeAPPLE", "glFogf", "glFogfv", "glFogi",
            "glGenFencesAPPLE", "glGenTextures", "glGetError", "glGetIntegerv", "glGetString", "glIsEnabled", "glIsFenceAPPLE",
            "glLightModelfv", "glLightf", "glLightfv", "glLoadMatrixf", "glLockArraysEXT", "glMaterialfv", "glMatrixMode",
            "glNormal3fv", "glNormalPointer", "glPixelStorei", "glPolygonOffset", "glReadPixels", "glScissor",
            "glSetFenceAPPLE", "glTestFenceAPPLE", "glTestObjectAPPLE", "glTexCoordPointer", "glTexEnvi", "glTexGeni",
            "glTexImage2D", "glTexParameteri", "glTexSubImage2D", "glUnlockArraysEXT", "glVertexArrayParameteriAPPLE",
            "glVertexArrayRangeAPPLE", "glVertexPointer", "glViewport",
        ];
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
