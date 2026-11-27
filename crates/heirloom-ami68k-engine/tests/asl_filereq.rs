//! Host oracle tests for the stateless Amiga `asl.library` file requester adapter (`amiga_asl.rs`, REUSE U9).
//!
//! Proves:
//! 1. `AslRequest(ASL_FileRequest)` calls `GuestUi::open_file_request` with parsed ASL tags (title, initial_dir,
//!    initial_file, Open/Save mode), parks until `FileRequestAnswered`, populates `fr_Drawer` and `fr_File` in
//!    guest RAM, and returns `1` (`TRUE`).
//! 2. Cancel (`FileRequestAnswered { path: None }`) returns `0` (`FALSE`).
//! 3. NEGATIVE CONTROLS: unsupported host (`RecordingGuestUi::new()` default) returns `0` (`FALSE`);
//!    stale `FileRequestAnswered` for an unknown `RequestRef` is refused by name.
mod common;
use common::{machine, RES};
use heirloom_ami68k_engine::asl::split_drawer_and_file;
use heirloom_fmt_hunk::writer::{build, Hunk};
use heirloom_ui_abi::{FileRequestMode, FileRequestModel, GuestUiEvent, UiCall, UiObjectRef};

/// Tiny 68k program that opens `asl.library`, calls `AllocAslRequest(0, tags)` -> `AslRequest(req, NULL)`,
/// copies `fr_Drawer` and `fr_File` pointers + result to RES, and calls `FreeAslRequest`.
fn build_asl_prog(title_addr: u32, dir_addr: u32, file_addr: u32, save_mode: u32) -> (Vec<u8>, Vec<u8>) {
    // Tags table placed at $50000:
    // ASLFR_TitleText (0x80080001), ASLFR_InitialFile (0x80080008), ASLFR_InitialDrawer (0x80080009),
    // ASLFR_DoSaveMode (0x8008002C), TAG_DONE (0)
    let mut tags = Vec::new();
    for v in [
        0x8008_0001u32, title_addr,
        0x8008_0009u32, dir_addr,
        0x8008_0008u32, file_addr,
        0x8008_002Cu32, save_mode,
        0u32, 0u32,
    ] {
        tags.extend(v.to_be_bytes());
    }
    // Hand-assembled 68k code (using ExecBase at 4, OpenLibrary = -552, CloseLibrary = -414,
    // AllocAslRequest = -48, FreeAslRequest = -54, AslRequest = -60):
    let mut code = Vec::new();
    fn w(code: &mut Vec<u8>, x: u16) { code.extend(x.to_be_bytes()); }
    fn l(code: &mut Vec<u8>, x: u32) { code.extend(x.to_be_bytes()); }
    // movea.l 4.w, a6
    w(&mut code, 0x2C78); w(&mut code, 0x0004);
    // lea $50200, a1 ("asl.library") ; moveq #0, d0 ; jsr -552(a6)
    w(&mut code, 0x43F9); l(&mut code, 0x0005_0200);
    w(&mut code, 0x7000);
    w(&mut code, 0x4EAE); w(&mut code, (-552i16) as u16);
    // movea.l d0, a5 (save AslBase) ; movea.l d0, a6
    w(&mut code, 0x2A40); w(&mut code, 0x2C40);
    // moveq #0, d0 (ASL_FileRequest) ; lea $50000, a0 (tags) ; jsr -48(a6) (AllocAslRequest)
    w(&mut code, 0x7000);
    w(&mut code, 0x41F9); l(&mut code, 0x0005_0000);
    w(&mut code, 0x4EAE); w(&mut code, (-48i16) as u16);
    // move.l d0, d7 (save req) ; move.l d0, RES+0
    w(&mut code, 0x2E00);
    w(&mut code, 0x23C0); l(&mut code, RES);
    // movea.l d7, a0 ; suba.l a1, a1 ; jsr -60(a6) (AslRequest)
    w(&mut code, 0x2047); w(&mut code, 0x93C9);
    w(&mut code, 0x4EAE); w(&mut code, (-60i16) as u16);
    // move.l d0, RES+4 (AslRequest result)
    w(&mut code, 0x23C0); l(&mut code, RES + 4);
    // movea.l d7, a0 ; move.l 4(a0), RES+8 (fr_File) ; move.l 8(a0), RES+12 (fr_Drawer)
    w(&mut code, 0x2047);
    w(&mut code, 0x23E8); w(&mut code, 4); l(&mut code, RES + 8);
    w(&mut code, 0x23E8); w(&mut code, 8); l(&mut code, RES + 12);
    // Copy 32 bytes from fr_Drawer (RES+12) -> $50400 and fr_File (RES+8) -> $50480 before FreeAslRequest.
    // tst.l RES+12 ; beq.s +18 ; movea.l RES+12, a0 ; lea $50400, a1 ; moveq #31, d1 ; move.b (a0)+,(a1)+ ; dbf d1,-2
    w(&mut code, 0x4AB9); l(&mut code, RES + 12);
    w(&mut code, 0x6712); // beq +18 bytes
    w(&mut code, 0x2079); l(&mut code, RES + 12);
    w(&mut code, 0x43F9); l(&mut code, 0x0005_0400);
    w(&mut code, 0x721F);
    w(&mut code, 0x12D8);
    w(&mut code, 0x51C9); w(&mut code, 0xFFFC);
    // Same for fr_File (RES+8) -> $50480:
    w(&mut code, 0x4AB9); l(&mut code, RES + 8);
    w(&mut code, 0x6712);
    w(&mut code, 0x2079); l(&mut code, RES + 8);
    w(&mut code, 0x43F9); l(&mut code, 0x0005_0480);
    w(&mut code, 0x721F);
    w(&mut code, 0x12D8);
    w(&mut code, 0x51C9); w(&mut code, 0xFFFC);
    // movea.l d7, a0 ; jsr -54(a6) (FreeAslRequest)
    w(&mut code, 0x2047);
    w(&mut code, 0x4EAE); w(&mut code, (-54i16) as u16);
    // CloseLibrary(AslBase): movea.l 4.w, a6 ; movea.l a5, a1 ; jsr -414(a6) ; rts
    w(&mut code, 0x2C78); w(&mut code, 0x0004);
    w(&mut code, 0x224D);
    w(&mut code, 0x4EAE); w(&mut code, (-414i16) as u16);
    w(&mut code, 0x4E75);
    (code, tags)
}

