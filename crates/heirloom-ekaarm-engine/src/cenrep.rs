//! Central Repository (`CRepository`, centralrepository.h). A repository is a text file of the CenRep server's private directory
//! in the device ROM (`\private\10202be9\<uid>.txt`, UTF-16, sections `[Main]` with lines `key type meta value caps`); the package
//! drive may shadow it. The guest holds a CBase-derived object; the keys are host state. A repository nobody has leaves
//! `KErrNotFound`, which needs the EHABI unwinder (slice 1b): a named refusal. Only `int`, `real` and `string` keys are parsed;
//! `Set`/`Create`/`Delete` are not implemented (named refusals).
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::cleanup::CleanupItem;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

#[derive(Debug, Clone, PartialEq)]
pub enum CenValue {
    Int(i32),
    Real(f64),
    Str(String),
}

const KERR_NOT_FOUND: u32 = -1i32 as u32;
const KERR_ARGUMENT: u32 = -6i32 as u32;
const REPO_BYTES: u32 = 16;
const VTABLE: &[(usize, &str)] = &[(0, "_ZN11CRepositoryD1Ev"), (1, "_ZN11CRepositoryD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")];

fn decode(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c));
        char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn parse_int(t: &str) -> Option<i64> {
    let (neg, t) = t.strip_prefix('-').map_or((false, t), |r| (true, r));
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) { i64::from_str_radix(h, 16).ok()? } else { t.parse().ok()? };
    Some(if neg { -v } else { v })
}

/// The `[Main]` keys of a repository text file.
pub fn parse(bytes: &[u8]) -> Result<BTreeMap<u32, CenValue>, String> {
    let text = decode(bytes);
    let mut out = BTreeMap::new();
    let mut in_main = false;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(sec) = line.strip_prefix('[') {
            in_main = sec.trim_end_matches(']').eq_ignore_ascii_case("main");
            continue;
        }
        if !in_main || line.is_empty() { continue; }
        let mut it = line.splitn(3, char::is_whitespace);
        let (Some(key), Some(ty), Some(rest)) = (it.next(), it.next(), it.next()) else { return Err(format!("line {}: {line}", n + 1)) };
        let key = parse_int(key).ok_or_else(|| format!("line {}: bad key {key}", n + 1))? as u32;
        let rest = rest.trim_start();
        let value = match ty {
            "int" => CenValue::Int(parse_int(rest.split_whitespace().next().unwrap_or("")).ok_or_else(|| format!("line {}: bad int", n + 1))? as i32),
            "real" => CenValue::Real(rest.split_whitespace().next().unwrap_or("").parse().map_err(|_| format!("line {}: bad real", n + 1))?),
            "string" => {
                let body = rest.strip_prefix('"').ok_or_else(|| format!("line {}: unquoted string", n + 1))?;
                let mut s = String::new();
                let mut chars = body.chars();
                while let Some(c) = chars.next() {
                    match c { '"' => break, '\\' => if let Some(e) = chars.next() { s.push(e) }, c => s.push(c) }
                }
                CenValue::Str(s)
            }
            _ => continue, // binary and string8 keys are not read yet; asking for one finds nothing (KErrNotFound is wrong for them, so Get refuses). DELETE WHEN a corpus app's Get names one (then parse it)
        };
        out.insert(key, value);
    }
    Ok(out)
}

impl Process {
    fn repo_open(&mut self, uid: u32) -> Result<u32, String> {
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive"))?;
        let bytes = [format!("\\private\\10202be9\\{uid:08x}.txt")].iter().find_map(|p| drive.read_owned(p).ok());
        let Some(bytes) = bytes else {
            return Err(format!("NEXT-NEEDED leave CRepository::NewL({uid:#010x}): no repository file on the drive or in the ROM leaves KErrNotFound (EHABI unwinding is slice 1b)"));
        };
        let keys = parse(&bytes).map_err(|e| format!("repository {uid:#010x}: {e}"))?;
        let obj = self.alloc_zeroed(REPO_BYTES).ok_or_else(|| String::from("no guest heap"))?;
        let vptr = self.build_vtable("CRepository", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(obj, vptr).map_err(|_| String::from("repository object unwritable"))?;
        self.obj_set(obj, HostObj::Repo(keys))?;
        Ok(obj)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(if m.starts_with("NEXT-NEEDED") { m } else { format!("NEXT-NEEDED cenrep {name}: {m}") });
    Some(match name {
        "_ZN11CRepository4NewLE4TUid" | "_ZN11CRepository5NewLCE4TUid" => match p.repo_open(r0) {
            Ok(o) => {
                if name.ends_with("NewLCE4TUid") { p.nat.cleanup.push(CleanupItem::Base(o)); }
                p.cpu.regs[0] = o;
                NativeOutcome::Return
            }
            Err(m) => refuse(m),
        },
        // TInt CRepository::Get(TUint32 aKey, TInt& aValue)
        "_ZN11CRepository3GetEmRi" => {
            let found = match p.obj_get(r0) { Some(HostObj::Repo(k)) => k.get(&r1).cloned(), _ => return Some(refuse(format!("{r0:#x} is not a live repository"))) };
            match found {
                None => { p.cpu.regs[0] = KERR_NOT_FOUND; NativeOutcome::Return }
                Some(CenValue::Int(v)) => match p.mem.vm.write_u32(r2, v as u32) {
                    Ok(()) => { p.cpu.regs[0] = 0; NativeOutcome::Return }
                    Err(_) => refuse(format!("the result at {r2:#x} is unwritable")),
                },
                Some(_) => { p.cpu.regs[0] = KERR_ARGUMENT; NativeOutcome::Return }
            }
        }
        "_ZN11CRepositoryD1Ev" | "_ZN11CRepositoryD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
            NativeOutcome::Return
        }
        _ => return None,
    })
}

