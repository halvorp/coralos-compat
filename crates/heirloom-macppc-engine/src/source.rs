//! Where a PowerPC fragment's bytes come from: a bare PEF file, or an application file (MacBinary / AppleSingle with
//! the data fork inside, or an AppleDouble header file whose data fork is a separate file) whose `cfrg` 0 resource
//! names the fragment.
use crate::cfrg::{self, CfrgError, WHERE_DATA_FORK, USAGE_APPLICATION};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use heirloom_fmt_pef::Pef;
use heirloom_fmt_resfork::container::ContainerError;
use heirloom_fmt_resfork::{ForkError, ResFork};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Container(ContainerError),
    Fork(ForkError),
    /// The application has no `cfrg` 0 resource: it is not a CFM (PowerPC or fat) application.
    NoCfrg,
    Cfrg(CfrgError),
    /// `cfrg` has no `pwpc` application member (a 68k-only or library-only file).
    NoPowerPcMember,
    /// The container does not hold the data fork the fragment is in and none was given.
    NeedDataFork,
    /// The fragment is in memory or the resource fork, not the data fork.
    UnsupportedLocation(u8),
    /// offset/length run past the data fork.
    OutOfFork { offset: u32, length: u32, fork: usize },
}
impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Container(e) => write!(f, "{e}"),
            SourceError::Fork(e) => write!(f, "{e}"),
            SourceError::NoCfrg => write!(f, "no 'cfrg' 0 resource: not a Code Fragment Manager application"),
            SourceError::Cfrg(e) => write!(f, "{e}"),
            SourceError::NoPowerPcMember => write!(f, "'cfrg' has no 'pwpc' application member"),
            SourceError::NeedDataFork => write!(f, "this container holds only the resource fork; pass the data fork file (the fragment lives in it)"),
            SourceError::UnsupportedLocation(w) => write!(f, "the fragment is stored in location {w} (0 memory, 2 resource fork); only the data fork is supported"),
            SourceError::OutOfFork { offset, length, fork } => write!(f, "fragment offset {offset} length {length} runs past the {fork}-byte data fork"),
        }
    }
}
impl core::error::Error for SourceError {}

/// The bytes of the PowerPC application fragment in `file`, and its name. `data_fork` is the separate data fork file
/// when `file` is an AppleDouble header (or a bare resource fork).
pub fn fragment_bytes(file: &[u8], data_fork: Option<&[u8]>) -> Result<(String, Vec<u8>), SourceError> {
    if Pef::has_magic(file) { return Ok(("(bare PEF)".into(), file.to_vec())); }
    let opened = heirloom_fmt_resfork::open(file).map_err(SourceError::Container)?;
    let fork = ResFork::parse(file[opened.rsrc.clone()].to_vec()).map_err(SourceError::Fork)?;
    let i = fork.find(*b"cfrg", 0).ok_or(SourceError::NoCfrg)?;
    let members = cfrg::parse(fork.data(i).unwrap_or(&[])).map_err(SourceError::Cfrg)?;
    let m = members.iter().find(|m| &m.arch == b"pwpc" && m.usage == USAGE_APPLICATION).ok_or(SourceError::NoPowerPcMember)?;
    if m.location != WHERE_DATA_FORK { return Err(SourceError::UnsupportedLocation(m.location)); }
    let data: &[u8] = match (&opened.data, data_fork) {
        (_, Some(d)) => d,
        (Some(r), None) => &file[r.clone()],
        (None, None) => return Err(SourceError::NeedDataFork),
    };
    let end = if m.length == 0 { data.len() } else { (m.offset as usize).saturating_add(m.length as usize) };
    if m.offset as usize > data.len() || end > data.len() { return Err(SourceError::OutOfFork { offset: m.offset, length: m.length, fork: data.len() }); }
    let name = if m.name.is_empty() { opened.name.unwrap_or_else(|| format!("(cfrg member)")) } else { m.name.clone() };
    Ok((name, data[m.offset as usize..end].to_vec()))
}

/// The PowerPC IMPORT-LIBRARY members of the `cfrg` of `file` that live in its data fork: private shared libraries an application carries inside its own file (Blizzard's
/// StormCarbon, CursorDeviceLib). `(name, PEF bytes)`; the application's own fragment is [`fragment_bytes`]. Empty for a bare PEF or a file with no such members.
pub fn library_fragments(file: &[u8], data_fork: Option<&[u8]>) -> Vec<(String, Vec<u8>)> {
    let Ok(opened) = heirloom_fmt_resfork::open(file) else { return Vec::new() };
    let Ok(fork) = ResFork::parse(file[opened.rsrc.clone()].to_vec()) else { return Vec::new() };
    let Some(i) = fork.find(*b"cfrg", 0) else { return Vec::new() };
    let Ok(members) = cfrg::parse(fork.data(i).unwrap_or(&[])) else { return Vec::new() };
    let data: &[u8] = match (&opened.data, data_fork) { (_, Some(d)) => d, (Some(r), None) => &file[r.clone()], (None, None) => return Vec::new() };
    members.iter().filter(|m| &m.arch == b"pwpc" && m.usage == cfrg::USAGE_IMPORT_LIBRARY && m.location == WHERE_DATA_FORK).filter_map(|m| {
        let end = if m.length == 0 { data.len() } else { (m.offset as usize).saturating_add(m.length as usize) };
        (m.offset as usize <= data.len() && end <= data.len() && Pef::has_magic(&data[m.offset as usize..end])).then(|| (m.name.clone(), data[m.offset as usize..end].to_vec()))
    }).collect()
}
