//! Amiga Workbench icons (no_std + alloc): a program's `<name>.info` to the picture Workbench shows for it, best member first.
//!
//! FORMAT FACTS (read from AROS `workbench/libs/icon/` - diskobjio.c, diskobjNIio.c, diskobj35io.c, diskobjPNGio.c, support.c;
//! AROS Public License 1.1, an MPL-1.1 derivative: facts only, this code is written from them - and checked against the corpus's
//! real icons by tests/real_icons.rs):
//! * A `.info` is a big-endian `DiskObject` (78 bytes: magic `0xE310`, version 1, an Intuition `Gadget` whose `GadgetRender` /
//!   `SelectRender` say whether a normal / selected `Image` follows and whose `UserData` low byte is the Workbench revision,
//!   then `do_Type`, the `DefaultTool` / `ToolTypes` / `DrawerData` / `ToolWindow` flags), then IN THIS ORDER, each only when
//!   its flag is set: an old `DrawerData` (56 bytes), the normal `Image` (20 bytes + `Depth` planes of `((Width+15)/16)*2`
//!   bytes per row), the selected `Image`, the default tool string, the tool types (a `u32` = (count+1)*4, then the strings),
//!   the tool window string, the OS2 `DrawerData` extension (6 bytes, revision 1 only), and last an OS3.5 `FORM ICON`.
//!   A string is a `u32` length (NUL included) and its bytes.
//! * Planar images carry no palette: Workbench draws them with the screen's first pens - OS1.x blue / white / black / orange,
//!   OS2/3 grey / black / white / blue (4 pens) and the 8-pen MagicWB set (the de-facto 8-colour icon palette). Pen 0 is the
//!   window background, so it is transparent here.
//! * NewIcons ([`newicons`]), ColorIcons ([`coloricon`]) and PNG icons ([`png`]) carry their own palette or RGBA.
//!
//! Best first: a PNG icon (the whole file is a PNG), the ColorIcon's ARGB / PNG chunk, its palette-mapped IMAG, a NewIcon, the
//! planar image. The SELECTED image of every form is ignored. Every malformed input is a named [`AmiIconError`] or a member
//! skipped for the next; nothing here panics on file contents.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod coloricon;
pub mod newicons;
pub mod png;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use heirloom_common::bytes::{be16, be32};
pub use heirloom_icon::IconImage;

/// Why a `.info` gave no picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmiIconError {
    /// Not a DiskObject (magic 0xE310) and not a PNG icon.
    NotAnIcon,
    /// The DiskObject ends before the part named.
    Truncated(&'static str),
    /// Every member present was refused; the reasons, best member first.
    NoMember(Vec<&'static str>),
}

impl fmt::Display for AmiIconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AmiIconError::NotAnIcon => write!(f, "not an Amiga icon (no DiskObject magic 0xE310, no PNG signature)"),
            AmiIconError::Truncated(w) => write!(f, "the icon ends inside its {w}"),
            AmiIconError::NoMember(r) => write!(f, "no icon image could be decoded ({})", r.join("; ")),
        }
    }
}
impl core::error::Error for AmiIconError {}

/// One planar `Image`: its geometry and plane bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planar {
    pub w: u32,
    pub h: u32,
    pub depth: u32,
    pub plane_pick: u8,
    pub plane_on_off: u8,
    pub planes: Vec<u8>,
}

/// The parts of a DiskObject an icon picture comes from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiskObject {
    /// `Gadget.UserData & 0xFF`: 0 = an OS1.x icon, 1 = OS2 and later.
    pub revision: u8,
    pub normal: Option<Planar>,
    pub tool_types: Vec<String>,
    /// Where the OS3.5 `FORM ICON` would start (the end of the classic part).
    pub rest: usize,
}

const IMAGE_LEN: usize = 20;

fn string_at(b: &[u8], at: &mut usize, what: &'static str) -> Result<String, AmiIconError> {
    let len = be32(b, *at).ok_or(AmiIconError::Truncated(what))? as usize;
    let body = b.get(*at + 4..*at + 4 + len).ok_or(AmiIconError::Truncated(what))?;
    *at += 4 + len;
    let text = body.split(|&c| c == 0).next().unwrap_or(&[]);
    Ok(text.iter().map(|&c| c as char).collect()) // ISO-8859-1, byte = code point
}

fn image_at(b: &[u8], at: &mut usize) -> Result<Planar, AmiIconError> {
    let h = b.get(*at..*at + IMAGE_LEN).ok_or(AmiIconError::Truncated("Image"))?;
    let (w, ht, depth) = (be16(h, 4).unwrap_or(0) as u32, be16(h, 6).unwrap_or(0) as u32, be16(h, 8).unwrap_or(0) as u32);
    let size = (w.div_ceil(16) * 2 * ht * depth) as usize;
    let planes = b.get(*at + IMAGE_LEN..*at + IMAGE_LEN + size).ok_or(AmiIconError::Truncated("Image planes"))?.to_vec();
    *at += IMAGE_LEN + size;
    Ok(Planar { w, h: ht, depth, plane_pick: h[14], plane_on_off: h[15], planes })
}

