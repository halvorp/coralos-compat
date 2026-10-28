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
    /// Mesa's desktop OpenGL (compatibility profile, the pinned EGL build) defines it: the worker resolves it by
    /// eglGetProcAddress (`userland/coral-heirloom-gl/src/gl_gen.rs`) and refuses to start when it is missing.
    Mesa,
    /// Not a GL command (AGL is Apple's API; the APPLE_* extensions are the engine's own): the Coral adapter.
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
    e("glActiveTextureARB", Home::Ext(Ext::ArbMultitexture), 0b1111, Owner::Mesa).on(),
    e("glAlphaFunc", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glBindTexture", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glBlendFunc", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glClear", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glClearColor", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glClientActiveTextureARB", Home::Ext(Ext::ArbMultitexture), 0b1111, Owner::Mesa).on(),
    e("glColorMaterial", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glColorPointer", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glDeleteFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glDeleteTextures", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glDepthFunc", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glDepthMask", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glDepthRange", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glDisable", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glDisableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glDrawBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glDrawElements", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glEnable", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glEnableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glFinish", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glFinishFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glFlushVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glFogf", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glFogfv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glFogi", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glGenFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glGenTextures", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glGetError", Home::Core(Version::V1_0), 0b1100, Owner::Mesa).on(),
    e("glGetIntegerv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glGetString", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glIsEnabled", Home::Core(Version::V1_0), 0b1100, Owner::Mesa).on(),
    e("glLightModelfv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glLightf", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glLightfv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glLoadMatrixf", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glLockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Mesa).on(),
    e("glMaterialfv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glMatrixMode", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glNormal3fv", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glNormalPointer", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glPixelStorei", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glPolygonOffset", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glReadBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glReadPixels", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glScissor", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glSetFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glTexCoordPointer", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glTexEnvi", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glTexGeni", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glTexImage2D", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glTexParameteri", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glTexSubImage2D", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glUnlockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Mesa).on(),
    e("glVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glVertexPointer", Home::Core(Version::V1_1), 0b1111, Owner::Mesa).on(),
    e("glViewport", Home::Core(Version::V1_0), 0b1111, Owner::Mesa).on(),
    e("glmSetFunc", Home::Glm, 0b1111, Owner::Adapter),
    e("glmSetMode", Home::Glm, 0b1111, Owner::Adapter),
    e("gluErrorString", Home::Glu, 0b1100, Owner::Adapter),
    e("glDrawRangeElements", Home::Core(Version::V1_2), 0b0000, Owner::Mesa).on(),
    e("glTexImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Mesa).on(),
    e("glTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Mesa).on(),
    e("glCopyTexSubImage3D", Home::Core(Version::V1_2), 0b0000, Owner::Mesa).on(),
    e("glIsFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestFenceAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glTestObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glFinishObjectAPPLE", Home::Ext(Ext::AppleFence), 0b0000, Owner::Adapter).on(),
    e("glVertexArrayParameteriAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b0000, Owner::Adapter).on(),
    // ---- the rest of core GL 1.0 / 1.1 / 1.2 (completeness rows, appended so no existing entry index moves) ----
    e("glAccum", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glBegin", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glBitmap", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glCallList", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glCallLists", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glClearAccum", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glClearDepth", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glClearIndex", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glClearStencil", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3b", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3bv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3ub", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3ubv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3ui", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3uiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3us", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor3usv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4b", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4bv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4ub", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4ubv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4ui", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4uiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4us", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColor4usv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glColorMask", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glCopyPixels", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glCullFace", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glDeleteLists", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glDrawPixels", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEdgeFlag", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEdgeFlagv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEnd", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEndList", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalMesh1", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalMesh2", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalPoint1", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glEvalPoint2", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glFeedbackBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glFlush", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glFogiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glFrontFace", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glFrustum", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGenLists", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetBooleanv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetClipPlane", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetDoublev", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetFloatv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetLightfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetMapdv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetMapiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetMaterialfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexImage", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexLevelParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexLevelParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glGetTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glHint", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexMask", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexd", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexdv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexi", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexs", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIndexsv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glInitNames", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glIsList", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLightModelf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLightModeli", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLightModeliv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLighti", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLightiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLineStipple", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLineWidth", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glListBase", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLoadIdentity", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLoadMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLoadName", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glLogicOp", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMap1d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMap1f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMap2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMap2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMapGrid1d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMapGrid1f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMapGrid2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMapGrid2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMaterialf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMateriali", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMaterialiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMultMatrixd", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glMultMatrixf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNewList", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3b", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3bv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glNormal3sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glOrtho", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPassThrough", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelMapfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelMapuiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelMapusv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelStoref", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelTransferf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelTransferi", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPixelZoom", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPointSize", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPolygonMode", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPolygonStipple", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPopAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPopMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPopName", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPushAttrib", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPushMatrix", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glPushName", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos2sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos3sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRasterPos4sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectd", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectdv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRecti", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectiv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRects", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRectsv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRenderMode", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRotated", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glRotatef", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glScaled", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glScalef", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glSelectBuffer", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glShadeModel", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glStencilFunc", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glStencilMask", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glStencilOp", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord1sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord2sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord3sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexCoord4sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexEnvf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexEnvfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexEnviv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexGend", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexGendv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexGenf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexGenfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexGeniv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexImage1D", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexParameterf", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexParameterfv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTexParameteriv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTranslated", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glTranslatef", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex2sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex3sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4d", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4dv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4f", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4fv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4i", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4iv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4s", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glVertex4sv", Home::Core(Version::V1_0), 0b0000, Owner::Mesa).on(),
    e("glAreTexturesResident", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glArrayElement", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glCopyTexImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glCopyTexImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glCopyTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glCopyTexSubImage2D", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glDrawArrays", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glEdgeFlagPointer", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glGetPointerv", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glIndexPointer", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glIndexub", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glIndexubv", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glInterleavedArrays", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glIsTexture", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glPopClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glPrioritizeTextures", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glPushClientAttrib", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
    e("glTexSubImage1D", Home::Core(Version::V1_1), 0b0000, Owner::Mesa).on(),
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
    /// 2026-10-27) lists 306 / 30 / 4 commands, and Mesa's compatibility profile defines every one of them (the worker resolves all 340 by
    /// eglGetProcAddress on QEMU virgl and the Z83's iris, 2026-10-28), so every row names `Owner::Mesa`. GL 1.2's
    /// imaging subset (`glBlendColor`, `glBlendEquation`, ...) is OPTIONAL in 1.2 (spec Appendix / `ARB_imaging`) and is
    /// not part of the three features' core lists, so it is not a hole.
    #[test]
    fn the_core_lists_are_the_registrys() {
        let t = core_tally(ENTRIES);
        assert_eq!([t[0].listed, t[1].listed, t[2].listed], [306, 30, 4]);
        assert!(ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_))).all(|e| e.owner == Owner::Mesa));
    }

    /// The tally the claim waits on (updated in the same change that switches a row on).
    #[test]
    fn the_tally_is_what_the_implemented_flags_say() {
        let t = core_tally(ENTRIES);
        assert_eq!([t[0].implemented, t[1].implemented, t[2].implemented], [306, 30, 4]);
        assert_eq!([t[0].complete, t[1].complete, t[2].complete], [306, 30, 4], "every GL 1.0-1.2 command runs on Mesa with nothing refused");
        let gaps: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| e.gap.is_some()).map(|e| e.name).collect();
        assert!(gaps.is_empty(), "{gaps:?}");
        let refused: alloc::vec::Vec<&str> = ENTRIES.iter().filter(|e| matches!(e.home, Home::Core(_)) && !e.implemented).map(|e| e.name).collect();
        assert!(refused.is_empty(), "{refused:?}");
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
            "glTexImage2D", "glTexImage3D", "glTexParameteri", "glTexSubImage1D", "glTexSubImage2D", "glTexSubImage3D", "glUnlockArraysEXT", "glVertexArrayParameteriAPPLE",
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
