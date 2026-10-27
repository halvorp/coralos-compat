//! Host-side extractor: the S60 CDL layout instances of one screen style, from the Symbian Foundation `layouts/aknlayout2` source
//! tree (EPL-1.0; the owner's local reference input), into ONE data file the engine loads at run time (`cdl.rs`). The data file is
//! generated, never committed: this tool is the only thing in the repo that knows the source tree's shape.
//!
//! The tool wrapping this is `examples/gen_cdl_layout`: gen_cdl_layout <aknlayout2-dir> <generated-subdir e.g. Qhd_touch_akn_app> <out.cdldata> [<avkon.rsg>]
//!
//! The optional avkon.rsg (the SDK's own header of resource ids) supplies the `R_AVKON_*PANE*` ids avkon compares a status-pane
//! layout against: they are numbers the owner's SDK defines, so they travel in the data file by name, not in the engine.
//!
//! What it reads (all under `<aknlayout2-dir>/generated`): the shared instances (`layoutmetadata_*.cpp`, `adaptation_layer_*.cpp`)
//! and the screen style's own (`<subdir>/*.cpp`: the byte-coded layout instances, the font and display-metric instances, the packs).
//! Per instance: its namespace, its interface UID (from `generated_inc/*.cdl.common.hrh`), the API table `KCdlImpl` in API-id order
//! (each entry: nothing, a constant the function returns, a decode operation over the byte-coded data, or code this tool cannot
//! read), and for a decode instance the byte-coded blob, the offset into it and `KDataLookup`.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::vec;
use std::fmt::Write as _;
use std::path::Path;
use crate::cdlpe::{Customisation, Pe};


/// Extract the instances of `generated/<style_dir>` (and the shared ones) of the source tree `root` into the data file's text.
/// `rsg` is the SDK's avkon.rsg (optional). With `dll` (the SDK's emulator build of the style's layout instances) the byte-coded
/// layout data and its lookup tables are the DLL's, which is the platform's own (the source tree is a later revision): the source
/// then only names things. Returns the file and a one-line summary.
pub fn extract(root: &Path, style_dir: &str, rsg: Option<&Path>, dll: Option<&Path>) -> Result<(String, String), String> {
    let gen = root.join("generated");
    let uids = interface_uids(&root.join("generated_inc"));
    let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut insts: Vec<Inst> = Vec::new();
    let mut packs = String::new();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for dir in [gen.clone(), gen.join(style_dir)] {
        let rd = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut v: Vec<_> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
        v.sort();
        files.extend(v.into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cpp")) && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase() != "layoutinst.cpp")));
    }
    let dll_bytes = match dll {
        Some(p) => Some((p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?)),
        None => None,
    };
    let overlay = match &dll_bytes {
        Some((stem, b)) => {
            let pe = Pe::parse(b)?;
            let custs = pe.customisations()?;
            let ids = instance_ids(&gen.join(style_dir).join("dllinstances.hrh"));
            Some((stem.clone(), pe, custs, ids))
        }
        None => None,
    };
    let (mut n_inst, mut n_pack, mut from_dll, mut unaligned) = (0, 0, 0, 0);
    // interface uid -> api count of the SDK's DLL when the source's api names line up with it, else None.
    let mut api_limit: BTreeMap<u32, Option<usize>> = BTreeMap::new();
    for f in &files {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        if text.contains("const LayoutPack::SCdlImpl KCdlImpl") {
            packs.push_str(&pack(&text));
            n_pack += 1;
        } else if let Some(mut inst) = instance(&text, &uids, &mut blobs, overlay.is_none()) {
            if let (Some((stem, pe, custs, ids)), true) = (&overlay, inst.data.is_some()) {
                match from_sdk_dll(&mut inst, stem, pe, custs, ids, &mut blobs) {
                    Ok(aligned) => {
                        from_dll += 1;
                        let e = api_limit.entry(inst.uid).or_insert(Some(inst.lookup.len()));
                        if !aligned || *e != Some(inst.lookup.len()) {
                            *e = None;
                            unaligned += 1;
                        }
                    }
                    Err(e) => return Err(format!("{}: {e}", inst.name)),
                }
            }
            insts.push(inst);
            n_inst += 1;
        }
    }
    let mut out = String::from("HEIRLOOM-CDL-LAYOUT 1\n");
    for (name, bytes) in &blobs {
        let _ = writeln!(out, "blob {name} {}", bytes.len());
        for b in bytes {
            let _ = write!(out, "{b:02x}");
        }
        out.push('\n');
    }
    out.push_str(&interfaces(&root.join("generated_inc"), &uids, &api_limit));
    for i in &insts {
        out.push_str(&i.render());
    }
    out.push_str(&packs);
    if let Some(rsg) = rsg {
        for line in std::fs::read_to_string(rsg).map_err(|e| format!("{}: {e}", rsg.display()))?.lines() {
            let mut t = line.split_whitespace();
            if let (Some("#define"), Some(name), Some(v)) = (t.next(), t.next(), t.next()) {
                if name.starts_with("R_AVKON_") && name.contains("PANE") {
                    let _ = writeln!(out, "rsg {name} {}", v.trim_start_matches("0x"));
                }
            }
        }
    }
    Ok((out, format!("{n_inst} instances ({from_dll} with the SDK DLL's data, {unaligned} whose api ids differ from the source tree), {n_pack} packs, {} blobs", blobs.len())))
}

/// `#define KDllInst_<Name> 0x<id>` of the style's dllinstances.hrh: lower-case name -> instance id.
pub fn instance_ids(path: &Path) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for line in std::fs::read_to_string(path).unwrap_or_default().lines() {
        let mut t = line.split_whitespace();
        if let (Some("#define"), Some(n), Some(v)) = (t.next(), t.next(), t.next()) {
            if let (Some(name), Ok(id)) = (n.strip_prefix("KDllInst_"), u32::from_str_radix(v.trim_start_matches("0x"), 16)) {
                m.insert(name.to_ascii_lowercase(), id);
            }
        }
    }
    m
}

