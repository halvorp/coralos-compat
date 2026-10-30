//! os-ami.library's table (feature `os-library`): the Amiga fixture hunk run DIRECTLY on a recording UI and run THROUGH the table -
//! Control frames in, UI frames + Reports out through an OsHost, decoded by WireHost onto a recording UI - makes the SAME UI calls,
//! the Amiga-only controls (assigns, MUI's host profile) reach the machine, and the table refuses what it must by name.
//! Run: cargo test -p heirloom-ami68k-engine --features os-library --test os_face
#![cfg(feature = "os-library")]

use core::ffi::c_void;
use heirloom_ami68k_engine::os_face::{metrics_of, AmiFamily};
use heirloom_ami68k_engine::{fixture, os_vtable, Amiga};
use heirloom_common::GuestMachine;
use heirloom_os_abi::serve::{FamilyHooks, HostTransport, OsFamily, Session};
use heirloom_os_abi::*;
use heirloom_ui_abi::wire::host::WireHost;
use heirloom_ui_abi::wire::proxy::ProxyUi;
use heirloom_ui_abi::wire::{decode, encode_control, Control, Frame, HostProfile, Report, WireStep};
use heirloom_ui_abi::*;
use std::sync::Once;

const BUDGET: u64 = 100_000;

fn bind() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert_eq!((os_vtable().bind_cpu)(heirloom_cpu_68k::vtable()), BIND_OK));
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
        let (mut why, mut n) = ([0u8; 256], 0usize);
        let inst = (os_vtable().create)(profile.as_ptr(), profile.len(), &host, why.as_mut_ptr(), why.len(), &mut n);
        if inst.is_null() {
            return Err(String::from_utf8_lossy(&why[..n]).into_owned());
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

fn program(file: &[u8], name: &str) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for (i, chunk) in file.chunks(100).enumerate() {
        out.push(control(Control::Program { offset: (i * 100) as u32, total: file.len() as u32, chunk: chunk.to_vec() }));
    }
    out.push(control(Control::StartFile { name: name.into() }));
    out
}

/// A session around a machine made the way the table makes one, so a test can read the machine after a control.
fn session() -> Session<Amiga<heirloom_os_abi::serve::LibUi>> {
    bind();
    let host = OsHost { ctx: core::ptr::null_mut(), emit: noop_emit, lease_canvas: noop_lease, read_stream: no_streams };
    let m = AmiFamily::create("", ProxyUi::new(HostTransport::new(host))).unwrap();
    Session::new(m, FamilyHooks::of::<AmiFamily>())
}
extern "C" fn noop_emit(_: *mut c_void, _: *const u8, _: usize) {}
extern "C" fn noop_lease(_: *mut c_void, _: u32, _: i32, _: i32, _: *const u32, _: usize) -> u32 { 0 }

#[test]
fn the_table_is_the_ami_68k_family_at_the_frozen_version() {
    let t = os_vtable();
    assert_eq!((t.version, t.family, t.size as usize), (OS_ABI_VERSION, FAMILY_AMI68K, core::mem::size_of::<OsVTable>()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_AMI68K) }, Ok(()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_MAC68K) }, Err(OpenRefusal::FamilyNotServed { header: FAMILY_AMI68K, required: FAMILY_MAC68K }));
}

#[test]
fn the_fixture_makes_the_same_ui_calls_directly_and_through_the_table() {
    bind();
    let hunk = fixture::build_hunk(None);
    let mut direct = Amiga::new(RecordingGuestUi::new()).unwrap();
    direct.start_file(&hunk, "Hello").unwrap();
    direct.advance_clock_to(0).unwrap();
    let (step, stats) = direct.step(BUDGET);
    assert!(direct.ui().calls.iter().any(|c| matches!(c, UiCall::OpenWindow(w) if w.title == "Hello")), "the fixture opens its window directly");

    let mut remote = Remote::new("").unwrap();
    assert_eq!(remote.send(&program(&hunk, "Hello")), APPLY_OK);
    assert_eq!(remote.send(&[control(Control::Advance { virtual_us: 0 }), control(Control::Step { budget: BUDGET })]), APPLY_OK);
    assert_eq!(remote.host.ui().calls, direct.ui().calls, "the table path made exactly the direct path's calls");
    let wire = match &step {
        heirloom_ami68k_engine::Step::Exited => WireStep::Exited,
        heirloom_ami68k_engine::Step::Blocked => WireStep::Blocked,
        heirloom_ami68k_engine::Step::BudgetSpent => WireStep::BudgetSpent,
        heirloom_ami68k_engine::Step::Fault(f) => WireStep::Fault(f.clone()),
    };
    assert_eq!(remote.reports[0], Report::Started(Ok(())));
    match &remote.reports[1] {
        Report::Stepped { step, insns, .. } => assert_eq!((step, *insns), (&wire, stats.insns)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_assign_and_an_assign_add_reach_the_machines_dos() {
    let mut s = session();
    let batch: Vec<u8> = [
        control(Control::Assign { name: "DOPUS5".into(), target: "RAM:".into(), add: false }),
        control(Control::Assign { name: "LIBS".into(), target: "RAM:T".into(), add: true }),
    ]
    .concat();
    s.apply(&batch).unwrap();
    let vol = &s.machine().fs.vol;
    assert_eq!(vol.assign_targets("DOPUS5"), Some(&["RAM:".to_string()][..]));
    assert_eq!(vol.assign_targets("LIBS"), Some(&["SYS:Libs".to_string(), "RAM:T".to_string()][..]), "ADD keeps the first directory");
}

#[test]
fn a_bad_assign_name_ends_the_guest_by_name() {
    bind();
    let batch: Vec<u8> = [control(Control::Assign { name: "BAD:".into(), target: "RAM:".into(), add: false }), control(Control::Step { budget: 1 })].concat();
    // The fault is the next Step's report (the machine never runs).
    let mut remote = Remote::new("").unwrap();
    assert_eq!(remote.send(&[batch]), APPLY_OK);
    match &remote.reports[..] {
        [Report::Stepped { step: WireStep::Fault(why), insns: 0, .. }] => assert!(why.contains("not an AmigaDOS assign name"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_host_profile_becomes_muis_metrics_with_text_sized_by_the_guest_font() {
    let mut s = session();
    let p = HostProfile { toggle: (20, 20), control_h: 28, pad_w: 16, label_right: Some(8), group_head: 28, group_inset: 12, label_gap: 12, spacing: (8, 4), win_pad: 12 };
    s.apply(&control(Control::HostProfile(p))).unwrap();
    let m = s.machine().objects.mui_metrics;
    assert_eq!(m, metrics_of(&p));
    assert_eq!((m.toggle, m.control_h, m.label_right, m.spacing, m.win_pad), ((20, 20), 28, Some(8), (8, 4), 12));
    assert!(m.text_w.is_none(), "the host's measurer stays in the host process");
}

#[test]
fn an_unknown_profile_is_refused_by_name_and_makes_no_machine() {
    bind();
    let err = Remote::new("a4000").err().expect("refused");
    assert!(err.contains("a4000"), "{err}");
}
