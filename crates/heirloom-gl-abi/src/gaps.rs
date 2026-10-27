//! ARGUMENT VALUES a callable entry refuses by name, because the stack under the worker (gl4es over GLES 2, unmodified) cannot
//! do what GL 1.2 says the value does. Each row is a [`crate::contract::Entry::gap`]: the entry runs for every other value, a
//! version or extension that lists it is NOT claimable, and the value is refused where it is first seen (the engine stops the
//! program with the marker; the worker refuses the frame) - never silently rendered as something else.
use alloc::format;
use alloc::string::String;

/// One refused argument value: `refuses` judges the call's words.
#[derive(Copy, Clone)]
pub struct ValueGap {
    pub call: &'static str,
    /// When `Some(v)`: the row refuses exactly the calls whose first word is `v` (a capability, a client-state name).
    pub eq0: Option<u64>,
    pub refuses: fn(&[u64]) -> bool,
    pub why: &'static str,
}

fn never(_: &[u64]) -> bool {
    false
}

const fn gap(call: &'static str, refuses: fn(&[u64]) -> bool, why: &'static str) -> ValueGap {
    ValueGap { call, eq0: None, refuses, why }
}

const fn capability(cap: u64, why: &'static str) -> ValueGap {
    ValueGap { call: "glEnable", eq0: Some(cap), refuses: never, why }
}

fn is_flat(w: &[u64]) -> bool {
    w.first() == Some(&0x1D00)
}

const GL_SPOT_CUTOFF: u64 = 0x1206;

/// `glLight*(light, GL_SPOT_CUTOFF, value)`: refused unless the value is 180 (no spotlight).
fn spot_cutoff_float(w: &[u64]) -> bool {
    w.get(1) == Some(&GL_SPOT_CUTOFF) && w.get(2).is_some_and(|v| f32::from_bits(*v as u32) != 180.0)
}

fn spot_cutoff_int(w: &[u64]) -> bool {
    w.get(1) == Some(&GL_SPOT_CUTOFF) && w.get(2).is_some_and(|v| *v as u32 as i32 != 180)
}

fn draws_front(w: &[u64]) -> bool {
    matches!(w.first(), Some(0x0400 | 0x0404 | 0x0406 | 0x0408))
}

fn render_mode_not_render(w: &[u64]) -> bool {
    matches!(w.first(), Some(0x1C01 | 0x1C02))
}

fn pixels_without_a_gl4es_path(w: &[u64]) -> bool {
    matches!(w.first(), Some(0x1900 | 0x1902))
}

fn copies_depth(w: &[u64]) -> bool {
    w.get(4) == Some(&0x1801)
}

fn get_tex_image_unserved(w: &[u64]) -> bool {
    w.first() == Some(&0x0DE0) || w.get(1).is_some_and(|l| *l != 0)
}

const GL_TEXTURE_ENV_MODE: u64 = 0x2200;
const GL_BLEND: u64 = 0x0BE2;

/// `glTexEnv*(GL_TEXTURE_ENV, GL_TEXTURE_ENV_MODE, GL_BLEND)`.
fn tex_env_blend_int(w: &[u64]) -> bool {
    w.get(1) == Some(&GL_TEXTURE_ENV_MODE) && w.get(2) == Some(&GL_BLEND)
}

fn tex_env_blend_float(w: &[u64]) -> bool {
    w.get(1) == Some(&GL_TEXTURE_ENV_MODE) && w.get(2).is_some_and(|v| f32::from_bits(*v as u32) == GL_BLEND as f32)
}

const BLEND: &str = "GL_TEXTURE_ENV_MODE = GL_BLEND: gl4es's fragment shader applies the blend twice (fpe_shader.c appends the mix statement a second time)";

fn enables_edge_flag_array(w: &[u64]) -> bool {
    w.first() == Some(&0x8079)
}

const SPOT: &str = "GL_SPOT_CUTOFF other than 180: gl4es never updates its cosine-cutoff uniform (its inverted memcmp in fpe.c), so no cone is cut";

