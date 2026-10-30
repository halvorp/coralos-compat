//! os-mac.library's table as macppc.engine sees it (feature `os-library`): OS ABI v2's multi-family face serves the PowerPC Mac
//! beside the 68k one, binds cpu-ppc.library's PpcVTable (CPU ABI v3) and no other CPU contract for it, and the PowerPC fixture
//! application run DIRECTLY and THROUGH the table makes the same UI calls.
//! Run: cargo test -p heirloom-macppc-engine --features os-library --test os_face
#![cfg(feature = "os-library")]

use core::ffi::c_void;
use heirloom_common::GuestMachine;
use heirloom_macppc_engine::{fixture, os_mac_vtable, MacPpc};
use heirloom_os_abi::*;
use heirloom_ui_abi::wire::host::WireHost;
use heirloom_ui_abi::wire::{decode, encode_control, Control, Frame, Report, WireStep};
use heirloom_ui_abi::*;
use std::sync::Once;

const WALL: i64 = 1_790_000_000;
const BUDGET: u64 = 2_000_000;

fn bind() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| assert_eq!((os_mac_vtable().bind_cpu_ppc)(heirloom_cpu_ppc::table::vtable()), BIND_OK));
}

fn app() -> Vec<u8> {
    fixture::macbinary_app(&fixture::startup_app())
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

/// A machine of `family` made through `create_for_family`.
struct Remote {
    inst: *mut OsInstance,
    sink: Box<Sink>,
    host: WireHost<RecordingGuestUi>,
    reports: Vec<Report>,
}

impl Remote {
    fn new(family: u32, profile: &str) -> Result<Remote, String> {
        let mut sink = Box::new(Sink::default());
        let host = OsHost { ctx: &mut *sink as *mut Sink as *mut c_void, emit, lease_canvas: lease };
        let mut why = [0u8; 256];
        let mut why_len = 0usize;
        let inst = (os_mac_vtable().create_for_family)(family, profile.as_ptr(), profile.len(), &host, why.as_mut_ptr(), why.len(), &mut why_len);
        if inst.is_null() {
            return Err(String::from_utf8_lossy(&why[..why_len]).into_owned());
        }
        Ok(Remote { inst, sink, host: WireHost::new(RecordingGuestUi::new()), reports: Vec::new() })
    }

    fn send(&mut self, frames: &[Vec<u8>]) -> i32 {
        let batch: Vec<u8> = frames.concat();
        let rc = (os_mac_vtable().apply)(self.inst, batch.as_ptr(), batch.len());
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
        (os_mac_vtable().destroy)(self.inst);
    }
}

fn control(c: Control) -> Vec<u8> {
    let mut b = Vec::new();
    encode_control(&c, &mut b).unwrap();
    b
}

fn program(file: &[u8], name: &str) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = file
        .chunks(1000)
        .enumerate()
        .map(|(i, chunk)| control(Control::Program { offset: (i * 1000) as u32, total: file.len() as u32, chunk: chunk.to_vec() }))
        .collect();
    out.push(control(Control::StartFile { name: name.into() }));
    out
}

#[test]
fn os_mac_library_serves_both_mac_families_and_each_names_its_cpu_contract() {
    let t = os_mac_vtable();
    assert_eq!((t.version, t.family, t.size as usize), (OS_ABI_VERSION, FAMILY_MAC68K, core::mem::size_of::<OsVTable>()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_MAC68K) }, Ok(()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_MACPPC) }, Ok(()));
    assert_eq!(unsafe { check_ptr(t, OS_ABI_VERSION, FAMILY_EKAARM) }, Err(OpenRefusal::FamilyNotServed { header: FAMILY_MAC68K, required: FAMILY_EKAARM }));
    let ask = |family| {
        let (mut c, mut v) = (0u32, 0u32);
        ((t.cpu_abi_for_family)(family, &mut c, &mut v), c, v)
    };
    assert_eq!(ask(FAMILY_MACPPC), (FAMILY_SERVED, CPU_CONTRACT_PPC, ppc::PPC_ABI_VERSION));
    assert_eq!(ask(FAMILY_MAC68K), (FAMILY_SERVED, CPU_CONTRACT_V2, cpu_abi::CPU_ABI_VERSION));
    assert_eq!(ask(FAMILY_AMI68K).0, FAMILY_NOT_SERVED);
}

#[test]
fn each_family_binds_only_its_own_cpu_contract() {
    let t = os_mac_vtable();
    // The PowerPC CPU's v3 table offered as a v2 CpuVTable: no family of os-mac.library takes it (its header is not CPU ABI v2).
    let ppc_as_v2 = heirloom_cpu_ppc::table::vtable() as *const ppc::PpcVTable as *const CpuVTable;
    assert_eq!((t.bind_cpu)(ppc_as_v2), BIND_REFUSED);
    // A null PowerPC table, and a v2-shaped table offered as a PpcVTable, are refused before anything is bound.
    assert_eq!((t.bind_cpu_ppc)(core::ptr::null()), BIND_REFUSED);
    let fake_v2 = [cpu_abi::CPU_ABI_VERSION, 64, cpu_abi::ARCH_PPC, 0u32, 0, 0, 0, 0];
    assert_eq!((t.bind_cpu_ppc)(fake_v2.as_ptr() as *const ppc::PpcVTable), BIND_REFUSED);
}

#[test]
fn the_powerpc_fixture_makes_the_same_ui_calls_directly_and_through_the_table() {
    bind();
    let app = app();
    let mut direct = MacPpc::new(RecordingGuestUi::new()).unwrap();
    direct.set_wall_clock(WALL).unwrap();
    direct.start_file(&app, "PPC Hello").unwrap();
    direct.advance_clock_to(0).unwrap();
    let (first, _) = direct.step(BUDGET);
    assert!(direct.ui().calls.iter().any(|c| matches!(c, UiCall::OpenWindow(_))), "the fixture opens a window directly: {:?}", direct.ui().calls);

    let mut remote = Remote::new(FAMILY_MACPPC, "").unwrap();
    assert_eq!(remote.send(&[control(Control::SetWallClock { local_unix_secs: WALL })]), APPLY_OK);
    assert_eq!(remote.send(&program(&app, "PPC Hello")), APPLY_OK);
    assert_eq!(remote.send(&[control(Control::Advance { virtual_us: 0 }), control(Control::Step { budget: BUDGET })]), APPLY_OK);
    assert_eq!(remote.reports[0], Report::WallClock(Ok(())));
    assert_eq!(remote.reports[1], Report::Started(Ok(())));
    assert_eq!(remote.host.ui().calls, direct.ui().calls, "the table path made exactly the direct path's calls");
    let Some(Report::Stepped { step, .. }) = remote.reports.get(2) else { panic!("a Stepped report: {:?}", remote.reports) };
    let want = match first {
        heirloom_common::Step::Exited => WireStep::Exited,
        heirloom_common::Step::Blocked => WireStep::Blocked,
        heirloom_common::Step::BudgetSpent => WireStep::BudgetSpent,
        heirloom_common::Step::Fault(f) => WireStep::Fault(f),
    };
    assert_eq!(*step, want);
}

#[test]
fn a_family_or_profile_os_mac_does_not_serve_is_refused_by_name() {
    bind();
    let e = Remote::new(FAMILY_MACPPC, "mac-ii").err().expect("a 68k profile is not a PowerPC machine");
    assert!(e.contains("macppc") && e.contains("mac-ii"), "{e}");
    let e = Remote::new(FAMILY_EKAARM, "").err().expect("os-mac.library serves no Symbian machine");
    assert!(e.contains("no family 4"), "{e}");
}
