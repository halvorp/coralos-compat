//! Slice 3, second half: arrays, central repository, time, telephony, ROM fallback, control geometry and activation.
mod support;
use heirloom_cpu_arm::Memory;
use heirloom_ekaarm_engine::cenrep;
use heirloom_ekaarm_engine::drive::{Drive, RomSource};
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_SVC_BASE};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{loader, Outcome, Process};

const BX_LR: u32 = 0xE12F_FF1E;

fn words(w: &[u32]) -> Vec<u8> { w.iter().flat_map(|x| x.to_le_bytes()).collect() }

fn proc(names: &[&str]) -> Process {
    let mut all: Vec<&str> = names.to_vec();
    all.extend(["_ZN4User6AllocZEi", "_ZN4User4FreeEPv"]);
    loader::bare(&all).unwrap()
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub"));
    for (i, a) in args.iter().enumerate() { p.cpu.regs[i] = *a; }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) { (NativeOutcome::Return, r) => r, (o, _) => panic!("{name}: {o:?}") }
}

fn refused(p: &mut Process, name: &str, args: &[u32]) -> String {
    match call(p, name, args) { (NativeOutcome::Refused(m), _) => m, (o, _) => panic!("{name}: {o:?}") }
}

fn alloc(p: &mut Process, n: u32) -> u32 { ok(p, "_ZN4User6AllocZEi", &[n]) }
fn rd(p: &mut Process, a: u32) -> u32 { p.mem.read_u32(a).unwrap() }
fn wr(p: &mut Process, a: u32, v: u32) { p.mem.write_u32(a, v).unwrap(); }

fn array_ops(rep: &str) {
    let (ctor, insert, at, reset, dtor) = ("_ZN13CArrayFixBaseC1EPFP8CBufBaseiEii", "_ZN13CArrayFixBase7InsertLEiPKv", "_ZNK13CArrayFixBase2AtEi", "_ZN13CArrayFixBase5ResetEv", "_ZN13CArrayFixBaseD1Ev");
    let mut p = proc(&[ctor, insert, at, reset, dtor, rep]);
    let rep_addr = p.engine_stub(rep).unwrap();
    let arr = alloc(&mut p, 24);
    ok(&mut p, ctor, &[arr, rep_addr, 8, 4]); // 8-byte records, granularity 4
    let rec = alloc(&mut p, 8);
    for (i, v) in [10u32, 30, 20, 40, 50, 60].iter().enumerate() {
        wr(&mut p, rec, *v);
        wr(&mut p, rec + 4, *v + 1);
        // 10, 30, then 20 inserted at index 1, 40 at the end, 50 at 0, 60 at 2
        let idx = [0, 1, 1, 3, 0, 2][i];
        ok(&mut p, insert, &[arr, idx, rec]);
    }
    assert_eq!(rd(&mut p, arr + 4), 6, "the inline Count() reads iCount");
    let got: Vec<u32> = (0..6).map(|i| { let a = ok(&mut p, at, &[arr, i]); assert_eq!(rd(&mut p, a + 4), rd(&mut p, a) + 1); rd(&mut p, a) }).collect();
    assert_eq!(got, vec![50, 10, 60, 20, 30, 40]);
    assert!(refused(&mut p, at, &[arr, 6]).contains("outside an array of 6"));
    assert!(refused(&mut p, insert, &[arr, 9, rec]).contains("insert at 9"));
    ok(&mut p, reset, &[arr]);
    assert_eq!(rd(&mut p, arr + 4), 0);
    ok(&mut p, dtor, &[arr]);
}

#[test]
fn flat_and_segmented_arrays_keep_records_in_order_and_count_inline() {
    array_ops("_ZN8CBufFlat4NewLEi");
    array_ops("_ZN7CBufSeg4NewLEi");
}

#[test]
fn a_segmented_array_keeps_record_addresses_stable_across_inserts() {
    let (ctor, insert, at) = ("_ZN13CArrayFixBaseC1EPFP8CBufBaseiEii", "_ZN13CArrayFixBase7InsertLEiPKv", "_ZNK13CArrayFixBase2AtEi");
    let mut p = proc(&[ctor, insert, at]);
    let rep = p.engine_stub("_ZN7CBufSeg4NewLEi").unwrap();
    let arr = alloc(&mut p, 24);
    ok(&mut p, ctor, &[arr, rep, 4, 2]);
    let rec = alloc(&mut p, 4);
    wr(&mut p, rec, 7);
    ok(&mut p, insert, &[arr, 0, rec]);
    let first = ok(&mut p, at, &[arr, 0]);
    for _ in 0..20 { ok(&mut p, insert, &[arr, 0, rec]); }
    assert_eq!(ok(&mut p, at, &[arr, 20]), first, "the first record did not move");
}

