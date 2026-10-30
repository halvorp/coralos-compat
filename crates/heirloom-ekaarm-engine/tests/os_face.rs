//! os-eka.library's table (feature `os-library`): the Nokia File Browser run DIRECTLY on a recording UI and run THROUGH the table -
//! Control frames in (the Z volume, the three named inputs, the program), UI frames + Reports out through an OsHost, decoded by
//! WireHost onto a recording UI - makes the SAME UI calls, and the table refuses what it must by name.
//! Run: cargo test -p heirloom-ekaarm-engine --features os-library --test os_face
//! The File Browser case reads owner-local test input (the corpus, the SDK ROM, the layout data) and SKIPs when it is absent.
#![cfg(feature = "os-library")]

use core::ffi::c_void;
use heirloom_common::{GuestMachine, Step};
use heirloom_ekaarm_engine::os_face::os_vtable;
use heirloom_ekaarm_engine::romtree::TreeRom;
use heirloom_ekaarm_engine::EkaArm;
use heirloom_os_abi::*;
use heirloom_ui_abi::wire::host::WireHost;
use heirloom_ui_abi::wire::session::inputs::{EKA_LAYOUT, EKA_ORDINALS, EKA_ROM};
use heirloom_ui_abi::wire::{decode, encode_control, Control, Frame, Report, WireStep};
use heirloom_ui_abi::*;
use heirloom_volume::Tree;
use std::path::Path;
use std::sync::Once;

const DRIVE: &str = "/data/build/corpus-symbian/drive/nokiafilebrowser";
const ZROOT: &str = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb/z";
const ROM_LIST: &str = "/build/patches/ekarom/filebrowser.txt";
const LAYOUT: &str = "/build/patches/ekarom/qhd.cdldata";
const BUDGET: u64 = 1_000_000;
/// The host's Down key (CoralOS key-wire WIRE_DOWN; the engine maps it to EStdKeyDownArrow, keymap.rs).
const HOST_KEY_DOWN: u32 = 0x0E;
const ORDINALS: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");

fn bind() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert_eq!((os_vtable().bind_cpu)(heirloom_cpu_arm::vtable()), BIND_OK));
}

#[derive(Default)]
struct Sink {
    frames: Vec<Vec<u8>>,
    canvases: Vec<Vec<u32>>,
}

extern "C" fn emit(ctx: *mut c_void, frame: *const u8, len: usize) {
    let sink = unsafe { &mut *(ctx as *mut Sink) };
    sink.frames.push(unsafe { std::slice::from_raw_parts(frame, len) }.to_vec());
}

extern "C" fn lease(ctx: *mut c_void, _win: u32, _w: i32, _h: i32, pixels: *const u32, count: usize) -> u32 {
    let sink = unsafe { &mut *(ctx as *mut Sink) };
    sink.canvases.push(unsafe { std::slice::from_raw_parts(pixels, count) }.to_vec());
    sink.canvases.len() as u32
}

struct Remote {
    inst: *mut OsInstance,
    sink: Box<Sink>,
    host: WireHost<RecordingGuestUi>,
    reports: Vec<Report>,
}

impl Remote {
    fn new(profile: &str) -> Result<Remote, String> {
        let mut sink = Box::new(Sink::default());
        let host = OsHost { ctx: &mut *sink as *mut Sink as *mut c_void, emit, lease_canvas: lease, read_stream: no_streams };
        let (mut why, mut why_len) = ([0u8; 256], 0usize);
        let inst = (os_vtable().create)(profile.as_ptr(), profile.len(), &host, why.as_mut_ptr(), why.len(), &mut why_len);
        if inst.is_null() {
            return Err(String::from_utf8_lossy(&why[..why_len]).into_owned());
        }
        Ok(Remote { inst, sink, host: WireHost::new(RecordingGuestUi::new()), reports: Vec::new() })
    }

    fn send(&mut self, frames: &[Vec<u8>]) -> i32 {
        let batch: Vec<u8> = frames.concat();
        let rc = (os_vtable().apply)(self.inst, batch.as_ptr(), batch.len());
        for f in std::mem::take(&mut self.sink.frames) {
            match decode(&f).expect("the library emits only frames that decode").0 {
                Frame::Report(r) => self.reports.push(r),
                Frame::Lease { region, .. } => {
                    self.host.store_canvas(region, self.sink.canvases[region as usize - 1].clone());
                    self.host.apply(&f).unwrap();
                }
                _ => self.host.apply(&f).unwrap(),
            }
        }
        rc
    }

    fn last_step(&self) -> Option<&WireStep> {
        self.reports.iter().rev().find_map(|r| if let Report::Stepped { step, .. } = r { Some(step) } else { None })
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        (os_vtable().destroy)(self.inst);
    }
}

