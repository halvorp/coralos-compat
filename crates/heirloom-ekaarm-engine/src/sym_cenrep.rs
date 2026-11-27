//! Stateless Symbian Central Repository (`CRepository`, centralrepository.h) adapter over `guestsvc_abi::GuestPrefs`.
//!
//! REUSE U6 (CoralOS `docs/heirloom/21-SHARED-SERVICES.md` + component-model C5):
//! - All persistent key reads/writes/deletes go directly to the engine process's native `prefs.service` session
//!   (`Space::Own`, loader-admitted bundle namespace) under `cenrep/<repo_uid_8hex>/<key_id_8hex>`.
//! - Immutable ROM/package `.txt` files (`\private\10202be9\<uid>.txt`) and S60 platform defaults serve purely as
//!   read-only fallback defaults when no key has been stored in `prefs.service`; the engine holds ZERO mutable
//!   or persistent repository state of its own.
//! - Without a `(prefs)` grant (`GuestSvcError::NotGranted` or unattached host), all mutations (`Set`/`Create`/`Delete`)
//!   return `KErrPermissionDenied` (-46) and never fall back to silent in-engine storage.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestsvc_abi::{GuestSvcError, GuestValue};
use crate::cleanup::CleanupItem;
use crate::desc;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{HostObj, RepoHandle};

#[derive(Debug, Clone, PartialEq)]
pub enum CenValue {
    Int(i32),
    Real(f64),
    Str(String),
    Bin(Vec<u8>),
}

const KERR_NONE: u32 = 0;
const KERR_NOT_FOUND: u32 = -1i32 as u32;
const KERR_ARGUMENT: u32 = -6i32 as u32;
const KERR_ALREADY_EXISTS: u32 = -11i32 as u32;
const KERR_SERVER_TERMINATED: u32 = -15i32 as u32;
const KERR_OVERFLOW: u32 = -22i32 as u32;
const KERR_PERMISSION_DENIED: u32 = -46i32 as u32;
const REPO_BYTES: u32 = 16;
const VTABLE: &[(usize, &str)] = &[
    (0, "_ZN11CRepositoryD1Ev"),
    (1, "_ZN11CRepositoryD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
];

fn prefs_key(uid: u32, key: u32) -> String {
    format!("cenrep/{uid:08x}/{key:08x}")
}

fn prefs_prefix(uid: u32) -> String {
    format!("cenrep/{uid:08x}/")
}

fn map_svc_err(err: GuestSvcError) -> u32 {
    match err {
        GuestSvcError::NotGranted => KERR_PERMISSION_DENIED,
        GuestSvcError::TooLarge => KERR_OVERFLOW,
        GuestSvcError::BadName | GuestSvcError::Refused(_) => KERR_ARGUMENT,
        GuestSvcError::Lost | GuestSvcError::Stalled | GuestSvcError::Unreadable => KERR_SERVER_TERMINATED,
    }
}

fn guest_value_to_cen(v: GuestValue) -> Option<CenValue> {
    match v {
        GuestValue::Int(n) => i32::try_from(n).ok().map(CenValue::Int),
        GuestValue::Bool(b) => Some(CenValue::Int(i32::from(b))),
        GuestValue::Real(r) => Some(CenValue::Real(r)),
        GuestValue::Text(s) => Some(CenValue::Str(s)),
        GuestValue::Bytes(b) => Some(CenValue::Bin(b)),
    }
}

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
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else {
        t.parse().ok()?
    };
    Some(if neg { -v } else { v })
}

