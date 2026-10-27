//! The CDL layout path against the owner's local inputs (all skip cleanly when absent): the Symbian Foundation UI-resources source
//! tree (reference) the extractor reads, and the SDK's emulator build of the same layout DLL, which holds the very same byte-coded data.
use heirloom_ekaarm_engine::cdl::CdlState;
use heirloom_ekaarm_engine::cdlextract;
use heirloom_ekaarm_engine::layoutmath::{compose, layout_line, layout_rect, Rect};
use std::path::Path;

const TREE: &str = "/data/refs/heirloom/symbian-uiresources/layouts/aknlayout2";
const RSG: &str = "/data/build/corpus-symbian/sdk/sdk/CPP_API/epoc32/include/avkon.rsg";
/// The emulator build of the 360x640 touch layout instances (`akn_app_qhd_touch.mk`: `uid = 0x10283389`), and of the shared ones
/// (`common_dll = 101fe2aa.dll`: the LayoutMetaData instances live there).
const UDEB: &str = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb";

fn qhd_dll() -> String {
    format!("{UDEB}/10283389.dll")
}

fn extracted() -> Option<String> {
    if !Path::new(TREE).is_dir() || !Path::new(RSG).is_file() || !Path::new(&qhd_dll()).is_file() {
        println!("SKIP: {TREE}, {RSG} or the SDK's layout DLL absent");
        return None;
    }
    Some(cdlextract::extract(Path::new(TREE), "Qhd_touch_akn_app", Some(Path::new(RSG)), Some(Path::new(&qhd_dll()))).expect("extract").0)
}

fn loaded() -> Option<CdlState> {
    let text = extracted()?;
    let mut s = CdlState::default();
    s.load(text.as_bytes(), 360, 640).expect("load");
    Some(s)
}

/// The layout data the engine reads is the SDK DLL's own: the instance's lookup and base address land inside the DLL's section the
/// blob was cut from, the source tree's later api names are cut to the DLL's api count, and the interface whose ids differ from
/// the source's (`AknLayoutScalable_Apps`) carries no names rather than wrong ones.
#[test]
fn layout_data_is_the_sdk_dlls_and_names_stop_where_the_dlls_apis_stop() {
    let Some(text) = extracted() else { return };
    let dll = std::fs::read(qhd_dll()).unwrap();
    let mut s = CdlState::default();
    s.load(text.as_bytes(), 360, 640).unwrap();
    let data = s.data.as_ref().unwrap();
    assert_eq!(data.blobs.len(), 1, "one section: the DLL's .rdata");
    let blob = data.blobs.values().next().unwrap();
    assert!(dll.windows(64).any(|w| w == &blob[..64]), "the blob is the DLL's bytes");
    let avkon = s.uid_of("AknLayoutScalable_Avkon").unwrap();
    assert_eq!(data.interfaces[&avkon].1.len(), 5192, "the SDK's avkon interface has 5192 apis (the source tree's later one has more)");
    let inst = data.instances.iter().find(|i| i.name == "AknLayoutScalable_Elaf_php_av_qhd_prt_tch_Normal").unwrap();
    assert_eq!((inst.lookup.len(), inst.ops.len()), (5192, 5192));
    assert!(inst.lookup.iter().all(|&o| inst.data.as_ref().unwrap().1 + (o as usize) < blob.len()));
    let apps = s.uid_of("AknLayoutScalable_Apps").unwrap();
    assert!(data.interfaces[&apps].1.is_empty(), "apps api ids differ between the DLL and the source tree");
    assert!(s.window_layout(apps, "main_pane", 0).unwrap_err().contains("no api"));
}

/// The metadata constants come from the source tree; the SDK's own DLL (101fe2aa.dll) implements the same constants:
/// every instance returns `mov eax, 0|1` for the api it implements.
#[test]
fn metadata_constants_match_the_sdk_dll() {
    let Some(text) = extracted() else { return };
    let Ok(dll) = std::fs::read(format!("{UDEB}/101fe2aa.dll")) else { println!("SKIP: 101fe2aa.dll absent"); return };
    let pe = heirloom_ekaarm_engine::cdlpe::Pe::parse(&dll).unwrap();
    let custs = pe.customisations().unwrap();
    let ids = cdlextract::instance_ids(&Path::new(TREE).join("generated").join("dllinstances.hrh"));
    let mut s = CdlState::default();
    s.load(text.as_bytes(), 360, 640).unwrap();
    let mut checked = 0;
    for inst in s.data.as_ref().unwrap().instances.iter().filter(|i| i.name.starts_with("LayoutMetaData_")) {
        let c = custs.iter().find(|c| c.id == ids[&inst.name.to_ascii_lowercase()]).unwrap();
        assert_eq!(c.api_size as usize, inst.ops.len(), "{}", inst.name);
        for (k, op) in inst.ops.iter().enumerate() {
            let f = pe.thunk(c, k as u32).unwrap_or(0);
            match op {
                heirloom_ekaarm_engine::cdl::Op::None => assert_eq!(f, 0, "{} api {k}", inst.name),
                heirloom_ekaarm_engine::cdl::Op::Const(v) => {
                    // push ebp; mov ebp, esp; mov eax, imm32; leave; ret
                    let code = pe.bytes(f, 10).unwrap();
                    assert_eq!(&code[..4], &[0x55, 0x89, 0xe5, 0xb8], "{} api {k}", inst.name);
                    assert_eq!(u32::from_le_bytes([code[4], code[5], code[6], code[7]]), *v, "{} api {k}", inst.name);
                    checked += 1;
                }
                other => panic!("{other:?}"),
            }
        }
    }
    assert!(checked >= 16, "{checked}");
}

