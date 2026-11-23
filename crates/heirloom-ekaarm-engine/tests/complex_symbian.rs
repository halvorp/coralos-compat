//! Unit tests and negative controls for complex Symbian S60 runtime primitives:
//! HAL attributes, CTimer, CDirectScreenBitmap, CDirectScreenAccess active state,
//! TBidiText, RFile::Seek (Symbian TSeek), Drive delete/rename, C stdlib (clib),
//! ARM EHABI leave/vec helpers, and guest E32 DLL resolution.
mod support;
use heirloom_ekaarm_engine::cpu::Memory;
use heirloom_ekaarm_engine::drive::Drive;
use heirloom_ekaarm_engine::layout::{CODE_BASE, EXIT_SVC, STUB_SVC_BASE};
use heirloom_ekaarm_engine::natives::{self, NativeOutcome};
use heirloom_ekaarm_engine::{dll, loader, Outcome, Process};
use heirloom_fmt_e32::OrdinalTable;
use heirloom_ui_abi::{GuestRect, RecordingGuestUi, UiCall, WindowModel, WindowRole, CallbackToken};

const BX_LR: u32 = 0xE12F_FF1E;

fn words(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn proc(names: &[&str]) -> Process {
    let mut all: Vec<&str> = names.to_vec();
    all.extend(["_ZN4User6AllocZEi", "_ZN4User4FreeEPv"]);
    loader::bare(&all).unwrap()
}

fn call(p: &mut Process, name: &str, args: &[u32]) -> (NativeOutcome, u32) {
    let idx = p.stubs.iter().position(|s| s.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("{name} is not a stub"));
    for (i, a) in args.iter().enumerate() {
        p.cpu.regs[i] = *a;
    }
    let o = natives::call(p, idx);
    (o, p.cpu.regs[0])
}

fn ok(p: &mut Process, name: &str, args: &[u32]) -> u32 {
    match call(p, name, args) {
        (NativeOutcome::Return, r) => r,
        (o, _) => panic!("{name}: {o:?}"),
    }
}

fn refused(p: &mut Process, name: &str, args: &[u32]) -> String {
    match call(p, name, args) {
        (NativeOutcome::Refused(m), _) => m,
        (o, _) => panic!("{name}: expected Refused, got {o:?}"),
    }
}

fn alloc(p: &mut Process, n: u32) -> u32 {
    ok(p, "_ZN4User6AllocZEi", &[n])
}

fn rd(p: &mut Process, a: u32) -> u32 {
    p.mem.read_u32(a).unwrap()
}

fn wr(p: &mut Process, a: u32, v: u32) {
    p.mem.write_u32(a, v).unwrap();
}

fn write_cstr(p: &mut Process, addr: u32, s: &[u8]) {
    for (i, &b) in s.iter().enumerate() {
        p.mem.write_u8(addr + i as u32, b).unwrap();
    }
    p.mem.write_u8(addr + s.len() as u32, 0).unwrap();
}

fn read_cstr(p: &mut Process, mut addr: u32) -> String {
    let mut out = Vec::new();
    while let Ok(b) = p.mem.read_u8(addr) {
        if b == 0 { break; }
        out.push(b);
        addr += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_tbuf16(p: &mut Process, addr: u32, text: &str) {
    let u: Vec<u16> = text.encode_utf16().collect();
    wr(p, addr, (3u32 << 28) | u.len() as u32);
    wr(p, addr + 4, 256);
    for (i, c) in u.iter().enumerate() {
        p.mem.write_u16(addr + 8 + i as u32 * 2, *c).unwrap();
    }
}

#[test]
fn hal_get_reports_screen_and_rejects_unknown_attribute() {
    let get = "_ZN3HAL3GetEN7HALData10TAttributeERi";
    let mut p = proc(&[get]);
    let out = alloc(&mut p, 4);
    assert_eq!(ok(&mut p, get, &[31, out]), 0); // EDisplayXPixels
    assert_eq!(rd(&mut p, out), 360);
    assert_eq!(ok(&mut p, get, &[32, out]), 0); // EDisplayYPixels
    assert_eq!(rd(&mut p, out), 640);
    assert_eq!(ok(&mut p, get, &[76, out]), 0); // EDisplayBitsPerPixel
    assert_eq!(rd(&mut p, out), 32);
    // Negative control: unknown HAL attribute returns KErrNotSupported (-5).
    assert_eq!(ok(&mut p, get, &[9999, out]) as i32, -5);
}

#[test]
fn clib_strings_printf_sscanf_strtok_and_stdio_work() {
    let names = [
        "malloc", "free", "strlen", "strcpy", "strcmp", "strcasecmp", "strtok",
        "sprintf", "sscanf", "strtol", "isdigit", "fopen", "fwrite", "fseek", "ftell", "fread", "fclose",
    ];
    let mut p = proc(&names);
    let mut drv = Drive::new();
    drv.put_file("C:\\private\\00000000\\test.txt", b"initial".to_vec()).unwrap();
    p.set_drive(drv);

    let buf = ok(&mut p, "malloc", &[128]);
    assert_ne!(buf, 0);
    let fmt = alloc(&mut p, 32);
    write_cstr(&mut p, fmt, b"n=%d,hex=%02x");
    assert_eq!(ok(&mut p, "sprintf", &[buf, fmt, 42, 0x0a]), 11);
    assert_eq!(read_cstr(&mut p, buf), "n=42,hex=0a");
    assert_eq!(ok(&mut p, "strlen", &[buf]), 11);

    // sscanf
    let sfmt = alloc(&mut p, 32);
    let v1 = alloc(&mut p, 4);
    let v2 = alloc(&mut p, 4);
    write_cstr(&mut p, sfmt, b"n=%d,hex=%x");
    assert_eq!(ok(&mut p, "sscanf", &[buf, sfmt, v1, v2]), 2);
    assert_eq!(rd(&mut p, v1), 42);
    assert_eq!(rd(&mut p, v2), 0x0a);

    // strtok + strcasecmp + isdigit
    let csv = alloc(&mut p, 32);
    let sep = alloc(&mut p, 8);
    write_cstr(&mut p, csv, b"Alpha,BETA");
    write_cstr(&mut p, sep, b",");
    let t1 = ok(&mut p, "strtok", &[csv, sep]);
    assert_eq!(read_cstr(&mut p, t1), "Alpha");
    let t2 = ok(&mut p, "strtok", &[0, sep]);
    assert_eq!(read_cstr(&mut p, t2), "BETA");
    assert_eq!(ok(&mut p, "strtok", &[0, sep]), 0);
    let cmp = alloc(&mut p, 16);
    write_cstr(&mut p, cmp, b"beta");
    assert_eq!(ok(&mut p, "strcasecmp", &[t2, cmp]), 0);
    assert_eq!(ok(&mut p, "isdigit", &[b'7' as u32]), 1);
    assert_eq!(ok(&mut p, "isdigit", &[b'x' as u32]), 0);

    // stdio fopen/fwrite/fseek/ftell/fread/fclose + negative control (missing file returns NULL)
    let path = alloc(&mut p, 32);
    let mode_w = alloc(&mut p, 8);
    let mode_r = alloc(&mut p, 8);
    write_cstr(&mut p, path, b"C:\\private\\00000000\\out.dat");
    write_cstr(&mut p, mode_w, b"wb");
    write_cstr(&mut p, mode_r, b"rb");
    let missing = alloc(&mut p, 32);
    write_cstr(&mut p, missing, b"C:\\private\\00000000\\nope.dat");
    assert_eq!(ok(&mut p, "fopen", &[missing, mode_r]), 0, "missing read file must return NULL");

    let fp = ok(&mut p, "fopen", &[path, mode_w]);
    assert_ne!(fp, 0);
    let payload = alloc(&mut p, 16);
    write_cstr(&mut p, payload, b"hello!");
    assert_eq!(ok(&mut p, "fwrite", &[payload, 1, 6, fp]), 6);
    assert_eq!(ok(&mut p, "ftell", &[fp]), 6);
    assert_eq!(ok(&mut p, "fclose", &[fp]), 0);

    let fp2 = ok(&mut p, "fopen", &[path, mode_r]);
    assert_ne!(fp2, 0);
    assert_eq!(ok(&mut p, "fseek", &[fp2, 2, 0]), 0); // SEEK_SET +2
    let rbuf = alloc(&mut p, 16);
    assert_eq!(ok(&mut p, "fread", &[rbuf, 1, 4, fp2]), 4);
    assert_eq!(read_cstr(&mut p, rbuf), "llo!");
    assert_eq!(ok(&mut p, "fclose", &[fp2]), 0);
    ok(&mut p, "free", &[buf]);
}

#[test]
fn rfile_seek_uses_symbian_tseek_modes_and_drive_delete_rename() {
    let names = [
        "_ZN3RFs7ConnectEi",
        "_ZN5RFile4OpenER3RFsRK7TDesC16j",
        "_ZNK5RFile4SeekE5TSeekRi",
        "_ZNK5RFile4ReadER5TDes8",
        "_ZN5RFile5CloseEv",
        "_ZN3RFs6RenameERK7TDesC16S2_",
        "_ZN3RFs6DeleteERK7TDesC16",
    ];
    let mut p = proc(&names);
    let mut drv = Drive::new();
    drv.put_file("C:\\data\\blob.bin", b"0123456789".to_vec()).unwrap();
    p.set_drive(drv);

    let rfs = alloc(&mut p, 8);
    assert_eq!(ok(&mut p, names[0], &[rfs, 4]), 0);
    let path = alloc(&mut p, 64);
    write_tbuf16(&mut p, path, "C:\\data\\blob.bin");
    let rfile = alloc(&mut p, 8);
    assert_eq!(ok(&mut p, names[1], &[rfile, rfs, path, 0]), 0);

    let pos = alloc(&mut p, 4);
    // Symbian ESeekStart = 1: seek to offset 4
    wr(&mut p, pos, 4);
    assert_eq!(ok(&mut p, names[2], &[rfile, 1, pos]), 0);
    assert_eq!(rd(&mut p, pos), 4);
    // Symbian ESeekCurrent = 2: advance +2 -> 6
    wr(&mut p, pos, 2);
    assert_eq!(ok(&mut p, names[2], &[rfile, 2, pos]), 0);
    assert_eq!(rd(&mut p, pos), 6);
    // Symbian ESeekEnd = 3: -3 from end (10) -> 7
    wr(&mut p, pos, (-3i32) as u32);
    assert_eq!(ok(&mut p, names[2], &[rfile, 3, pos]), 0);
    assert_eq!(rd(&mut p, pos), 7);
    ok(&mut p, names[4], &[rfile]);

    // Rename & delete
    let new_path = alloc(&mut p, 64);
    write_tbuf16(&mut p, new_path, "C:\\data\\moved.bin");
    assert_eq!(ok(&mut p, names[5], &[rfs, path, new_path]), 0);
    assert!(p.host.drive.as_ref().unwrap().exists("C:\\data\\moved.bin"));
    assert!(!p.host.drive.as_ref().unwrap().exists("C:\\data\\blob.bin"));
    assert_eq!(ok(&mut p, names[6], &[rfs, new_path]), 0);
    assert!(!p.host.drive.as_ref().unwrap().exists("C:\\data\\moved.bin"));
    // Negative control: deleting an already-deleted file returns KErrNotFound (-1).
    assert_eq!(ok(&mut p, names[6], &[rfs, new_path]) as i32, -1);
}

#[test]
fn direct_screen_bitmap_presents_pixels_and_dsa_sets_active_flag() {
    let names = [
        "_ZN19CDirectScreenBitmap4NewLEv",
        "_ZN19CDirectScreenBitmap6CreateERK5TRectNS_14TSettingsFlagsE",
        "_ZN19CDirectScreenBitmap11BeginUpdateER22TAcceleratedBitmapInfo",
        "_ZN19CDirectScreenBitmap9EndUpdateER14TRequestStatus",
        "_ZN19CDirectScreenBitmap5CloseEv",
        "_ZN19CDirectScreenAccess4NewLER10RWsSessionR15CWsScreenDeviceR11RWindowBaseR19MDirectScreenAccess",
        "_ZN19CDirectScreenAccess6StartLEv",
    ];
    let mut p = proc(&names);
    let mut ui = RecordingGuestUi::new();
    let win = heirloom_ui_abi::GuestUi::open_window(
        &mut ui,
        WindowModel {
            title: String::from("test"),
            rect: GuestRect { left: 0, top: 0, width: 4, height: 2 },
            role: WindowRole::Document,
            closable: true,
            resizable: false,
            token: CallbackToken(1),
        },
    ).unwrap();
    p.host.window = Some(win);
    p.set_ui(ui);

    // DSB lifecycle
    let dsb = ok(&mut p, names[0], &[]);
    assert_ne!(dsb, 0);
    // Negative control: BeginUpdate before Create returns KErrNotReady (-18).
    let info = alloc(&mut p, 32);
    assert_eq!(ok(&mut p, names[2], &[dsb, info]) as i32, -18);

    let rect = alloc(&mut p, 16);
    for (i, v) in [0u32, 0, 4, 2].iter().enumerate() { wr(&mut p, rect + i as u32 * 4, *v); }
    assert_eq!(ok(&mut p, names[1], &[dsb, rect, 1]), 0);
    assert_eq!(ok(&mut p, names[2], &[dsb, info]), 0);
    let fb = rd(&mut p, info + 4);
    assert_ne!(fb, 0);
    assert_eq!((rd(&mut p, info + 8), rd(&mut p, info + 12), rd(&mut p, info + 16)), (4, 2, 16));
    // Write one red pixel (BGRX LE = 00 00 FF FF) at (0,0)
    wr(&mut p, fb, 0xFFFF_0000);
    let status = alloc(&mut p, 4);
    wr(&mut p, status, 0x8000_0001);
    ok(&mut p, names[3], &[dsb, status]);
    assert_eq!(rd(&mut p, status), 0, "EndUpdate completes TRequestStatus with KErrNone");
    let presented = p.ui_as::<RecordingGuestUi>().unwrap().calls.iter().any(|c| matches!(c, UiCall::PresentPlane(_, pl) if pl.rect.width == 4 && pl.rect.height == 2));
    assert!(presented, "EndUpdate must present pixels to GuestUi");
    ok(&mut p, names[4], &[dsb]);

    // CDirectScreenAccess::StartL sets CActive FLAG_ACTIVE (bit 0 at +8)
    let dsa = ok(&mut p, names[5], &[0, 0, 0, 0]);
    assert_eq!(rd(&mut p, dsa + 8) & 1, 0);
    ok(&mut p, names[6], &[dsa]);
    assert_eq!(rd(&mut p, dsa + 8) & 1, 1, "CDirectScreenAccess::StartL must mark CActive active");
}

#[test]
fn ehabi_leave_without_descriptor_refuses_and_vec_ctor_runs_guest_ctor() {
    let names = [
        "_ZN4User5LeaveEi",
        "_ZN4User12LeaveIfErrorEi",
        "__aeabi_vec_ctor_nocookie_nodtor",
    ];
    let mut p = proc(&names);
    // Negative control: User::Leave on image with no exception descriptor refuses cleanly.
    assert!(refused(&mut p, names[0], &[(-5i32) as u32]).contains("no E32 exception descriptor"));
    assert_eq!(ok(&mut p, names[1], &[0]), 0);
    assert!(refused(&mut p, names[1], &[(-1i32) as u32]).contains("no E32 exception descriptor"));

    // Positive test: __aeabi_vec_ctor_nocookie_nodtor calls guest ctor on 3 elements of 8 bytes.
    // Guest ctor at CODE_BASE: STR r0, [r0]; BX LR  (writes each element's own address into its word 0)
    // Main at CODE_BASE + 0x10: SVC #(STUB_SVC_BASE + 2); SVC #EXIT_SVC
    let code = words(&[
        0xE580_0000, // str r0, [r0]
        BX_LR,
        0, 0,
        0xEF00_0000 | (STUB_SVC_BASE + 2),
        0xEF00_0000 | EXIT_SVC,
    ]);
    p.map_code(CODE_BASE, &code).unwrap();
    let arr = alloc(&mut p, 24);
    p.cpu.regs[0] = arr;
    p.cpu.regs[1] = CODE_BASE;
    p.cpu.regs[2] = 8;
    p.cpu.regs[3] = 3;
    p.cpu.set_pc_interworking(CODE_BASE + 0x10);
    match p.run(10_000) {
        Outcome::Exited(ret) => assert_eq!(ret as u32, arr),
        o => panic!("unexpected outcome: {o:?}"),
    }
    assert_eq!((rd(&mut p, arr), rd(&mut p, arr + 8), rd(&mut p, arr + 16)), (arr, arr + 8, arr + 16));
}

#[test]
fn guest_dll_resolver_returns_zero_when_no_companion_dll_present() {
    let mut p = proc(&[]);
    p.set_drive(Drive::new());
    let table = OrdinalTable::parse("").unwrap();
    assert_eq!(dll::resolve_guest_dlls(&mut p, &table).unwrap(), 0);
}
