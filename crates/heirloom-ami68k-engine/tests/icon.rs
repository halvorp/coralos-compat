//! icon.library driven by a compiled 68k program (tests/guest/c/icon.c) over real `.info` bytes.
mod common;
use heirloom_ami68k_engine::icon::default_icon;
use heirloom_ami68k_engine::icon_format::Icon;
use common::*;

const PROG: &[u8] = include_bytes!("guest/icon.bin");

fn tool_info() -> Vec<u8> {
    let mut i = default_icon(4);
    i.default_tool = Some(b"SYS:C/More".to_vec());
    i.tool_types = Some(vec![b"ICON=Foo".to_vec(), b"(OFF=1)".to_vec(), b"KEYS=A|b".to_vec(), b"FLAG".to_vec()]);
    i.images[0].as_mut().unwrap().data[0] = 0xAB;
    i.to_bytes()
}

#[test]
fn disk_objects_load_from_info_files_answer_tooltype_queries_save_and_free() {
    let mut a = machine();
    a.fs.vol.put_file("SYS:Tool.info", tool_info()).unwrap();
    a.fs.vol.mkdirs("SYS:Dir").unwrap();
    a.fs.vol.put_file("SYS:Prog", vec![0, 0, 3, 0xF3, 0, 0, 0, 0]).unwrap();
    let r = run(&mut a, PROG);
    assert_ne!(r[0], 0, "GetDiskObject(SYS:Tool) finds Tool.info");
    assert_eq!(r[1], 4, "do_Type WBPROJECT");
    assert_eq!(r[2], 1, "do_DefaultTool");
    assert_eq!((r[3], r[4]), (1, 1), "FindToolType ICON=Foo and (case-insensitively) keys=A|b");
    assert_eq!(r[5], 0, "FindToolType on a disabled (OFF=1) entry is NULL");
    assert_eq!(r[6], 1, "a bare FLAG gives the empty tail, not NULL");
    assert_eq!((r[7], r[8], r[9]), (1, 0, 0), "MatchToolValue: B in A|b yes, C no, a value containing | never");
    assert_eq!(r[10], 24 | (16 << 16), "gadget image is 24x16");
    assert_eq!((r[11], r[12]), (1, 0xAB), "ImageData points at the file's planar bytes");
    assert_eq!((r[13], r[14]), (0, 205), "a missing .info is NULL with ERROR_OBJECT_NOT_FOUND");
    assert_eq!(r[15], 1, "PutDiskObject on RAM:");
    assert_eq!((r[16], r[17]), (0, 214), "PutDiskObject on read-only SYS: fails with ERROR_DISK_WRITE_PROTECTED");
    assert_eq!(r[18], 1, "DupDiskObjectA makes an independent copy");
    assert_eq!((r[19], r[20], r[21]), (2, 1, 400), "GetDiskObjectNew of a directory without .info: WBDRAWER with DrawerData");
    assert_eq!(r[22], 3, "GetDiskObjectNew of an executable: WBTOOL");
    assert_eq!(r[23], 0, "GetDiskObjectNew of nothing: NULL");
    assert_eq!(r[24], 3, "GetDefDiskObject(WBTOOL)");
    assert_eq!(r[25], 0, "GetDefDiskObject(99): NULL");
    assert_eq!((r[26], r[27]), (1, 0), "DeleteDiskObject once works, twice fails");
    let out = a.fs.vol.resolve(0, "RAM:Out.info").unwrap();
    assert_eq!(a.fs.vol.file_data(out).unwrap(), &tool_info()[..], "PutDiskObject wrote exactly the bytes GetDiskObject read");
    assert!(a.fs.vol.resolve(0, "RAM:Gone.info").is_err());
    assert!(a.fs.vol.resolve(0, "SYS:Out.info").is_err(), "negative control: the refused write left nothing behind");
    assert!(a.fs.icons.is_empty(), "every DiskObject the program got was freed");
}

/// Real Workbench icons from the corpus (not committed; the test says so and skips when the tree is absent).
#[test]
fn real_info_files_parse_and_survive_a_write_back_round_trip() {
    let mut seen = 0;
    for f in ["/data/build/corpus-bin/igame/iGame/iGame.info", "/data/build/corpus-bin/dopus5/DOpus5.info", "/data/build/corpus-bin/igame/iGame/iGame.guide.info"] {
        let Ok(bytes) = std::fs::read(f) else { eprintln!("skipped {f}: corpus not present"); continue };
        let i = Icon::parse(&bytes).unwrap_or_else(|e| panic!("{f}: {e:?}"));
        let back = i.to_bytes();
        assert_eq!(Icon::parse(&back).unwrap(), i, "{f}: parse(write(x)) == x");
        assert!(i.images[0].is_some(), "{f}: has an image");
        seen += 1;
    }
    let _ = seen; // all absent = nothing proven, which the eprintln above reported
    assert!(Icon::parse(b"not an icon at all, not an icon at all, not an icon at all, not an icon at all, not an ico").is_err(), "negative control");
}
