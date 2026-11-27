//! Host oracle tests for REUSE U6 (Symbian CenRep + `CDictionaryFileStore` backed by `guestsvc_abi::GuestPrefs`)
//! and REUSE U5 (`RTz` backed by CoralOS `"eka-tz"` launch record).
//!
//! Proves:
//! 1. `CRepository` `Set`/`Create`/`Delete`/`Get` (Int, Real, TDes16, TDes8) persist across full guest restart
//!    (fresh `Process` over the same `FakePrefs`), with ROM `.txt` defaults strictly read-only fallback.
//! 2. `CDictionaryFileStore` `WriteL`/`CommitL` -> `IsPresentL` -> `RDictionaryReadStream::OpenLC`/`ReadL` persists
//!    across full guest restart (zero persistent state inside the engine), and `RemoveL` deletes from `FakePrefs`.
//! 3. Negative controls: `FakePrefs::not_granted()` surfaces `KErrPermissionDenied` (-46) / named refusal and NEVER
//!    falls back to in-engine storage; oversized values (>1024 B) return `KErrOverflow` (-22); non-finite `TReal`
//!    returns `KErrArgument` (-6); `Lost`/`Stalled` return `KErrServerTerminated` (-15).
//! 4. `RTz::ConvertToLocalTime` / `ConvertToUniversalTime` apply the `"eka-tz"` launch record UTC offset, and
//!    malformed `"eka-tz"` records are rejected with `"eka-tz:"`.
use std::cell::RefCell;
use std::rc::Rc;

use guestsvc_abi::{FakePrefs, GuestPrefs, GuestSvcError, GuestValue};
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::drive::Drive;
use heirloom_ekaarm_engine::loader::{self, Process};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{cenrep, sym_tz};

/// Shared wrapper around `FakePrefs` so two independent guest `Process` instances (simulating relaunch)
/// talk to the exact same backing `prefs.service` store.
#[derive(Clone)]
struct SharedPrefs(Rc<RefCell<FakePrefs>>);

impl GuestPrefs for SharedPrefs {
    fn get(&mut self, key: &str) -> Result<Option<GuestValue>, GuestSvcError> {
        self.0.borrow_mut().get(key)
    }
    fn set(&mut self, key: &str, value: &GuestValue) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().set(key, value)
    }
    fn delete(&mut self, key: &str) -> Result<(), GuestSvcError> {
        self.0.borrow_mut().delete(key)
    }
    fn keys(&mut self, prefix: &str) -> Result<Vec<String>, GuestSvcError> {
        self.0.borrow_mut().keys(prefix)
    }
}

fn proc_with_stubs(names: &[&str]) -> Process {
    let mut all: Vec<&str> = names.to_vec();
    all.extend(["_ZN4User6AllocZEi", "_ZN4User4FreeEPv"]);
    let mut p = loader::bare(&all).unwrap();
    p.uid3 = 0x1028_28d6;
    p
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub"));
    for (i, &a) in args.iter().enumerate() {
        p.cpu.regs[i] = a;
    }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) {
        (NativeOutcome::Return, r) => r,
        (other, _) => panic!("{name}: {other:?}"),
    }
}

fn refused(p: &mut Process, name: &str, args: &[u32]) -> String {
    match call(p, name, args) {
        (NativeOutcome::Refused(m), _) => m,
        (other, _) => panic!("{name}: expected Refused, got {other:?}"),
    }
}

fn alloc(p: &mut Process, size: u32) -> u32 {
    ok(p, "_ZN4User6AllocZEi", &[size])
}

fn rd(p: &mut Process, addr: u32) -> u32 {
    p.mem.read_u32(addr).unwrap()
}

fn wr(p: &mut Process, addr: u32, v: u32) {
    p.mem.write_u32(addr, v).unwrap();
}

fn tptrc16(p: &mut Process, s: &str) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let buf = alloc(p, (units.len() as u32 * 2).max(2));
    for (i, u) in units.iter().enumerate() {
        p.mem.write_u16(buf + i as u32 * 2, *u).unwrap();
    }
    let des = alloc(p, 8);
    wr(p, des, (1 << 28) | units.len() as u32); // EPtrC
    wr(p, des + 4, buf);
    des
}

fn tbuf16(p: &mut Process, max_units: u32) -> u32 {
    let des = alloc(p, 8 + max_units * 2);
    wr(p, des, 3 << 28); // EBuf, len = 0
    wr(p, des + 4, max_units);
    des
}

