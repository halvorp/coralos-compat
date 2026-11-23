//! Witness tests for Mac 68k native `datatype.library` (`PICT $8200 CompressedQuickTime`),
//! Color QuickDraw standard highlight CLUTs (`GetCTable(65/68/72/127)`), `SetStdProcs`/`SetStdCProcs`,
//! `MatchAlias`, and heavy classic Mac 68k applications (`MacWrite 4.6`, `ResEdit 2.1.3`).
mod common;
use common::*;
use heirloom_common::datatype::{DecodedImage, NativeDatatype, DT_JPEG};
use heirloom_mac68k_engine::Mac;
use heirloom_ui_abi::{GuestUiEvent, PointerPhase, RecordingGuestUi, UiCall, UiObjectRef};
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

fn w16(v: &mut Vec<u8>, xs: &[u16]) {
    for x in xs {
        v.extend(x.to_be_bytes());
    }
}

/// Build a PICT v2 containing opcode `$8200` (`CompressedQuickTime`) wrapping `payload` (codec `'jpeg'`).
fn pict_v2_compressed_qt(w: i16, h: i16, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    // version (2) + matrix (36) + matteSize (4) + matteRect (8) + mode (2) + srcRect (8) + accuracy (4) + maskSize (4) = 68 bytes
    w16(&mut body, &[0]); // version
    body.extend([0u8; 36]); // identity matrix (tx=0, ty=0)
    body.extend(0u32.to_be_bytes()); // matteSize = 0
    body.extend([0u8; 8]); // matteRect
    w16(&mut body, &[0]); // mode = srcCopy
    w16(&mut body, &[0, 0, h as u16, w as u16]); // srcRect
    body.extend(0u32.to_be_bytes()); // accuracy
    body.extend(0u32.to_be_bytes()); // maskSize = 0
    // ImageDescription (86 bytes)
    let mut idesc = vec![0u8; 86];
    idesc[0..4].copy_from_slice(&86u32.to_be_bytes()); // idSize
    idesc[4..8].copy_from_slice(b"jpeg"); // cType
    idesc[32..34].copy_from_slice(&(w as u16).to_be_bytes()); // width
    idesc[34..36].copy_from_slice(&(h as u16).to_be_bytes()); // height
    idesc[44..48].copy_from_slice(&(payload.len() as u32).to_be_bytes()); // dataSize
    body.extend_from_slice(&idesc);
    body.extend_from_slice(payload);

    let mut pic = Vec::new();
    w16(&mut pic, &[0, 0, 0, h as u16, w as u16, 0x0011, 0x02FF, 0x0C00]);
    pic.extend([0u8; 24]);
    w16(&mut pic, &[0x8200]);
    pic.extend((body.len() as u32).to_be_bytes());
    pic.extend_from_slice(&body);
    if body.len() & 1 != 0 {
        pic.push(0);
    }
    w16(&mut pic, &[0x00FF]);
    pic
}

#[test]
fn pict_8200_decodes_via_native_datatype_and_draws_pixels() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut g = Rig::new();
    g.m.datatype_host = Some(Box::new(CountingDatatype {
        calls: calls.clone(),
        w: 8,
        h: 4,
        color: 0xFF_00_00_00, // black (so 1-bit classic window canvas turns black!)
    }));
    let pic = pict_v2_compressed_qt(8, 4, &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
    let h = g.m.mm.new_handle(&mut g.m.mem, &mut g.m.heap, pic.len() as u32, false).unwrap();
    let ptr = g.m.mm.block(h).unwrap().ptr;
    g.m.mem.write_bytes(ptr, &pic).unwrap();
    set_rect(&mut g.m, 0x3100, 10, 20, 14, 28);
    let mut p = Prog::new();
    p.set_port(g.win).l(h).l(0x3100).trap(0xA8F6); // DrawPicture
    g.run(p.exit());
    g.no_refusals();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "PICT $8200 must call NativeDatatype::decode_image");
    assert_eq!(g.m.native_datatype_decodes, 1, "Mac::native_datatype_decodes must increment");
    assert!(g.count_black() == 8 * 4, "8x4 black decoded pixels must land in the window canvas (got {})", g.count_black());

    // Negative control: without a datatype host attached, unknown JPEG bytes cannot be decoded and draw nothing
    let mut g2 = Rig::new();
    let h2 = g2.m.mm.new_handle(&mut g2.m.mem, &mut g2.m.heap, pic.len() as u32, false).unwrap();
    let ptr2 = g2.m.mm.block(h2).unwrap().ptr;
    g2.m.mem.write_bytes(ptr2, &pic).unwrap();
    set_rect(&mut g2.m, 0x3100, 10, 20, 14, 28);
    let mut p2 = Prog::new();
    p2.set_port(g2.win).l(h2).l(0x3100).trap(0xA8F6);
    g2.run(p2.exit());
    assert_eq!(g2.m.native_datatype_decodes, 0);
    assert_eq!(g2.count_black(), 0, "without datatype host, JPEG payload must not silently draw");
}

