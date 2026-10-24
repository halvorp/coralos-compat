//! Offscreen graphics worlds by their PowerPC C names: creation over guest-memory pixels, drawing into one, `CopyBits` between worlds and to a window, the port accessors,
//! the locked-pixels state, disposal, and the named refusals (direct depths, a pointer that is no world).
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x4000;
const R1: u32 = 0x4100;
const R2: u32 = 0x4110;
const RGB: u32 = 0x4120;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    m.bind(name);
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    match m.glue_call(name, &mut regs) { GlueOutcome::Refused(w) => w, o => panic!("{name}: expected a refusal, got {o:?}") }
}
fn rect(m: &mut M, at: u32, t: i16, l: i16, b: i16, r: i16) { for (i, v) in [t, l, b, r].into_iter().enumerate() { m.mac.mem.write_u16(at + 2 * i as u32, v as u16).unwrap(); } }
fn new_world(m: &mut M, depth: u32, w: i16, h: i16) -> u32 {
    rect(m, R1, 0, 0, h, w);
    assert_eq!(call(m, "NewGWorld", &[OUT, depth, R1, 0, 0, 0]), 0);
    m.mac.mem.read_u32(OUT).unwrap()
}
fn pixels(m: &M, pm: u32, n: u32) -> Vec<u8> { let base = m.mac.pix_base_addr(pm).unwrap(); m.get_bytes(base, n, "t").unwrap() }

#[test]
fn an_eight_bit_world_is_guest_pixels_that_drawing_and_copybits_reach() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    let (port0, _) = m.mac.gw_get().unwrap();
    let a = new_world(&mut m, 8, 16, 8);
    let b = new_world(&mut m, 8, 16, 8);
    assert!(a != 0 && b != 0 && a != b);
    // creation does not change thePort
    assert_eq!(m.mac.gw_get().unwrap().0, port0);
    let pm = call(&mut m, "GetGWorldPixMap", &[a]);
    assert_eq!(call(&mut m, "GetPixRowBytes", &[pm]), 16);
    assert_ne!(call(&mut m, "GetPixBaseAddr", &[pm]), 0);
    assert_eq!(call(&mut m, "LockPixels", &[pm]), 1);
    assert_eq!(pixels(&m, pm, 16 * 8), vec![0u8; 16 * 8], "a new world is cleared");
    // GetPortBounds reads the port rect
    call(&mut m, "GetPortBounds", &[a, R2]);
    assert_eq!((0..4).map(|i| m.mac.mem.read_u16(R2 + 2 * i).unwrap()).collect::<Vec<_>>(), vec![0, 0, 8, 16]);
    // draw into A: black-pattern paint of a 4x4 rectangle in a colour
    call(&mut m, "SetGWorld", &[a, 0]);
    assert_eq!(m.mac.gw_get().unwrap().0, a);
    for (i, v) in [0xFFFFu16, 0, 0].into_iter().enumerate() { m.mac.mem.write_u16(RGB + 2 * i as u32, v).unwrap(); }
    call(&mut m, "RGBForeColor", &[RGB]);
    rect(&mut m, R2, 2, 2, 6, 6);
    call(&mut m, "PaintRect", &[R2]);
    let drawn = pixels(&m, pm, 16 * 8);
    assert!(drawn.iter().any(|&p| p != 0), "PaintRect must reach the world's guest pixels");
    assert_eq!(drawn[0], 0, "outside the rectangle stays clear");
    assert_ne!(drawn[3 * 16 + 3], 0);
    // CopyBits A -> B through GetPortBitMapForCopyBits
    let (sa, sb) = (call(&mut m, "GetPortBitMapForCopyBits", &[a]), call(&mut m, "GetPortBitMapForCopyBits", &[b]));
    rect(&mut m, R1, 0, 0, 8, 16);
    call(&mut m, "CopyBits", &[sa, sb, R1, R1, 0, 0]);
    let pmb = call(&mut m, "GetGWorldPixMap", &[b]);
    assert_eq!(pixels(&m, pmb, 16 * 8), drawn, "CopyBits between worlds copies the pixels");
    // locked state is recorded; purge flags are separate
    call(&mut m, "UnlockPixels", &[pm]);
    assert_eq!(call(&mut m, "GetPixelsState", &[pm]) & 0x80, 0);
    call(&mut m, "AllowPurgePixels", &[pm]);
    assert_eq!(call(&mut m, "GetPixelsState", &[pm]) & 0x40, 0x40);
    // dispose: the world is gone, later use is refused BY NAME, and thePort is not left dangling
    call(&mut m, "DisposeGWorld", &[a]);
    assert_ne!(m.mac.gw_get().unwrap().0, a);
    assert!(refused(&mut m, "GetGWorldPixMap", &[a]).contains("not a graphics world"));
    assert!(refused(&mut m, "DisposeGWorld", &[a]).contains("not a graphics world"));
}

