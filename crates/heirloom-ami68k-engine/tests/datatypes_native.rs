//! Witness + functional tests for AmigaOS `datatypes.library` bridged to `NativeDatatype`
//! (CoralOS `datatype.library` on target) and heavy real-world Amiga apps (MultiView, PicShow, Annotate).
use heirloom_ami68k_engine::Amiga;
use heirloom_common::datatype::{DecodedImage, NativeDatatype, DT_JPEG, DT_PNG};
use heirloom_common::driver::GuestMachine;
use heirloom_common::Step;
use heirloom_ui_abi::{DrawOp, GuestUiEvent, RecordingGuestUi, UiCall};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct CountingHostDatatype {
    calls: Arc<AtomicUsize>,
    use_pil: bool,
}

impl NativeDatatype for CountingHostDatatype {
    fn decode_image(&self, data: &[u8], _fit_w: u32, _fit_h: u32) -> Option<DecodedImage> {
        let k = heirloom_common::datatype::identify_magic(data);
        if self.use_pil && matches!(k, DT_PNG | DT_JPEG | heirloom_common::datatype::DT_GIF | heirloom_common::datatype::DT_WEBP) {
            use std::io::Write;
            use std::process::{Command, Stdio};
            let mut child = Command::new("python3")
                .args([
                    "-c",
                    "import sys,io,struct; from PIL import Image; im=Image.open(io.BytesIO(sys.stdin.buffer.read())).convert('RGBA'); w,h=im.size; sys.stdout.buffer.write(struct.pack('<II',w,h)+im.tobytes())",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            child.stdin.take()?.write_all(data).ok()?;
            let out = child.wait_with_output().ok()?;
            if !out.status.success() || out.stdout.len() < 8 {
                return None;
            }
            let w = u32::from_le_bytes(out.stdout[0..4].try_into().ok()?);
            let h = u32::from_le_bytes(out.stdout[4..8].try_into().ok()?);
            let raw = &out.stdout[8..];
            if raw.len() != (w as usize) * (h as usize) * 4 {
                return None;
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            let pixels = raw
                .chunks_exact(4)
                .map(|c| ((c[3] as u32) << 24) | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | (c[2] as u32))
                .collect();
            return Some(DecodedImage { width: w, height: h, pixels });
        }
        if k == DT_PNG {
            self.calls.fetch_add(1, Ordering::Relaxed);
            return Some(DecodedImage {
                width: 4,
                height: 2,
                pixels: vec![
                    0xFFFF_0000, 0xFF00_FF00, 0xFF00_00FF, 0xFFFF_FF00,
                    0xFF11_2233, 0xFF44_5566, 0xFF77_8899, 0xFFAA_BBCC,
                ],
            });
        }
        None
    }
}

fn load_dir_into(a: &mut Amiga<RecordingGuestUi>, dev: &str, dir: &Path) {
    fn walk(vol: &mut heirloom_ami68k_engine::volume::Volume, prefix: &str, dir: &Path) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let guest = if prefix.ends_with(':') { format!("{prefix}{name}") } else { format!("{prefix}/{name}") };
            if p.is_dir() {
                let _ = vol.mkdirs(&guest);
                walk(vol, &guest, &p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                let _ = vol.put_file(&guest, bytes);
            }
        }
    }
    walk(&mut a.fs.vol, dev, dir);
}

#[test]
fn native_datatype_host_serves_amiga_datatypes_library_end_to_end() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut a = Amiga::new(RecordingGuestUi::new()).expect("machine");
    a.datatype_host = Some(Box::new(CountingHostDatatype { calls: Arc::clone(&calls), use_pil: false }));
    a.fs.vol.put_file("SYS:sample.png", b"\x89PNG\r\n\x1a\nfake-png-payload".to_vec()).unwrap();

    // Build a tiny 68k Hunk calling OpenLibrary("datatypes.library"), NewDTObjectA("SYS:sample.png", NULL),
    // GetDTAttrsA(obj, {PDTA_BitMapHeader, &bmhd, TAG_DONE}), OpenWindow, DrawDTObjectA, DisposeDTObject, CloseLibrary.
    let mut code = Vec::<u8>::new();
    let emit16 = |c: &mut Vec<u8>, w: u16| c.extend_from_slice(&w.to_be_bytes());
    let emit32 = |c: &mut Vec<u8>, l: u32| c.extend_from_slice(&l.to_be_bytes());
    let base = heirloom_ami68k_engine::machine::LOAD_BASE;

    // Place strings & taglist at known fixed offset base + 0x200
    let data_base = base + 0x200;
    let dt_lib_name = data_base;       // "datatypes.library\0"
    let file_name = data_base + 0x20;  // "SYS:sample.png\0"
    let tag_list = data_base + 0x40;   // { PDTA_BitMapHeader (0x8000_10C9), bmhd_out, TAG_DONE, 0 }
    let bmhd_out = data_base + 0x60;   // u32 out
    let nw_struct = data_base + 0x80;  // struct NewWindow (48 bytes)

    // movea.l 4.w, a6
    emit16(&mut code, 0x2C78); emit16(&mut code, 0x0004);
    // lea dt_lib_name(pc)... simpler: movea.l #dt_lib_name, a1; moveq #0, d0; jsr -552(a6)
    emit16(&mut code, 0x227C); emit32(&mut code, dt_lib_name);
    emit16(&mut code, 0x7000);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-552i16) as u16);
    // movea.l d0, a5 (DataTypesBase)
    emit16(&mut code, 0x2A40);
    // movea.l a5, a6; move.l #file_name, d0; suba.l a0, a0; jsr -48(a6) (NewDTObjectA)
    emit16(&mut code, 0x2C4D);
    emit16(&mut code, 0x203C); emit32(&mut code, file_name);
    emit16(&mut code, 0x91C8);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-48i16) as u16);
    // move.l d0, d7 (obj)
    emit16(&mut code, 0x2E00);
    // GetDTAttrsA(a0=obj, a2=tag_list) -> jsr -66(a6)
    emit16(&mut code, 0x2047);
    emit16(&mut code, 0x247C); emit32(&mut code, tag_list);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-66i16) as u16);
    // OpenWindow(a0=nw_struct) via INTUITION_BASE (0x2000), LVO -204
    emit16(&mut code, 0x2C7C); emit32(&mut code, heirloom_ami68k_engine::machine::INTUITION_BASE);
    emit16(&mut code, 0x207C); emit32(&mut code, nw_struct);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-204i16) as u16);
    // movea.l d0, a4 (Window); movea.l 50(a4), a0 (RPort); movea.l d7, a1 (obj)
    emit16(&mut code, 0x2840);
    emit16(&mut code, 0x206C); emit16(&mut code, 50);
    emit16(&mut code, 0x2247);
    // DrawDTObjectA(a0=rp, a1=obj, d0=4, d1=14, d2=4, d3=2, d4=0, d5=0, a2=0) -> jsr -126(a6)
    emit16(&mut code, 0x2C4D);
    emit16(&mut code, 0x7004); emit16(&mut code, 0x720E);
    emit16(&mut code, 0x7404); emit16(&mut code, 0x7602);
    emit16(&mut code, 0x7800); emit16(&mut code, 0x7A00);
    emit16(&mut code, 0x95CA);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-126i16) as u16);
    // DisposeDTObject(a0=obj) -> jsr -54(a6) (leave window open so step() flushes PresentPlane)
    emit16(&mut code, 0x2047);
    emit16(&mut code, 0x4EAE); emit16(&mut code, (-54i16) as u16);
    // rts
    emit16(&mut code, 0x4E75);

    assert!(code.len() <= 0x200);
    code.resize(0x300, 0);
    let put = |buf: &mut [u8], off: usize, bytes: &[u8]| buf[off..off + bytes.len()].copy_from_slice(bytes);
    put(&mut code, 0x200, b"datatypes.library\0");
    put(&mut code, 0x220, b"SYS:sample.png\0");
    // PDTA_BitMapHeader = 0x8000_1000 + 201 = 0x8000_10C9
    put(&mut code, 0x240, &0x8000_10C9u32.to_be_bytes());
    put(&mut code, 0x244, &bmhd_out.to_be_bytes());
    // struct NewWindow: Left=0, Top=0, Width=100, Height=80, Flags=0x1000 (WFLG_ACTIVATE)
    put(&mut code, 0x280 + 4, &100u16.to_be_bytes());
    put(&mut code, 0x280 + 6, &80u16.to_be_bytes());
    put(&mut code, 0x280 + 14, &0x0000_1000u32.to_be_bytes());

    let mut hunk = Vec::new();
    let put32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_be_bytes());
    let longs = (code.len() as u32 + 3) / 4;
    put32(&mut hunk, 0x0000_03F3); // HUNK_HEADER
    put32(&mut hunk, 0);           // no resident libs
    put32(&mut hunk, 1);           // 1 hunk
    put32(&mut hunk, 0);           // first 0
    put32(&mut hunk, 0);           // last 0
    put32(&mut hunk, longs);       // size in longs
    put32(&mut hunk, 0x0000_03E9); // HUNK_CODE
    put32(&mut hunk, longs);
    hunk.extend_from_slice(&code);
    while hunk.len() % 4 != 0 { hunk.push(0); }
    put32(&mut hunk, 0x0000_03F2); // HUNK_END
    let r = a.run_hunk(&hunk, 100_000).expect("run");
    assert_eq!(r.stats.bus_faults, 0);
    assert!(r.stats.halted, "guest must finish cleanly");
    assert_eq!(calls.load(Ordering::Relaxed), 1, "must invoke NativeDatatype host decoder once");
    assert_eq!(a.datatypes.native_decodes, 1);

    let bmhd_ptr = a.mem.read_u32(bmhd_out).unwrap();
    assert_ne!(bmhd_ptr, 0, "PDTA_BitMapHeader must be populated");
    assert_eq!(a.mem.read_u16(bmhd_ptr).unwrap(), 4, "BitMapHeader width");
    assert_eq!(a.mem.read_u16(bmhd_ptr + 2).unwrap(), 2, "BitMapHeader height");

    let has_pixels = a.ui.calls.iter().any(|c| match c {
        UiCall::PresentPlane(_, plane) => plane.ops.iter().any(|op| match op {
            DrawOp::Pixels { rgb, .. } => rgb.contains(&0x00FF_0000) && rgb.contains(&0x00AA_BBCC),
            _ => false,
        }),
        _ => false,
    });
    assert!(has_pixels, "DrawDTObjectA must publish truecolor native pixels to PresentPlane");
}

