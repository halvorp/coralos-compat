//! Classic Mac application icons (no_std + alloc), for every Heirloom Mac application (68k and PowerPC alike).
//!
//! * [`app_family_id`]: the Finder's lookup. The application's `'BNDL'` (the one whose signature is the file's creator, else
//!   the first) lists its `'FREF'`s; the FREF for file type `'APPL'` names a LOCAL icon id; the BNDL's `'ICN#'` table maps that
//!   local id to the resource id every member of the icon family carries (Inside Macintosh: Macintosh Toolbox Essentials,
//!   "Finder Interface", the bundle resource).
//! * [`family_image`]: the best member of family `id`, decoded to straight RGBA: the `'icns'` resource's 32-bit members first
//!   (`it32`+`t8mk` 128 px, `ih32`+`h8mk` 48, `il32`+`l8mk` 32), then the 8-bit members (`ich8` 48, `icl8` 32 with the
//!   `ich#`/`ICN#` mask), the 16 px ones (`is32`+`s8mk`, `ics8`+`ics#`), the 4-bit ones (`ich4`, `icl4`, `ics4`) and last the
//!   black-and-white `ich#`/`ICN#`/`ics#`. A member is looked up inside the family's `'icns'` resource and then as a resource
//!   of its own type and the family's id (Icon Services reads both).
//! * [`clut`]: the system colour tables the 8-bit and 4-bit members index.
//!
//! The 32-bit members are three channel planes (red, green, blue), each packed by the icns run-length scheme: a control byte
//! `c < 0x80` copies the next `c + 1` bytes, `c >= 0x80` repeats the next byte `c - 125` times. `it32` data starts with four
//! zero bytes. A member whose length is exactly `w * h * 4` is unpacked ARGB. Every malformed input is a named [`IconError`]
//! or a skipped member; nothing here panics on file contents.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod clut;

use alloc::vec::Vec;
use core::fmt;
use heirloom_common::bytes::{be16, be32};
use heirloom_fmt_resfork::container::{self, ContainerError};
use heirloom_fmt_resfork::{ForkError, OsType, ResFork};

/// One decoded icon: `w` x `h` straight-alpha RGBA8, row-major, and which member(s) it came from (`"it32+t8mk"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconImage {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub source: &'static str,
}

/// Why an application's icon could not be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconError {
    Container(ContainerError),
    Fork(ForkError),
    /// The fork has no `'BNDL'`: the Finder shows the generic application icon.
    NoBundle,
    /// No `'FREF'` the bundle lists is for file type `'APPL'`.
    NoAppFref,
    /// The bundle's `'ICN#'` table does not map the APPL FREF's local id.
    NoIconMapping(i16),
    /// Family `id` has no member this decoder reads.
    NoFamily(i16),
}

impl fmt::Display for IconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IconError::Container(e) => write!(f, "{e}"),
            IconError::Fork(e) => write!(f, "{e}"),
            IconError::NoBundle => write!(f, "no 'BNDL' resource: the desktop shows the generic application icon"),
            IconError::NoAppFref => write!(f, "the bundle has no 'FREF' for file type 'APPL'"),
            IconError::NoIconMapping(l) => write!(f, "the bundle's 'ICN#' table does not map local icon id {l}"),
            IconError::NoFamily(id) => write!(f, "icon family {id} has no member this decoder reads"),
        }
    }
}
impl core::error::Error for IconError {}

/// The application icon of a Mac file (MacBinary, AppleSingle, AppleDouble or a raw resource fork).
pub fn app_icon(file: &[u8]) -> Result<IconImage, IconError> {
    let opened = container::open(file).map_err(IconError::Container)?;
    let fork = ResFork::parse(file[opened.rsrc.clone()].to_vec()).map_err(IconError::Fork)?;
    let id = app_family_id(&fork, opened.creator)?;
    family_image(&fork, id).ok_or(IconError::NoFamily(id))
}

fn res<'a>(fork: &'a ResFork, ty: &OsType, id: i16) -> Option<&'a [u8]> {
    fork.find(*ty, id).and_then(|i| fork.data(i))
}

/// One BNDL's type tables: `(type, [(local id, resource id)])`.
fn bndl_tables(b: &[u8]) -> Vec<(OsType, Vec<(i16, i16)>)> {
    let mut out = Vec::new();
    let Some(n) = be16(b, 6) else { return out };
    let mut pos = 8usize;
    for _ in 0..(n as i16 as i32 + 1).max(0) {
        let (Some(ty), Some(c)) = (b.get(pos..pos + 4), be16(b, pos + 4)) else { break };
        let count = (c as i16 as i32 + 1).max(0) as usize;
        let mut pairs = Vec::with_capacity(count);
        for k in 0..count {
            let at = pos + 6 + 4 * k;
            let (Some(l), Some(r)) = (be16(b, at), be16(b, at + 2)) else { return out };
            pairs.push((l as i16, r as i16));
        }
        out.push((ty.try_into().unwrap_or([0; 4]), pairs));
        pos += 6 + 4 * count;
    }
    out
}

