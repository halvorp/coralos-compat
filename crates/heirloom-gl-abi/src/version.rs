//! The GL version ladder and the extensions this product may ever advertise.

/// A desktop-GL core version. The ladder ends at 1.2: that is what the games ask for (ruling 22).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    V1_0,
    V1_1,
    V1_2,
}

impl Version {
    pub const ALL: [Version; 3] = [Version::V1_0, Version::V1_1, Version::V1_2];
    /// `(major, minor)`.
    pub const fn pair(self) -> (u8, u8) {
        match self {
            Version::V1_0 => (1, 0),
            Version::V1_1 => (1, 1),
            Version::V1_2 => (1, 2),
        }
    }
}

/// An extension that can appear in `GL_EXTENSIONS`. The set is closed: a string not listed here cannot be advertised.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ext {
    ArbMultitexture,
    ExtCompiledVertexArray,
    AppleFence,
    AppleVertexArrayRange,
}

impl Ext {
    pub const ALL: [Ext; 4] = [Ext::ArbMultitexture, Ext::ExtCompiledVertexArray, Ext::AppleFence, Ext::AppleVertexArrayRange];
    /// The exact token a guest compares against.
    pub const fn name(self) -> &'static str {
        match self {
            Ext::ArbMultitexture => "GL_ARB_multitexture",
            Ext::ExtCompiledVertexArray => "GL_EXT_compiled_vertex_array",
            Ext::AppleFence => "GL_APPLE_fence",
            Ext::AppleVertexArrayRange => "GL_APPLE_vertex_array_range",
        }
    }
}

/// Where an entry point belongs: the capability that must be fully implemented before it may be claimed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Home {
    /// Part of core GL at this version.
    Core(Version),
    /// Part of this extension.
    Ext(Ext),
    /// Apple GL (AGL) context/drawable/renderer API.
    Agl,
    /// `glm*` memory hooks (OpenGLMemory).
    Glm,
    /// GLU.
    Glu,
}
