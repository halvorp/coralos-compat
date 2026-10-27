#!/usr/bin/env python3
"""Generate the GL call-family tables from the Khronos registry.

  python3 -I tools/gen-gl-sigs.py <gl.xml> <family_table.rs> [<worker gl4es_gen.rs>]

Reads the commands of OpenGL 1.0, 1.1 and 1.2 (the `<feature>` elements `GL_VERSION_1_0` .. `GL_VERSION_1_2`), keeps the ones
that are REGULAR (every parameter a scalar, a constant-length input vector, a pname-sized input vector or a pname-sized
output vector: no pixel images, no client arrays, no evaluator point grids) and are not one of the hand-written entries below,
and writes
  * `crates/heirloom-gl-abi/src/family_table.rs`: the data `heirloom_gl_abi::family` interprets (the engine encodes from it,
    the worker decodes with it), and
  * (optional) the worker's `gl4es_gen.rs`: one typed call into gl4es per entry. That file belongs to the coralos tree's
    `userland/coral-heirloom-gl`; it is the same table, so the two halves cannot disagree about a parameter's type.
The registry's file is not stored here: its SHA-256 is written into the output, and `--check` re-derives and compares.
"""
import hashlib
import re
import sys
import xml.etree.ElementTree as ET

# Entries with a hand-written encoder in `gl_c.rs` / `gl_c/state.rs` (they predate the generator).
HAND = set("""glAlphaFunc glBindTexture glBlendFunc glClear glClearColor glColorMaterial glDepthFunc glDepthMask glDepthRange glDisable
glEnable glFinish glFogf glFogfv glFogi glGenTextures glGetError glGetString glLightModelfv glLightf
glLightfv glLoadMatrixf glMaterialfv glMatrixMode glNormal3fv glPixelStorei glPolygonOffset glReadPixels glScissor glTexEnvi
glTexGeni glTexImage2D glTexParameteri glTexSubImage2D glViewport glDeleteTextures glColorPointer glDrawElements
glEnableClientState glDisableClientState glNormalPointer glTexCoordPointer glVertexPointer""".split())
# gl4es defines these as no-op stubs and they carry bulk data or need real work: hand-written families. (The scalar stubs
# glAccum, glClearAccum, glClearIndex, glIndexf, glIndexMask, glPassThrough, glDrawBuffer, glReadBuffer are generated: the engine
# and the worker decide each from the specification.)
STUBS = set("""glFeedbackBuffer glGetPolygonStipple glPolygonStipple glPrioritizeTextures""".split())
# Calls whose semantics live partly in the engine (client state, pixel store) or need bulk data: hand-written families.
BESPOKE = set("""glPixelStoref glPushClientAttrib glPopClientAttrib glArrayElement glDrawArrays glDrawRangeElements glGetPointerv
glInterleavedArrays glIndexPointer glEdgeFlagPointer glRenderMode glCopyTexSubImage3D""".split())

TY = {"GLenum": "Enum", "GLint": "Int", "GLuint": "Uint", "GLsizei": "Sizei", "GLshort": "Short", "GLushort": "Ushort",
      "GLbyte": "Byte", "GLubyte": "Ubyte", "GLboolean": "Bool", "GLbitfield": "Bitfield", "GLfloat": "Float", "GLclampf": "Float",
      "GLdouble": "Double", "GLclampd": "Double"}
CT = {"Enum": "u32", "Uint": "u32", "Bitfield": "u32", "Int": "i32", "Sizei": "i32", "Short": "i16", "Ushort": "u16", "Byte": "i8",
      "Ubyte": "u8", "Bool": "u8", "Float": "f32", "Double": "f64"}
RET = {"void": "Void", "GLboolean": "Bool", "GLuint": "Uint", "GLint": "Int"}


def text(e, skip):
    parts = [e.text or ""]
    for ch in e:
        if ch.tag == "ptype":
            parts.append(ch.text or "")
        elif ch.tag != skip:
            parts.append("".join(ch.itertext()))
        parts.append(ch.tail or "")
    return " ".join(x.strip() for x in parts if x.strip())


def vec_kind(name, getter):
    for key, kind in (("Fog", "Fog"), ("LightModel", "LightModel"), ("Light", "Light"), ("Material", "MaterialGet" if getter else "Material"),
                      ("TexEnv", "TexEnv"), ("TexGen", "TexGen"), ("TexLevelParameter", "TexLevel"),
                      ("TexParameter", "TexParamGet" if getter else "TexParam")):
        if key in name:
            return kind
    if name in ("glGetBooleanv", "glGetDoublev", "glGetFloatv", "glGetIntegerv"):
        return "State"
    return None