fn tptrc8(p: &mut Process, bytes: &[u8]) -> u32 {
    let buf = alloc(p, (bytes.len() as u32).max(1));
    for (i, &b) in bytes.iter().enumerate() {
        p.mem.write_u8(buf + i as u32, b).unwrap();
    }
    let des = alloc(p, 8);
    wr(p, des, (1 << 28) | bytes.len() as u32);
    wr(p, des + 4, buf);
    des
}

fn tbuf8(p: &mut Process, max_bytes: u32) -> u32 {
    let des = alloc(p, 8 + max_bytes);
    wr(p, des, 3 << 28); // EBuf, len = 0
    wr(p, des + 4, max_bytes);
    des
}

fn read_des16(p: &Process, des: u32) -> String {
    let units = heirloom_ekaarm_engine::desc::read16(p, des).unwrap();
    heirloom_ekaarm_engine::desc::to_utf8(&units)
}

fn read_des8(p: &Process, des: u32) -> Vec<u8> {
    heirloom_ekaarm_engine::desc::read8(p, des).unwrap()
}

fn write_f64_guest(p: &mut Process, v: f64) -> u32 {
    let a = alloc(p, 8);
    let bits = v.to_bits();
    wr(p, a, bits as u32);
    wr(p, a + 4, (bits >> 32) as u32);
    a
}

fn read_f64_guest(p: &mut Process, a: u32) -> f64 {
    let lo = rd(p, a);
    let hi = rd(p, a + 4);
    f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
}

#[test]
fn cenrep_writes_persist_across_guest_relaunch_and_shadow_rom_defaults() {
    let stubs = &[
        "_ZN11CRepository4NewLE4TUid",
        "_ZN11CRepository3GetEmRi",
        "_ZN11CRepository3GetEmRd",
        "_ZN11CRepository3GetEmR6TDes16",
        "_ZN11CRepository3GetEmR5TDes8",
        "_ZN11CRepository3SetEmi",
        "_ZN11CRepository3SetEmRKd",
        "_ZN11CRepository3SetEmRK7TDesC16",
        "_ZN11CRepository3SetEmRK6TDesC8",
        "_ZN11CRepository6CreateEmi",
        "_ZN11CRepository6DeleteEm",
        "_ZN11CRepositoryD0Ev",
    ];
    let store = SharedPrefs(Rc::new(RefCell::new(FakePrefs::new())));

    // --- Launch 1: open ProfileEngine defaults, mutate keys of all 4 types, then drop Process completely ---
    {
        let mut p = proc_with_stubs(stubs);
        p.set_drive(Drive::new());
        p.prefs_host = Some(Box::new(store.clone()));

        let repo = ok(&mut p, "_ZN11CRepository4NewLE4TUid", &[cenrep::KCRUID_PROFILE_ENGINE]);
        let out_i = alloc(&mut p, 4);
        // Reads ROM platform default before any write.
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x7E00_0008, out_i]), 0);
        assert_eq!(rd(&mut p, out_i) as i32, 7);

        // Override existing int key via Set; Create on existing key fails with KErrAlreadyExists (-11).
        assert_eq!(ok(&mut p, "_ZN11CRepository6CreateEmi", &[repo, 0x7E00_0008, 3]) as i32, -11);
        assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmi", &[repo, 0x7E00_0008, 4]), 0);

        // Create a brand new dynamic int key, real key, string key, and binary key.
        assert_eq!(ok(&mut p, "_ZN11CRepository6CreateEmi", &[repo, 0x0000_0099, -123i32 as u32]), 0);
        let r_in = write_f64_guest(&mut p, 12.25);
        assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmRKd", &[repo, 0x0000_009A, r_in]), 0);
        let s_in = tptrc16(&mut p, "Oslo-Ringtone");
        assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmRK7TDesC16", &[repo, 0x0000_009B, s_in]), 0);
        let b_in = tptrc8(&mut p, &[0xCA, 0xFE, 0xBA, 0xBE]);
        assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmRK6TDesC8", &[repo, 0x0000_009C, b_in]), 0);
    }

    // Verify exact keys landed in prefs.service (never private engine RAM).
    assert_eq!(
        store.0.borrow().entries().get("cenrep/101f8798/7e000008"),
        Some(&GuestValue::Int(4))
    );
    assert_eq!(
        store.0.borrow().entries().get("cenrep/101f8798/0000009b"),
        Some(&GuestValue::Text("Oslo-Ringtone".into()))
    );

    // --- Launch 2: completely fresh Process over the same FakePrefs sees every persisted value ---
    {
        let mut p = proc_with_stubs(stubs);
        p.set_drive(Drive::new());
        p.prefs_host = Some(Box::new(store.clone()));

        let repo = ok(&mut p, "_ZN11CRepository4NewLE4TUid", &[cenrep::KCRUID_PROFILE_ENGINE]);
        let out_i = alloc(&mut p, 4);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x7E00_0008, out_i]), 0);
        assert_eq!(rd(&mut p, out_i) as i32, 4, "persisted override survived guest restart");

        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x0000_0099, out_i]), 0);
        assert_eq!(rd(&mut p, out_i) as i32, -123);

        let out_r = alloc(&mut p, 8);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRd", &[repo, 0x0000_009A, out_r]), 0);
        assert_eq!(read_f64_guest(&mut p, out_r), 12.25);

        let out_s = tbuf16(&mut p, 32);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmR6TDes16", &[repo, 0x0000_009B, out_s]), 0);
        assert_eq!(read_des16(&p, out_s), "Oslo-Ringtone");

        let out_b = tbuf8(&mut p, 16);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmR5TDes8", &[repo, 0x0000_009C, out_b]), 0);
        assert_eq!(read_des8(&p, out_b), vec![0xCA, 0xFE, 0xBA, 0xBE]);

        // Delete removes override from prefs.service (restoring ROM default 7 for 0x7E00_0008 and removing 0x99).
        assert_eq!(ok(&mut p, "_ZN11CRepository6DeleteEm", &[repo, 0x7E00_0008]), 0);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x7E00_0008, out_i]), 0);
        assert_eq!(rd(&mut p, out_i) as i32, 7);

        assert_eq!(ok(&mut p, "_ZN11CRepository6DeleteEm", &[repo, 0x0000_0099]), 0);
        assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x0000_0099, out_i]) as i32, -1);
    }
}