/// Replace a decode instance's source data by the SDK DLL's: its lookup, its byte-coded data (the DLL's whole section, offset
/// by the instance's base) and the api count. Returns whether the source's op names line up one-to-one with the DLL's functions
/// (the same function pointer exactly where the source repeats an operation): when they do, the source's api ids are the DLL's.
fn from_sdk_dll(inst: &mut Inst, stem: &str, pe: &Pe, custs: &[Customisation], ids: &BTreeMap<String, u32>, blobs: &mut BTreeMap<String, Vec<u8>>) -> Result<bool, String> {
    let id = *ids.get(&inst.name.to_ascii_lowercase()).ok_or("not named in dllinstances.hrh")?;
    let c = custs.iter().find(|c| c.id == id).ok_or_else(|| format!("the DLL has no customisation {id}"))?;
    if c.uid != inst.uid {
        return Err(format!("customisation {id} implements {:#x}, the source says {:#x}", c.uid, inst.uid));
    }
    let first = pe.thunk(c, 0).ok_or("no function table")?;
    let (lookup_va, data_va) = pe.impl_data(first)?;
    let lookup_bytes = pe.bytes(lookup_va, c.api_size as usize * 2).ok_or("lookup table outside the image")?;
    inst.lookup = lookup_bytes.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32).collect();
    let (sec_va, sec) = pe.section_of(data_va).ok_or("byte-coded data outside the image")?;
    let blob = format!("sdk-{stem}-{sec_va:x}");
    blobs.entry(blob.clone()).or_insert_with(|| sec.to_vec());
    inst.data = Some((blob, data_va - sec_va));
    // Alignment of the source's op names with the DLL's functions.
    let (mut by_thunk, mut by_op): (BTreeMap<u32, &str>, BTreeMap<&str, u32>) = (BTreeMap::new(), BTreeMap::new());
    let mut aligned = inst.ops.len() >= c.api_size as usize;
    for k in 0..c.api_size {
        let (Some(t), Some(op)) = (pe.thunk(c, k), inst.ops.get(k as usize)) else { aligned = false; break };
        if *by_thunk.entry(t).or_insert(op.as_str()) != op.as_str() || *by_op.entry(op.as_str()).or_insert(t) != t {
            aligned = false;
            break;
        }
    }
    if aligned {
        inst.ops.truncate(c.api_size as usize);
    } else {
        inst.ops = vec![String::from("o:api-ids-differ-from-the-source-tree"); c.api_size as usize];
    }
    Ok(aligned)
}

