//! The FSRef calls (`heirloom_macppc_engine::fsref_c`) over the one volume tree: an FSRef made from an FSSpec, a parameter block or a parent and a
//! Unicode name names the same object; FSGetCatalogInfo reads its catalog record; FSDeleteObject removes it and the reference then fails.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const SPEC: u32 = 0x3000;
const REF: u32 = 0x3100;
const REF2: u32 = 0x3200;
const CAT: u32 = 0x3300;
const NAME: u32 = 0x3400;
const PB: u32 = 0x3600;
const UNI: u32 = 0x3700;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn pstr(m: &mut M, at: u32, s: &str) { m.mac.mem.write_u8(at, s.len() as u8).unwrap(); m.mac.mem.write_bytes(at + 1, s.as_bytes()).unwrap(); }

/// "Macintosh HD:Adobe:Plug" (data "hello", type 8BFM creator 8BIM).
fn machine() -> (M, u32) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let root = m.mac.fm.def_vol;
    let dir = m.mac.fm.tree.mkdirs(root, ["Adobe"].iter().copied()).unwrap();
    let f = m.mac.fm.tree.put_file(dir, "Plug", b"hello".to_vec()).unwrap();
    m.mac.fm.tree.get_mut(f).finfo.file_type = *b"8BFM";
    m.mac.fm.tree.get_mut(f).finfo.creator = *b"8BIM";
    let id = m.mac.fm.dir_id(dir);
    (m, id)
}

#[test]
fn an_fsref_from_a_spec_reads_its_catalog_info_name_spec_and_parent() {
    let (mut m, dir) = machine();
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, dir, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    // nodeFlags | volume | parentDirID | nodeID | finderInfo | dataSizes
    let which = 0x2 | 0x4 | 0x8 | 0x10 | 0x800 | 0x4000;
    m.mac.mem.zero(SPEC, 70).unwrap();
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, which, CAT, UNI, SPEC, REF2]), 0);
    let r16 = |m: &M, o: u32| m.mac.mem.read_u16(CAT + o).unwrap();
    let r32 = |m: &M, o: u32| m.mac.mem.read_u32(CAT + o).unwrap();
    assert_eq!((r16(&m, 0), r16(&m, 2) as i16, r32(&m, 4)), (0, -1, dir), "a file (no directory bit), on the boot volume, in Adobe");
    assert_eq!(&r32(&m, 72).to_be_bytes(), b"8BFM");
    assert_eq!(&r32(&m, 76).to_be_bytes(), b"8BIM");
    assert_eq!(r32(&m, 108), 5, "the data fork's logical size (low word of the UInt64)");
    assert_eq!(m.mac.mem.read_u16(UNI).unwrap(), 4);
    assert_eq!(m.mac.mem.read_u16(UNI + 2).unwrap(), u16::from(b'P'), "the HFSUniStr255 name");
    assert_eq!(m.mac.mem.read_u32(SPEC + 2).unwrap(), dir, "the FSSpec names the same file");
    // the parent's reference is the Adobe folder: a directory whose node ID is dir
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF2, 0x2 | 0x10, CAT, 0, 0, 0]), 0);
    assert_eq!((r16(&m, 0) & 0x10, r32(&m, 8)), (0x10, dir));
    // FSMakeFSRefUnicode(parent, "Plug") names the file again
    for (i, c) in "Plug".bytes().enumerate() { m.mac.mem.write_u16(UNI + 2 * i as u32, u16::from(c)).unwrap(); }
    assert_eq!(call(&mut m, "FSMakeFSRefUnicode", &[REF2, 4, UNI, 0, REF2 + 0x80]), 0);
    assert_eq!(call(&mut m, "FSCompareFSRefs", &[REF, REF2 + 0x80]), 0, "the same object");
    assert_eq!(call(&mut m, "FSCompareFSRefs", &[REF, REF2]), -1420, "the file is not its folder");
}

#[test]
fn pbmakefsrefsync_and_fsdeleteobject_and_the_errors() {
    let (mut m, dir) = machine();
    m.mac.mem.zero(PB, 80).unwrap();
    pstr(&mut m, NAME, "Plug");
    m.mac.mem.write_u32(PB + 18, NAME).unwrap();
    m.mac.mem.write_u32(PB + 48, dir).unwrap();
    m.mac.mem.write_u32(PB + 60, REF).unwrap();
    assert_eq!(call(&mut m, "PBMakeFSRefSync", &[PB]), 0);
    assert_eq!(m.mac.mem.read_u16(PB + 16).unwrap(), 0, "ioResult");
    assert_eq!(call(&mut m, "FSDeleteObject", &[REF]), 0);
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, 0x10, CAT, 0, 0, 0]), -43, "the object is gone: fnfErr");
    assert_eq!(call(&mut m, "PBMakeFSRefSync", &[PB]), -43, "NEGATIVE CONTROL: no such file");
    assert_eq!(m.mac.mem.read_u16(PB + 16).unwrap() as i16, -43);
    m.mac.mem.write_bytes(REF2, &[0u8; 80]).unwrap();
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF2, 0x10, CAT, 0, 0, 0]), -50, "not an FSRef this File Manager made: paramErr");
    // a folder with items is not deleted
    let root = m.mac.fm.def_vol;
    let d2 = m.mac.fm.tree.mkdirs(root, ["Full"].iter().copied()).unwrap();
    m.mac.fm.tree.put_file(d2, "x", vec![1]).unwrap();
    pstr(&mut m, NAME, "Full");
    m.mac.mem.write_u32(PB + 48, 2).unwrap();
    assert_eq!(call(&mut m, "PBMakeFSRefSync", &[PB]), 0);
    assert_eq!(call(&mut m, "FSDeleteObject", &[REF]), -47, "fBsyErr");
}