fn control(c: Control) -> Vec<u8> {
    let mut b = Vec::new();
    encode_control(&c, &mut b).unwrap();
    b
}

const CHUNK: usize = 256 * 1024;

fn blob(bytes: &[u8], mut make: impl FnMut(u32, u32, Vec<u8>) -> Control) -> Vec<Vec<u8>> {
    if bytes.is_empty() {
        return vec![control(make(0, 0, Vec::new()))];
    }
    bytes.chunks(CHUNK).enumerate().map(|(i, c)| control(make((i * CHUNK) as u32, bytes.len() as u32, c.to_vec()))).collect()
}

fn input(name: &str, bytes: &[u8]) -> Vec<Vec<u8>> {
    blob(bytes, |offset, total, chunk| Control::Input { name: name.into(), offset, total, chunk })
}

fn fill(tree: &mut Tree, vol: usize, dir: &Path, guest: &[String]) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().into_string().unwrap();
        let mut path = guest.to_vec();
        if e.file_type().unwrap().is_dir() {
            path.push(name);
            let d = tree.mkdirs(vol, path.iter().map(|s| s.as_str())).unwrap();
            let _ = d;
            fill(tree, vol, &e.path(), &path);
        } else {
            let d = tree.mkdirs(vol, path.iter().map(|s| s.as_str())).unwrap();
            tree.put_file(d, &name, std::fs::read(e.path()).unwrap()).unwrap();
        }
    }
}

/// The staged ROM subset (the stage recipe's list: one ROM-relative path per line, a trailing / = a directory) as one volume.
fn rom_snapshot() -> Vec<u8> {
    let mut t = Tree::new(255);
    let v = t.add_volume("ROM", false);
    for rel in std::fs::read_to_string(ROM_LIST).unwrap().lines().map(str::trim).filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
        if rel.ends_with('/') {
            t.mkdirs(v, parts.iter().copied()).unwrap();
        } else {
            let (leaf, dirs) = parts.split_last().unwrap();
            let d = t.mkdirs(v, dirs.iter().copied()).unwrap();
            t.put_file(d, leaf, std::fs::read(format!("{ZROOT}/{rel}")).unwrap()).unwrap();
        }
    }
    heirloom_volume::snapshot::encode(&t)
}

fn corpus_present() -> bool {
    [DRIVE, ZROOT, ROM_LIST, LAYOUT].iter().all(|p| Path::new(p).exists())
}

#[test]
fn the_table_is_the_symbian_family_at_the_frozen_version() {
    let t = os_vtable();
    assert_eq!((t.version, t.family, t.size as usize), (OS_ABI_VERSION, FAMILY_EKAARM, core::mem::size_of::<OsVTable>()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_EKAARM) }, Ok(()));
}

#[test]
fn the_file_browser_makes_the_same_ui_calls_directly_and_through_the_table() {
    if !corpus_present() {
        println!("SKIP: the Symbian corpus / SDK ROM / layout data is absent");
        return;
    }
    bind();
    let image = std::fs::read(format!("{DRIVE}/sys/bin/filebrowser.exe")).unwrap();
    let layout = std::fs::read(LAYOUT).unwrap();
    let rom = rom_snapshot();
    let mut z = Tree::new(255);
    let zv = z.add_volume("Z", false);
    fill(&mut z, zv, Path::new(DRIVE), &[]);
    let z_snap = heirloom_volume::snapshot::encode(&z);

    // Direct.
    let mut direct = EkaArm::new(RecordingGuestUi::new(), heirloom_fmt_e32::OrdinalTable::parse(ORDINALS).unwrap());
    direct.set_rom(Box::new(TreeRom::from_snapshot(&rom).unwrap()));
    direct.set_cdl_data(layout.clone());
    heirloom_volume::snapshot::merge(direct.volume(), &heirloom_volume::snapshot::decode(&z_snap).unwrap()).unwrap();
    direct.start_file(&image, "sys\\bin\\filebrowser.exe").unwrap();
    let mut steps = Vec::new();
    for _ in 0..200 {
        let (s, _) = direct.step(BUDGET);
        let done = s != Step::BudgetSpent;
        steps.push(s);
        if done {
            break;
        }
    }
    let opened = direct.ui().calls.iter().any(|c| matches!(c, UiCall::OpenWindow(_)));
    assert!(opened, "the File Browser opens its window directly (steps {steps:?})");
    // The guest's first window as the recording UI named it, and a host Down key (press + release) delivered to it.
    let win = UiObjectRef { id: 1, generation: 1 };
    let key = |down| GuestUiEvent::Key { win, code: HOST_KEY_DOWN, text: String::new(), modifiers: Modifiers::default(), down };
    let before_key = direct.ui().calls.len();
    direct.deliver(key(true)).unwrap();
    direct.deliver(key(false)).unwrap();
    let after_key = direct.step(BUDGET).0;
    assert!(direct.ui().calls.len() > before_key, "Down changed the File Browser's list directly");

    // Through the table.
    let mut remote = Remote::new("").unwrap();
    let mut setup = blob(&z_snap, |offset, total, chunk| Control::Volume { offset, total, chunk });
    setup.extend(input(EKA_ORDINALS, ORDINALS.as_bytes()));
    setup.extend(input(EKA_LAYOUT, &layout));
    setup.extend(input(EKA_ROM, &rom));
    setup.extend(blob(&image, |offset, total, chunk| Control::Program { offset, total, chunk }));
    setup.push(control(Control::StartFile { name: "sys\\bin\\filebrowser.exe".into() }));
    assert_eq!(remote.send(&setup), APPLY_OK);
    assert_eq!(remote.reports, vec![Report::Started(Ok(()))]);
    for _ in 0..steps.len() {
        assert_eq!(remote.send(&[control(Control::Step { budget: BUDGET })]), APPLY_OK);
    }
    let (press, release) = (remote.host.encode_host_event(key(true)), remote.host.encode_host_event(key(false)));
    assert_eq!(remote.send(&[press, release, control(Control::Step { budget: BUDGET })]), APPLY_OK);
    println!("file browser: {} steps, last {:?}, {} UI calls", steps.len(), steps.last(), direct.ui().calls.len());
    assert_eq!(remote.host.ui().calls, direct.ui().calls, "the table path made exactly the direct path's calls");
    steps.push(after_key);
    let last = match steps.last().unwrap() {
        Step::Exited => WireStep::Exited,
        Step::Blocked => WireStep::Blocked,
        Step::BudgetSpent => WireStep::BudgetSpent,
        Step::Fault(f) => WireStep::Fault(f.clone()),
    };
    assert_eq!(remote.last_step(), Some(&last));
}