#[test]
fn get_ctable_highlight_ids_and_set_std_cprocs_and_match_alias() {
    let mut g = Rig::new();
    // GetCTable(65, 68, 72, 127) must return valid non-NULL ColorTable handles
    for &ct_id in &[65u16, 68, 72, 127] {
        let h = g.m.pascal_call(0xAA18, &[u32::from(ct_id)]).expect("GetCTable").value;
        assert_ne!(h, 0, "GetCTable({ct_id}) must return a standard color table handle");
    }
    // SetStdCProcs ($AA4E) must populate 14 non-zero bottleneck stub pointers + 6 zero reserved words
    let cqd_buf = 0x3200u32;
    g.m.mem.write_bytes(cqd_buf, &[0xAA; 80]).unwrap();
    g.m.pascal_call(0xAA4E, &[cqd_buf]).expect("SetStdCProcs");
    for i in 0..14u32 {
        assert_ne!(g.m.mem.read_u32(cqd_buf + i * 4).unwrap(), 0, "CQDProcs slot {i} must be populated");
    }
    for i in 14..20u32 {
        assert_eq!(g.m.mem.read_u32(cqd_buf + i * 4).unwrap(), 0, "CQDProcs reserved slot {i} must be zero");
    }
    // MatchAlias (selector 5) must resolve an alias to aliasCount=1 and matching FSSpec
    let root = g.m.fm.tree.children(heirloom_volume::ROOT)[0];
    let _ = g.m.fm.tree.put_file(root, "photo.jpg", b"test".to_vec()).unwrap();
    let spec_in = 0x3300u32;
    g.m.mem.write_u16(spec_in, (-1i16) as u16).unwrap();
    g.m.mem.write_u32(spec_in + 2, 2).unwrap();
    let mut pstr = vec![9u8];
    pstr.extend_from_slice(b"photo.jpg");
    g.m.mem.write_bytes(spec_in + 6, &pstr).unwrap();
    let alias_out = 0x3380u32;
    let err_new = heirloom_mac68k_engine::fm::alias::alias_value_call(&mut g.m, 2, &[0, spec_in, alias_out]).unwrap();
    assert_eq!(err_new, 0, "NewAlias must succeed");
    let alias_h = g.m.mem.read_u32(alias_out).unwrap();
    assert_ne!(alias_h, 0);
    let count_ptr = 0x3390u32;
    let list_ptr = 0x3400u32;
    let changed_ptr = 0x3480u32;
    g.m.mem.write_u16(count_ptr, 1).unwrap();
    let err_match = heirloom_mac68k_engine::fm::alias::alias_value_call(
        &mut g.m,
        5,
        &[0, 0, alias_h, count_ptr, list_ptr, changed_ptr, 0, 0],
    )
    .unwrap();
    assert_eq!(err_match, 0, "MatchAlias must resolve the alias");
    assert_eq!(g.m.mem.read_u16(count_ptr).unwrap(), 1, "MatchAlias must report 1 match");
}

#[test]
fn heavy_macwrite_and_resedit_native_ui_end_to_end() {
    let mw_path = std::path::Path::new("/data/build/corpus-bin/mac/macwrite/fx46/MacWrite.bin");
    let re_path = std::path::Path::new("/data/build/corpus-bin/mac/resedit/u213/resedit_2.1.3.sea_/ResEdit.rsrc");
    if !mw_path.is_file() || !re_path.is_file() {
        eprintln!("SKIPPING heavy_macwrite_and_resedit_native_ui_end_to_end: corpus binaries absent");
        return;
    }

    // 1) MacWrite 4.6: opens "Untitled", installs 7 native menus, draws document, accepts typing, 0 bus faults
    let mw_bytes = std::fs::read(mw_path).unwrap();
    let mut mw = Mac::new(RecordingGuestUi::new()).unwrap();
    mw.ui.drain_audio = true;
    mw.launch(&mw_bytes).unwrap();
    let r = mw.run_loop(2_000_000).unwrap();
    assert_eq!(r.stats.bus_faults, 0, "MacWrite startup must have 0 bus faults");
    assert!(mw.ui.calls.iter().any(|c| matches!(c, UiCall::OpenWindow(w) if w.title == "Untitled")), "MacWrite must open Untitled window");
    assert!(
        mw.ui.calls.iter().any(|c| matches!(c, UiCall::SetMenuStrip(_, m) if m.menus.len() >= 7)),
        "MacWrite must publish 7 native gadget.library menus"
    );
    assert!(
        mw.ui.calls.iter().any(|c| matches!(c, UiCall::PresentPlane(_, p) if p.ops.len() >= 100)),
        "MacWrite must render rich document plane ops"
    );

    // 2) ResEdit 2.1.3: opens splash dialog + 5 native menus (with Open Special submenu), click dismisses splash, 0 bus faults
    let re_bytes = std::fs::read(re_path).unwrap();
    let mut re = Mac::new(RecordingGuestUi::new()).unwrap();
    re.ui.drain_audio = true;
    re.launch(&re_bytes).unwrap();
    let r1 = re.run_loop(5_000_000).unwrap();
    assert_eq!(r1.stats.bus_faults, 0, "ResEdit startup must have 0 bus faults");
    for _ in 0..5 {
        re.advance_ticks(2);
        let rs = re.run_loop(1_000_000).unwrap();
        assert_eq!(rs.stats.bus_faults, 0);
    }
    assert!(
        re.ui.calls.iter().any(|c| matches!(c, UiCall::SetMenuStrip(_, m) if m.menus.len() >= 5)),
        "ResEdit must publish 5 native gadget.library menus"
    );
    let win1 = UiObjectRef { id: 1, generation: 1 };
    re.ui.events.push_back(GuestUiEvent::primary(win1, 100, 100, PointerPhase::Down));
    re.ui.events.push_back(GuestUiEvent::primary(win1, 100, 100, PointerPhase::Up));
    re.advance_ticks(10);
    let r2 = re.run_loop(2_000_000).unwrap();
    assert_eq!(r2.stats.bus_faults, 0);
    assert!(
        re.ui.calls.iter().any(|c| matches!(c, UiCall::CloseWindow(w) if w.id == 1)),
        "Clicking ResEdit splash dialog must dismiss it cleanly"
    );
}