/// Parse the immutable `[Main]` default keys of a ROM/package repository text file.
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
        if !in_main || line.is_empty() {
            continue;
        }
        let mut it = line.splitn(3, char::is_whitespace);
        let (Some(key), Some(ty), Some(rest)) = (it.next(), it.next(), it.next()) else {
            return Err(format!("line {}: {line}", n + 1));
        };
        let key = parse_int(key).ok_or_else(|| format!("line {}: bad key {key}", n + 1))? as u32;
        let rest = rest.trim_start();
        let value = match ty {
            "int" => CenValue::Int(
                parse_int(rest.split_whitespace().next().unwrap_or(""))
                    .ok_or_else(|| format!("line {}: bad int", n + 1))? as i32,
            ),
            "real" => CenValue::Real(
                rest.split_whitespace()
                    .next()
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| format!("line {}: bad real", n + 1))?,
            ),
            "string" => {
                let body = rest.strip_prefix('"').ok_or_else(|| format!("line {}: unquoted string", n + 1))?;
                let mut s = String::new();
                let mut chars = body.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => {
                            if let Some(e) = chars.next() {
                                s.push(e);
                            }
                        }
                        c => s.push(c),
                    }
                }
                CenValue::Str(s)
            }
            _ => continue,
        };
        out.insert(key, value);
    }
    Ok(out)
}

/// Symbian S60 `KCRUidProfileEngine` (`ProfileEngineSDKCRKeys.h`): active profile settings
/// shipped in every S60 ROM at `Z:\private\10202be9\101f8798.txt`.
pub const KCRUID_PROFILE_ENGINE: u32 = 0x101f_8798;

