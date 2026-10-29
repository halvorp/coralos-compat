//! os-mac.library's table (feature `os-library`): the Mac 68k fixture application run DIRECTLY on a recording UI and run THROUGH
//! the table - Control frames in, UI frames + Reports out through an OsHost, decoded by WireHost onto a recording UI - makes the
//! SAME UI calls with the same ids and pixels, and the table refuses what it must by name.
//! Run: cargo test -p heirloom-mac68k-engine --features os-library --test os_face
#![cfg(feature = "os-library")]

use core::ffi::c_void;
use heirloom_common::GuestMachine;
use heirloom_mac68k_engine::{fixture, os_vtable, Mac};
use heirloom_os_abi::*;
use heirloom_ui_abi::wire::host::WireHost;
use heirloom_ui_abi::wire::{decode, encode_call, encode_control, Control, Frame, Report, WireStep};
use heirloom_ui_abi::*;
use std::sync::Once;

const WALL: i64 = 1_790_000_000;
const BUDGET: u64 = 200_000;

fn bind() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert_eq!((os_vtable().bind_cpu)(heirloom_cpu_68k::vtable()), BIND_OK));
}

/// The engine's side of the table in a test: every emitted frame, and the canvases leased by region.
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
        let host = OsHost { ctx: &mut *sink as *mut Sink as *mut c_void, emit, lease_canvas: lease };
        let mut why = [0u8; 256];
        let mut why_len = 0usize;
        let inst = (os_vtable().create)(profile.as_ptr(), profile.len(), &host, why.as_mut_ptr(), why.len(), &mut why_len);
        if inst.is_null() {
            return Err(String::from_utf8_lossy(&why[..why_len]).into_owned());
        }
        Ok(Remote { inst, sink, host: WireHost::new(RecordingGuestUi::new()), reports: Vec::new() })
    }

    /// Send `frames` (one batch), then play the host: UI frames onto the recording UI (a Lease hands its pixels over), Reports kept.
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
    for (i, chunk) in file.chunks(BLOB_CHUNK_TEST).enumerate() {
        out.push(control(Control::Program { offset: (i * BLOB_CHUNK_TEST) as u32, total: file.len() as u32, chunk: chunk.to_vec() }));
    }
    out.push(control(Control::StartFile { name: name.into() }));
    out
}

/// Small chunks so the fixture really arrives in several pieces.
const BLOB_CHUNK_TEST: usize = 1000;

#[test]
fn the_table_is_the_mac_68k_family_at_the_frozen_version() {
    let t = os_vtable();
    assert_eq!((t.version, t.family, t.size as usize), (OS_ABI_VERSION, FAMILY_MAC68K, core::mem::size_of::<OsVTable>()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_MAC68K) }, Ok(()));
}

