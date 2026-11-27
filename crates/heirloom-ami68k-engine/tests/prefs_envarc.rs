//! Host oracle tests for REUSE U7 (Amiga `ENV:` / `ENVARC:` backed by `guestsvc_abi::GuestPrefs`).
//!
//! Proves:
//! 1. Running the real 68k `vars.bin` guest fixture with `FakePrefs` attached persists `ENVARC:G`
//!    into `prefs.service` (`envarc/g`), NOT private RAM `ENVARC:`, and a completely fresh `Amiga`
//!    relaunch over the same `FakePrefs` automatically seeds `ENV:G` from `prefs.service`.
//! 2. `envarc_save` / `envarc_load` / `seed_env_from_prefs` handle nested paths (`Zune/DOpus.cfg`)
//!    case-insensitively across restarts.
//! 3. Negative controls: `FakePrefs::not_granted()` fails `SetVar(GVF_SAVE_VAR)` cleanly with
//!    `ERROR_WRITE_PROTECTED` (223) and writes NOTHING to `FakePrefs` or RAM `ENVARC:`; oversized
//!    values (>1024 B) return `ERROR_LINE_TOO_LONG` (120); invalid names (`../escape`) return
//!    `ERROR_INVALID_COMPONENT_NAME` (210); `Lost`/`Stalled` return `ERROR_DEVICE_NOT_MOUNTED` (218).
mod common;
use std::cell::RefCell;
use std::rc::Rc;

use common::*;
use guestsvc_abi::{FakePrefs, GuestPrefs, GuestSvcError, GuestValue};
use heirloom_ami68k_engine::dos_err::*;
use heirloom_ami68k_engine::dos_vars::{envarc_key, envarc_load, envarc_save};

const VARS_PROG: &[u8] = include_bytes!("guest/vars.bin");
const NEG1: u32 = u32::MAX;

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

#[test]
fn envarc_persists_across_amiga_relaunch_and_seeds_env_on_boot() {
    let shared = SharedPrefs(Rc::new(RefCell::new(FakePrefs::new())));

    // Launch 1: real 68k vars.bin sets global G with GVF_SAVE_VAR, then deletes ENV:G (leaving ENVARC:G saved).
    {
        let mut a = machine();
        a.set_prefs_host(Box::new(shared.clone()));
        let r = run(&mut a, VARS_PROG);
        assert_eq!((r[15], r[16]), (NEG1, 4), "SetVar GVF_GLOBAL_ONLY|GVF_SAVE_VAR succeeded");
        assert_eq!((r[39], r[40]), (NEG1, NEG1), "DeleteVar GLOBAL_ONLY removed ENV:G without resurrecting from ENVARC:");
        assert!(a.fs.vol.resolve(0, "ENV:G").is_err(), "ENV:G deleted from live session");
        assert!(a.fs.vol.resolve(0, "ENVARC:G").is_err(), "no private RAM ENVARC:G copy when prefs_host is attached");
    }

    // Verify ENVARC:G landed in CoralOS prefs.service under envarc/g.
    assert_eq!(
        shared.0.borrow().entries().get("envarc/g"),
        Some(&GuestValue::Bytes(b"glob".to_vec()))
    );

    // Also persist a nested MUI config key (Zune/DOpus.cfg) in prefs.service.
    {
        let mut a = machine();
        a.set_prefs_host(Box::new(shared.clone()));
        envarc_save(&mut a, "Zune/DOpus.cfg", b"FORM1234").unwrap();
    }

    // Launch 2: brand-new Amiga instance over the same FakePrefs automatically seeds ENV: from envarc/.
    {
        let mut a2 = machine();
        a2.set_prefs_host(Box::new(shared.clone()));
        let n_g = a2.fs.vol.resolve(0, "ENV:G").expect("ENV:G seeded from prefs.service on boot");
        assert_eq!(a2.fs.vol.file_data(n_g).unwrap(), b"glob");
        let n_zune = a2.fs.vol.resolve(0, "ENV:Zune/DOpus.cfg").expect("nested ENV:Zune/DOpus.cfg seeded on boot");
        assert_eq!(a2.fs.vol.file_data(n_zune).unwrap(), b"FORM1234");
        assert_eq!(envarc_load(&mut a2, "Zune/DOpus.cfg").unwrap(), Some(b"FORM1234".to_vec()));
    }
}

#[test]
fn envarc_negative_controls_not_granted_too_large_bad_name_and_lost() {
    // 1. NotGranted: guest vars.bin SetVar(GVF_GLOBAL_ONLY|GVF_SAVE_VAR) MUST fail cleanly (returns 0, not DOSTRUE)
    // and never fall back to RAM ENVARC: or write to FakePrefs.
    let denied = SharedPrefs(Rc::new(RefCell::new(FakePrefs::not_granted())));
    {
        let mut a = machine();
        a.set_prefs_host(Box::new(denied.clone()));
        let r = run(&mut a, VARS_PROG);
        assert_eq!(r[15], 0, "SetVar GVF_SAVE_VAR must fail (DOSFALSE=0) when prefs is NotGranted");
        assert!(denied.0.borrow().entries().is_empty(), "nothing stored in FakePrefs when NotGranted");
        assert!(a.fs.vol.resolve(0, "ENVARC:G").is_err(), "no silent fallback into RAM ENVARC:");
        assert_eq!(envarc_save(&mut a, "foo", b"bar"), Err(ERROR_WRITE_PROTECTED));
        assert_eq!(envarc_load(&mut a, "foo"), Err(ERROR_WRITE_PROTECTED));
    }

    // 2. Bad names, oversized payloads, and Lost/Stalled errors map to named AmigaDOS errors.
    let store = SharedPrefs(Rc::new(RefCell::new(FakePrefs::new())));
    let mut a = machine();
    a.set_prefs_host(Box::new(store.clone()));

    for bad in ["", "..", "../escape", "a/../b", "has space", "a//b"] {
        assert_eq!(envarc_key(bad), Err(ERROR_INVALID_COMPONENT_NAME), "bad={bad:?}");
        assert_eq!(envarc_save(&mut a, bad, b"x"), Err(ERROR_INVALID_COMPONENT_NAME));
    }
    let big = vec![0x41u8; guestsvc_abi::VALUE_MAX + 1];
    assert_eq!(envarc_save(&mut a, "toolong", &big), Err(ERROR_LINE_TOO_LONG));

    store.0.borrow_mut().set_injected_error(Some(GuestSvcError::Lost));
    assert_eq!(envarc_save(&mut a, "lostkey", b"x"), Err(ERROR_DEVICE_NOT_MOUNTED));
    store.0.borrow_mut().set_injected_error(Some(GuestSvcError::Stalled));
    assert_eq!(envarc_load(&mut a, "lostkey"), Err(ERROR_DEVICE_NOT_MOUNTED));
    assert!(store.0.borrow().entries().is_empty());
}
