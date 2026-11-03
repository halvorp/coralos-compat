//! Symbian (S60) application icons (no_std + alloc): an installed app's icon as the launcher shows it.
//!
//! THE CHAIN (Symbian's own lookup; facts from the S60 SDK's AppRegistrationInfo.rh documentation, checked on the corpus):
//! 1. `private/10003a3f/import/apps/*_reg.rsc`: resource 1 is `APP_REGISTRATION_INFO` - `LONG reserved, LLINK reserved,
//!    LTEXT app_file, LONG attributes, LTEXT localisable_resource_file, LONG localisable_resource_id, ...`. The one whose
//!    `app_file` is the program's name (case-insensitive, without `.exe`) is this app's.
//! 2. `localisable_resource_file` (a drive-less path with no extension: `.rsc`, else `.r01`, `.r00`) holds, at
//!    `localisable_resource_id`, `LOCALISABLE_APP_INFO` - `LONG reserved, LLINK reserved, LTEXT short_caption`, then
//!    `CAPTION_AND_ICON_INFO` (`LONG reserved, LLINK reserved, LTEXT caption, WORD number_of_icons, LTEXT icon_file`).
//! 3. `icon_file`: a `.mif` ([`mif`]: binary SVG-T, [`svgb`]) or a `.mbm` ([`mbm`]: a colour bitmap and its mask).
//!
//! Paths are matched case-insensitively, as the platform's file server does. Every missing link is a named [`SymIconError`].
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod mbm;
pub mod mif;
pub mod svgb;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;
use heirloom_fmt_rsc::{Reader, RscFile};
pub use heirloom_icon::{IconImage, Picture};

/// The registration resources' directory (the AppArc import directory every installed app's `_reg.rsc` lands in).
pub const REG_DIR: &str = "private/10003a3f/import/apps";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymIconError {
    /// No `_reg.rsc` names this program.
    NoRegistration(String),
    /// A file of the chain is not in the install tree.
    Missing(String),
    /// A resource of the chain did not parse (the file, and why).
    Resource(String, String),
    /// The registration names no icon file.
    NoIconFile,
    /// The icon file holds nothing this decoder reads (why).
    Icon(String),
}

impl fmt::Display for SymIconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SymIconError::NoRegistration(p) => write!(f, "no {REG_DIR}/*_reg.rsc registers {p}"),
            SymIconError::Missing(p) => write!(f, "{p} is not in the install tree"),
            SymIconError::Resource(p, why) => write!(f, "{p}: {why}"),
            SymIconError::NoIconFile => write!(f, "the app's localisable resource names no icon file"),
            SymIconError::Icon(why) => write!(f, "{why}"),
        }
    }
}
impl core::error::Error for SymIconError {}

/// A file of the install tree, by its tree-relative path (`/`-separated).
pub type ReadFn<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;
/// The names in a directory of the install tree.
pub type ListFn<'a> = &'a dyn Fn(&str) -> Vec<String>;

/// `path` (a Symbian path, `\`-separated, maybe with a drive) resolved case-insensitively against the tree; with `exts`, the
/// first extension that exists is appended.
fn resolve(list: ListFn, path: &str, exts: &[&str]) -> Option<String> {
    let p = path.replace('\\', "/");
    let p = if p.as_bytes().get(1) == Some(&b':') { &p[2..] } else { &p[..] };
    let p = p.trim_start_matches('/');
    let mut dir = String::new();
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let names = list(&dir);
        let want: Vec<String> = if last && !exts.is_empty() { exts.iter().map(|e| alloc::format!("{part}{e}")).collect() } else { alloc::vec![part.to_string()] };
        let found = want.iter().find_map(|w| names.iter().find(|n| n.eq_ignore_ascii_case(w)))?;
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(found);
    }
    Some(dir)
}

fn rsc_err(path: &str, e: impl fmt::Debug) -> SymIconError {
    SymIconError::Resource(path.into(), alloc::format!("{e:?}"))
}