fn setup_and_run(
    a: &mut heirloom_ami68k_engine::Amiga<heirloom_ui_abi::RecordingGuestUi>,
    title: &str,
    dir: &str,
    file: &str,
    save_mode: u32,
) {
    let (title_addr, dir_addr, file_addr) = (0x50240u32, 0x50280u32, 0x502C0u32);
    let mut write_z = |at: u32, s: &str| {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        a.mem.write_bytes(at, &b).unwrap();
    };
    write_z(0x50200, "asl.library");
    write_z(title_addr, title);
    write_z(dir_addr, dir);
    write_z(file_addr, file);
    let (code, tags) = build_asl_prog(title_addr, dir_addr, file_addr, save_mode);
    a.mem.write_bytes(0x50000, &tags).unwrap();
    let r = a.run_hunk(&build(&[Hunk::code(code, vec![])], None), 1_000_000).unwrap();
    assert!(r.stats.halted && r.stats.bus_faults == 0, "{:?} {:?}\n{:#?}", r.stats, a.refused, a.log);
}

#[test]
fn split_drawer_and_file_handles_coralos_and_amiga_paths() {
    assert_eq!(split_drawer_and_file("HOME:Documents/notes.txt", false), ("HOME:Documents".into(), "notes.txt".into()));
    assert_eq!(split_drawer_and_file("RAM:out.iff", false), ("RAM:".into(), "out.iff".into()));
    assert_eq!(split_drawer_and_file("Documents/sub", true), ("Documents/sub".into(), String::new()));
}

#[test]
fn asl_file_request_opens_native_requester_and_populates_drawer_and_file() {
    let mut a = machine();
    a.ui.file_answers.push_back(Some(String::from("HOME:Documents/notes.txt")));
    setup_and_run(&mut a, "Open Document", "HOME:Documents", "readme.txt", 0);

    assert_eq!(a.mem.read_u32(RES + 4).unwrap(), 1, "AslRequest returned TRUE");
    assert_eq!(a.mem.read_cstr(0x50400).unwrap(), "HOME:Documents");
    assert_eq!(a.mem.read_cstr(0x50480).unwrap(), "notes.txt");
    assert!(
        a.ui.calls.iter().any(|c| matches!(
            c,
            UiCall::OpenFileRequest(None, FileRequestModel { title, mode: FileRequestMode::Open, initial_dir, initial_file })
            if title == "Open Document" && initial_dir == "HOME:Documents" && initial_file == "readme.txt"
        )),
        "host received OpenFileRequest with ASL tags: {:?}",
        a.ui.calls
    );
    assert!(a.refused.is_empty(), "{:?}", a.refused);

    // Save mode + Cancel -> returns FALSE (0).
    let mut a2 = machine();
    a2.ui.file_answers.push_back(None);
    setup_and_run(&mut a2, "Save As", "RAM:", "out.txt", 1);
    assert_eq!(a2.mem.read_u32(RES + 4).unwrap(), 0, "Cancel returns FALSE");
    assert!(a2.ui.calls.iter().any(|c| matches!(
        c,
        UiCall::OpenFileRequest(None, FileRequestModel { mode: FileRequestMode::Save, .. })
    )));

    // NEGATIVE CONTROLS: unsupported host returns FALSE without blocking; stale FileRequestAnswered refused by name.
    let mut a3 = machine();
    setup_and_run(&mut a3, "No UI", "", "", 0);
    assert_eq!(a3.mem.read_u32(RES + 4).unwrap(), 0, "unsupported host returns FALSE");
    let stale = a3.deliver(GuestUiEvent::FileRequestAnswered {
        req: UiObjectRef { id: 999, generation: 1 },
        path: Some("RAM:bad".into()),
    });
    assert!(stale.is_err(), "stale FileRequestAnswered must be refused by name");
}