#[test]
fn heavy_amiga_apps_multiview_picshow_annotate_work_end_to_end() {
    let mv_path = Path::new("/data/build/corpus-bin/aros-m68k/sys/Utilities/MultiView");
    let aros_mini = Path::new("/data/build/corpus-bin/aros-m68k/mini");
    let picshow_dir = Path::new("/tmp/heirloom-heavy-apps/amiga/picshow/PicShow/PicShow");
    let ann_dir = Path::new("/tmp/heirloom-heavy-apps/amiga/annotate/Annotate_usr/Annotate");
    if !mv_path.exists() || !picshow_dir.exists() || !ann_dir.exists() {
        eprintln!("SKIPPED heavy_amiga_apps_multiview_picshow_annotate_work_end_to_end (local corpus absent)");
        return;
    }

    // 1. MultiView opening real 800x600 JPEG via NativeDatatype + native gadget.library menus + clean Quit
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        a.datatype_host = Some(Box::new(CountingHostDatatype { calls: Arc::clone(&calls), use_pil: true }));
        load_dir_into(&mut a, "SYS:", aros_mini);
        a.fs.vol.add_device("APP", false);
        load_dir_into(&mut a, "APP:", picshow_dir);
        a.fs.vol.assign("PROGDIR", "APP:");
        a.cli_args = Some(String::from("APP:PicShow.jpg\n"));
        let bytes = std::fs::read(mv_path).unwrap();
        a.start_file(&bytes, "MultiView").unwrap();
        let (st, stats) = a.step(2_000_000);
        assert_eq!(stats.bus_faults, 0);
        assert!(matches!(st, Step::Blocked | Step::BudgetSpent), "MultiView running: {st:?}");
        assert!(calls.load(Ordering::Relaxed) >= 1, "MultiView must decode JPEG via NativeDatatype");
        let win = *a.windows.values().next().expect("MultiView window open");
        let quit_tok = a.ui.calls.iter().find_map(|c| match c {
            UiCall::SetMenuStrip(_, mm) => mm.menus.iter().flat_map(|m| &m.items).find(|i| i.label == "Quit").map(|i| i.token),
            _ => None,
        }).expect("MultiView Quit menu item");
        a.deliver(GuestUiEvent::MenuCommand { win, token: quit_tok }).unwrap();
        let (st2, stats2) = a.step(2_000_000);
        assert_eq!(stats2.bus_faults, 0);
        assert_eq!(st2, Step::Exited, "MultiView must quit cleanly on Project->Quit");
    }

    // 2. PicShow decoding real 800x600 JPEG via NativeDatatype + ILBM nav skin + 5 menus + clean Quit
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        a.datatype_host = Some(Box::new(CountingHostDatatype { calls: Arc::clone(&calls), use_pil: true }));
        a.fs.vol.add_device("APP", false);
        load_dir_into(&mut a, "APP:", picshow_dir);
        a.fs.vol.assign("PROGDIR", "APP:");
        a.cli_args = Some(String::from("APP:PicShow.jpg\n"));
        let bytes = std::fs::read(picshow_dir.join("PicShow")).unwrap();
        a.start_file(&bytes, "PicShow").unwrap();
        let mut st = Step::BudgetSpent;
        for _ in 0..8 {
            let (s, stats) = a.step(5_000_000);
            assert_eq!(stats.bus_faults, 0);
            st = s;
            if st == Step::Blocked {
                if let Some(dl) = a.next_timer_deadline() {
                    a.advance_clock_to(dl).unwrap();
                    continue;
                }
            }
            break;
        }
        assert!(matches!(st, Step::Blocked | Step::BudgetSpent), "PicShow running: {st:?}");
        assert!(calls.load(Ordering::Relaxed) >= 1, "PicShow must decode JPEG via NativeDatatype");
        let picshow_pixels = a.ui.calls.iter().any(|c| match c {
            UiCall::PresentPlane(_, p) => p.ops.iter().any(|op| matches!(op, DrawOp::Pixels { rect, rgb } if rect.width >= 320 && rect.height >= 200 && rgb.iter().any(|&px| px != 0))),
            _ => false,
        });
        assert!(picshow_pixels, "PicShow must blit decoded JPEG truecolor pixels to its window plane");
        let (win, quit_tok) = a.ui.calls.iter().find_map(|c| match c {
            UiCall::SetMenuStrip(w, mm) => mm.menus.iter().flat_map(|m| &m.items).find(|i| i.label == "Quit").map(|i| (*w, i.token)),
            _ => None,
        }).expect("PicShow Quit menu item");
        a.deliver(GuestUiEvent::MenuCommand { win, token: quit_tok }).unwrap();
        let mut st2 = Step::BudgetSpent;
        for _ in 0..8 {
            let (s, stats2) = a.step(5_000_000);
            assert_eq!(stats2.bus_faults, 0);
            st2 = s;
            if st2 == Step::Exited { break; }
            if st2 == Step::Blocked {
                if let Some(dl) = a.next_timer_deadline() {
                    a.advance_clock_to(dl).unwrap();
                    continue;
                }
            }
            break;
        }
        if st2 != Step::Exited {
            eprintln!("PicShow post-Quit log tail: {:?}", &a.log[a.log.len().saturating_sub(20)..]);
        }
        assert_eq!(st2, Step::Exited, "PicShow must quit cleanly on Project->Quit");
    }

    // 3. Annotate 3.1.1 (621 KB programmer's editor) opening 542-line README + native text ops + menus + clean Quit
    {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        a.ui.auto_cancel_requests = false;
        a.fs.vol.add_device("APP", false);
        load_dir_into(&mut a, "APP:", ann_dir);
        a.fs.vol.assign("PROGDIR", "APP:");
        a.cli_args = Some(String::from("APP:Annotate.readme\n"));
        let bytes = std::fs::read(ann_dir.join("Ann.AmigaOS3")).unwrap();
        a.start_file(&bytes, "Annotate").unwrap();
        let mut answered = std::collections::BTreeSet::new();
        let mut st = Step::BudgetSpent;
        for _ in 0..8 {
            let (s, stats) = a.step(5_000_000);
            assert_eq!(stats.bus_faults, 0);
            st = s;
            let mut did_answer = false;
            let reqs: Vec<_> = a.ui.open_requests().to_vec();
            for req in reqs {
                if answered.insert(req.id) {
                    let _ = a.deliver(GuestUiEvent::RequestAnswered { req, button: 0 });
                    did_answer = true;
                }
            }
            if did_answer { continue; }
            if st == Step::Blocked {
                if let Some(dl) = a.next_timer_deadline() {
                    a.advance_clock_to(dl).unwrap();
                    continue;
                }
            }
            break;
        }
        assert!(matches!(st, Step::Blocked | Step::BudgetSpent), "Annotate running: {st:?}");
        let loaded_text = a.ui.calls.iter().any(|c| match c {
            UiCall::PresentPlane(_, p) => p.ops.iter().any(|op| match op {
                DrawOp::Text { string, .. } => string.contains("APP:Annotate.readme") || string.contains("Short:"),
                _ => false,
            }),
            _ => false,
        });
        assert!(loaded_text, "Annotate must load and render APP:Annotate.readme as native DrawOp::Text");
        let (win, quit_tok) = a.ui.calls.iter().find_map(|c| match c {
            UiCall::SetMenuStrip(w, mm) => mm.menus.iter().flat_map(|m| &m.items).find(|i| i.label == "Quit").map(|i| (*w, i.token)),
            _ => None,
        }).expect("Annotate Quit menu item");
        a.deliver(GuestUiEvent::MenuCommand { win, token: quit_tok }).unwrap();
        let mut st2 = Step::BudgetSpent;
        for _ in 0..8 {
            let (s, stats2) = a.step(5_000_000);
            assert_eq!(stats2.bus_faults, 0);
            st2 = s;
            if st2 == Step::Exited { break; }
            let mut did_answer = false;
            let reqs: Vec<_> = a.ui.open_requests().to_vec();
            for req in reqs {
                if answered.insert(req.id) {
                    let _ = a.deliver(GuestUiEvent::RequestAnswered { req, button: 0 });
                    did_answer = true;
                }
            }
            if did_answer { continue; }
            if st2 == Step::Blocked {
                if let Some(dl) = a.next_timer_deadline() {
                    a.advance_clock_to(dl).unwrap();
                    continue;
                }
            }
            break;
        }
        if st2 != Step::Exited {
            eprintln!("Annotate post-Quit calls: {:?}", a.ui.calls.iter().rev().take(5).collect::<Vec<_>>());
            eprintln!("Annotate post-Quit log: {:?}", &a.log[a.log.len().saturating_sub(15)..]);
        }
        assert_eq!(st2, Step::Exited, "Annotate must quit cleanly on Project->Quit");
    }
}