def classify(name, params):
    """The parameter list of a regular call, or None when the call is not regular."""
    getter = name.startswith("glGet")
    out = []
    for decl, pname, ln in params:
        m = re.fullmatch(r"(const )?(GL\w+) ?(\*)?", decl.strip())
        if not m or m.group(2) not in TY:
            return None
        const, ty, ptr = m.group(1), TY[m.group(2)], m.group(3)
        if not ptr:
            out.append(f"P::S({ty})")
        elif ln and ln.isdigit() and const:
            out.append(f"P::V({ty}, {ln})")
        elif ln and ln.isdigit() and getter and not const:
            if name != "glGetClipPlane":
                return None
            out.append(f"P::Out({ty}, VecKind::Four)")
        elif ln and ln.startswith("COMPSIZE(pname)"):
            kind = vec_kind(name, getter)
            if kind is None:
                return None
            out.append(f"P::{'In' if const else 'Out'}({ty}, VecKind::{kind})")
        else:
            return None
    return out


def main():
    xml_path, table_path = sys.argv[1], sys.argv[2]
    worker_path = sys.argv[3] if len(sys.argv) > 3 else None
    raw = open(xml_path, "rb").read()
    root = ET.fromstring(raw)
    cmds = {}
    for c in root.find("commands"):
        proto = c.find("proto")
        name = proto.find("name").text
        cmds[name] = (text(proto, "name"), [(text(q, "name"), q.find("name").text, q.get("len")) for q in c.findall("param")])
    core = []
    for f in root.findall("feature"):
        if f.get("name") in ("GL_VERSION_1_0", "GL_VERSION_1_1", "GL_VERSION_1_2"):
            for r in f.findall("require"):
                core += [x.get("name") for x in r.findall("command")]
    rows, skipped = [], []
    for name in core:
        if name in HAND or name in STUBS or name in BESPOKE:
            continue
        ret, params = cmds[name]
        sig = classify(name, params)
        if sig is None or ret not in RET:
            skipped.append(name)
            continue
        rows.append((name, RET[ret], sig))
    digest = hashlib.sha256(raw).hexdigest()
    head = ("// GENERATED by tools/gen-gl-sigs.py from the Khronos registry gl.xml (sha256 {d}): do not edit.\n"
            "// {n} regular commands of OpenGL 1.0-1.2; hand-written entries and the bespoke families are listed in the generator.\n").format(d=digest, n=len(rows))
    with open(table_path, "w") as o:
        o.write(head)
        o.write("use crate::family::{Sig, Ty::*, P, R};\nuse crate::sig::VecKind;\n\n")
        o.write("pub static GENERATED: &[Sig] = &[\n")
        for name, ret, sig in rows:
            o.write(f'    Sig::new("{name}", R::{ret}, &[{", ".join(sig)}]),\n')
        o.write("];\n\n")
        o.write("/// The registry file this table was derived from.\npub const REGISTRY_SHA256: &str = \"%s\";\n" % digest)
    if worker_path:
        write_worker(worker_path, rows, digest)
    print(f"{len(rows)} generated; bespoke ({len(skipped)}): {' '.join(skipped)}", file=sys.stderr)


def conv(ty, expr):
    return {"Enum": f"{expr} as u32", "Uint": f"{expr} as u32", "Bitfield": f"{expr} as u32", "Int": f"{expr} as u32 as i32",
            "Sizei": f"{expr} as u32 as i32", "Short": f"{expr} as u32 as i32 as i16", "Ushort": f"{expr} as u16",
            "Byte": f"{expr} as u32 as i32 as i8", "Ubyte": f"{expr} as u8", "Bool": f"u8::from({expr} != 0)",
            "Float": f"f32::from_bits({expr} as u32)", "Double": f"f64::from_bits({expr})"}[ty]


def to_word(ty, expr):
    return {"Float": f"u64::from({expr}.to_bits())", "Double": f"{expr}.to_bits()", "Bool": f"u64::from({expr} != 0)",
            "Int": f"u64::from({expr} as u32)", "Sizei": f"u64::from({expr} as u32)", "Short": f"u64::from({expr} as i32 as u32)",
            "Byte": f"u64::from({expr} as i32 as u32)"}.get(ty, f"u64::from({expr})")