#[test]
fn a_buffer_factory_that_is_neither_flat_nor_segmented_is_refused() {
    let ctor = "_ZN13CArrayFixBaseC1EPFP8CBufBaseiEii";
    let mut p = proc(&[ctor]);
    let arr = alloc(&mut p, 24);
    assert!(refused(&mut p, ctor, &[arr, 0x400001, 4, 4]).contains("neither CBufFlat::NewL nor CBufSeg::NewL"));
}

#[test]
fn central_repository_text_is_parsed_and_a_missing_one_leaves() {
    let text = "\u{feff}cenrep\r\nversion 1\r\n[owner]\r\n0x100058F8\r\n[Main]\r\n0x1 int 5 0 cap_rd=alwayspass\r\n0x2 int -3 0\r\n0x3 string \"Z:\\\\Data\\\\x.aac\" 0 cap_rd=alwayspass\r\n0x4 binary 007E 0\r\n";
    let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let keys = cenrep::parse(&utf16).unwrap();
    assert_eq!(keys.get(&1), Some(&cenrep::CenValue::Int(5)));
    assert_eq!(keys.get(&2), Some(&cenrep::CenValue::Int(-3)));
    assert_eq!(keys.get(&3), Some(&cenrep::CenValue::Str("Z:\\Data\\x.aac".into())));
    assert!(!keys.contains_key(&4), "binary keys are not read");

    let (newl, get) = ("_ZN11CRepository4NewLE4TUid", "_ZN11CRepository3GetEmRi");
    let mut p = proc(&[newl, get]);
    let mut d = Drive::new();
    d.put_file("\\private\\10202be9\\0000abcd.txt", utf16).unwrap();
    p.set_drive(d);
    let repo = ok(&mut p, newl, &[0xabcd]);
    let out = alloc(&mut p, 4);
    assert_eq!(ok(&mut p, get, &[repo, 1, out]), 0);
    assert_eq!(rd(&mut p, out) as i32, 5);
    assert_eq!(ok(&mut p, get, &[repo, 99, out]) as i32, -1, "KErrNotFound");
    assert_eq!(ok(&mut p, get, &[repo, 3, out]) as i32, -6, "a string key read as an int: KErrArgument");
    let m = refused(&mut p, newl, &[0x1234]);
    assert!(m.contains("leave CRepository::NewL(0x00001234)") && m.contains("KErrNotFound"), "{m}");
}