/// `_CDL_<ns>_KCdlInterfaceUidValue 0x...` for every interface: namespace -> uid.
fn interface_uids(dir: &Path) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for e in std::fs::read_dir(dir).expect("read generated_inc").filter_map(Result::ok) {
        let p = e.path();
        if !p.to_string_lossy().to_ascii_lowercase().ends_with(".cdl.common.hrh") {
            continue;
        }
        for line in std::fs::read_to_string(&p).unwrap_or_default().lines() {
            if let Some(rest) = line.trim().strip_prefix("#define _CDL_") {
                if let Some((ns, v)) = rest.split_once("_KCdlInterfaceUidValue") {
                    if let Ok(u) = u32::from_str_radix(v.trim().trim_start_matches("0x"), 16) {
                        m.insert(ns.to_string(), u);
                    }
                }
            }
        }
    }
    m
}

/// The text between the first `{` after `marker` and its matching `}`.
fn braced<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let at = text.find(marker)?;
    let open = at + text[at..].find('{')?;
    let mut depth = 0;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[open + 1..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn instance(text: &str, uids: &BTreeMap<String, u32>, blobs: &mut BTreeMap<String, Vec<u8>>, read_blobs: bool) -> Option<Inst> {
    let at = text.find("SCdlImpl KCdlImpl")?;
    // `const <InterfaceNs>::SCdlImpl KCdlImpl`
    let head = &text[..at];
    let iface = head.rsplit("const ").next()?.trim().trim_end_matches("::").to_string();
    let ns = text[..at].rsplit("\nnamespace ").next()?.split(|c: char| !c.is_alphanumeric() && c != '_').next()?.to_string();
    let uid = *uids.get(&iface).or_else(|| uids.get(&iface.replace("_", " "))).unwrap_or_else(|| panic!("no interface uid for {iface} ({ns})"));
    // Function bodies: constants and decode calls, by function name.
    let mut fns: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(i) = l.find("AknLayoutScalableDecode::") {
            if let Some(name) = fn_name(&l[..i]) {
                let op = l[i + "AknLayoutScalableDecode::".len()..].split('(').next().unwrap_or("");
                fns.insert(name, format!("d:{op}"));
            }
        }
    }
    // `T Name()\n\t{\n\treturn X;\n\t}`
    let lines: Vec<&str> = text.lines().collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.ends_with("()") && !t.contains(';') && !t.starts_with("//") && i + 3 < lines.len() && lines[i + 1].trim() == "{" {
            if let Some(v) = lines[i + 2].trim().strip_prefix("return ").and_then(|r| r.strip_suffix(';')) {
                if lines[i + 3].trim() == "}" {
                    if let Some(name) = fn_name(t) {
                        fns.entry(name).or_insert_with(|| format!("c:{}", v.trim()));
                    }
                }
            }
        }
    }
    let table = braced(text, "SCdlImpl KCdlImpl")?;
    let mut ops: Vec<String> = Vec::new();
    for ent in table.lines().map(|l| l.split("//").next().unwrap_or("").trim()).filter(|l| !l.is_empty()) {
        for tok in ent.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            ops.push(if tok == "NULL" || tok == "0" { "-".into() } else {
                let n = tok.trim_start_matches('&');
                fns.get(n).cloned().unwrap_or_else(|| format!("o:{n}"))
            });
        }
    }
    let mut inst = Inst { name: ns, uid, data: None, lookup: Vec::new(), ops };
    if let Some(i) = text.find("KImplData = {") {
        // `{ KDataLookup, <ns>::<Blob> + 0xOFF }`
        let seg = &text[i..text[i..].find('}').map(|e| i + e)?];
        let blob = seg.split("::").nth(1)?.split(|c: char| !c.is_alphanumeric() && c != '_').next()?.to_string();
        let off = u32::from_str_radix(seg.rsplit("0x").next()?.trim().trim_end_matches(|c: char| !c.is_ascii_hexdigit()), 16).ok()?;
        if read_blobs && !blobs.contains_key(&blob) {
            if let Some(bytes) = blob_bytes(text, &blob) {
                blobs.insert(blob.clone(), bytes);
            }
        }
        inst.data = Some((blob, off));
        inst.lookup = if read_blobs { lookup(text)? } else { Vec::new() };
    }
    Some(inst)
}

/// One instance as the data file says it.
struct Inst {
    name: String,
    uid: u32,
    /// (blob name, offset into it).
    data: Option<(String, u32)>,
    lookup: Vec<u32>,
    ops: Vec<String>,
}

impl Inst {
    fn render(&self) -> String {
        let mut s = format!("instance {} {:08x}\n", self.name, self.uid);
        if let Some((blob, off)) = &self.data {
            let _ = writeln!(s, "data {blob} {off}");
            s.push_str("lookup");
            for v in &self.lookup {
                let _ = write!(s, " {v:x}");
            }
            s.push('\n');
        }
        s.push_str("impl");
        for o in &self.ops {
            s.push(' ');
            s.push_str(o);
        }
        s.push('\n');
        s
    }
}

