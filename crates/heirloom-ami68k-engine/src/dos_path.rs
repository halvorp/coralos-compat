//! AmigaDOS path-string utilities: FilePart, PathPart, AddPart. The string logic is pure (tested below); the
//! handlers only move bytes between guest memory and it.
use alloc::string::String;
use alloc::format;
use crate::fs::DOSTRUE;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// Offset of the last component of `p` (after the last '/' or ':').
pub fn file_part(p: &str) -> usize { p.rfind(|c| c == '/' || c == ':').map_or(0, |i| i + 1) }

/// Offset of the end of the directory part: the last '/', or just after the volume colon, or 0.
pub fn path_part(p: &str) -> usize {
    match p.rfind('/') {
        Some(i) => i,
        None => p.find(':').map_or(0, |i| i + 1),
    }
}

/// `dir` + `file` as AddPart builds it: an absolute `file` (with a volume) replaces `dir`; each leading '/' of
/// `file` removes one trailing component of `dir`; otherwise a '/' joins them unless `dir` is empty or ends in ':' or '/'.
pub fn add_part(dir: &str, file: &str) -> String {
    if file.contains(':') { return file.into(); }
    let mut d = String::from(dir);
    let mut f = file;
    while let Some(rest) = f.strip_prefix('/') {
        f = rest;
        let cut = if d.ends_with('/') { d.len() - 1 } else { d.len() };
        let trimmed = &d[..cut];
        let end = trimmed.rfind(|c| c == '/' || c == ':').map_or(0, |i| if trimmed.as_bytes()[i] == b':' { i + 1 } else { i });
        d.truncate(end);
    }
    if f.is_empty() || d.is_empty() || d.ends_with(':') || d.ends_with('/') { d.push_str(f); } else { d.push('/'); d.push_str(f); }
    d
}

/// FilePart(d1 = path) -> pointer to the last component inside the same string.
pub fn file_part_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let at = cpu.dreg(1);
    let s = a.mem.read_cstr(at).map_err(Refusal::mem("FilePart.path"))?;
    let r = at + file_part(&s) as u32;
    cpu.set_d0(r);
    Ok(format!("[dos] FilePart \"{s}\" -> +{}", file_part(&s)))
}

/// PathPart(d1 = path) -> pointer to the end of the directory part inside the same string.
pub fn path_part_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let at = cpu.dreg(1);
    let s = a.mem.read_cstr(at).map_err(Refusal::mem("PathPart.path"))?;
    cpu.set_d0(at + path_part(&s) as u32);
    Ok(format!("[dos] PathPart \"{s}\" -> +{}", path_part(&s)))
}

/// AddPart(d1 = dirname buffer, d2 = filename, d3 = buffer size) -> DOSTRUE, or FALSE when the result does not fit.
pub fn add_part_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (buf, file, size) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let dir = a.mem.read_cstr(buf).map_err(Refusal::mem("AddPart.dirname"))?;
    let file = a.mem.read_cstr(file).map_err(Refusal::mem("AddPart.filename"))?;
    let r = add_part(&dir, &file);
    if r.len() as u64 + 1 > size as u64 { cpu.set_d0(0); return Ok(format!("[dos] AddPart \"{dir}\" + \"{file}\" does not fit {size} bytes -> FALSE")); }
    a.mem.write_bytes(buf, r.as_bytes()).map_err(Refusal::mem("AddPart.buffer"))?;
    a.mem.write_u8(buf + r.len() as u32, 0).map_err(Refusal::mem("AddPart.buffer"))?;
    cpu.set_d0(DOSTRUE);
    Ok(format!("[dos] AddPart \"{dir}\" + \"{file}\" -> \"{r}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_and_path_part() {
        assert_eq!(&"xxx:yyy/zzz"[file_part("xxx:yyy/zzz")..], "zzz");
        assert_eq!(&"xxx:yyy"[file_part("xxx:yyy")..], "yyy");
        assert_eq!(&"xxx:"[file_part("xxx:")..], "");
        assert_eq!(&"plain"[file_part("plain")..], "plain");
        assert_eq!(&"xxx:yyy/zzz"[path_part("xxx:yyy/zzz")..], "/zzz");
        assert_eq!(&"xxx:yyy"[path_part("xxx:yyy")..], "yyy");
        assert_eq!(path_part("plain"), 0);
    }

    #[test]
    fn add_part_joins_and_goes_up() {
        assert_eq!(add_part("SYS:Libs", "foo"), "SYS:Libs/foo");
        assert_eq!(add_part("SYS:", "foo"), "SYS:foo");
        assert_eq!(add_part("SYS:Libs/", "foo"), "SYS:Libs/foo");
        assert_eq!(add_part("", "foo"), "foo");
        assert_eq!(add_part("SYS:Libs", "RAM:x"), "RAM:x", "an absolute file replaces the dir");
        assert_eq!(add_part("a/b", "/c"), "a/c");
        assert_eq!(add_part("SYS:a", "/c"), "SYS:c");
        assert_eq!(add_part("a", "/c"), "c");
        assert_ne!(add_part("a", "c"), "ac", "negative control: a separator is inserted");
    }
}
