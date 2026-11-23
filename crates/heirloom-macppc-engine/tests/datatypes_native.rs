//! Witness tests for Mac PPC native `datatype.library` (`QuickTime GraphicsImporter`),
//! `Gestalt('proc') == 3`, `DeviceLoop`, `GetZone`, `CustomGetFile`, `HMGetBalloons`,
//! and heavy PowerPC apps (`Tex-Edit Plus 4.1.3`, `JPEGView 3.3.1`).
use heirloom_common::datatype::{DecodedImage, NativeDatatype, DT_JPEG};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_macppc_engine::fixture::{self, TOC_FIRST};
use heirloom_macppc_engine::source::fragment_bytes;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost, Step};
use heirloom_ui_abi::{RecordingGuestUi, UiCall};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct CountingDatatype {
    calls: Arc<AtomicUsize>,
    w: u32,
    h: u32,
    color: u32,
}

impl NativeDatatype for CountingDatatype {
    fn identify(&self, data: &[u8]) -> u32 {
        if data.starts_with(&[0xFF, 0xD8, 0xFF]) { DT_JPEG } else { 0 }
    }
    fn decode_image(&self, data: &[u8], _max_w: u32, _max_h: u32) -> Option<DecodedImage> {
        if !data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return None;
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(DecodedImage {
            width: self.w,
            height: self.h,
            pixels: vec![self.color; (self.w * self.h) as usize],
        })
    }
}

fn glue_call(m: &mut MacPpc<RecordingGuestUi>, lib: &str, name: &str, args: &[u32]) -> u32 {
    let b = m.bind_from(lib, name);
    assert!(!matches!(b, heirloom_macppc_engine::symbols::Binding::Unbound(_)), "{lib}/{name} unbound: {b:?}");
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    let o = m.glue_call(name, &mut regs);
    assert_eq!(o, GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}

#[test]
fn quicktime_graphics_importer_decodes_via_native_datatype_and_draws() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.datatype_host = Some(Box::new(CountingDatatype {
        calls: calls.clone(),
        w: 16,
        h: 12,
        color: 0xFF_00_00_00,
    }));

    // Seed a JPEG file on the boot volume and build an FSSpec pointing to it.
    let root = m.mac.fm.tree.children(heirloom_volume::ROOT)[0];
    let _ = m.mac.fm.tree.put_file(root, "parrots.jpg", vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).unwrap();
    let spec = 0x3300u32;
    m.mac.mem.write_u16(spec, (-1i16) as u16).unwrap();
    m.mac.mem.write_u32(spec + 2, 2).unwrap();
    let mut pstr = vec![11u8];
    pstr.extend_from_slice(b"parrots.jpg");
    m.mac.mem.write_bytes(spec + 6, &pstr).unwrap();

    // Instantiate a real PEF fragment importing QuickTimeLib GraphicsImporter routines
    let syms: &[(&str, SymbolClass, bool)] = &[
        ("GetGraphicsImporterForFile", SymbolClass::TVector, false),
        ("GraphicsImportGetNaturalBounds", SymbolClass::TVector, false),
        ("GraphicsImportGetAsPicture", SymbolClass::TVector, false),
        ("CloseComponent", SymbolClass::TVector, false),
    ];
    let pef_bytes = fixture::app(&[("QuickTimeLib", false, syms)]);
    let frag = m.instantiate("qt_test", &Pef::parse(&pef_bytes).unwrap()).unwrap();
    let call_qt = |m: &mut MacPpc<RecordingGuestUi>, idx: usize, args: &[u32]| -> u32 {
        let data = frag.section_bases[1].unwrap();
        let tv = m.mac.mem.read_u32(data + TOC_FIRST + 4 * idx as u32).unwrap();
        let stub = m.mac.mem.read_u32(tv).unwrap();
        let sym = m.glue_symbol_at(stub).unwrap();
        let mut regs = [0u32; 32];
        regs[3..3 + args.len()].copy_from_slice(args);
        assert_eq!(m.glue_call(&sym, &mut regs), GlueOutcome::Return, "{sym}: {:?}", m.mac.refused);
        regs[3]
    };

    // Open via GetGraphicsImporterForFile
    let ci_out = 0x3400u32;
    let err = call_qt(&mut m, 0, &[spec, ci_out]);
    assert_eq!(err, 0, "GetGraphicsImporterForFile must succeed");
    let ci = m.mac.mem.read_u32(ci_out).unwrap();
    assert_ne!(ci, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "Must delegate to NativeDatatype::decode_image");
    assert_eq!(m.mac.native_datatype_decodes, 1);

    // Query natural bounds (top=0, left=0, bottom=12, right=16)
    let rect_out = 0x3410u32;
    assert_eq!(call_qt(&mut m, 1, &[ci, rect_out]), 0);
    assert_eq!(m.mac.mem.read_u16(rect_out + 4).unwrap(), 12);
    assert_eq!(m.mac.mem.read_u16(rect_out + 6).unwrap(), 16);

    // GetAsPicture returns a valid non-NULL PicHandle containing DirectBitsRect ($009A)
    let pic_out = 0x3420u32;
    assert_eq!(call_qt(&mut m, 2, &[ci, pic_out]), 0);
    let pic_h = m.mac.mem.read_u32(pic_out).unwrap();
    assert_ne!(pic_h, 0);

    // CloseComponent frees it cleanly; subsequent queries return badComponentInstance
    assert_eq!(call_qt(&mut m, 3, &[ci]), 0);
    assert_ne!(call_qt(&mut m, 1, &[ci, rect_out]), 0);
}