#[test]
fn one_bit_worlds_pad_rows_and_update_replaces_the_world() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    let a = new_world(&mut m, 1, 40, 4);
    let pm = call(&mut m, "GetGWorldPixMap", &[a]);
    assert_eq!(call(&mut m, "GetPixRowBytes", &[pm]), 8, "40 one-bit pixels pad to two 32-bit words");
    // UpdateGWorld rebuilds at the new bounds (depth 0 keeps the depth) and returns the new port in the out parameter
    m.mac.mem.write_u32(OUT, a).unwrap();
    rect(&mut m, R1, 0, 0, 10, 100);
    let flags = call(&mut m, "UpdateGWorld", &[OUT, 0, R1, 0, 0, 0]);
    assert_eq!(flags & (1 << 31), 0, "no error flag");
    let a2 = m.mac.mem.read_u32(OUT).unwrap();
    let pm2 = call(&mut m, "GetGWorldPixMap", &[a2]);
    assert_eq!(call(&mut m, "GetPixRowBytes", &[pm2]), 16);
    call(&mut m, "GetPixBounds", &[pm2, R2]);
    assert_eq!(m.mac.mem.read_u16(R2 + 6).unwrap(), 100);
    assert_eq!(m.mac.cqd.gworlds.len(), 1, "the old world was disposed");
}

#[test]
fn direct_depths_and_bad_geometry_are_named_refusals_not_silent_success() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    rect(&mut m, R1, 0, 0, 8, 8);
    for d in [16u32, 32] {
        let why = refused(&mut m, "NewGWorld", &[OUT, d, R1, 0, 0, 0]);
        assert!(why.contains(&format!("{d}-bit pixels are not implemented")), "{why}");
    }
    assert_eq!(m.mac.cqd.gworlds.len(), 0, "a refused world leaves nothing behind");
    // an empty rectangle is paramErr (-50), reported as the QDErr result, and the out pointer is zeroed
    rect(&mut m, R1, 0, 0, 0, 0);
    m.mac.mem.write_u32(OUT, 0xDEAD).unwrap();
    assert_eq!(call(&mut m, "NewGWorld", &[OUT, 8, R1, 0, 0, 0]) as i32, -50);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0);
    // depth 0 means the main device's depth
    rect(&mut m, R1, 0, 0, 4, 4);
    assert_eq!(call(&mut m, "NewGWorld", &[OUT, 0, R1, 0, 0, 0]), 0);
}


#[test]
fn copybits_from_a_world_to_a_window_reaches_the_window_canvas() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    call(&mut m, "InitGraf", &[0x1000]);
    call(&mut m, "InitWindows", &[]);
    rect(&mut m, R1, 40, 40, 80, 80);
    m.mac.mem.write_u8(0x4200, 0).unwrap(); // empty title
    let win = call(&mut m, "NewWindow", &[0, R1, 0x4200, 1, 0, 0xFFFF_FFFF, 0, 0]);
    assert_ne!(win, 0);
    let w = new_world(&mut m, 1, 16, 16);
    // a 1-bit world: set its first row black by writing the guest pixels (the pixel buffer IS guest memory)
    let pm = call(&mut m, "GetGWorldPixMap", &[w]);
    let base = call(&mut m, "GetPixBaseAddr", &[pm]);
    m.mac.mem.write_bytes(base, &[0xFF, 0xFF]).unwrap();
    let (src, dst) = (call(&mut m, "GetPortBitMapForCopyBits", &[w]), call(&mut m, "GetPortBitMapForCopyBits", &[win]));
    rect(&mut m, R1, 0, 0, 16, 16);
    rect(&mut m, R2, 0, 0, 16, 16);
    call(&mut m, "SetPort", &[win]);
    call(&mut m, "CopyBits", &[src, dst, R1, R2, 0, 0]);
    let canvas = &m.mac.qd.canvases[&win];
    assert!((0..16).all(|x| canvas.get(x, 0) == Some(1)), "the world's black first row must be on the window canvas: {:?}", m.mac.log);
    assert!((0..16).all(|x| canvas.get(x, 1) == Some(0)), "the rest of the world is white");
}

#[test]
fn a_carbon_program_has_quickdraw_initialised_without_calling_initgraf() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    assert_eq!(m.mac.qd_globals, 0);
    // NEGATIVE control: an InterfaceLib (Classic) import does not initialise QuickDraw for the program
    m.bind_from("InterfaceLib", "SetPort");
    assert_eq!(m.mac.qd_globals, 0, "a Classic program must call InitGraf itself");
    // the first CarbonLib import does what InitGraf(&qd.thePort) would
    m.bind_from("CarbonLib", "SetPort");
    assert_ne!(m.mac.qd_globals, 0);
    let w = new_world(&mut m, 8, 8, 8);
    call(&mut m, "SetGWorld", &[w, 0]);
    assert_eq!(m.mac.gw_get().unwrap().0, w);
}