#[test]
fn cenrep_negative_controls_not_granted_too_large_nan_and_lost() {
    let stubs = &[
        "_ZN11CRepository4NewLE4TUid",
        "_ZN11CRepository3GetEmRi",
        "_ZN11CRepository3SetEmi",
        "_ZN11CRepository3SetEmRKd",
        "_ZN11CRepository3SetEmRK6TDesC8",
        "_ZN11CRepository6DeleteEm",
    ];
    let denied = SharedPrefs(Rc::new(RefCell::new(FakePrefs::not_granted())));
    let mut p = proc_with_stubs(stubs);
    p.set_drive(Drive::new());
    p.prefs_host = Some(Box::new(denied.clone()));

    let repo = ok(&mut p, "_ZN11CRepository4NewLE4TUid", &[cenrep::KCRUID_PROFILE_ENGINE]);
    let out_i = alloc(&mut p, 4);

    // Static ROM default remains readable without a prefs grant, but an unknown key returns KErrPermissionDenied (-46).
    assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x7E00_0008, out_i]), 0);
    assert_eq!(rd(&mut p, out_i) as i32, 7);
    assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x0000_9999, out_i]) as i32, -46);

    // All mutations return KErrPermissionDenied (-46) and never fall back to silent in-engine storage.
    assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmi", &[repo, 0x7E00_0008, 1]) as i32, -46);
    assert_eq!(ok(&mut p, "_ZN11CRepository6DeleteEm", &[repo, 0x7E00_0008]) as i32, -46);
    assert!(denied.0.borrow().entries().is_empty());
    // Reading back still yields unmodified ROM default 7, proving nothing was secretly mutated in RAM.
    assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 0x7E00_0008, out_i]), 0);
    assert_eq!(rd(&mut p, out_i) as i32, 7);

    // Grant access, then test TooLarge (>1024 B -> KErrOverflow -22), Real(NaN) (-> KErrArgument -6), and Lost (-> -15).
    denied.0.borrow_mut().set_granted(true);
    let big = tptrc8(&mut p, &vec![0x5Au8; guestsvc_abi::VALUE_MAX + 1]);
    assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmRK6TDesC8", &[repo, 1, big]) as i32, -22);
    let nan = write_f64_guest(&mut p, f64::NAN);
    assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmRKd", &[repo, 2, nan]) as i32, -6);
    denied.0.borrow_mut().set_injected_error(Some(GuestSvcError::Lost));
    assert_eq!(ok(&mut p, "_ZN11CRepository3SetEmi", &[repo, 3, 9]) as i32, -15);
    denied.0.borrow_mut().set_injected_error(Some(GuestSvcError::Stalled));
    assert_eq!(ok(&mut p, "_ZN11CRepository3GetEmRi", &[repo, 3, out_i]) as i32, -15);
}

