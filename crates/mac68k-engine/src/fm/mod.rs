//! The Macintosh File Manager over the shared volume tree ([`retro_volume`], the same tree the Amiga engine's dos.library
//! uses): this is the HFS NAMING AND SEMANTICS layer (volume refnums, working directories, dirIDs, `Vol:folder:file`
//! paths, fork refnums with a file mark, OSErr codes) on a platform-neutral tree of volumes, directories and files with
//! data AND resource forks, File info (FInfo) and dates. Inside Macintosh: Files.
//!
//! * [`names`]: (vRefNum, dirID, name) -> a node, the HFS way.
//! * [`pb`]: parameter-block field offsets (IOParam / FileParam / HFileInfo / DirInfo / VolumeParam / WDPBRec / FCBPBRec / FSSpec).
//! * [`forks`]: open files: Open, Close, Read, Write, EOF and mark.
//! * [`catalog`]: Create, Delete, Rename, GetFileInfo / SetFileInfo / GetCatInfo, DirCreate, CatMove, locks.
//! * [`volumes`]: GetVol, SetVol, GetVolInfo, FlushVol, working directories, FCB info.
//! * [`traps`]: the trap glue: the classic `_Open` ... `_SetFPos` traps (and their HFS forms, bit 9), `_HFSDispatch` /
//!   `_FSDispatch` selectors, ioResult / ioCompletion handling.
//! * [`fsspec`]: the high-level FSSpec calls (`_HighLevelFSDispatch`).
//! * [`resfiles`]: OpenResFile / CreateResFile / CloseResFile on files' resource forks.
pub mod alias;
pub mod catalog;
pub mod forks;
pub mod fsspec;
pub mod names;
pub mod pb;
pub mod resfiles;
pub mod traps;
pub mod volumes;

use alloc::collections::BTreeMap;
use alloc::string::String;
use retro_volume::{NodeId, Tree, VolErr};

/// File Manager OSErr codes (Inside Macintosh: Files, "Result codes").
pub mod err {
    pub const NO_ERR: i16 = 0;
    pub const DIR_FUL_ERR: i16 = -33;
    pub const DSK_FUL_ERR: i16 = -34;
    pub const NSV_ERR: i16 = -35;
    pub const IO_ERR: i16 = -36;
    pub const BD_NAM_ERR: i16 = -37;
    pub const FN_OPN_ERR: i16 = -38;
    pub const EOF_ERR: i16 = -39;
    pub const POS_ERR: i16 = -40;
    pub const TMFO_ERR: i16 = -42;
    pub const FNF_ERR: i16 = -43;
    pub const W_PR_ERR: i16 = -44;
    pub const F_LCKD_ERR: i16 = -45;
    pub const V_LCKD_ERR: i16 = -46;
    pub const F_BSY_ERR: i16 = -47;
    pub const DUP_FN_ERR: i16 = -48;
    pub const OP_WR_ERR: i16 = -49;
    pub const PARAM_ERR: i16 = -50;
    pub const RF_NUM_ERR: i16 = -51;
    pub const GFP_ERR: i16 = -52;
    pub const PERM_ERR: i16 = -54;
    pub const WR_PERM_ERR: i16 = -61;
    pub const DIR_NF_ERR: i16 = -120;
    pub const TMWDO_ERR: i16 = -121;
    pub const BAD_MOV_ERR: i16 = -122;
}

/// The root directory of every volume has this dirID, and its parent this one (Files.h fsRtDirID / fsRtParID).
pub const RT_DIR_ID: u32 = 2;
pub const RT_PAR_ID: u32 = 1;
/// dirIDs and file numbers of everything else: the tree's node id plus this (so never 1 or 2).
const ID_BASE: u32 = 16;
/// Allocation block size reported to programs (physical lengths are rounded up to it).
pub const ALLOC_BLOCK: u32 = 512;
/// Working-directory reference numbers count down from here (volume refnums are -1, -2, ...).
const WD_BASE: i16 = -0x4000;
/// The volume the machine boots with.
pub const BOOT_VOLUME: &str = "Macintosh HD";
/// HFS names are at most 31 bytes.
pub const MAX_NAME: usize = 31;