def write_worker(path, rows, digest):
    with open(path, "w") as o:
        o.write("//! GENERATED by tools/gen-gl-sigs.py (compat repo) from the Khronos registry gl.xml (sha256 %s): do not edit.\n" % digest)
        o.write("//! One typed call into gl4es per regular GL 1.0-1.2 command; the argument words are `heirloom_gl_abi::family`'s layout.\n")
        o.write("use alloc::vec::Vec;\nuse heirloom_gl_abi::family::var_len;\nuse heirloom_gl_abi::sig::VecKind;\n\n")
        o.write("extern \"C\" {\n")
        for name, ret, sig in rows:
            if any("VecKind::State" in p for p in sig):
                continue  # answered by the worker's own state query (shadow.rs), never by one gl4es call
            args = []
            for i, p in enumerate(sig):
                m = re.match(r"P::(\w+)\((\w+)", p)
                kind, ty = m.group(1), m.group(2)
                args.append(f"a{i}: {CT[ty]}" if kind == "S" else f"a{i}: *{'mut' if kind == 'Out' else 'const'} {CT[ty]}")
            r = {"Void": "", "Bool": " -> u8", "Uint": " -> u32", "Int": " -> i32"}[ret]
            o.write(f"    fn gl4es_{name}({', '.join(args)}){r};\n")
        o.write("}\n\n")
        o.write("/// Run the regular call `name` on the current gl4es context; `Some(result words)`, or `None` when `name` is not a\n")
        o.write("/// generated call (or its query is answered by the worker itself).\n")
        o.write("///\n/// # Safety\n/// A gl4es context must be current, and `w` must satisfy `heirloom_gl_abi::family::check_words` for `name`.\n")
        o.write("pub unsafe fn call(name: &str, w: &[u64]) -> Option<Vec<u64>> {\n    Some(match name {\n")
        for name, ret, sig in rows:
            if any("VecKind::State" in p for p in sig):
                continue
            at, call_args, pre, post = 0, [], [], ""
            for i, p in enumerate(sig):
                m = re.match(r"P::(\w+)\((\w+)(?:, (?:VecKind::)?(\w+))?", p)
                kind, ty, third = m.group(1), m.group(2), m.group(3)
                if kind == "S":
                    call_args.append(conv(ty, f"w[{at}]"))
                    at += 1
                elif kind == "V":
                    n = int(third)
                    call_args.append(f"[{', '.join(conv(ty, f'w[{at + k}]') for k in range(n))}].as_ptr()")
                    at += n
                elif kind == "In":
                    pre.append(f"let n = var_len(VecKind::{third}, w[{at - 1}] as u32).unwrap_or(0);")
                    pre.append(f"let mut v: Vec<{CT[ty]}> = w[{at}..{at}+n].iter().map(|x| {conv(ty, '*x')}).collect();")
                    pre.append(f"v.resize(n.max(4), <{CT[ty]}>::default()); // gl4es's integer wrappers read four values even for a three-value parameter")
                    call_args.append("v.as_ptr()")
                    at = None  # an input vector is the last parameter
                else:  # Out
                    n = "4usize" if third == "Four" else f"var_len(VecKind::{third}, w[{at - 1}] as u32).unwrap_or(0)"
                    pre.append(f"let n = {n};")
                    pre.append(f"let mut v = alloc::vec![<{CT[ty]}>::default(); n.max(16)]; // room for any query gl4es answers")
                    call_args.append("v.as_mut_ptr()")
                    post = f"v.truncate(n); return Some(v.iter().map(|x| {to_word(ty, '(*x)')}).collect());"
            call = f"gl4es_{name}({', '.join(call_args)})"
            if not pre and not post:
                if ret == "Void":
                    o.write(f'        "{name}" => {{ {call}; Vec::new() }}\n')
                else:
                    o.write(f'        "{name}" => alloc::vec![{to_word(ret, call)}],\n')
                continue
            o.write(f'        "{name}" => {{\n')
            for line in pre:
                o.write(f"            {line}\n")
            if post:
                o.write(f"            {call};\n            {post}\n")
            elif ret == "Void":
                o.write(f"            {call};\n            Vec::new()\n")
            else:
                o.write(f"            alloc::vec![{to_word(ret, call)}]\n")
            o.write("        }\n")
        o.write("        _ => return None,\n    })\n}\n")


if __name__ == "__main__":
    main()