#[test]
fn the_default_pack_stacks_metadata_by_the_device_profile() {
    let Some(s) = loaded() else { return };
    assert_eq!(s.pack.as_deref(), Some("Elaf_Qhd_touch_Normal_Pack"));
    // defaults all false, then scrollbar and pensupport on top (the pack's contents): a touch phone in portrait.
    assert_eq!(s.meta("IsPenEnabled"), Ok(true));
    assert_eq!(s.meta("IsScrollbarEnabled"), Ok(true));
    assert_eq!(s.meta("IsLandscapeOrientation"), Ok(false));
    assert_eq!(s.meta("IsMirrored"), Ok(false));
    assert_eq!(s.meta("IsMSKEnabled"), Ok(false), "msk = no in configure_qhd_touch.mk");
    assert!(CdlState::default().implementor(0x10204fc6, 0).unwrap_err().contains("no CDL layout data"));
}

/// The decoded geometry is internally consistent (the numbers come from the data; the checks are relations that must hold of any
/// sane layout): the screen is 360x640; in the small status pane layout the top area, the main pane and the control pane tile the
/// application window top to bottom without gaps.
#[test]
fn decoded_panes_tile_the_screen() {
    let Some(s) = loaded() else { return };
    let uid = s.uid_of("AknLayoutScalable_Avkon").unwrap();
    let win = |name: &str, v: i32| s.window_layout(uid, name, v).unwrap_or_else(|e| panic!("{name}({v}): {e}"));
    let line = layout_line(&win("Screen", 0));
    assert_eq!((line.w, line.h), (360, 640));
    let screen = Rect::new(0, 0, 360, 640);
    let app = layout_rect(screen, &win("application_window", 0));
    assert_eq!(app, screen);
    let top = layout_rect(app, &win("area_top_pane", 1));
    let main = layout_rect(screen, &compose(win("application_window", 0), win("main_pane", 6)));
    let bottom = layout_rect(screen, &win("area_bottom_pane", 0));
    assert!(top.height() > 0 && main.height() > 0 && bottom.height() > 0, "{top:?} {main:?} {bottom:?}");
    assert_eq!((top.x0, top.x1), (0, 360));
    assert_eq!((main.x0, main.x1), (0, 360));
    assert_eq!(top.y0, 0, "the small status pane is the top of the screen");
    assert_eq!(main.y0, top.y1, "the main pane starts where the top area ends");
    assert_eq!(main.y1, bottom.y0, "the main pane ends where the control pane starts");
    assert_eq!(bottom.y1, 640);
    // the table lookups agree with a direct line: a table of the same pane would be asked through `ParameterLimits`.
    let uid_meta = s.uid_of("Layout_Meta_Data").unwrap();
    assert!(s.api_id(uid_meta, "IsPenEnabled").is_ok());
    assert!(s.window_layout(uid, "no_such_pane", 0).is_err());
    assert!(s.window_layout(uid, "main_pane", 9999).unwrap_err().contains("variety"), "a variety past the pane's is refused");
}

/// FileBrowser, given the layout data, gets past its first layout queries (the CDL meta data functions, the main and navigation pane
/// rectangles, the container's SetRect, the navigation pane's label), constructs its list box (tests/listbox.rs), activates its view and waits in its scheduler (tests/keys.rs).
#[test]
fn file_browser_runs_through_its_list_box_construction() {
    use heirloom_ekaarm_engine::hostdir::{load_dir, DirRom};
    use heirloom_ekaarm_engine::{loader, Outcome};
    use heirloom_fmt_e32::OrdinalTable;
    const ORD_TSV: &str = include_str!("../../heirloom-fmt-e32/data/eka-ordinals.tsv");
    let exe = "/data/build/corpus-symbian/e32/FileBrowser.exe";
    let tree = "/data/build/corpus-symbian/drive/nokiafilebrowser";
    let rom = "/data/build/corpus-symbian/sdk/sdk/Emulator/epoc32/release/winscw/udeb/z";
    let Some(text) = extracted() else { return };
    let Ok(bytes) = std::fs::read(exe) else { println!("SKIP: {exe} absent"); return };
    if !Path::new(tree).is_dir() || !Path::new(rom).is_dir() { println!("SKIP: corpus absent"); return; }
    let mut p = loader::load(&bytes, &OrdinalTable::parse(ORD_TSV).unwrap()).unwrap();
    let (mut drive, _) = load_dir(Path::new(tree)).unwrap();
    drive.set_rom(Box::new(DirRom::new(Path::new(rom)).unwrap()));
    p.set_drive(drive);
    p.set_ui(heirloom_ui_abi::RecordingGuestUi::new());
    p.set_cdl_data(text.as_bytes()).unwrap();
    p.trace = Some(Vec::new());
    // Past the list box and every view activation: the application waits in its scheduler for its first event.
    match p.run(50_000_000) { Outcome::Blocked => {}, o => panic!("{o:?}") }
    let trace = p.trace.as_ref().unwrap().join("\n");
    for needle in ["CdlEngine11GetFunctionE4TUidi", "cdl@", "LayoutMetricsRect", "CreateMessageLabelL", "5PushL"] {
        assert!(trace.contains(needle), "{needle} not called:\n{trace}");
    }
    // the navigation pane got a decorator and pushed it; the label text is the application's own
    assert_eq!(p.nat.shell.navi_stack.len(), 1);
}
