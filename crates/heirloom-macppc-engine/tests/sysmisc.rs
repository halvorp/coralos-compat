//! DM screen devices, the machine location, DebugStr and PBHGetVolParms by C name (specs: macos9-specs/InterfaceLib/*).
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3300;
const PB: u32 = 0x3400;
const BUF: u32 = 0x3500;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}

#[test]
fn one_display_the_screen_device_walk() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    let main = call(&mut m, "GetMainDevice", &[]);
    assert_ne!(main, 0);
    // spec vectors: First(true) == First(false) == GetMainDevice; Next(that, true) == NULL; Next(NULL, false) == NULL
    assert_eq!((call(&mut m, "DMGetFirstScreenDevice", &[1]), call(&mut m, "DMGetFirstScreenDevice", &[0])), (main, main));
    assert_eq!(call(&mut m, "DMGetNextScreenDevice", &[main, 1]), 0);
    assert_eq!(call(&mut m, "DMGetNextScreenDevice", &[0, 0]), 0);
    // DMGetDisplayIDByGDevice: the main display is id 1; a non-screen handle is kDMGenErr unless failToMain; a NULL out is paramErr (spec: Misc/DMGetDisplayIDByGDevice)
    assert_eq!(call(&mut m, "DMGetDisplayIDByGDevice", &[main, OUT, 0]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 1);
    assert_eq!(call(&mut m, "DMGetDisplayIDByGDevice", &[0x1234, OUT, 0]) as i32, -71130);
    assert_eq!(call(&mut m, "DMGetDisplayIDByGDevice", &[0x1234, OUT, 1]), 0);
    assert_eq!(call(&mut m, "DMGetDisplayIDByGDevice", &[main, 0, 0]) as i32, -50);
    // NEGATIVE control: a handle that is not in the device list has no successor
    assert_eq!(call(&mut m, "DMGetNextScreenDevice", &[0x1234, 0]), 0);
}

#[test]
fn location_debugstr_and_vol_parms() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    // ReadLocation: fresh XPRAM is zeros; WriteLocation then ReadLocation returns the same 12 bytes
    m.mac.mem.write_bytes(OUT, &[0xAA; 12]).unwrap();
    call(&mut m, "ReadLocation", &[OUT]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 12).unwrap()).unwrap(), &[0u8; 12]);
    let loc = [0x0a, 0xaa, 0xaa, 0xaa, 0x05, 0x55, 0x55, 0x55, 0x00, 0x00, 0x0e, 0x10];
    m.mac.mem.write_bytes(BUF, &loc).unwrap();
    call(&mut m, "WriteLocation", &[BUF]);
    call(&mut m, "ReadLocation", &[OUT]);
    assert_eq!(m.mac.mem.bytes(m.mac.mem.span(OUT, 12).unwrap()).unwrap(), &loc);
    // DebugStr logs and returns
    m.mac.mem.write_u8(OUT, 5).unwrap(); m.mac.mem.write_bytes(OUT + 1, b"Hello").unwrap();
    call(&mut m, "DebugStr", &[OUT]);
    assert!(m.mac.log.iter().any(|l| l == "[debugstr] Hello"));
    // PBHGetVolParmsSync: version 2 buffer on the default volume (ioVRefNum 0 with no name = the default volume)
    for o in 0..0x30 { m.mac.mem.write_u8(PB + o, 0).unwrap(); }
    m.mac.mem.write_u32(PB + 32, BUF).unwrap();
    m.mac.mem.write_u32(PB + 36, 64).unwrap();
    m.mac.mem.write_bytes(BUF, &[0xFF; 32]).unwrap();
    assert_eq!(call(&mut m, "PBHGetVolParmsSync", &[PB]), 0, "{:?}", m.mac.log.last());
    assert_eq!(m.mac.mem.read_u32(PB + 40).unwrap(), 20, "ioActCount: the version 2 record");
    assert_eq!(m.mac.mem.read_u16(BUF).unwrap(), 2, "vMVersion");
    assert_eq!(m.mac.mem.read_u8(BUF + 20).unwrap(), 0xFF, "nothing written past the record");
    // a short buffer is truncated, not overrun; NEGATIVE control: an unknown volume is nsvErr (-35)
    m.mac.mem.write_u32(PB + 36, 2).unwrap();
    call(&mut m, "PBHGetVolParmsSync", &[PB]);
    assert_eq!(m.mac.mem.read_u32(PB + 40).unwrap(), 2);
    m.mac.mem.write_u16(PB + 22, 0x7777).unwrap();
    assert_eq!(call(&mut m, "PBHGetVolParmsSync", &[PB]) as i32, -35);
}

