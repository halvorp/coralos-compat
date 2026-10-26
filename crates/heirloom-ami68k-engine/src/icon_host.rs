//! Host-drawn default icons. The engine ships NO icon artwork (the public MIT repo cannot carry GPL artwork): for a
//! default icon it asks the host (`GuestUi::icon_image`) for pixels of a generic role and converts them here, at run
//! time, into a real planar guest Image.
//!
//! RULE `REAL-INFO-KEEPS-ITS-ARTWORK`: this path runs only where Workbench itself would use a default icon
//! (GetDefDiskObject, NewDiskObject, and GetDiskObjectNew on an object with no `.info`). A real `.info` file always
//! delivers its own Image; nothing here ever replaces it.
//!
//! PEN MAPPING. Icon Images are 3 bitplanes (PlanePick 7, PlaneOnOff 0), so they can use screen pens 0..8, the pens the
//! system reserves (graphics/state.rs `Palette`). Each pixel takes the pen NEAREST (squared RGB distance, lowest pen on
//! a tie) to its colour in the LIVE screen palette, so the Gute Form palette the theme puts in pens 0..8 decides the
//! look: pen 0 = background/transparent, 1 = outline/text, 2 = highlight, 3..7 = the theme's mid tones and accent.
//! Transparent pixels (alpha < 0x80) always take pen 0.
use crate::icon_format::*;
use crate::graphics::state::Palette;
use heirloom_ui_abi::{GuestUi, IconRole};

/// Edge of the square icons asked of the host: 32 px is the classic Workbench 2+ icon scale (icons are ~30-50 px wide)
/// and a power-of-two the host's vector art rasterises cleanly.
pub const ICON_SIZE: u32 = 32;
const DEPTH: usize = 3;
/// Pens an icon may use: 0..ICON_PENS, the system-reserved ones.
pub const ICON_PENS: usize = 1 << DEPTH;

/// The role a Workbench `do_Type` is drawn as; None = the engine has no role for it (WBAPPICON).
pub fn role_of(ty: u8) -> Option<IconRole> {
    Some(match ty { 1 => IconRole::Disk, 2 => IconRole::Drawer, 3 => IconRole::Tool, 4 => IconRole::Project, 5 => IconRole::Trash, 6 => IconRole::Device, 7 => IconRole::Kick, _ => return None })
}

/// The pen of `argb` among `pens` (0xRRGGBB values for pens 0..), see the module's PEN MAPPING.
pub fn nearest_pen(argb: u32, pens: &[u32; ICON_PENS]) -> u8 {
    if (argb >> 24) < 0x80 { return 0; }
    let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as i32;
    let d = |p: u32| [16, 8, 0].iter().map(|&s| (ch(argb, s) - ch(p, s)).pow(2)).sum::<i32>();
    (0..ICON_PENS).min_by_key(|&i| (d(pens[i]), i)).expect("8 pens") as u8
}

/// Planar Image (ImageData layout: plane after plane, each `height` rows of word-aligned bits) for `pixels` (w x h).
pub fn planar_image(pixels: &[u32], w: u32, h: u32, pens: &[u32; ICON_PENS]) -> Image {
    let words = ((w + 15) / 16) as usize;
    let mut data = alloc::vec![0u8; DEPTH * h as usize * words * 2];
    for (i, &px) in pixels.iter().enumerate() {
        let (x, y) = (i % w as usize, i / w as usize);
        let pen = nearest_pen(px, pens);
        for p in 0..DEPTH {
            if pen >> p & 1 == 1 { data[(p * h as usize + y) * words * 2 + x / 8] |= 0x80 >> (x % 8); }
        }
    }
    let mut hdr = [0u8; IMAGE_HDR];
    for (at, v) in [(4, w as u16), (6, h as u16), (8, DEPTH as u16)] { hdr[at..at + 2].copy_from_slice(&v.to_be_bytes()); }
    hdr[14] = (ICON_PENS - 1) as u8; // PlanePick
    Image { hdr, data }
}

/// Ask the host for `role`'s artwork and build the planar Image. Err = the NAMED reason there is none (for the log).
pub fn host_image<U: GuestUi>(ui: &mut U, role: IconRole, palette: &Palette) -> Result<Image, alloc::string::String> {
    use alloc::format;
    let px = ui.icon_image(role, ICON_SIZE, ICON_SIZE).ok_or_else(|| format!("host has no {role:?} artwork"))?;
    if px.len() != (ICON_SIZE * ICON_SIZE) as usize { return Err(format!("host returned {} pixels for {role:?}, wanted {}", px.len(), ICON_SIZE * ICON_SIZE)); }
    let mut pens = [0u32; ICON_PENS];
    pens.copy_from_slice(&palette.rgb[..ICON_PENS]);
    Ok(planar_image(&px, ICON_SIZE, ICON_SIZE, &pens))
}

/// Make `icon` (a box default) show `img` instead: the same DiskObject, the gadget sized to the Image.
pub fn with_image(mut icon: Icon, img: Image) -> Icon {
    let (w, h) = (u16::from_be_bytes([img.hdr[4], img.hdr[5]]), u16::from_be_bytes([img.hdr[6], img.hdr[7]]));
    icon.head[12..14].copy_from_slice(&w.to_be_bytes());
    icon.head[14..16].copy_from_slice(&h.to_be_bytes());
    icon.images[0] = Some(img);
    icon
}
