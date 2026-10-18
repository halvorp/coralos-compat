//! Host-drawn default icons (src/icon_host.rs): GetDefDiskObject & co. ask the host for generic-role pixels; a real
//! `.info` keeps its own artwork (REAL-INFO-KEEPS-ITS-ARTWORK); no host artwork = the named placeholder box.
mod common;
use amiga68k_engine::icon::default_icon;
use amiga68k_engine::icon_host::{nearest_pen, planar_image, ICON_PENS};
use common::*;
use guestui_abi::IconRole;

const PROG: &[u8] = include_bytes!("guest/icon_def.bin");
/// A synthetic pattern (NOT artwork): a diagonal gradient with a transparent corner, colours spread over the system pens.
fn synthetic(seed: u32) -> Vec<u32> {
    (0..32 * 32u32).map(|i| { let (x, y) = (i % 32, i / 32); if x < 4 && y < 4 { 0 } else { 0xFF00_0000 | ((x * 8 + seed) & 0xFF) << 16 | (y * 8) << 8 | ((x ^ y) * 8 & 0xFF) } }).collect()
}
const SYS: [u32; ICON_PENS] = [0xAAAAAA, 0x000000, 0xFFFFFF, 0x6688BB, 0x999999, 0xBBBBBB, 0xEEBB99, 0x4488DD];

/// The Image (width, height, depth, planar bytes) of the DiskObject at guest address `d`.
fn gadget_image(a: &amiga68k_engine::Amiga<guestui_abi::RecordingGuestUi>, d: u32) -> (u16, u16, u16, Vec<u8>) {
    let img = a.mem.read_u32(d + 22).unwrap();
    let (w, h, depth) = (a.mem.read_u16(img + 4).unwrap(), a.mem.read_u16(img + 6).unwrap(), a.mem.read_u16(img + 8).unwrap());
    let data = a.mem.read_u32(img + 10).unwrap();
    let n = (w as usize + 15) / 16 * 2 * h as usize * depth as usize;
    (w, h, depth, (0..n as u32).map(|i| a.mem.read_u8(data + i).unwrap()).collect())
}

fn seeded() -> amiga68k_engine::Amiga<guestui_abi::RecordingGuestUi> {
    let mut a = machine();
    let mut tool = default_icon(4);
    tool.images[0].as_mut().unwrap().data[0] = 0xAB;
    a.fs.vol.put_file("SYS:Tool.info", tool.to_bytes()).unwrap();
    a.fs.vol.mkdirs("SYS:Dir").unwrap();
    a
}

#[test]
fn defaults_come_from_the_host_pixels_and_real_info_files_keep_their_own() {
    let mut a = seeded();
    a.ui.icon_art.insert(IconRole::Drawer, (32, 32, synthetic(0)));
    run(&mut a, PROG);
    let want = planar_image(&synthetic(0), 32, 32, &SYS);
    for (slot, what) in [(0x1F000, "GetDefDiskObject(WBDRAWER)"), (0x1F004, "GetDiskObjectNew of a drawer with no .info")] {
        let d = a.mem.read_u32(slot).unwrap();
        assert_ne!(d, 0, "{what}");
        let (w, h, depth, bytes) = gadget_image(&a, d);
        assert_eq!((w, h, depth), (32, 32, 3), "{what}");
        assert_eq!(bytes, want.data, "{what}: ImageData equals the quantised host pixels");
        assert_eq!(a.mem.read_u16(d + 12).unwrap(), 32, "{what}: gadget width follows the Image");
    }
    assert_eq!(a.ui.icon_requests[0], (IconRole::Drawer, 32, 32));
    let t = a.mem.read_u32(0x1F008).unwrap();
    let (w, h, _, bytes) = gadget_image(&a, t);
    assert_eq!((w, h, bytes[0]), (24, 16, 0xAB), "the real Tool.info kept its own 24x16 artwork");
    assert_eq!(a.ui.icon_requests.len(), 2, "a real .info never asks the host");
    assert!(a.log.iter().any(|l| l.contains("[host artwork: Drawer]")), "logged: {:#?}", a.log);
}

#[test]
fn no_host_artwork_falls_back_to_the_box_and_says_so() {
    let mut a = seeded();
    run(&mut a, PROG); // negative control: the host has no art
    let d = a.mem.read_u32(0x1F000).unwrap();
    let (w, h, depth, bytes) = gadget_image(&a, d);
    assert_eq!((w, h, depth), (24, 16, 1));
    assert_eq!(bytes, default_icon(2).images[0].as_ref().unwrap().data, "the old outlined box");
    assert!(a.log.iter().any(|l| l.contains("FALLBACK-BOX: host has no Drawer artwork")), "{:#?}", a.log);
    // And the host pixels really differ from the box (the positive test above cannot pass on the box).
    assert_ne!(planar_image(&synthetic(0), 32, 32, &SYS).data, default_icon(2).images[0].as_ref().unwrap().data);
}

#[test]
fn wrong_sized_host_art_is_refused_by_name_not_guessed() {
    let mut a = seeded();
    a.ui.icon_art.insert(IconRole::Drawer, (16, 16, vec![0xFFFFFFFF; 256])); // a size the engine did not ask for
    run(&mut a, PROG);
    assert_eq!(gadget_image(&a, a.mem.read_u32(0x1F000).unwrap()).2, 1, "still the box");
    assert!(a.log.iter().any(|l| l.contains("FALLBACK-BOX")));
}

#[test]
fn pen_mapping_is_nearest_in_the_live_palette_and_transparent_is_pen_zero() {
    assert_eq!(nearest_pen(0xFF000000, &SYS), 1, "black -> the outline pen");
    assert_eq!(nearest_pen(0xFFFFFFFF, &SYS), 2, "white -> the highlight pen");
    assert_eq!(nearest_pen(0x00FFFFFF, &SYS), 0, "transparent -> background pen 0");
    let mut themed = SYS;
    themed[5] = 0xFF0000; // a theme that puts its accent in pen 5
    assert_eq!(nearest_pen(0xFFEE1111, &themed), 5);
    assert_ne!(nearest_pen(0xFFEE1111, &SYS), 5, "negative control: the stock palette maps it elsewhere");
}