/// FSRefMakePath is Mac OS 9 CarbonLib's: the HFS full path, ':'-joined, a folder ending in ':', NUL-terminated (Installer VISE's RecordAppLoc
/// records its helper's path with it); a buffer too small is -210; FSPathMakeRef turns the path back into the same object.
#[test]
fn fsrefmakepath_is_the_hfs_full_path_and_fspathmakeref_its_inverse() {
    let (mut m, dir) = machine();
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, dir, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    const PATH: u32 = 0x3800;
    assert_eq!(call(&mut m, "FSRefMakePath", &[REF, PATH, 256]), 0);
    assert_eq!(m.cstr(PATH, "t").unwrap(), b"Macintosh HD:Adobe:Plug");
    assert_eq!(call(&mut m, "FSRefMakePath", &[REF, PATH, 10]), -210, "NEGATIVE CONTROL: the buffer is too small");
    assert_eq!(call(&mut m, "FSPathMakeRef", &[PATH, REF2, OUT_DIR]), 0);
    assert_eq!(call(&mut m, "FSCompareFSRefs", &[REF, REF2]), 0, "the path names the same file");
    assert_eq!(m.mac.mem.read_u8(OUT_DIR).unwrap(), 0, "not a directory");
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, 0, 0, 0, 0, REF2]), 0);
    assert_eq!(call(&mut m, "FSRefMakePath", &[REF2, PATH, 256]), 0);
    assert_eq!(m.cstr(PATH, "t").unwrap(), b"Macintosh HD:Adobe:", "a folder ends in ':'");
}
const OUT_DIR: u32 = 0x3900;

/// FSSetCatalogInfo writes the Finder info, the creation / content-modification dates and the lock, and reading them back agrees (the tryout
/// set-up tool VSetupT stamps the files it writes with it); a bit HFS does not keep is named, not an error.
#[test]
fn fssetcataloginfo_sets_finder_info_dates_and_the_lock() {
    let (mut m, dir) = machine();
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, dir, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    m.mac.mem.zero(CAT, 144).unwrap();
    m.mac.mem.write_u16(CAT, 1).unwrap(); // locked
    m.mac.mem.write_u32(CAT + 18, 0xB96A_1D80).unwrap(); // createDate.lowSeconds
    m.mac.mem.write_bytes(CAT + 72, b"8BPSMeSa").unwrap();
    m.mac.mem.write_u16(CAT + 80, 0x0100).unwrap(); // fdFlags
    assert_eq!(call(&mut m, "FSSetCatalogInfo", &[REF, 0x2 | 0x20 | 0x800 | 0x400, CAT]), 0);
    assert!(m.mac.log.iter().any(|l| l.contains("whichInfo bits 0x400")), "permissions are named, not kept");
    m.mac.mem.zero(CAT, 144).unwrap();
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, 0x2 | 0x20 | 0x800, CAT, 0, 0, 0]), 0);
    assert_eq!(m.mac.mem.read_u16(CAT).unwrap() & 1, 1, "locked");
    assert_eq!(m.mac.mem.read_u32(CAT + 18).unwrap(), 0xB96A_1D80);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(CAT + 72, 8).unwrap()).unwrap(), b"8BPSMeSa");
    assert_eq!(call(&mut m, "FSDeleteObject", &[REF]), -45, "fLckdErr: the lock is real");
}