/// The function name in `<return type> Name(<params>) ...` (text up to the body or the call).
fn fn_name(sig: &str) -> Option<String> {
    let before = sig.split('(').next()?.trim();
    let name = before.rsplit(|c: char| c.is_whitespace()).next()?;
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| name.to_string())
}

fn blob_bytes(text: &str, blob: &str) -> Option<Vec<u8>> {
    let marker = format!("{blob}[] = {{");
    let at = text.find(&marker)? + marker.len();
    let end = at + text[at..].find("};")?;
    let mut v = Vec::new();
    for line in text[at..end].lines() {
        let l = line.trim();
        if l.starts_with("//") {
            continue;
        }
        for t in l.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            v.push(u8::from_str_radix(t.trim_start_matches("0x"), 16).ok()?);
        }
    }
    Some(v)
}

fn lookup(text: &str) -> Option<Vec<u32>> {
    let at = text.find("KDataLookup[AknLayoutScalable_")?;
    let at = at + text[at..].find("] =")?;
    let end = at + text[at..].find("};")?;
    let mut v = Vec::new();
    for line in text[at..end].lines() {
        let l = line.split("//").next().unwrap_or("").trim();
        for t in l.split(',').map(str::trim).filter(|t| t.starts_with("0x")) {
            v.push(u32::from_str_radix(&t[2..], 16).ok()?);
        }
    }
    Some(v)
}

/// A `LayoutPack` instance: name, size, layout id, zoom, style hash, priority, app uid, contents in stack order.
fn pack(text: &str) -> String {
    let get = |k: &str| -> String {
        let at = text.find(k).unwrap_or_else(|| panic!("pack without {k}"));
        text[at + k.len()..].split(';').next().unwrap_or("").trim().trim_start_matches('=').trim().to_string()
    };
    let name = text.split("_LIT(name,\"").nth(1).and_then(|s| s.split('"').next()).unwrap_or("?").to_string();
    let size = get("SCdlSize const size").trim_matches(|c| c == '{' || c == '}' || c == ' ').replace(',', " ");
    let id = get("TAknLayoutId const id");
    let zoom = get("TAknUiZoom const zoom");
    let hash = get("TInt const styleHash").split_whitespace().next().unwrap_or("0").to_string();
    let prio = get("TInt const priority");
    let appuid = get("TInt const appUid");
    let mut s = format!("pack {name} {size} {id} {zoom} {hash} {prio} {appuid}\ncontents");
    let body = braced(text, "CDL_ARRAY_START(TCdlRef, contents)").unwrap_or("");
    for ent in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let e = ent.trim_start_matches(',').trim();
        if let Some(n) = e.strip_prefix("LOCAL_CDL_REF(").and_then(|r| r.split(')').next()) {
            s.push_str(&format!(" {n}"));
        } else if let Some(n) = e.strip_prefix('{').and_then(|r| r.split("::KCdlInstanceId").next()) {
            s.push_str(&format!(" {n}"));
        }
    }
    s.push('\n');
    s
}

/// `interface <uid> <api names in id order>` for every interface header (`enum TApiId`).
fn interfaces(dir: &Path, uids: &BTreeMap<String, u32>, limit: &BTreeMap<u32, Option<usize>>) -> String {
    let mut out = String::new();
    let mut files: Vec<_> = std::fs::read_dir(dir).expect("read generated_inc").filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    for p in files {
        if !p.to_string_lossy().to_ascii_lowercase().ends_with(".cdl.common.h") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let Some(ns) = text.split("\nnamespace ").nth(1).and_then(|r| r.split(|c: char| !c.is_alphanumeric() && c != '_').next()) else { continue };
        let Some(uid) = uids.get(ns) else { continue };
        let Some(body) = braced(&text, "enum TApiId") else { continue };
        let mut names: Vec<&str> = body.split(',').map(str::trim).filter_map(|n| n.strip_prefix("EApiId_")).collect();
        // The SDK DLL's interface has fewer apis than the source tree's later one: names past it are not the platform's.
        match limit.get(uid) {
            Some(Some(n)) => names.truncate(*n),
            Some(None) => names.clear(),
            None => {}
        }
        let _ = writeln!(out, "interface {uid:08x} {ns} {}", names.join(" "));
    }
    out
}