/// The classic DiskObject: its planar normal image, tool types and where the classic part ends.
pub fn disk_object(b: &[u8]) -> Result<DiskObject, AmiIconError> {
    if be16(b, 0) != Some(0xE310) {
        return Err(AmiIconError::NotAnIcon);
    }
    let f = |o: usize| be32(b, o).ok_or(AmiIconError::Truncated("DiskObject"));
    let (gadget_render, select_render, user_data) = (f(22)?, f(26)?, f(44)?);
    let (default_tool, tool_types, drawer_data, tool_window) = (f(50)?, f(54)?, f(66)?, f(70)?);
    let mut d = DiskObject { revision: user_data as u8, ..Default::default() };
    let mut at = 78usize;
    if drawer_data != 0 {
        at += 56;
    }
    if gadget_render != 0 {
        d.normal = Some(image_at(b, &mut at)?);
    }
    if select_render != 0 {
        image_at(b, &mut at)?; // the selected image is ignored
    }
    if default_tool != 0 {
        string_at(b, &mut at, "default tool")?;
    }
    if tool_types != 0 {
        let n = (be32(b, at).ok_or(AmiIconError::Truncated("tool types"))? / 4).saturating_sub(1);
        at += 4;
        for _ in 0..n {
            d.tool_types.push(string_at(b, &mut at, "tool type")?);
        }
    }
    if tool_window != 0 {
        string_at(b, &mut at, "tool window")?;
    }
    if drawer_data != 0 && d.revision == 1 {
        at += 6;
    }
    d.rest = at.min(b.len());
    Ok(d)
}

/// OS1.x Workbench pens.
pub const PENS_OS1: [u32; 4] = [0x0055AA, 0xFFFFFF, 0x000000, 0xFF8800];
/// OS2 / OS3 Workbench pens (4 colours).
pub const PENS_OS2: [u32; 4] = [0xAAAAAA, 0x000000, 0xFFFFFF, 0x6688BB];
/// The MagicWB 8-pen set (OS2/3 icons of depth 3).
pub const PENS_MAGICWB: [u32; 8] = [0x959595, 0x000000, 0xFFFFFF, 0x3B67A2, 0x7B7B7B, 0xAFAFAF, 0xAA907C, 0xFFA997];

/// A planar image in the Workbench pens of `revision`; pen 0 transparent. None for a depth the pens do not cover.
pub fn planar_image(p: &Planar, revision: u8) -> Option<IconImage> {
    let pens: &[u32] = match (revision, p.depth) {
        (0, 1..=2) => &PENS_OS1,
        (_, 1..=2) => &PENS_OS2,
        (r, 3) if r != 0 => &PENS_MAGICWB,
        _ => return None,
    };
    let row = (p.w.div_ceil(16) * 2) as usize;
    let plane = row * p.h as usize;
    let mut stored = 0usize;
    let mut source = Vec::new(); // per bitplane: Some(stored plane index) or None (the PlaneOnOff bit)
    for k in 0..p.depth {
        if p.plane_pick & (1 << k) != 0 {
            source.push(Some(stored));
            stored += 1;
        } else {
            source.push(None);
        }
    }
    if p.planes.len() < stored * plane {
        return None;
    }
    let mut rgba = Vec::with_capacity((p.w * p.h * 4) as usize);
    for y in 0..p.h as usize {
        for x in 0..p.w as usize {
            let mut pen = 0usize;
            for (k, s) in source.iter().enumerate() {
                let bit = match s {
                    Some(i) => p.planes[i * plane + y * row + x / 8] & (0x80 >> (x % 8)) != 0,
                    None => p.plane_on_off & (1 << k) != 0,
                };
                pen |= (bit as usize) << k;
            }
            let c = *pens.get(pen)?;
            rgba.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, if pen == 0 { 0 } else { 0xFF }]);
        }
    }
    Some(IconImage { w: p.w, h: p.h, rgba, source: if pens.len() == 8 { "planar+magicwb" } else if revision == 0 { "planar+os1" } else { "planar+os2" } })
}

/// The picture Workbench shows for this `.info`, best member first (see the module note).
pub fn app_icon(info: &[u8]) -> Result<IconImage, AmiIconError> {
    let mut refused = Vec::new();
    if info.starts_with(&png::SIGNATURE) {
        return png::decode(info).map_err(|e| AmiIconError::NoMember(alloc::vec![e]));
    }
    let d = disk_object(info)?;
    match coloricon::decode(&info[d.rest..]) {
        Ok(Some(img)) => return Ok(img),
        Ok(None) => {}
        Err(e) => refused.push(e),
    }
    match newicons::decode(&d.tool_types) {
        Ok(Some(img)) => return Ok(img),
        Ok(None) => {}
        Err(e) => refused.push(e),
    }
    if let Some(p) = &d.normal {
        match planar_image(p, d.revision) {
            Some(img) => return Ok(img),
            None => refused.push("a planar image deeper than the Workbench pens this decoder knows"),
        }
    }
    if refused.is_empty() {
        refused.push("the DiskObject has no image");
    }
    Err(AmiIconError::NoMember(refused))
}

#[cfg(test)]
mod tests;