struct MemRom(Vec<(&'static str, Vec<u8>)>);
impl RomSource for MemRom {
    fn read(&self, path: &str) -> Option<Vec<u8>> { self.0.iter().find(|(p, _)| p.eq_ignore_ascii_case(path)).map(|(_, b)| b.clone()) }
    fn exists(&self, path: &str) -> bool { self.read(path).is_some() }
    fn list(&self, _: &str) -> Option<Vec<String>> { None }
}

#[test]
fn the_rom_sits_behind_the_package_and_never_shadows_it() {
    let mut d = Drive::new();
    d.put_file("\\a.txt", b"package".to_vec()).unwrap();
    d.set_rom(Box::new(MemRom(vec![("\\a.txt", b"rom".to_vec()), ("\\b.txt", b"rom-only".to_vec())])));
    assert_eq!(d.read_owned("\\a.txt").unwrap(), b"package");
    assert_eq!(d.read_owned("\\b.txt").unwrap(), b"rom-only");
    assert!(d.exists_anywhere("\\b.txt") && !d.exists("\\b.txt"), "package-only exists() does not see the ROM");
    assert!(d.read_owned("\\c.txt").is_err());
}

#[test]
fn a_date_time_becomes_microseconds_since_year_zero() {
    let (ctor, assign) = ("_ZN9TDateTimeC1Ei6TMonthiiiii", "_ZN5TTimeaSERK9TDateTime");
    let mut p = proc(&[ctor, assign]);
    // TDateTime(1970, January(0), day 0, 0:00:00): the Unix epoch is 62167219200 s after year 0
    let dt = alloc(&mut p, 28);
    let sp = p.cpu.regs[13];
    for (i, v) in [0u32, 0, 0, 0].iter().enumerate() { wr(&mut p, sp + i as u32 * 4, *v); }
    ok(&mut p, ctor, &[dt, 1970, 0, 0]);
    assert_eq!((rd(&mut p, dt), rd(&mut p, dt + 4), rd(&mut p, dt + 8)), (1970, 0, 0));
    let t = alloc(&mut p, 8);
    ok(&mut p, assign, &[t, dt]);
    let micro = rd(&mut p, t) as u64 | (rd(&mut p, t + 4) as u64) << 32;
    assert_eq!(micro, 62_167_219_200u64 * 1_000_000);
    // 2000-03-01 12:30:15.5 (month 2, day 0 are zero-based): a leap year, so day 31+29 before March 1
    for (i, v) in [30u32, 15, 500_000].iter().enumerate() { wr(&mut p, sp + 4 + i as u32 * 4, *v); }
    wr(&mut p, sp, 12);
    ok(&mut p, ctor, &[dt, 2000, 2, 0]);
    ok(&mut p, assign, &[t, dt]);
    let micro = rd(&mut p, t) as u64 | (rd(&mut p, t + 4) as u64) << 32;
    let days_to_2000 = 62_167_219_200u64 / 86400 + 10957;
    assert_eq!(micro, ((((days_to_2000 + 31 + 29) * 24 + 12) * 60 + 30) * 60 + 15) * 1_000_000 + 500_000);
    wr(&mut p, dt + 4, 12); // month 12 is not a month
    assert!(refused(&mut p, assign, &[t, dt]).contains("out of range"));
}

#[test]
fn a_device_without_a_telephony_server_says_not_supported_and_keeps_zero_handles() {
    let names = ["_ZN10RTelServerC1Ev", "_ZN10RTelServer7ConnectEi", "_ZN6RPhoneC1Ev", "_ZN6RPhone4OpenER10RTelServerRK7TDesC16", "_ZN5RLineC1Ev", "_ZN5RLine4OpenER6RPhoneRK7TDesC16", "_ZN6RPhone5CloseEv"];
    let mut p = proc(&names);
    let (srv, phone, line) = (alloc(&mut p, 8), alloc(&mut p, 8), alloc(&mut p, 8));
    for a in [srv, phone, line] { wr(&mut p, a, 0xdead); wr(&mut p, a + 4, 0xbeef); }
    ok(&mut p, names[0], &[srv]);
    ok(&mut p, names[2], &[phone]);
    ok(&mut p, names[4], &[line]);
    assert_eq!((rd(&mut p, srv), rd(&mut p, phone), rd(&mut p, phone + 4), rd(&mut p, line), rd(&mut p, line + 4)), (0, 0, 0, 0, 0), "value-initialised");
    assert_eq!(rd(&mut p, srv + 4), 0xbeef, "RTelServer is one word");
    assert_eq!(ok(&mut p, names[1], &[srv, 4]) as i32, -5);
    assert_eq!(ok(&mut p, names[3], &[phone, srv, 0]) as i32, -5);
    assert_eq!(ok(&mut p, names[5], &[line, phone, 0]) as i32, -5);
    assert_eq!((rd(&mut p, srv), rd(&mut p, phone)), (0, 0), "the handles stay 0");
    ok(&mut p, names[6], &[phone]); // Close on a 0 handle is a no-op
    wr(&mut p, phone, 5);
    assert!(refused(&mut p, names[6], &[phone]).contains("closing an open handle"));
}

/// A control whose vtable slots 8 (ActivateL), 27 (SizeChanged) and 28 (PositionChanged) are guest routines that append `this` to a
/// log at `LOG` (word 0 is the count). Each routine is 0x20 bytes at `base + 0x20 * k`: load the log address from its literal pool,
/// bump the count, store r0 at `log[count]`, return.
fn control_with_guest_overrides(p: &mut Process, base: u32) -> u32 {
    let obj = alloc(p, 0x40);
    let vt = alloc(p, 0xa0);
    wr(p, obj, vt);
    let mut code = Vec::new();
    for _ in 0..3 {
        // ldr r3,[pc,#0x10] ; ldr r2,[r3] ; add r2,r2,#1 ; str r2,[r3] ; str r0,[r3,r2,lsl #2] ; bx lr ; .word LOG ; (pad)
        code.extend([0xE59F_3010, 0xE593_2000, 0xE282_2001, 0xE583_2000, 0xE783_0102, BX_LR, LOG, 0]);
    }
    p.map_code(base, &words(&code)).unwrap();
    for (slot, k) in [(8u32, 0u32), (27, 1), (28, 2)] { wr(p, vt + slot * 4, base + k * 0x20); }
    obj
}

const LOG: u32 = 0x0900_0000;

#[test]
fn set_rect_stores_geometry_and_notifies_size_then_position_through_the_guests_overrides() {
    let (setrect, rect) = ("_ZN11CCoeControl7SetRectERK5TRect", "_ZNK11CCoeControl4RectEv");
    let mut p = proc(&[setrect, rect]);
    p.map_data(LOG, &[0; 16]).unwrap();
    let obj = control_with_guest_overrides(&mut p, CODE_BASE + 0x1000);
    // SetRect(10, 20, 110, 220) from a zeroed control: both size and position change
    let r = alloc(&mut p, 16);
    for (i, v) in [10u32, 20, 110, 220].iter().enumerate() { wr(&mut p, r + i as u32 * 4, *v); }
    // run through the machine so the chain of guest calls executes: SVC SetRect; SVC exit
    p.map_code(CODE_BASE, &words(&[0xEF00_0000 | STUB_SVC_BASE, 0xEF00_0000 | EXIT_SVC])).unwrap();
    p.cpu.regs[..2].copy_from_slice(&[obj, r]);
    p.cpu.set_pc_interworking(CODE_BASE);
    assert!(matches!(p.run(10_000), Outcome::Exited(_)));
    assert_eq!((rd(&mut p, obj + 20), rd(&mut p, obj + 24), rd(&mut p, obj + 28), rd(&mut p, obj + 32)), (10, 20, 100, 200), "iPosition and iSize");
    assert_eq!(rd(&mut p, LOG), 2, "two notifications");
    assert_eq!((rd(&mut p, LOG + 4), rd(&mut p, LOG + 8)), (obj, obj));
    assert_eq!(p.callback_depth(), 0);
    // Rect() reads them back as (left, top, right, bottom)
    let out = alloc(&mut p, 16);
    ok(&mut p, rect, &[out, obj]);
    assert_eq!([rd(&mut p, out), rd(&mut p, out + 4), rd(&mut p, out + 8), rd(&mut p, out + 12)], [10, 20, 110, 220]);
}

#[test]
fn a_rom_path_cannot_walk_out_of_the_rom_root() {
    use heirloom_ekaarm_engine::drive::RomSource;
    let base = std::env::temp_dir().join(format!("ekarom-{}", std::process::id()));
    std::fs::create_dir_all(base.join("rom/sys")).unwrap();
    std::fs::write(base.join("secret.txt"), b"host").unwrap();
    std::fs::write(base.join("rom/sys/in.txt"), b"rom").unwrap();
    let rom = heirloom_ekaarm_engine::hostdir::DirRom::new(&base.join("rom")).unwrap();
    assert_eq!(rom.read("Z:\\sys\\IN.TXT").as_deref(), Some(&b"rom"[..]), "case-insensitive inside the root");
    assert_eq!(rom.read("Z:\\..\\secret.txt"), None, "never outside it");
    assert!(!rom.exists("Z:\\sys\\..\\..\\secret.txt"));
    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_app_ui_factory_reaches_the_status_pane_whose_title_text_becomes_the_window_title() {
    use heirloom_ui_abi::{CallbackToken, GuestRect, GuestUi, RecordingGuestUi, UiCall, WindowModel, WindowRole};
    let names = ["_ZNK9CEikonEnv12AppUiFactoryEv", "CAknAppUiFactory::StatusPane", "_ZNK18CEikStatusPaneBase8ControlLE4TUid", "_ZN13CAknTitlePane8SetTextLERK7TDesC16", "_ZN18CEikStatusPaneBase12SwitchLayoutLEi", "_ZNK18CEikStatusPaneBase18CurrentLayoutResIdEv"];
    let mut p = proc(&names);
    p.set_drive(support::drive_with_rom());
    let mut ui = RecordingGuestUi::new();
    let win = ui.open_window(WindowModel { title: "Caption".into(), rect: GuestRect { left: 0, top: 0, width: 360, height: 640 }, role: WindowRole::Document, closable: true, resizable: false, token: CallbackToken(0) }).unwrap();
    p.set_ui(ui);
    p.host.window = Some(win);
    let factory = ok(&mut p, names[0], &[]);
    assert_eq!(ok(&mut p, names[0], &[]), factory, "one factory per process");
    // MEikAppUiFactory::StatusPane is virtual slot 20 (confirmed against FileBrowser's `ldr r1,[vtbl,#0x50]`)
    let vptr = rd(&mut p, factory);
    let slot20 = rd(&mut p, vptr + 20 * 4);
    assert_eq!(p.stubs[((slot20 - heirloom_ekaarm_engine::layout::STUB_BASE) / 8) as usize].name.as_deref(), Some("CAknAppUiFactory::StatusPane"));
    let pane = ok(&mut p, names[1], &[factory]);
    // the pane's own vtable has SwitchLayoutL at slot 3 (FileBrowser calls `[vtbl,#0xc]`)
    let pv = rd(&mut p, pane);
    let s3 = rd(&mut p, pv + 12);
    assert_eq!(p.stubs[((s3 - heirloom_ekaarm_engine::layout::STUB_BASE) / 8) as usize].name.as_deref(), Some("_ZN18CEikStatusPaneBase12SwitchLayoutLEi"));
    assert!(refused(&mut p, names[5], &[pane]).contains("no layout was switched"));
    ok(&mut p, names[4], &[pane, 0x08cc0167]);
    assert_eq!(ok(&mut p, names[5], &[pane]), 0x08cc0167);
    let title = ok(&mut p, names[2], &[pane, 0x1000_59BC]);
    assert_eq!(ok(&mut p, names[2], &[pane, 0x1000_59BC]), title);
    assert!(refused(&mut p, names[2], &[pane, 0x1234]).contains("no control implemented"));
    // SetTextL(const TDesC16&) shows as the window title, through the seam, UTF-8 at the boundary
    let text = {
        let units: Vec<u16> = "Přehled".encode_utf16().collect();
        let data = alloc(&mut p, 32);
        for (i, u) in units.iter().enumerate() { p.mem.write_u16(data + i as u32 * 2, *u).unwrap(); }
        let d = alloc(&mut p, 8);
        wr(&mut p, d, units.len() as u32 | 1 << 28);
        wr(&mut p, d + 4, data);
        d
    };
    ok(&mut p, names[3], &[title, text]);
    let ui = p.ui_as::<RecordingGuestUi>().unwrap();
    assert!(ui.calls.iter().any(|c| matches!(c, UiCall::SetWindowTitle(_, t) if t == "Přehled")), "{:?}", ui.calls);
}

#[test]
fn rect_constructors_write_corners() {
    let names = ["_ZN5TRectC1Ev", "_ZN5TRectC1ERK6TPointRK5TSize"];
    let mut p = proc(&names);
    let r = alloc(&mut p, 16);
    for i in 0..4 { wr(&mut p, r + i * 4, 0xffff); }
    ok(&mut p, names[0], &[r]);
    assert_eq!([rd(&mut p, r), rd(&mut p, r + 4), rd(&mut p, r + 8), rd(&mut p, r + 12)], [0; 4]);
    let (pt, sz) = (alloc(&mut p, 8), alloc(&mut p, 8));
    for (a, v) in [(pt, 10u32), (pt + 4, 20), (sz, 100), (sz + 4, 200)] { wr(&mut p, a, v); }
    ok(&mut p, names[1], &[r, pt, sz]);
    assert_eq!([rd(&mut p, r), rd(&mut p, r + 4), rd(&mut p, r + 8), rd(&mut p, r + 12)], [10, 20, 110, 220]);
}

/// The corpus run: FileBrowser against its own package and the SDK emulator's ROM builds its menu bar and CBA from resource files
/// (the CBA resource is in the ROM's avkon file) and stops at the layout engine. Skipped when the owner's corpus is absent.
#[test]
fn file_browser_builds_its_menu_bar_and_cba_from_the_rom_and_stops_at_the_layout_engine() {
    use heirloom_ekaarm_engine::hostdir::{load_dir, DirRom};
    use heirloom_fmt_e32::OrdinalTable;
    const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
    let exe = "/data/build/corpus-symbian/e32/FileBrowser.exe";
    let tree = "/data/build/corpus-symbian/drive/nokiafilebrowser";
    let rom = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb/z";
    let Ok(bytes) = std::fs::read(exe) else { println!("SKIP: {exe} absent"); return };
    if !std::path::Path::new(tree).is_dir() || !std::path::Path::new(rom).is_dir() { println!("SKIP: corpus absent"); return; }
    let mut p = loader::load(&bytes, &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap();
    let (mut drive, _) = load_dir(std::path::Path::new(tree)).unwrap();
    drive.set_rom(Box::new(DirRom::new(std::path::Path::new(rom)).unwrap()));
    p.set_drive(drive);
    p.set_ui(heirloom_ui_abi::RecordingGuestUi::new());
    let m = match p.run(50_000_000) { Outcome::Refused(m) => m, o => panic!("{o:?}") };
    println!("{m}");
    assert!(m.contains("CdlEngine/55"), "{m}");
    let log = p.log.join("\n");
    assert!(log.contains("[shell] menu bar: App[] View[File,Edit,View,Disk admin,Tools,Settings,About,Exit]"), "{log}");
    assert!(log.contains("[shell] cba: 3000=Options | 3009=Exit"), "{log}");
    assert!(log.contains("[viewappui] activating view"), "{log}");
}
