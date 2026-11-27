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

/// Symbian S60 `KCRUidProfileEngine` (`ProfileEngineSDKCRKeys.h`): active profile settings
/// shipped in every S60 ROM at `Z:\private\10202be9\101f8798.txt`.
pub const KCRUID_PROFILE_ENGINE: u32 = 0x101f_8798;

/// Built-in Symbian S60 platform default Central Repositories when a minimal ROM/drive omits the
/// corresponding `Z:\private\10202be9\<uid>.txt` file. A file present on the drive or in the ROM
/// always shadows these defaults.
fn platform_default_repo(uid: u32) -> Option<BTreeMap<u32, CenValue>> {
    if uid != KCRUID_PROFILE_ENGINE {
        return None;
    }
    // General profile defaults from S60 SDK `ProfileEngineSDKCRKeys.h` and ROM `101f8798.txt`.
    let mut m = BTreeMap::new();
    for (k, v) in [
        // Active profile read-only SDK keys (`ProfileEngineSDKCRKeys.h`)
        (0x7E00_0001u32, 0i32), // KProEngActiveProfile = 0 (General)
        (0x7E00_0002, 0),       // KProEngActiveRingingType = 0 (Ringing)
        (0x7E00_0004, 1),       // KProEngActiveKeypadVolume = 1
        (0x7E00_0008, 7),       // KProEngActiveRingingVolume = 7
        (0x7E00_0019, 2),       // KProEngActiveMediaVolume = 2
        (0x7E00_001A, 2),       // KProEngActiveTextToSpeech = 2
        (0x7E00_0020, 1),       // KProEngActiveWarningTones = 1 (On)
        (0x7E00_0040, 1),       // KProEngActiveMessageAlert = 1 (On)
        // General profile (id 0) keys from S60 ROM `101f8798.txt`
        (0x0000_0007, 0),       // ringing type = Ringing
        (0x0000_0008, 1),       // keypad volume = 1
        (0x0000_0009, 7),       // ringing volume = 7
        (0x0000_000A, 0),       // vibrating alert = Off
        (0x0000_000B, 1),       // warning tones = On
        (0x0000_0010, 2),       // touch screen tones = 2
        (0x0000_0011, 7),       // ringing volume (alt) = 7
        (0x0000_0012, 0),
        (0x0000_0013, 0),
        (0x0000_0016, 0),
        (0x0000_0017, 0),
        (0x0000_0018, 0),
        (0x0000_0019, 2),
        (0x0000_001A, 2),
        // Global profile engine metadata keys from S60 ROM `101f8798.txt`
        (0x8000_0003, -1),
        (0x8000_0004, -1),
        (0x8000_0005, -1),
        (0x8000_0100, 1),
        (0x8000_0101, 0),
        (0x8000_0102, 0),
        (0x8000_0103, 0),
    ] {
        m.insert(k, CenValue::Int(v));
    }
    Some(m)
}

impl Process {
    fn repo_open(&mut self, uid: u32) -> Result<u32, String> {
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive"))?;
        let keys = match [format!("\\private\\10202be9\\{uid:08x}.txt")].iter().find_map(|p| drive.read_owned(p).ok()) {
            Some(bytes) => parse(&bytes).map_err(|e| format!("repository {uid:#010x}: {e}"))?,
            None => match platform_default_repo(uid) {
                Some(default_keys) => default_keys,
                None => return Err(format!("NEXT-NEEDED leave CRepository::NewL({uid:#010x}): no repository file on the drive or in the ROM leaves KErrNotFound (EHABI unwinding is slice 1b)")),
            },
        };
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
            Err(m) => {
                if !p.exception_descriptors.is_empty() && m.contains("no repository file") {
                    crate::ehabi::leave(p, -1)
                } else {
                    refuse(m)
                }
            }
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