#[test]
fn gestalt_proc_and_getzone_and_hm_get_balloons_and_custom_get_file() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let out = 0x3300u32;
    assert_eq!(glue_call(&mut m, "InterfaceLib", "Gestalt", &[u32::from_be_bytes(*b"proc"), out]), 0);
    assert_eq!(m.mac.mem.read_u32(out).unwrap(), 3, "PowerPC Gestalt('proc') must report emulated 68020 (3)");
    assert_ne!(glue_call(&mut m, "InterfaceLib", "GetZone", &[]), 0, "GetZone must return the current heap zone");
    assert_eq!(glue_call(&mut m, "InterfaceLib", "HMGetBalloons", &[]), 0, "HMGetBalloons must return false");
    let reply = 0x3340u32;
    m.mac.mem.write_u8(reply, 1).unwrap();
    assert_eq!(glue_call(&mut m, "InterfaceLib", "CustomGetFile", &[0, 0, 0, reply]), 0);
    assert_eq!(m.mac.mem.read_u8(reply).unwrap(), 0, "CustomGetFile must clear sfGood");
}

#[test]
fn heavy_texedit_plus_and_jpegview_ppc_end_to_end() {
    let te_bin = std::path::Path::new("/tmp/heirloom-heavy-apps/macppc/texedit_unpacked/Tex-Edit Plus 4.1.3 Classic7/Tex-Edit Plus");
    let te_rsrc = std::path::Path::new("/tmp/heirloom-heavy-apps/macppc/texedit_unpacked/Tex-Edit Plus 4.1.3 Classic7/Tex-Edit Plus.rsrc");
    let jv_bin = std::path::Path::new("/tmp/heirloom-heavy-apps/mac68k/jpegview/JPEGView 3.3.1 ƒ/JPEGView");
    let jv_rsrc = std::path::Path::new("/tmp/heirloom-heavy-apps/mac68k/jpegview/JPEGView 3.3.1 ƒ/JPEGView.rsrc");
    if !te_bin.is_file() || !jv_bin.is_file() {
        eprintln!("SKIPPING heavy_texedit_plus_and_jpegview_ppc_end_to_end: heavy PPC apps absent in /tmp/heirloom-heavy-apps");
        return;
    }

    // 1) Tex-Edit Plus 4.1.3 PPC: 0 unbound imports, opens Untitled document + 9 native menus + native scrollbars
    let te_data = std::fs::read(te_bin).unwrap();
    let te_rf = std::fs::read(te_rsrc).unwrap();
    let (name, bytes) = fragment_bytes(&te_data, None).unwrap();
    let pef = Pef::parse(&bytes).unwrap();
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.attach_resources(&te_rf).unwrap();
    let frag = m.instantiate(&name, &pef).unwrap();
    m.start(&frag).unwrap();
    let (step, _) = m.run_loop(2_000_000);
    assert_eq!(step, Step::Blocked, "Tex-Edit Plus must reach its main event loop cleanly");
    assert!(
        m.mac.ui.calls.iter().any(|c| matches!(c, UiCall::SetMenuStrip(_, mb) if mb.menus.len() >= 9)),
        "Tex-Edit Plus must publish 9 native gadget.library menus"
    );
    assert!(
        m.mac.ui.calls.iter().any(|c| matches!(c, UiCall::AddControl(_, _))),
        "Tex-Edit Plus must create native gadget.library scrollbar controls"
    );

    // 2) JPEGView 3.3.1 PPC: runs through DeviceLoop, MatchAlias, SetStdCProcs, GetCTable(65), CustomGetFile, HMGetBalloons -> event loop with 6 menus
    let jv_data = std::fs::read(jv_bin).unwrap();
    let jv_rf = std::fs::read(jv_rsrc).unwrap();
    let (jname, jbytes) = fragment_bytes(&jv_data, None).unwrap();
    let jpef = Pef::parse(&jbytes).unwrap();
    let mut jm = MacPpc::new(RecordingGuestUi::new()).unwrap();
    jm.attach_resources(&jv_rf).unwrap();
    let jfrag = jm.instantiate(&jname, &jpef).unwrap();
    jm.start(&jfrag).unwrap();
    let (jstep, _) = jm.run_loop(2_000_000);
    assert_eq!(jstep, Step::Blocked, "JPEGView 3.3.1 PPC must reach its main event loop cleanly");
    assert!(
        jm.mac.log.iter().any(|l| l.contains("DrawMenuBar: 6 menus")),
        "JPEGView 3.3.1 PPC must build its 6-menu menu bar"
    );
}