/// FSOpenIterator / FSGetCatalogInfoBulk / FSCloseIterator: a folder's items one call at a time (a CarbonLib program enumerating a folder: the
/// tryout DRM's License Manager reads its Licenses folder this way), then errFSNoMoreItems; a subtree iterator is refused by name.
#[test]
fn a_flat_catalog_iterator_returns_each_item_then_no_more_items() {
    let (mut m, _) = machine();
    let root = m.mac.fm.def_vol;
    let adobe = m.mac.fm.tree.mkdirs(root, ["Adobe"].iter().copied()).unwrap();
    m.mac.fm.tree.put_file(adobe, "Note", b"n".to_vec()).unwrap();
    pstr(&mut m, NAME, "Adobe");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, 0, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    const IT: u32 = 0x3800; const N: u32 = 0x3804; const STACK: u32 = 0x5000;
    assert_eq!(call(&mut m, "FSOpenIterator", &[REF, 1, IT]), -1423, "NEGATIVE CONTROL: kFSIterateSubtree is not provided");
    assert_eq!(call(&mut m, "FSOpenIterator", &[REF, 0, IT]), 0);
    let it = m.mac.mem.read_u32(IT).unwrap();
    m.mac.mem.write_u32(STACK + 56, UNI).unwrap(); // the 9th argument (names) is in the caller's parameter area
    let _ = m.bind("FSGetCatalogInfoBulk");
    let mut names = Vec::new();
    loop {
        let mut regs = [0u32; 32];
        regs[1] = STACK;
        regs[3..11].copy_from_slice(&[it, 1, N, 0, 0x0800, CAT, REF2, 0]);
        assert_eq!(m.glue_call("FSGetCatalogInfoBulk", &mut regs), GlueOutcome::Return);
        if regs[3] as i32 == -1417 { assert_eq!(m.mac.mem.read_u32(N).unwrap(), 0); break }
        assert_eq!(regs[3], 0);
        assert_eq!(m.mac.mem.read_u32(N).unwrap(), 1);
        let len = m.mac.mem.read_u16(UNI).unwrap();
        names.push((0..len).map(|i| m.mac.mem.read_u16(UNI + 2 + 2 * u32::from(i)).unwrap() as u8 as char).collect::<String>());
        assert!(names.len() < 5, "the iterator ends");
    }
    assert_eq!(names, ["Plug", "Note"]);
    // the refs array got each item's FSRef: the last one names "Note"
    m.mac.mem.write_u16(UNI, 0).unwrap();
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF2, 0, 0, UNI, 0, 0]), 0);
    assert_eq!((m.mac.mem.read_u16(UNI).unwrap(), m.mac.mem.read_u16(UNI + 2).unwrap()), (4, u16::from(b'N')));
    assert_eq!(call(&mut m, "FSCloseIterator", &[it]), 0);
    assert_eq!(call(&mut m, "FSCloseIterator", &[it]), -1420, "closed twice: errFSBadIterator");
}

#[test]
fn fscreate_directory_and_file_unicode_make_new_items_and_refuse_duplicates() {
    let (mut m, dir) = machine();
    pstr(&mut m, NAME, "");
    // a reference to the Adobe folder (the FSSpec of the folder itself: its parent's dirID and its name)
    pstr(&mut m, NAME, "Plug");
    assert_eq!(call(&mut m, "FSMakeFSSpec", &[0, dir, NAME, SPEC]), 0);
    assert_eq!(call(&mut m, "FSpMakeFSRef", &[SPEC, REF]), 0);
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, 0, 0, 0, 0, REF2]), 0); // REF2 = the folder
    let put = |m: &mut M, s: &str| for (i, c) in s.bytes().enumerate() { m.mac.mem.write_u16(UNI + 2 * i as u32, u16::from(c)).unwrap(); };
    put(&mut m, "Settings");
    let (new_ref, new_spec, new_id) = (0x3800u32, 0x3880u32, 0x38F0u32);
    assert_eq!(call(&mut m, "FSCreateDirectoryUnicode", &[REF2, 8, UNI, 0, 0, new_ref, new_spec, new_id]), 0);
    let made = m.mac.mem.read_u32(new_id).unwrap();
    assert_ne!(made, 0);
    assert_eq!(m.mac.mem.read_u32(new_spec + 2).unwrap(), dir, "the new folder's spec names its parent, Adobe");
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[new_ref, 0x2 | 0x10, CAT, 0, 0, 0]), 0);
    assert_eq!((m.mac.mem.read_u16(CAT).unwrap() & 0x10, m.mac.mem.read_u32(CAT + 8).unwrap()), (0x10, made), "a directory whose node ID is the new dirID");
    // a file inside it, with Finder info from the catalogInfo (whichInfo kFSCatInfoFinderInfo)
    put(&mut m, "Prefs");
    m.mac.mem.zero(CAT, 144).unwrap();
    m.mac.mem.write_bytes(CAT + 72, b"8BPF8BIM").unwrap();
    assert_eq!(call(&mut m, "FSCreateFileUnicode", &[new_ref, 5, UNI, 0x800, CAT, REF, 0]), 0);
    m.mac.mem.zero(CAT, 144).unwrap();
    assert_eq!(call(&mut m, "FSGetCatalogInfo", &[REF, 0x800 | 0x8, CAT, 0, 0, 0]), 0);
    assert_eq!((&m.mac.mem.read_u32(CAT + 72).unwrap().to_be_bytes(), m.mac.mem.read_u32(CAT + 4).unwrap()), (b"8BPF", made));
    // NEGATIVE controls: the same name again is dupFNErr (-48); a file reference as the parent is dirNFErr (-120)
    assert_eq!(call(&mut m, "FSCreateFileUnicode", &[new_ref, 5, UNI, 0, 0, 0, 0]), -48);
    put(&mut m, "Settings");
    assert_eq!(call(&mut m, "FSCreateDirectoryUnicode", &[REF2, 8, UNI, 0, 0, 0, 0, 0]), -48);
    assert_eq!(call(&mut m, "FSCreateDirectoryUnicode", &[REF, 8, UNI, 0, 0, 0, 0, 0]), -120);
}