/// The icon family id the Finder shows for this application: BNDL -> FREF 'APPL' -> local id -> the BNDL's 'ICN#' mapping.
/// The BNDL whose signature is `creator` is read first; without one (or with an unknown creator, `????`) every BNDL in order.
pub fn app_family_id(fork: &ResFork, creator: Option<OsType>) -> Result<i16, IconError> {
    let mut bndls: Vec<&[u8]> = (1..=fork.count(*b"BNDL")).filter_map(|n| fork.nth(*b"BNDL", n)).filter_map(|i| fork.data(i)).collect();
    if bndls.is_empty() {
        return Err(IconError::NoBundle);
    }
    if let Some(c) = creator {
        bndls.sort_by_key(|b| b.get(0..4) != Some(&c[..]));
    }
    let mut last = IconError::NoAppFref;
    for b in bndls {
        let tables = bndl_tables(b);
        let table = |ty: &OsType| tables.iter().find(|(t, _)| t == ty).map(|(_, p)| p.as_slice()).unwrap_or(&[]);
        let local = table(b"FREF").iter().filter_map(|&(_, rid)| res(fork, b"FREF", rid)).find(|f| f.get(0..4) == Some(b"APPL")).and_then(|f| be16(f, 4));
        let Some(local) = local.map(|l| l as i16) else { continue };
        match table(b"ICN#").iter().find(|&&(l, _)| l == local) {
            Some(&(_, rid)) => return Ok(rid),
            None => last = IconError::NoIconMapping(local),
        }
    }
    Err(last)
}

/// The elements of an `'icns'` container: `(type, data)`.
pub fn icns_elements(icns: &[u8]) -> Vec<(OsType, &[u8])> {
    let mut out = Vec::new();
    if icns.get(0..4) != Some(b"icns") {
        return out;
    }
    let end = be32(icns, 4).map_or(0, |l| (l as usize).min(icns.len()));
    let mut pos = 8usize;
    while pos + 8 <= end {
        let len = be32(icns, pos + 4).unwrap_or(0) as usize;
        if len < 8 || pos + len > end {
            break;
        }
        let ty: OsType = icns[pos..pos + 4].try_into().unwrap_or([0; 4]);
        out.push((ty, &icns[pos + 8..pos + len]));
        pos += len;
    }
    out
}

/// How a member's pixels are stored.
#[derive(Clone, Copy)]
enum Depth {
    /// Three run-length packed planes (or unpacked ARGB), alpha from an 8-bit mask member.
    Rgb { mask8: &'static OsType },
    Indexed8,
    Indexed4,
    /// The `'#'` member itself: icon plane then mask plane.
    Mono,
}

/// `(member, side, depth, its 1-bit '#' member, source name)`, best first.
const ORDER: [(&OsType, u32, Depth, &OsType, &str); 13] = [
    (b"it32", 128, Depth::Rgb { mask8: b"t8mk" }, b"\0\0\0\0", "it32"),
    (b"ih32", 48, Depth::Rgb { mask8: b"h8mk" }, b"ich#", "ih32"),
    (b"il32", 32, Depth::Rgb { mask8: b"l8mk" }, b"ICN#", "il32"),
    (b"ich8", 48, Depth::Indexed8, b"ich#", "ich8"),
    (b"icl8", 32, Depth::Indexed8, b"ICN#", "icl8"),
    (b"is32", 16, Depth::Rgb { mask8: b"s8mk" }, b"ics#", "is32"),
    (b"ics8", 16, Depth::Indexed8, b"ics#", "ics8"),
    (b"ich4", 48, Depth::Indexed4, b"ich#", "ich4"),
    (b"icl4", 32, Depth::Indexed4, b"ICN#", "icl4"),
    (b"ics4", 16, Depth::Indexed4, b"ics#", "ics4"),
    (b"ich#", 48, Depth::Mono, b"ich#", "ich#"),
    (b"ICN#", 32, Depth::Mono, b"ICN#", "ICN#"),
    (b"ics#", 16, Depth::Mono, b"ics#", "ics#"),
];

/// The best decodable member of icon family `id` (see the module note for the order).
pub fn family_image(fork: &ResFork, id: i16) -> Option<IconImage> {
    let icns = res(fork, b"icns", id).map(icns_elements).unwrap_or_default();
    let get = |ty: &OsType| icns.iter().find(|(t, _)| t == ty).map(|(_, d)| *d).or_else(|| res(fork, ty, id));
    best(&get)
}

/// The best decodable member of a whole `'icns'` container (an `.icns` file, or an `'icns'` resource's data).
pub fn icns_image(icns: &[u8]) -> Option<IconImage> {
    let el = icns_elements(icns);
    best(&|ty: &OsType| el.iter().find(|(t, _)| t == ty).map(|(_, d)| *d))
}

fn best<'a>(get: &dyn Fn(&OsType) -> Option<&'a [u8]>) -> Option<IconImage> {
    ORDER.iter().find_map(|&(ty, side, depth, mono, name)| {
        let data = get(ty)?;
        let mono_mask = get(mono).and_then(|m| plane(m, side, 1));
        let (rgba, source) = match depth {
            Depth::Rgb { mask8 } => {
                let alpha = get(mask8).filter(|m| m.len() == (side * side) as usize);
                let src = if alpha.is_some() { mask_name(name) } else { name };
                (rgb(data, side, *ty == *b"it32", alpha, mono_mask)?, src)
            }
            Depth::Indexed8 => (indexed(data, side, 8, mono_mask)?, name),
            Depth::Indexed4 => (indexed(data, side, 4, mono_mask)?, name),
            Depth::Mono => (mono_px(data, side)?, name),
        };
        Some(IconImage { w: side, h: side, rgba, source })
    })
}