#[test]
fn dictionary_store_persists_streams_across_guest_relaunch_and_enforces_not_granted() {
    let stubs = &[
        "_ZN3RFs7ConnectEi",
        "_ZN3RFs19SetSessionToPrivateEi",
        "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid",
        "_ZNK16CDictionaryStore10IsPresentLE4TUid",
        "_ZN16CDictionaryStore7RemoveLE4TUid",
        "_ZN16CDictionaryStore7CommitLEv",
        "_ZN22RDictionaryWriteStream8AssignLCER16CDictionaryStore4TUid",
        "_ZN12RWriteStream6WriteLERK6TDesC8",
        "_ZN12RWriteStream12WriteUint32LEm",
        "_ZN12RWriteStream7CommitLEv",
        "_ZN12RWriteStream5CloseEv",
        "_ZN21RDictionaryReadStream6OpenLCERK16CDictionaryStore4TUid",
        "_ZN11RReadStream5ReadLER5TDes8i",
        "_ZN11RReadStream11ReadUint32LEv",
        "_ZN11RReadStream5CloseEv",
    ];
    let store_uid = 0x1028_28d6u32;
    let stream_uid = 0x0000_4242u32;
    let shared = SharedPrefs(Rc::new(RefCell::new(FakePrefs::new())));

    // --- Launch 1: write stream + CommitL, drop Process completely ---
    {
        let mut p = proc_with_stubs(stubs);
        p.set_drive(Drive::new());
        p.prefs_host = Some(Box::new(shared.clone()));
        let fs = alloc(&mut p, 8);
        ok(&mut p, "_ZN3RFs7ConnectEi", &[fs, 0xFFFF_FFFF]);
        ok(&mut p, "_ZN3RFs19SetSessionToPrivateEi", &[fs, 2]);
        let name = tptrc16(&mut p, "filebrowser.ini");
        let dict = ok(&mut p, "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid", &[fs, name, store_uid]);
        assert_eq!(ok(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]), 0);

        let ws = alloc(&mut p, 16);
        ok(&mut p, "_ZN22RDictionaryWriteStream8AssignLCER16CDictionaryStore4TUid", &[ws, dict, stream_uid]);
        let payload = tptrc8(&mut p, b"coral-prefs");
        ok(&mut p, "_ZN12RWriteStream6WriteLERK6TDesC8", &[ws, payload]);
        ok(&mut p, "_ZN12RWriteStream12WriteUint32LEm", &[ws, 0x1234_5678]);
        // Before CommitL, nothing is persisted yet.
        assert_eq!(ok(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]), 0);
        ok(&mut p, "_ZN12RWriteStream7CommitLEv", &[ws]);
        ok(&mut p, "_ZN12RWriteStream5CloseEv", &[ws]);
        assert_eq!(ok(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]), 1);
    }

    // Verify key exists in FakePrefs under dict/<store_uid>/<stream_uid>.
    assert!(shared.0.borrow().entries().contains_key("dict/102828d6/00004242"));

    // --- Launch 2: fresh Process opens store, finds stream present, reads exact bytes, then removes it ---
    {
        let mut p = proc_with_stubs(stubs);
        p.set_drive(Drive::new());
        p.prefs_host = Some(Box::new(shared.clone()));
        let fs = alloc(&mut p, 8);
        ok(&mut p, "_ZN3RFs7ConnectEi", &[fs, 0xFFFF_FFFF]);
        ok(&mut p, "_ZN3RFs19SetSessionToPrivateEi", &[fs, 2]);
        let name = tptrc16(&mut p, "filebrowser.ini");
        let dict = ok(&mut p, "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid", &[fs, name, store_uid]);
        assert_eq!(ok(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]), 1);

        let rs = alloc(&mut p, 16);
        ok(&mut p, "_ZN21RDictionaryReadStream6OpenLCERK16CDictionaryStore4TUid", &[rs, dict, stream_uid]);
        let dst = tbuf8(&mut p, 32);
        ok(&mut p, "_ZN11RReadStream5ReadLER5TDes8i", &[rs, dst, 11]);
        assert_eq!(read_des8(&p, dst), b"coral-prefs");
        let tail = ok(&mut p, "_ZN11RReadStream11ReadUint32LEv", &[rs]);
        assert_eq!(tail, 0x1234_5678);
        ok(&mut p, "_ZN11RReadStream5CloseEv", &[rs]);

        ok(&mut p, "_ZN16CDictionaryStore7RemoveLE4TUid", &[dict, stream_uid]);
        assert_eq!(ok(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]), 0);
    }

    // --- Negative control: NotGranted refuses IsPresentL and CommitL cleanly, zero entries stored ---
    {
        let denied = SharedPrefs(Rc::new(RefCell::new(FakePrefs::not_granted())));
        let mut p = proc_with_stubs(stubs);
        p.set_drive(Drive::new());
        p.prefs_host = Some(Box::new(denied.clone()));
        let fs = alloc(&mut p, 8);
        ok(&mut p, "_ZN3RFs7ConnectEi", &[fs, 0xFFFF_FFFF]);
        ok(&mut p, "_ZN3RFs19SetSessionToPrivateEi", &[fs, 2]);
        let name = tptrc16(&mut p, "denied.ini");
        let dict = ok(&mut p, "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid", &[fs, name, store_uid]);
        let m_pres = refused(&mut p, "_ZNK16CDictionaryStore10IsPresentLE4TUid", &[dict, stream_uid]);
        assert!(m_pres.contains("not-granted") && m_pres.contains("-46"), "{m_pres}");

        let ws = alloc(&mut p, 16);
        ok(&mut p, "_ZN22RDictionaryWriteStream8AssignLCER16CDictionaryStore4TUid", &[ws, dict, stream_uid]);
        let payload = tptrc8(&mut p, b"secret");
        ok(&mut p, "_ZN12RWriteStream6WriteLERK6TDesC8", &[ws, payload]);
        let m_commit = refused(&mut p, "_ZN12RWriteStream7CommitLEv", &[ws]);
        assert!(m_commit.contains("not-granted") && m_commit.contains("-46"), "{m_commit}");
        assert!(denied.0.borrow().entries().is_empty());
    }
}

