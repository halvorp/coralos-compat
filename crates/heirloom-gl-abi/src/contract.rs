//! THE CONTRACT AS DATA: every GL/AGL entry point the four Warcraft III binaries import, plus the entries that make
//! each version/extension claim complete. Imports were read from the PEF import tables of
//! `roc-Warcraft_III`, `roc-Warcraft_III_World_Editor`, `tft-Warcraft_III_TFT` and `tft-Warcraft_III_World_Editor_TFT`
//! (libraries OpenGLLibrary, OpenGLUtility, OpenGLMemory; 73 distinct names) with `retro-pef` on 2026-10-25.
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
    /// True only when the worker executes this entry end to end.
    pub implemented: bool,
}

impl Entry {
    /// The same entry, executed end to end by the worker (the only way a row is switched on).
    pub const fn on(self) -> Entry {
        Entry { implemented: true, ..self }
    }
}

const fn e(name: &'static str, home: Home, imported_by: u8, owner: Owner) -> Entry {
    Entry { name, home, imported_by, owner, implemented: false }
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
    e("glAlphaFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glBindTexture", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es),
    e("glBlendFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glClear", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glClearColor", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glClientActiveTextureARB", Home::Ext(Ext::ArbMultitexture), 0b1111, Owner::Gl4es).on(),
    e("glColorMaterial", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glColorPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glDeleteFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glDeleteTextures", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es),
    e("glDepthFunc", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glDepthMask", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glDepthRange", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glDisable", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glDisableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glDrawBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glDrawElements", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glEnable", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glEnableClientState", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glFinish", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glFinishFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glFlushVertexArrayRangeAPPLE", Home::Ext(Ext::AppleVertexArrayRange), 0b1100, Owner::Adapter).on(),
    e("glFogf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glFogfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glFogi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glGenFencesAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glGenTextures", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es),
    e("glGetError", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es).on(),
    e("glGetIntegerv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glGetString", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glIsEnabled", Home::Core(Version::V1_0), 0b1100, Owner::Gl4es),
    e("glLightModelfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glLightf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glLightfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glLoadMatrixf", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glLockArraysEXT", Home::Ext(Ext::ExtCompiledVertexArray), 0b1111, Owner::Gl4es).on(),
    e("glMaterialfv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glMatrixMode", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es).on(),
    e("glNormal3fv", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glNormalPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glPixelStorei", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glPolygonOffset", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es),
    e("glReadBuffer", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glReadPixels", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glScissor", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glSetFenceAPPLE", Home::Ext(Ext::AppleFence), 0b1100, Owner::Adapter).on(),
    e("glTexCoordPointer", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es).on(),
    e("glTexEnvi", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glTexGeni", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glTexImage2D", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glTexParameteri", Home::Core(Version::V1_0), 0b1111, Owner::Gl4es),
    e("glTexSubImage2D", Home::Core(Version::V1_1), 0b1111, Owner::Gl4es),
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
pub const ENTRIES_DIGEST: u64 = 0xaf01_6f3d_89ce_d63e;

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

    /// The set of rows switched on is a reviewed list, not an accident: each name here has an engine encoder, a worker
    /// replay and a test that ran it end to end (docs: PHASE1-REPORT). Adding a row means adding it here in the same change.
    #[test]
    fn the_implemented_set_is_exactly_the_reviewed_list() {
        let want = [
            "aglChoosePixelFormat", "aglCreateContext", "aglDestroyContext", "aglDestroyPixelFormat", "aglGetError", "aglSetCurrentContext",
            "aglSetDrawable", "aglSwapBuffers", "glActiveTextureARB", "glClear", "glClientActiveTextureARB", "glColorPointer",
            "glDeleteFencesAPPLE", "glDisable", "glDisableClientState", "glDrawElements", "glEnable", "glEnableClientState",
            "glFinish", "glFinishFenceAPPLE", "glFinishObjectAPPLE", "glFlushVertexArrayRangeAPPLE", "glGenFencesAPPLE", "glGetError",
            "glGetString", "glIsFenceAPPLE", "glLoadMatrixf", "glLockArraysEXT", "glMatrixMode", "glNormalPointer", "glSetFenceAPPLE",
            "glTestFenceAPPLE", "glTestObjectAPPLE", "glTexCoordPointer", "glUnlockArraysEXT", "glVertexArrayParameteriAPPLE",
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