#[test]
fn the_fixture_makes_the_same_ui_calls_directly_and_through_the_table() {
    bind();
    let app = fixture::test_app_macbinary();
    // Direct.
    let mut direct = Mac::new(RecordingGuestUi::new()).unwrap();
    direct.set_wall_clock(WALL).unwrap();
    direct.start_file(&app, "Test App").unwrap();
    direct.advance_clock_to(0).unwrap();
    let (first, _) = direct.step(BUDGET);
    let win = direct.ui().calls.iter().find_map(|c| if let UiCall::OpenWindow(_) = c { Some(()) } else { None });
    assert!(win.is_some(), "the fixture opens a window directly");
    direct.deliver(GuestUiEvent::CloseRequested { win: UiObjectRef { id: 1, generation: 1 }, token: CallbackToken(0) }).unwrap();
    let (second, _) = direct.step(BUDGET);

    // Through the table.
    let mut remote = Remote::new("").unwrap();
    assert_eq!(remote.send(&[control(Control::SetWallClock { local_unix_secs: WALL })]), APPLY_OK);
    assert_eq!(remote.send(&program(&app, "Test App")), APPLY_OK);
    assert_eq!(remote.send(&[control(Control::Advance { virtual_us: 0 }), control(Control::Step { budget: BUDGET })]), APPLY_OK);
    let close = remote.host.encode_host_event(GuestUiEvent::CloseRequested { win: UiObjectRef { id: 1, generation: 1 }, token: CallbackToken(0) });
    assert_eq!(remote.send(&[close, control(Control::Step { budget: BUDGET })]), APPLY_OK);

    assert_eq!(remote.host.ui().calls, direct.ui().calls, "the table path made exactly the direct path's calls");
    let wire = |s: &heirloom_mac68k_engine::Step| match s {
        heirloom_mac68k_engine::Step::Exited => WireStep::Exited,
        heirloom_mac68k_engine::Step::Blocked => WireStep::Blocked,
        heirloom_mac68k_engine::Step::BudgetSpent => WireStep::BudgetSpent,
        heirloom_mac68k_engine::Step::Fault(f) => WireStep::Fault(f.clone()),
    };
    let steps: Vec<WireStep> = remote.reports.iter().filter_map(|r| if let Report::Stepped { step, .. } = r { Some(step.clone()) } else { None }).collect();
    assert_eq!(steps, vec![wire(&first), wire(&second)]);
    assert_eq!(remote.reports[0], Report::WallClock(Ok(())));
    assert_eq!(remote.reports[1], Report::Started(Ok(())));
}

#[test]
fn an_unknown_profile_is_refused_by_name_and_makes_no_machine() {
    bind();
    let err = Remote::new("mac-9000").err().expect("refused");
    assert!(err.contains("mac-9000"), "{err}");
}

#[test]
fn a_cpu_table_of_another_architecture_is_refused() {
    let real = heirloom_cpu_68k::vtable();
    let ppc: &'static CpuVTable = Box::leak(Box::new(CpuVTable {
        version: real.version, size: real.size, arch_id: heirloom_cpu_abi::ARCH_PPC, create: real.create, destroy: real.destroy,
        reset: real.reset, run: real.run, interrupt: real.interrupt, get_reg: real.get_reg, set_reg: real.set_reg,
        invalidate_code: real.invalidate_code, context_size: real.context_size, save_context: real.save_context,
        load_context: real.load_context, recent_pcs: real.recent_pcs,
    }));
    assert_eq!((os_vtable().bind_cpu)(ppc), BIND_REFUSED);
    assert_eq!((os_vtable().bind_cpu)(core::ptr::null()), BIND_REFUSED);
}

#[test]
fn a_ui_call_from_the_host_is_a_bad_frame_and_a_missing_program_is_named() {
    bind();
    let mut remote = Remote::new("").unwrap();
    let mut call = Vec::new();
    encode_call(&UiCall::CloseWindow(UiObjectRef { id: 1, generation: 1 }), &mut call).unwrap();
    assert_eq!(remote.send(&[call]), APPLY_BAD_FRAME);
    assert_eq!(remote.send(&[control(Control::StartFile { name: "x".into() })]), APPLY_OK);
    assert_eq!(remote.reports, vec![Report::Started(Err("start_file: no program was sent".into()))]);
    assert_eq!((os_vtable().apply)(core::ptr::null_mut(), core::ptr::null(), 0), APPLY_BAD_ARGS);
}

#[test]
fn a_chunk_out_of_order_ends_the_guest_by_name() {
    bind();
    let mut remote = Remote::new("").unwrap();
    let rc = remote.send(&[
        control(Control::Program { offset: 0, total: 10, chunk: vec![0; 4] }),
        control(Control::Program { offset: 6, total: 10, chunk: vec![0; 4] }),
        control(Control::Step { budget: 1 }),
    ]);
    assert_eq!(rc, APPLY_OK);
    match &remote.reports[..] {
        [Report::Stepped { step: WireStep::Fault(why), insns: 0, .. }] => assert!(why.contains("does not follow"), "{why}"),
        other => panic!("{other:?}"),
    }
}