/// Built-in read-only S60 platform default Central Repositories when a minimal ROM/drive omits the
/// corresponding `Z:\private\10202be9\<uid>.txt` file. A file present on the drive or in the ROM
/// always shadows these defaults, and persisted `prefs.service` values always shadow both.
fn platform_default_repo(uid: u32) -> Option<BTreeMap<u32, CenValue>> {
    if uid != KCRUID_PROFILE_ENGINE {
        return None;
    }
    let mut m = BTreeMap::new();
    for (k, v) in [
        (0x7E00_0001u32, 0i32),
        (0x7E00_0002, 0),
        (0x7E00_0004, 1),
        (0x7E00_0008, 7),
        (0x7E00_0019, 2),
        (0x7E00_001A, 2),
        (0x7E00_0020, 1),
        (0x7E00_0040, 1),
        (0x0000_0007, 0),
        (0x0000_0008, 1),
        (0x0000_0009, 7),
        (0x0000_000A, 0),
        (0x0000_000B, 1),
        (0x0000_0010, 2),
        (0x0000_0011, 7),
        (0x0000_0012, 0),
        (0x0000_0013, 0),
        (0x0000_0016, 0),
        (0x0000_0017, 0),
        (0x0000_0018, 0),
        (0x0000_0019, 2),
        (0x0000_001A, 2),
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
        let drive_bytes = self
            .host
            .drive
            .as_ref()
            .and_then(|d| d.read_owned(&format!("\\private\\10202be9\\{uid:08x}.txt")).ok());
        let defaults = match drive_bytes {
            Some(bytes) => parse(&bytes).map_err(|e| format!("repository {uid:#010x}: {e}"))?,
            None => match platform_default_repo(uid) {
                Some(default_keys) => default_keys,
                None => {
                    // If prefs.service already holds persisted keys for this repository UID, open it with empty ROM defaults.
                    let has_persisted = self
                        .prefs_host
                        .as_mut()
                        .and_then(|prefs| prefs.keys(&prefs_prefix(uid)).ok())
                        .is_some_and(|ks| !ks.is_empty());
                    if has_persisted {
                        BTreeMap::new()
                    } else if self.host.drive.is_none() {
                        return Err(String::from("no drive"));
                    } else {
                        return Err(format!(
                            "NEXT-NEEDED leave CRepository::NewL({uid:#010x}): no repository file on the drive or in the ROM leaves KErrNotFound (EHABI unwinding is slice 1b)"
                        ));
                    }
                }
            },
        };
        let obj = self.alloc_zeroed(REPO_BYTES).ok_or_else(|| String::from("no guest heap"))?;
        let vptr = self.build_vtable("CRepository", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(obj, vptr).map_err(|_| String::from("repository object unwritable"))?;
        self.obj_set(obj, HostObj::Repo(RepoHandle { uid, defaults }))?;
        Ok(obj)
    }

    /// Resolve `key` for live repository `repo_addr`: consult `prefs.service` first, then read-only ROM defaults.
    fn repo_get_value(&mut self, repo_addr: u32, key: u32) -> Result<Result<Option<CenValue>, u32>, String> {
        let (uid, default_val) = match self.obj_get(repo_addr) {
            Some(HostObj::Repo(h)) => (h.uid, h.defaults.get(&key).cloned()),
            _ => return Err(format!("{repo_addr:#x} is not a live repository")),
        };
        if let Some(prefs) = self.prefs_host.as_mut() {
            match prefs.get(&prefs_key(uid, key)) {
                Ok(Some(v)) => {
                    return Ok(match guest_value_to_cen(v) {
                        Some(cv) => Ok(Some(cv)),
                        None => Err(KERR_ARGUMENT),
                    });
                }
                Ok(None) => return Ok(Ok(default_val)),
                Err(GuestSvcError::NotGranted) => {
                    // Static read-only ROM defaults stay readable even when a bundle has no `(prefs)` grant;
                    // keys absent from ROM surface KErrPermissionDenied instead of pretending KErrNotFound.
                    if let Some(dv) = default_val {
                        return Ok(Ok(Some(dv)));
                    }
                    return Ok(Err(KERR_PERMISSION_DENIED));
                }
                Err(e) => return Ok(Err(map_svc_err(e))),
            }
        }
        Ok(Ok(default_val))
    }

    /// Persist `value` to `prefs.service` for live repository `repo_addr`. Never mutates in-engine state.
    fn repo_put_value(&mut self, repo_addr: u32, key: u32, value: GuestValue, must_not_exist: bool) -> Result<u32, String> {
        let (uid, has_default) = match self.obj_get(repo_addr) {
            Some(HostObj::Repo(h)) => (h.uid, h.defaults.contains_key(&key)),
            _ => return Err(format!("{repo_addr:#x} is not a live repository")),
        };
        let Some(prefs) = self.prefs_host.as_mut() else {
            return Ok(KERR_PERMISSION_DENIED);
        };
        let k = prefs_key(uid, key);
        if must_not_exist {
            match prefs.get(&k) {
                Ok(Some(_)) => return Ok(KERR_ALREADY_EXISTS),
                Ok(None) if has_default => return Ok(KERR_ALREADY_EXISTS),
                Ok(None) => {}
                Err(e) => return Ok(map_svc_err(e)),
            }
        }
        match prefs.set(&k, &value) {
            Ok(()) => Ok(KERR_NONE),
            Err(e) => Ok(map_svc_err(e)),
        }
    }

    /// Delete `key` from `prefs.service` for live repository `repo_addr`.
    fn repo_delete_key(&mut self, repo_addr: u32, key: u32) -> Result<u32, String> {
        let uid = match self.obj_get(repo_addr) {
            Some(HostObj::Repo(h)) => h.uid,
            _ => return Err(format!("{repo_addr:#x} is not a live repository")),
        };
        let Some(prefs) = self.prefs_host.as_mut() else {
            return Ok(KERR_PERMISSION_DENIED);
        };
        match prefs.delete(&prefs_key(uid, key)) {
            Ok(()) => Ok(KERR_NONE),
            Err(e) => Ok(map_svc_err(e)),
        }
    }
}

fn read_f64(p: &Process, addr: u32) -> Result<f64, String> {
    let lo = p.mem.vm.read_u32(addr).map_err(|_| format!("TReal at {addr:#x} is unreadable"))?;
    let hi = p.mem.vm.read_u32(addr.wrapping_add(4)).map_err(|_| format!("TReal high word at {addr:#x} is unreadable"))?;
    Ok(f64::from_bits((u64::from(hi) << 32) | u64::from(lo)))
}

fn write_f64(p: &mut Process, addr: u32, val: f64) -> Result<(), String> {
    let bits = val.to_bits();
    p.mem.vm.write_u32(addr, bits as u32).map_err(|_| format!("TReal at {addr:#x} is unwritable"))?;
    p.mem.vm.write_u32(addr.wrapping_add(4), (bits >> 32) as u32).map_err(|_| format!("TReal high word at {addr:#x} is unwritable"))?;
    Ok(())
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| {
        NativeOutcome::Refused(if m.starts_with("NEXT-NEEDED") {
            m
        } else {
            format!("NEXT-NEEDED cenrep {name}: {m}")
        })
    };
    Some(match name {
        "_ZN11CRepository4NewLE4TUid" | "_ZN11CRepository5NewLCE4TUid" => match p.repo_open(r0) {
            Ok(o) => {
                if name.ends_with("NewLCE4TUid") {
                    p.nat.cleanup.push(CleanupItem::Base(o));
                }
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
        "_ZN11CRepository3GetEmRi" => match p.repo_get_value(r0, r1) {
            Err(m) => refuse(m),
            Ok(Err(code)) => {
                p.cpu.regs[0] = code;
                NativeOutcome::Return
            }
            Ok(Ok(None)) => {
                p.cpu.regs[0] = KERR_NOT_FOUND;
                NativeOutcome::Return
            }
            Ok(Ok(Some(CenValue::Int(v)))) => match p.mem.vm.write_u32(r2, v as u32) {
                Ok(()) => {
                    p.cpu.regs[0] = KERR_NONE;
                    NativeOutcome::Return
                }
                Err(_) => refuse(format!("the result at {r2:#x} is unwritable")),
            },
            Ok(Ok(Some(_))) => {
                p.cpu.regs[0] = KERR_ARGUMENT;
                NativeOutcome::Return
            }
        },
        // TInt CRepository::Get(TUint32 aKey, TReal& aValue)
        "_ZN11CRepository3GetEmRd" => match p.repo_get_value(r0, r1) {
            Err(m) => refuse(m),
            Ok(Err(code)) => {
                p.cpu.regs[0] = code;
                NativeOutcome::Return
            }
            Ok(Ok(None)) => {
                p.cpu.regs[0] = KERR_NOT_FOUND;
                NativeOutcome::Return
            }
            Ok(Ok(Some(CenValue::Real(v)))) => match write_f64(p, r2, v) {
                Ok(()) => {
                    p.cpu.regs[0] = KERR_NONE;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            },
            Ok(Ok(Some(_))) => {
                p.cpu.regs[0] = KERR_ARGUMENT;
                NativeOutcome::Return
            }
        },
        // TInt CRepository::Get(TUint32 aKey, TDes16& aValue)
        "_ZN11CRepository3GetEmR6TDes16" => match p.repo_get_value(r0, r1) {
            Err(m) => refuse(m),
            Ok(Err(code)) => {
                p.cpu.regs[0] = code;
                NativeOutcome::Return
            }
            Ok(Ok(None)) => {
                p.cpu.regs[0] = KERR_NOT_FOUND;
                NativeOutcome::Return
            }
            Ok(Ok(Some(CenValue::Str(s)))) => {
                let units: Vec<u16> = s.encode_utf16().collect();
                match desc::write16(p, r2, &units) {
                    Ok(()) => {
                        p.cpu.regs[0] = KERR_NONE;
                        NativeOutcome::Return
                    }
                    Err(m) if m.contains("maximum length") => {
                        p.cpu.regs[0] = KERR_OVERFLOW;
                        NativeOutcome::Return
                    }
                    Err(m) => refuse(m),
                }
            }
            Ok(Ok(Some(_))) => {
                p.cpu.regs[0] = KERR_ARGUMENT;
                NativeOutcome::Return
            }
        },
        // TInt CRepository::Get(TUint32 aKey, TDes8& aValue)
        "_ZN11CRepository3GetEmR5TDes8" => match p.repo_get_value(r0, r1) {
            Err(m) => refuse(m),
            Ok(Err(code)) => {
                p.cpu.regs[0] = code;
                NativeOutcome::Return
            }
            Ok(Ok(None)) => {
                p.cpu.regs[0] = KERR_NOT_FOUND;
                NativeOutcome::Return
            }
            Ok(Ok(Some(CenValue::Bin(b)))) => match desc::write8(p, r2, &b) {
                Ok(()) => {
                    p.cpu.regs[0] = KERR_NONE;
                    NativeOutcome::Return
                }
                Err(m) if m.contains("maximum length") => {
                    p.cpu.regs[0] = KERR_OVERFLOW;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            },
            Ok(Ok(Some(CenValue::Str(s)))) => match desc::write8(p, r2, s.as_bytes()) {
                Ok(()) => {
                    p.cpu.regs[0] = KERR_NONE;
                    NativeOutcome::Return
                }
                Err(m) if m.contains("maximum length") => {
                    p.cpu.regs[0] = KERR_OVERFLOW;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            },
            Ok(Ok(Some(_))) => {
                p.cpu.regs[0] = KERR_ARGUMENT;
                NativeOutcome::Return
            }
        },
        // TInt CRepository::Set(TUint32 aKey, TInt aValue) / Create(TUint32 aKey, TInt aValue)
        "_ZN11CRepository3SetEmi" | "_ZN11CRepository6CreateEmi" => {
            let create = name.contains("6Create");
            match p.repo_put_value(r0, r1, GuestValue::Int(i64::from(r2 as i32)), create) {
                Ok(rc) => {
                    p.cpu.regs[0] = rc;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        // TInt CRepository::Set(TUint32 aKey, const TReal& aValue) / Create(TUint32 aKey, const TReal& aValue)
        "_ZN11CRepository3SetEmRKd" | "_ZN11CRepository6CreateEmRKd" => {
            let create = name.contains("6Create");
            let val = match read_f64(p, r2) {
                Ok(v) => v,
                Err(m) => return Some(refuse(m)),
            };
            match p.repo_put_value(r0, r1, GuestValue::Real(val), create) {
                Ok(rc) => {
                    p.cpu.regs[0] = rc;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        // TInt CRepository::Set(TUint32 aKey, const TDesC16& aValue) / Create(TUint32 aKey, const TDesC16& aValue)
        "_ZN11CRepository3SetEmRK7TDesC16" | "_ZN11CRepository6CreateEmRK7TDesC16" => {
            let create = name.contains("6Create");
            let text = match desc::read16(p, r2) {
                Ok(units) => desc::to_utf8(&units),
                Err(m) => return Some(refuse(m)),
            };
            match p.repo_put_value(r0, r1, GuestValue::Text(text), create) {
                Ok(rc) => {
                    p.cpu.regs[0] = rc;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        // TInt CRepository::Set(TUint32 aKey, const TDesC8& aValue) / Create(TUint32 aKey, const TDesC8& aValue)
        "_ZN11CRepository3SetEmRK6TDesC8" | "_ZN11CRepository6CreateEmRK6TDesC8" => {
            let create = name.contains("6Create");
            let bytes = match desc::read8(p, r2) {
                Ok(b) => b,
                Err(m) => return Some(refuse(m)),
            };
            match p.repo_put_value(r0, r1, GuestValue::Bytes(bytes), create) {
                Ok(rc) => {
                    p.cpu.regs[0] = rc;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        // TInt CRepository::Delete(TUint32 aKey)
        "_ZN11CRepository6DeleteEm" => match p.repo_delete_key(r0, r1) {
            Ok(rc) => {
                p.cpu.regs[0] = rc;
                NativeOutcome::Return
            }
            Err(m) => refuse(m),
        },
        "_ZN11CRepositoryD1Ev" | "_ZN11CRepositoryD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() {
                    let _ = h.free(r0);
                }
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