/// PBXGetVolInfoSync (the Warcraft III Installer calls it without testing the weak import): PBHGetVInfo's fields plus the 64-bit totals of the same
/// geometry, so a 6 GB disk is reported whole; a non-zero ioXVersion is paramErr; an unknown volume nsvErr.
#[test]
fn pbx_get_vol_info_reports_64_bit_sizes_of_the_volume_geometry() {
    use heirloom_common::GuestMachine;
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let vol = GuestMachine::volume(&mut m);
    let hd = vol.volume("Macintosh HD").unwrap();
    vol.set_capacity(hd, 6 << 30).unwrap();
    for o in 0..140 { m.mac.mem.write_u8(PB + o, 0).unwrap(); }
    m.mac.mem.write_u16(PB + 28, 1).unwrap(); // ioVolIndex 1
    assert_eq!(call(&mut m, "PBXGetVolInfoSync", &[PB]), 0, "{:?}", m.mac.log.last());
    let (blocks, size) = (u64::from(m.mac.mem.read_u16(PB + 46).unwrap()), u64::from(m.mac.mem.read_u32(PB + 48).unwrap()));
    let total = u64::from(m.mac.mem.read_u32(PB + 122).unwrap()) << 32 | u64::from(m.mac.mem.read_u32(PB + 126).unwrap());
    let free = u64::from(m.mac.mem.read_u32(PB + 130).unwrap()) << 32 | u64::from(m.mac.mem.read_u32(PB + 134).unwrap());
    assert!(total >= 6 << 30 && total < (6 << 30) + 2 * size, "the whole 6 GB, in whole blocks, past 4 GB: {total}");
    assert!(blocks * size <= 0x7FFF_FFFF, "the 16-bit count is the pinned classic one");
    assert!(free > 0 && free <= total);
    assert_eq!(m.mac.mem.read_u16(PB + 66).unwrap(), 1, "ioVDrvInfo");
    // the classic call on the same 6 GB volume: the 16-bit counts are pinned so blocks x size stays a positive 32-bit long (2 GB at most)
    for o in 0..140 { m.mac.mem.write_u8(PB + o, 0).unwrap(); }
    m.mac.mem.write_u16(PB + 28, 1).unwrap();
    assert_eq!(call(&mut m, "PBHGetVInfoSync", &[PB]), 0);
    let (nblk, frblk, bsize) = (u64::from(m.mac.mem.read_u16(PB + 46).unwrap()), u64::from(m.mac.mem.read_u16(PB + 62).unwrap()), u64::from(m.mac.mem.read_u32(PB + 48).unwrap()));
    assert!(nblk * bsize <= 0x7FFF_FFFF && frblk * bsize <= 0x7FFF_FFFF, "{nblk} {frblk} {bsize}");
    assert!(frblk * bsize > 0x7000_0000, "pinned near 2 GB, not wrapped to a small number");
    for o in 0..140 { m.mac.mem.write_u8(PB + o, 0).unwrap(); }
    m.mac.mem.write_u16(PB + 28, 1).unwrap();
    m.mac.mem.write_u32(PB + 24, 1).unwrap();
    assert_eq!(call(&mut m, "PBXGetVolInfoSync", &[PB]) as i32, -50, "NEGATIVE CONTROL: ioXVersion must be 0");
    m.mac.mem.write_u32(PB + 24, 0).unwrap();
    m.mac.mem.write_u16(PB + 28, 9).unwrap();
    assert_eq!(call(&mut m, "PBXGetVolInfoSync", &[PB]) as i32, -35);
}