/// Every refused argument value. A row's call must be a `partial` entry in the contract (a test pins this).
pub static VALUE_GAPS: &[ValueGap] = &[
    // GL 1.2 §2.13.7: flat shading takes the colour of the provoking vertex. gl4es stores the shade model and its generated
    // GLES 2 shaders always interpolate (GLSL ES 1.00 has no `flat`), so GL_FLAT would render smooth.
    gap("glShadeModel", is_flat, "GL_FLAT: gl4es's GLES2 shaders always interpolate colours"),
    // GL 1.2 §2.13.3: a spot cutoff of 0..90 degrees limits the light to a cone.
    gap("glLightf", spot_cutoff_float, SPOT),
    gap("glLightfv", spot_cutoff_float, SPOT),
    gap("glLighti", spot_cutoff_int, SPOT),
    gap("glLightiv", spot_cutoff_int, SPOT),
    // Capabilities whose behaviour gl4es over GLES 2 cannot render: enabling one is refused, so it is never "on" without effect.
    capability(0x0BF2, "GL_COLOR_LOGIC_OP: GLES 2 has no logic operation and gl4es does not emulate one (GL 1.2 §4.1.10)"),
    capability(0x0B10, "GL_POINT_SMOOTH: gl4es draws aliased points (GL 1.2 §3.3.1)"),
    capability(0x0B20, "GL_LINE_SMOOTH: gl4es draws aliased lines (GL 1.2 §3.4.2)"),
    capability(0x0B41, "GL_POLYGON_SMOOTH: gl4es draws aliased polygons (GL 1.2 §3.5.5)"),
    capability(0x0B42, "GL_POLYGON_STIPPLE: gl4es keeps no stipple mask and GLES 2 has no fragment stipple (GL 1.2 §3.5.2)"),
    capability(0x2A01, "GL_POLYGON_OFFSET_POINT: GLES 2 offsets filled polygons only (GL 1.2 §3.5.5)"),
    capability(0x2A02, "GL_POLYGON_OFFSET_LINE: GLES 2 offsets filled polygons only (GL 1.2 §3.5.5)"),
    capability(0x806F, "GL_TEXTURE_3D: gl4es treats a 3D texture as a 2D one (GL 1.2 §3.8.2)"),
    // GL 1.2 §2.8: an enabled edge-flag array gives each vertex of a draw its edge flag. A draw carries vertex, normal, colour and texture
    // coordinates only, so an enabled edge-flag array would be ignored.
    gap("glEnableClientState", enables_edge_flag_array, "GL_EDGE_FLAG_ARRAY: a draw does not carry per-vertex edge flags"),
    // GL 1.2 §4.2.1 / §4.3.2: the visual is double-buffered but has ONE colour image (the back buffer the program draws and reads);
    // there is no separate front image, so the front, left and both-buffer selections are refused rather than aliased to the back.
    gap("glDrawBuffer", draws_front, "a front buffer: the drawable is one image, there is no separate front buffer (GL_FRONT, GL_FRONT_LEFT, GL_LEFT, GL_FRONT_AND_BACK)"),
    gap("glReadBuffer", draws_front, "a front buffer: the drawable is one image, there is no separate front buffer (GL_FRONT, GL_FRONT_LEFT, GL_LEFT)"),
    // GL 1.2 §5.2, §5.3: selection and feedback transform and clip the primitives a program draws and report them instead of rasterising
    // them. The primitives are gl4es's (immediate mode, display lists, arrays): this worker has no vertex pipeline of its own to
    // report from, and gl4es's selection is not the specification's (it clips in x and y only and scales depth to INT_MAX).
    gap("glRenderMode", render_mode_not_render, "GL_SELECT / GL_FEEDBACK: the primitives are gl4es's and it has no conforming selection or feedback"),
    // GL 1.2 §3.6.4 / §4.3.1: GL_COLOR_INDEX pixels in RGBA mode go through the index-to-colour maps, GL_DEPTH_COMPONENT pixels write the depth
    // buffer; gl4es has neither (a GLES 2 context cannot write depth from client memory), so they are refused.
    gap("glDrawPixels", pixels_without_a_gl4es_path, "GL_COLOR_INDEX / GL_DEPTH_COMPONENT pixels: gl4es draws colour pixels only"),
    // GL 1.2 §4.3.3: a depth copy needs to read the depth buffer, which a GLES 2 context cannot do.
    gap("glCopyPixels", copies_depth, "GL_DEPTH: a GLES 2 context cannot read the depth buffer"),
    // GL 1.2 §6.1.4: glGetTexImage reads back a texture's stored level. gl4es reads only GL_TEXTURE_2D, and for a level other than 0 it
    // returns a successive halving of level 0 rather than the level the program uploaded.
    gap("glGetTexImage", get_tex_image_unserved, "a 1D texture or a level other than 0: gl4es reads only level 0 of 2D textures back"),
    // GL 1.2 §3.8.9 Table 3.8: C = C_f (1 - C_t) + C_c C_t.
    gap("glTexEnvi", tex_env_blend_int, BLEND),
    gap("glTexEnviv", tex_env_blend_int, BLEND),
    gap("glTexEnvf", tex_env_blend_float, BLEND),
    gap("glTexEnvfv", tex_env_blend_float, BLEND),
];