/// `(app_file, localisable_resource_file, localisable_resource_id)` of a `_reg.rsc`.
pub fn registration(bytes: &[u8], path: &str) -> Result<(String, String, u32), SymIconError> {
    let f = RscFile::parse(bytes).map_err(|e| rsc_err(path, e))?;
    let r = f.resource(1).map_err(|e| rsc_err(path, e))?;
    let mut rd = Reader::new(&r);
    let mut field = || -> Result<(String, String, u32), heirloom_fmt_rsc::RscError> {
        rd.long()?;
        rd.long()?;
        let app = rd.ltext()?;
        rd.long()?;
        let loc = rd.ltext()?;
        let id = rd.long()?;
        Ok((app, loc, id))
    };
    field().map_err(|e| rsc_err(path, e))
}

/// `(caption, icon_file)` of a localisable resource's `LOCALISABLE_APP_INFO` at `id`.
pub fn localisable(bytes: &[u8], id: u32, path: &str) -> Result<(String, String), SymIconError> {
    let f = RscFile::parse(bytes).map_err(|e| rsc_err(path, e))?;
    let r = f.resource(id).map_err(|e| rsc_err(path, e))?;
    let mut rd = Reader::new(&r);
    let mut field = || -> Result<(String, String), heirloom_fmt_rsc::RscError> {
        rd.long()?; // reserved
        rd.long()?; // reserved link
        rd.ltext()?; // short caption
        rd.long()?; // CAPTION_AND_ICON_INFO: reserved
        rd.long()?; // reserved link
        let caption = rd.ltext()?;
        rd.word()?; // number of icons
        Ok((caption, rd.ltext()?))
    };
    field().map_err(|e| rsc_err(path, e))
}

/// The icon a Symbian install tree shows for program `exe` (`sys/bin/filebrowser.exe` or just `filebrowser`).
pub fn app_icon(read: ReadFn, list: ListFn, exe: &str) -> Result<Picture, SymIconError> {
    let stem = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    let stem = stem.strip_suffix(".exe").or_else(|| stem.strip_suffix(".EXE")).unwrap_or(stem);
    let reg_dir = resolve(list, REG_DIR, &[]).ok_or_else(|| SymIconError::Missing(REG_DIR.into()))?;
    let mut found = None;
    for name in list(&reg_dir).iter().filter(|n| n.to_ascii_lowercase().ends_with("_reg.rsc")) {
        let p = alloc::format!("{reg_dir}/{name}");
        let Some(b) = read(&p) else { continue };
        if let Ok(reg) = registration(&b, &p) {
            if reg.0.eq_ignore_ascii_case(stem) {
                found = Some(reg);
                break;
            }
        }
    }
    let (_, loc, id) = found.ok_or_else(|| SymIconError::NoRegistration(stem.into()))?;
    let loc_path = resolve(list, &loc, &[".rsc", ".r01", ".r00"]).ok_or_else(|| SymIconError::Missing(alloc::format!("{loc}.rsc")))?;
    let lb = read(&loc_path).ok_or_else(|| SymIconError::Missing(loc_path.clone()))?;
    let (_, icon_file) = localisable(&lb, id, &loc_path)?;
    if icon_file.is_empty() {
        return Err(SymIconError::NoIconFile);
    }
    let ip = resolve(list, &icon_file, &[]).ok_or_else(|| SymIconError::Missing(icon_file.clone()))?;
    let ib = read(&ip).ok_or_else(|| SymIconError::Missing(ip.clone()))?;
    if ip.to_ascii_lowercase().ends_with(".mbm") {
        return mbm::icon(&ib).map(Picture::Raster).map_err(|e| SymIconError::Icon(alloc::format!("{ip}: {e}")));
    }
    mif::icon(&ib).map_err(|e| SymIconError::Icon(alloc::format!("{ip}: {e}")))
}

#[cfg(test)]
mod tests;