/// Open permission bits kept per open fork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Perm { pub read: bool, pub write: bool }

/// One open fork (an FCB): which file, which fork, the file mark.
#[derive(Debug, Clone)]
pub struct OpenFork { pub node: NodeId, pub rsrc: bool, pub perm: Perm, pub mark: u32 }

pub struct FileMgr {
    pub tree: Tree,
    forks: BTreeMap<i16, OpenFork>,
    next_ref: i16,
    /// Working directories: refnum -> (volume, directory).
    wds: BTreeMap<i16, (NodeId, NodeId)>,
    next_wd: i16,
    /// SetVol / GetVol: the default volume and directory.
    pub def_vol: NodeId,
    pub def_dir: NodeId,
    /// The clock: seconds since 1904-01-01 (the host sets it; creation and modification dates come from it).
    pub now_secs: u32,
}

impl Default for FileMgr { fn default() -> Self { Self::new() } }

impl FileMgr {
    /// A machine with one writable boot volume, [`BOOT_VOLUME`], as the default directory.
    pub fn new() -> FileMgr {
        let mut tree = Tree::new(MAX_NAME);
        let boot = tree.add_volume(BOOT_VOLUME, true);
        FileMgr { tree, forks: BTreeMap::new(), next_ref: 0, wds: BTreeMap::new(), next_wd: 0, def_vol: boot, def_dir: boot, now_secs: 3_600_000_000 }
    }

    /// Host-side: add a volume the guest can see.
    pub fn add_volume(&mut self, name: &str, writable: bool) -> NodeId { self.tree.add_volume(name, writable) }

    /// A volume's vRefNum: -1 for the first, -2 for the next ...
    pub fn vref_of(&self, vol: NodeId) -> i16 {
        -(1 + self.tree.children(retro_volume::ROOT).iter().position(|&c| c == vol).unwrap_or(0) as i16)
    }
    /// The dirID of a directory node.
    pub fn dir_id(&self, dir: NodeId) -> u32 { if self.tree.is_volume(dir) { RT_DIR_ID } else { dir as u32 + ID_BASE } }
    /// The file number of a file node (what `ioFlNum` / a CatInfo `ioDirID` holds for a file).
    pub fn file_id(&self, node: NodeId) -> u32 { node as u32 + ID_BASE }
    pub fn open_fork(&self, refnum: i16) -> Option<&OpenFork> { self.forks.get(&refnum) }
    pub fn open_forks(&self) -> impl Iterator<Item = (&i16, &OpenFork)> { self.forks.iter() }
    /// Is any fork of `node` open (for write when `for_write`)?
    pub fn is_open(&self, node: NodeId, for_write: bool) -> bool { self.forks.values().any(|f| f.node == node && (!for_write || f.perm.write)) }
}

/// Map the tree's refusals to the File Manager's result codes.
pub fn os_err(e: VolErr) -> i16 {
    match e {
        VolErr::NotFound => err::FNF_ERR, VolErr::Exists => err::DUP_FN_ERR, VolErr::WrongType => err::DIR_NF_ERR,
        VolErr::ReadOnly => err::W_PR_ERR, VolErr::InvalidName => err::BD_NAM_ERR, VolErr::NotEmpty => err::F_BSY_ERR,
        VolErr::AcrossVolumes | VolErr::InUse => err::BAD_MOV_ERR,
    }
}

/// Guest name bytes (MacRoman) as a tree name: one char per byte, so every name round-trips exactly.
pub fn name_of(bytes: &[u8]) -> String { bytes.iter().map(|&b| char::from(b)).collect() }
/// A tree name back to the guest's bytes.
pub fn bytes_of(name: &str) -> alloc::vec::Vec<u8> { name.chars().map(|c| c as u32 as u8).collect() }

impl<U: guestui_abi::GuestUi> crate::machine::Mac<U> {
    /// The guest clock for new files' dates: the `Time` global (seconds since 1904) once a program or the host has set it,
    /// else the File Manager's own start-up value.
    pub(crate) fn clock(&self) -> u32 {
        match self.mem.read_u32(crate::lowmem::TIME) { Ok(t) if t != 0 => t, _ => self.fm.now_secs }
    }
}