/// Calls whose effect is state or drawing this worker keeps itself, which gl4es's display lists cannot record: while a list is being
/// compiled (between `glNewList` and `glEndList`) they are refused by name rather than applied at once.
pub static IN_LIST_GAPS: &[(&str, &str)] = &[
    ("glDrawPixels", "the pixels are drawn by this worker, which a gl4es display list does not record"),
    ("glCopyPixels", "the pixels are copied and drawn by this worker, which a gl4es display list does not record"),
    ("glPixelMapfv", "pixel maps are this worker's state, which a gl4es display list does not record"),
    ("glPixelMapuiv", "pixel maps are this worker's state, which a gl4es display list does not record"),
    ("glPixelMapusv", "pixel maps are this worker's state, which a gl4es display list does not record"),
    ("glPrioritizeTextures", "texture priorities are this worker's state, which a gl4es display list does not record"),
    ("glPolygonStipple", "the stipple mask is this worker's state, which a gl4es display list does not record"),
];

/// The named refusal of `name` called while a display list is being compiled, or `None`.
pub fn refused_in_list(name: &str) -> Option<String> {
    IN_LIST_GAPS.iter().find(|(c, _)| *c == name).map(|(_, why)| format!("[HEIRLOOM-GL REFUSED call={name} reason=inside-a-display-list: {why}]"))
}

/// The named refusal for `args` of `name`, or `None` when the call is served.
pub fn refused_value(name: &str, args: &[u64]) -> Option<String> {
    VALUE_GAPS.iter().find(|g| g.call == name && g.eq0.map_or_else(|| (g.refuses)(args), |v| args.first() == Some(&v))).map(|g| format!("[HEIRLOOM-GL REFUSED call={name} reason={}]", g.why))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_refused_value_belongs_to_a_callable_entry_that_is_not_complete() {
        for g in VALUE_GAPS {
            let e = crate::contract::find(crate::contract::ENTRIES, g.call).map(|i| crate::contract::ENTRIES[i]).expect("a contract entry");
            assert!(e.implemented && !e.complete(), "{}: a value gap makes the entry callable but never complete", g.call);
        }
    }

    #[test]
    fn a_call_the_worker_keeps_itself_is_refused_inside_a_display_list_and_its_entry_is_not_complete() {
        assert!(refused_in_list("glDrawPixels").unwrap().contains("inside-a-display-list"));
        assert_eq!(refused_in_list("glBitmap"), None);
        for (c, _) in IN_LIST_GAPS {
            let e = crate::contract::find(crate::contract::ENTRIES, c).map(|i| crate::contract::ENTRIES[i]).expect("a contract entry");
            assert!(e.implemented && !e.complete(), "{c}");
        }
    }

    #[test]
    fn a_refused_value_is_named_and_every_other_value_is_served() {
        assert_eq!(
            refused_value("glShadeModel", &[0x1D00]).as_deref(),
            Some("[HEIRLOOM-GL REFUSED call=glShadeModel reason=GL_FLAT: gl4es's GLES2 shaders always interpolate colours]")
        );
        let f = |v: f32| u64::from(v.to_bits());
        assert!(refused_value("glLightf", &[0x4000, 0x1206, f(10.0)]).is_some());
        assert!(refused_value("glLightf", &[0x4000, 0x1206, f(180.0)]).is_none());
        assert!(refused_value("glLightf", &[0x4000, 0x1205, f(10.0)]).is_none(), "only the cutoff is refused");
        assert!(refused_value("glLighti", &[0x4000, 0x1206, 45]).is_some() && refused_value("glLighti", &[0x4000, 0x1206, 180]).is_none());
        assert!(refused_value("glLightiv", &[0x4000, 0x1206, 45]).is_some());
        assert_eq!(refused_value("glShadeModel", &[0x1D01]), None);
        assert_eq!(refused_value("glShadeModel", &[]), None);
        assert_eq!(refused_value("glCullFace", &[0x1D00]), None);
    }
}