#[test]
fn a_program_started_before_its_ordinal_table_is_refused_by_name() {
    bind();
    let mut remote = Remote::new("").unwrap();
    let mut frames = blob(b"not an E32 image", |offset, total, chunk| Control::Program { offset, total, chunk });
    frames.push(control(Control::StartFile { name: "x.exe".into() }));
    assert_eq!(remote.send(&frames), APPLY_OK);
    match &remote.reports[..] {
        [Report::Started(Err(why))] => assert!(why.contains("no ordinal table"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_input_a_bad_ordinal_table_and_an_empty_layout_end_the_guest_by_name() {
    bind();
    for (name, bytes, want) in [
        ("fonts", &b"x"[..], "takes \"eka-ordinals\""),
        (EKA_ORDINALS, &b"only\ttwo"[..], "line 1"),
        (EKA_LAYOUT, &b""[..], "layout data is empty"),
        (EKA_ROM, &b"not a snapshot"[..], "ROM snapshot is refused"),
    ] {
        let mut remote = Remote::new("").unwrap();
        let mut frames = input(name, bytes);
        frames.push(control(Control::Step { budget: 1 }));
        assert_eq!(remote.send(&frames), APPLY_OK);
        match remote.last_step() {
            Some(WireStep::Fault(why)) => assert!(why.contains(want), "{name}: {why}"),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn an_input_interrupted_by_another_is_refused_by_name() {
    bind();
    let mut remote = Remote::new("").unwrap();
    let frames = vec![
        control(Control::Input { name: EKA_LAYOUT.into(), offset: 0, total: 10, chunk: vec![1; 4] }),
        control(Control::Input { name: EKA_ROM.into(), offset: 0, total: 10, chunk: vec![1; 4] }),
        control(Control::Step { budget: 1 }),
    ];
    assert_eq!(remote.send(&frames), APPLY_OK);
    match remote.last_step() {
        Some(WireStep::Fault(why)) => assert!(why.contains("still arriving"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_profile_is_refused_by_name_and_makes_no_machine() {
    bind();
    let err = Remote::new("s60v3").err().expect("refused");
    assert!(err.contains("s60v3"), "{err}");
}

#[test]
fn a_cpu_table_of_another_architecture_is_refused() {
    let real = heirloom_cpu_arm::vtable();
    let m68k: &'static CpuVTable = Box::leak(Box::new(CpuVTable {
        version: real.version, size: real.size, arch_id: heirloom_cpu_abi::ARCH_M68K, create: real.create, destroy: real.destroy,
        reset: real.reset, run: real.run, interrupt: real.interrupt, get_reg: real.get_reg, set_reg: real.set_reg,
        invalidate_code: real.invalidate_code, context_size: real.context_size, save_context: real.save_context,
        load_context: real.load_context, recent_pcs: real.recent_pcs,
    }));
    assert_eq!((os_vtable().bind_cpu)(m68k), BIND_REFUSED);
    assert_eq!((os_vtable().bind_cpu)(core::ptr::null()), BIND_REFUSED);
}