fn mask_name(name: &'static str) -> &'static str {
    match name { "it32" => "it32+t8mk", "ih32" => "ih32+h8mk", "il32" => "il32+l8mk", "is32" => "is32+s8mk", n => n }
}

/// Plane `k` (0 = icon, 1 = mask) of a 1-bit `'#'` member, as one bool per pixel.
fn plane(m: &[u8], side: u32, k: usize) -> Option<Vec<bool>> {
    let bytes = (side * side / 8) as usize;
    let p = m.get(k * bytes..(k + 1) * bytes)?;
    Some((0..(side * side) as usize).map(|i| p[i / 8] & (0x80 >> (i % 8)) != 0).collect())
}

fn put(out: &mut Vec<u8>, rgb: u32, a: u8) {
    out.extend_from_slice(&[(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, a]);
}

fn indexed(data: &[u8], side: u32, bits: u32, mask: Option<Vec<bool>>) -> Option<Vec<u8>> {
    let n = (side * side) as usize;
    if data.len() < n * bits as usize / 8 {
        return None;
    }
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let rgb = if bits == 8 { clut::CLUT8[data[i] as usize] } else { clut::CLUT4[((data[i / 2] >> (4 * (1 - i % 2))) & 0xF) as usize] };
        put(&mut out, rgb, if mask.as_ref().is_none_or(|m| m[i]) { 0xFF } else { 0 });
    }
    Some(out)
}

fn mono_px(data: &[u8], side: u32) -> Option<Vec<u8>> {
    let (icon, mask) = (plane(data, side, 0)?, plane(data, side, 1)?);
    let mut out = Vec::with_capacity(icon.len() * 4);
    for (b, m) in icon.iter().zip(mask) {
        put(&mut out, if *b { 0 } else { 0xFFFFFF }, if m || *b { 0xFF } else { 0 });
    }
    Some(out)
}

/// The icns run-length scheme, until `n` bytes are out; None when the input ends first.
pub fn unpack(src: &[u8], n: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0usize;
    while out.len() < n {
        let c = *src.get(i)? as usize;
        i += 1;
        if c >= 0x80 {
            let v = *src.get(i)?;
            i += 1;
            out.resize(out.len() + c - 125, v);
        } else {
            out.extend_from_slice(src.get(i..i + c + 1)?);
            i += c + 1;
        }
    }
    out.truncate(n);
    Some(out)
}

fn rgb(data: &[u8], side: u32, it32: bool, alpha: Option<&[u8]>, mono_mask: Option<Vec<bool>>) -> Option<Vec<u8>> {
    let n = (side * side) as usize;
    let mut out = Vec::with_capacity(n * 4);
    let a = |i: usize| alpha.map_or_else(|| if mono_mask.as_ref().is_none_or(|m| m[i]) { 0xFF } else { 0 }, |a| a[i]);
    if data.len() == n * 4 {
        for i in 0..n {
            out.extend_from_slice(&[data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3], a(i)]);
        }
        return Some(out);
    }
    let packed = if it32 && data.get(0..4) == Some(&[0; 4]) { &data[4..] } else { data };
    let planes = unpack(packed, n * 3)?;
    for i in 0..n {
        out.extend_from_slice(&[planes[i], planes[n + i], planes[2 * n + i], a(i)]);
    }
    Some(out)
}

#[cfg(test)]
mod tests;