#[test]
fn rtz_applies_launch_record_offset_and_rejects_malformed_records() {
    // Positive parse & round-trip (UTC+2 CEST = +7200 s).
    let tz = sym_tz::parse_launch_record(b"v1\tEurope/Oslo\t7200\t1\n").unwrap();
    assert_eq!(tz.iana, "Europe/Oslo");
    assert_eq!(tz.offset_secs, 7200);
    assert!(tz.is_dst);

    // Negative controls: malformed records fail with "eka-tz:" prefix (required by engine-rt probe).
    for bad in [&b"\x00"[..], b"v2\tUTC\t0\t0\n", b"v1\t\t0\t0\n", b"v1\tUTC\tabc\t0\n", b"v1\tUTC\t99999\t0\n", b"v1\tUTC\t0\t2\n"] {
        let err = sym_tz::parse_launch_record(bad).unwrap_err();
        assert!(err.starts_with("eka-tz:"), "bad={bad:?} err={err:?}");
    }

    let stubs = &[
        "_ZN3RTzC1Ev",
        "_ZN3RTz7ConnectEv",
        "_ZNK3RTz18ConvertToLocalTimeER5TTime",
        "_ZNK3RTz22ConvertToUniversalTimeER5TTime",
        "_ZN3RTz5CloseEv",
    ];
    let mut p = proc_with_stubs(stubs);
    p.host_tz = tz;
    let rtz = alloc(&mut p, 8);
    ok(&mut p, "_ZN3RTzC1Ev", &[rtz]);
    assert_eq!(ok(&mut p, "_ZN3RTz7ConnectEv", &[rtz]), 0);

    // 100_000_000_000 us UTC -> +7_200_000_000 us in Europe/Oslo CEST, then back to UTC.
    let base_us: i64 = 63_000_000_000_000_000;
    let ttime = alloc(&mut p, 8);
    wr(&mut p, ttime, base_us as u32);
    wr(&mut p, ttime + 4, (base_us >> 32) as u32);

    assert_eq!(ok(&mut p, "_ZNK3RTz18ConvertToLocalTimeER5TTime", &[rtz, ttime]), 0);
    let local_us = ((rd(&mut p, ttime + 4) as i32 as i64) << 32) | (rd(&mut p, ttime) as i64);
    assert_eq!(local_us, base_us + 7_200_000_000);

    assert_eq!(ok(&mut p, "_ZNK3RTz22ConvertToUniversalTimeER5TTime", &[rtz, ttime]), 0);
    let back_us = ((rd(&mut p, ttime + 4) as i32 as i64) << 32) | (rd(&mut p, ttime) as i64);
    assert_eq!(back_us, base_us);
}
