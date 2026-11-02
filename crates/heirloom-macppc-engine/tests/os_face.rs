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
use heirloom_macppc_engine::os_face::MacPpcFamily;
use heirloom_os_abi::serve::{FamilyHooks, HostTransport, OsFamily, Session};
use heirloom_ui_abi::wire::proxy::ProxyUi;
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
    /// Every read_stream the library asked for: (locator, offset, capacity).
    reads: Vec<(String, u64, usize)>,
}

/// The engine's storage in a test: "Game/War3.mpq" holds `offset as u8`; any other locator is refused.
extern "C" fn read_stream(ctx: *mut c_void, locator: *const u8, len: usize, offset: u64, dst: *mut u8, cap: usize) -> isize {
    let sink = unsafe { &mut *(ctx as *mut Sink) };
    let locator = String::from_utf8(unsafe { std::slice::from_raw_parts(locator, len) }.to_vec()).unwrap();
    sink.reads.push((locator.clone(), offset, cap));
    if locator != "Game/War3.mpq" {
        return -1;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(dst, cap) };
    for (i, b) in out.iter_mut().enumerate() {
        *b = (offset as usize + i) as u8;
    }
    cap as isize
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
        let host = OsHost { ctx: &mut *sink as *mut Sink as *mut c_void, emit, lease_canvas: lease, read_stream: no_streams };
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

/// A PowerPC machine made the way the table makes one (Session::with_host), over `sink`'s storage.
fn session(sink: &mut Sink, wired: bool) -> Session<MacPpc<heirloom_os_abi::serve::LibUi>> {
    let host = OsHost { ctx: sink as *mut Sink as *mut c_void, emit, lease_canvas: lease, read_stream };
    let machine = MacPpcFamily::create("", ProxyUi::new(HostTransport::new(host))).unwrap();
    if wired { Session::with_host(machine, FamilyHooks::of::<MacPpcFamily>(), host) } else { Session::new(machine, FamilyHooks::of::<MacPpcFamily>()) }
}

/// The staged volume as the app ships it: one stream file named by its bundle path (its bytes stay in storage).
fn volume_with_a_stream() -> Vec<u8> {
    let mut t = heirloom_volume::Tree::new(31);
    let v = t.add_volume("Warcraft III", true);
    t.enable_streams();
    t.put_stream(v, "War3.mpq", 1000, "Game/War3.mpq").unwrap();
    t.put_stream(v, "Elsewhere", 10, "../outside").unwrap();
    let snap = heirloom_volume::snapshot::encode(&t);
    let mut b = Vec::new();
    encode_control(&Control::Volume { offset: 0, total: snap.len() as u32, chunk: snap }, &mut b).unwrap();
    b
}

#[test]
fn a_staged_stream_is_read_through_the_engines_storage_by_its_locator() {
    bind();
    let mut sink = Box::new(Sink::default());
    let mut s = session(&mut sink, true);
    s.apply(&volume_with_a_stream()).unwrap();
    let vol = s.machine().volume();
    let game = vol.volume("Warcraft III").unwrap();
    let f = vol.find_child(game, "War3.mpq").unwrap();
    assert_eq!((vol.is_stream(f), vol.file_len(f)), (true, Some(1000)));
    let mut buf = [0u8; 8];
    assert_eq!(vol.read_at(f, 300, &mut buf), Some(8));
    assert_eq!(buf, [44, 45, 46, 47, 48, 49, 50, 51], "bytes 300.. come from the engine's storage");
    assert_eq!(vol.read_at(f, 996, &mut buf), Some(4), "clamped to the stream's own length");
    let bad = vol.find_child(game, "Elsewhere").unwrap();
    assert_eq!(vol.read_at(bad, 0, &mut buf), None, "a locator the engine refuses is an I/O error, never zeros");
    assert_eq!(sink.reads, vec![(String::from("Game/War3.mpq"), 300, 8), (String::from("Game/War3.mpq"), 996, 4), (String::from("../outside"), 0, 8)]);
}

#[test]
fn a_session_not_wired_to_the_engines_storage_reads_no_stream() {
    // NEGATIVE CONTROL: the same volume in a session made without the host (Session::new) has no source: no bytes, no call out.
    bind();
    let mut sink = Box::new(Sink::default());
    let mut s = session(&mut sink, false);
    s.apply(&volume_with_a_stream()).unwrap();
    let vol = s.machine().volume();
    let f = vol.find_child(vol.volume("Warcraft III").unwrap(), "War3.mpq").unwrap();
    assert_eq!(vol.read_at(f, 0, &mut [0u8; 4]), None);
    assert!(sink.reads.is_empty());
}

/// What the engine's GL worker saw through the table's `attach_gl` (OS ABI v3): every request batch, and the reply it lends back.
#[derive(Default)]
struct GlSink {
    requests: Vec<Vec<u8>>,
    reply: Vec<u8>,
}

/// The engine's GL transport in a test: records each request batch and answers nothing (no request here needs an answer).
extern "C" fn gl_transact(ctx: *mut c_void, req: *const u8, len: usize, reply_len: *mut usize) -> *const u8 {
    // SAFETY: ctx is the test's boxed GlSink; the library lends `len` request bytes and one writable usize.
    let sink = unsafe { &mut *(ctx as *mut GlSink) };
    sink.requests.push(unsafe { std::slice::from_raw_parts(req, len) }.to_vec());
    unsafe { *reply_len = sink.reply.len() };
    sink.reply.as_ptr()
}

/// A PowerPC program that makes an AGL context and destroys it (the GL a game makes before its first frame).
fn gl_app() -> Vec<u8> {
    use heirloom_fmt_pef::loader::SymbolClass;
    use heirloom_macppc_engine::asm::Asm;
    let names = ["aglChoosePixelFormat", "aglCreateContext", "aglSetCurrentContext", "aglDestroyContext"];
    let toc = |n: &str| (fixture::TOC_FIRST as i32) + 4 * names.iter().position(|x| *x == n).unwrap() as i32;
    let mut a = Asm::new();
    a.mflr(0).stw(0, 8, 1).stwu(1, -160, 1).stw(31, 156, 1).stw(30, 152, 1);
    a.li(3, 0).li(4, 0).li(5, 0).call_toc(toc("aglChoosePixelFormat")).mr(31, 3);
    a.mr(3, 31).li(4, 0).call_toc(toc("aglCreateContext")).mr(30, 3);
    a.mr(3, 30).call_toc(toc("aglSetCurrentContext"));
    a.mr(3, 30).call_toc(toc("aglDestroyContext"));
    a.li(3, 0).lwz(30, 152, 1).lwz(31, 156, 1).addi(1, 1, 160).lwz(0, 8, 1).mtlr(0).blr();
    let syms: Vec<(&str, SymbolClass, bool)> = names.iter().map(|n| (*n, SymbolClass::TVector, false)).collect();
    let libs: [fixture::Lib; 1] = [("OpenGLLibrary", false, &syms)];
    fixture::macbinary_app(&fixture::app_with(&libs, &a.finish(), &[]))
}

/// Run `gl_app` through the table, with the engine's GL worker attached or not: the GL requests the worker saw.
fn gl_through_the_table(attach: bool) -> Vec<Vec<u8>> {
    bind();
    let mut remote = Remote::new(FAMILY_MACPPC, "").unwrap();
    let mut gl = Box::new(GlSink::default());
    if attach {
        let host = GlHost { ctx: &mut *gl as *mut GlSink as *mut c_void, transact: gl_transact };
        assert_eq!((os_mac_vtable().attach_gl)(remote.inst, &host), GL_ATTACH_OK);
    }
    assert_eq!(remote.send(&program(&gl_app(), "GL Context")), APPLY_OK);
    assert_eq!(remote.send(&[control(Control::Advance { virtual_us: 0 }), control(Control::Step { budget: BUDGET })]), APPLY_OK);
    assert!(matches!(remote.reports.last(), Some(Report::Stepped { .. })), "{:?}", remote.reports);
    gl.requests
}

#[test]
fn the_engines_gl_worker_attached_through_the_table_receives_the_guests_gl() {
    use heirloom_gl_abi::command::peek_len;
    use heirloom_gl_abi::{Command, Frame as GlFrame, ENTRIES};
    let sent = gl_through_the_table(true).concat();
    let mut commands = Vec::new();
    let mut at = 0;
    while let Ok(Some(k)) = peek_len(&sent[at..]) {
        commands.push(GlFrame::decode(&sent[at..at + k]).expect("the library sends whole GL frames").0.command);
        at += k;
    }
    let choose = ENTRIES.iter().position(|e| e.name == "aglChoosePixelFormat").unwrap() as u16;
    assert!(matches!(commands.first(), Some(Command::Call { entry, .. }) if *entry == choose), "the guest's first AGL call reached the engine's worker: {commands:?}");
    assert!(commands.iter().any(|c| matches!(c, Command::BatchEnd { .. })), "as whole batches: {commands:?}");
    // without a worker the machine keeps its no-GL behaviour: nothing is sent anywhere (AGL offers no pixel format)
    assert!(gl_through_the_table(false).is_empty());
}

#[test]
fn a_null_host_or_instance_is_bad_args_and_the_machine_keeps_running() {
    bind();
    let mut remote = Remote::new(FAMILY_MACPPC, "").unwrap();
    let mut gl = Box::new(GlSink::default());
    let host = GlHost { ctx: &mut *gl as *mut GlSink as *mut c_void, transact: gl_transact };
    assert_eq!((os_mac_vtable().attach_gl)(remote.inst, core::ptr::null()), GL_ATTACH_BAD_ARGS);
    assert_eq!((os_mac_vtable().attach_gl)(core::ptr::null_mut(), &host), GL_ATTACH_BAD_ARGS);
    assert_eq!(remote.send(&program(&app(), "PPC Hello")), APPLY_OK, "a refused attach leaves the machine as it was");
}

#[test]
fn the_engine_reads_the_powerpc_stage_clock_through_the_table() {
    bind();
    let mut remote = Remote::new(FAMILY_MACPPC, "").unwrap();
    let (mut line, mut n) = ([0u8; 512], 0usize);
    let perf = os_mac_vtable().perf;
    assert_eq!(perf(remote.inst, 0, line.as_mut_ptr(), line.len(), &mut n), PERF_REFUSED, "no rate, no line");
    assert_eq!(perf(core::ptr::null_mut(), 1000, line.as_mut_ptr(), line.len(), &mut n), PERF_REFUSED);
    // the first call switches the clock on; a guest that has swapped no frame has no line yet
    assert_eq!(perf(remote.inst, 1000, line.as_mut_ptr(), line.len(), &mut n), PERF_NONE);
    assert_eq!(remote.send(&program(&app(), "PPC Hello")), APPLY_OK);
    assert_eq!(remote.send(&[control(Control::Advance { virtual_us: 0 }), control(Control::Step { budget: BUDGET })]), APPLY_OK);
    assert_eq!(perf(remote.inst, 1000, line.as_mut_ptr(), line.len(), &mut n), PERF_NONE);
    assert_eq!(n, 0);
}
