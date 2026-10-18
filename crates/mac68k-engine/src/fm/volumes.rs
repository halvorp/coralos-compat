//! Volumes and working directories: GetVol / SetVol, GetVolInfo, OpenWD / CloseWD / GetWDInfo, GetFCBInfo (Inside
//! Macintosh: Files, "Volumes" and "Working directories"). Pure over [`FileMgr`].
use super::names::Located;
use super::{err, FileMgr, ALLOC_BLOCK};
use alloc::vec::Vec;
use retro_volume::{Kind, NodeId, ROOT};

/// The capacity every volume reports: 0xFFFF allocation blocks of [`ALLOC_BLOCK`] bytes.
pub const TOTAL_BLOCKS: u32 = 0xFFFF;
/// ioVAtrb: bit 15 = the volume is software locked / read-only here.
pub const VATRB_LOCKED: u16 = 0x8000;

/// What GetVolInfo reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolInfo {
    pub vref: i16,
    pub name: Vec<u8>,
    pub attrib: u16,
    pub files_in_root: u16,
    pub total_blocks: u16,
    pub free_blocks: u16,
    pub file_count: u32,
    pub dir_count: u32,
    pub created: u32,
}

impl FileMgr {
    /// Bytes used on a volume (both forks, rounded to allocation blocks).
    fn used(&self, dir: NodeId) -> u64 {
        self.tree.children(dir).iter().map(|&c| match &self.tree.get(c).kind {
            Kind::Dir(_) => self.used(c),
            Kind::File(d) => u64::from((d.len() as u32).div_ceil(ALLOC_BLOCK) + (self.tree.get(c).rsrc.len() as u32).div_ceil(ALLOC_BLOCK)) * u64::from(ALLOC_BLOCK),
        }).sum()
    }
    fn count(&self, dir: NodeId, dirs: bool) -> u32 {
        self.tree.children(dir).iter().map(|&c| if self.tree.is_dir(c) { u32::from(dirs) + self.count(c, dirs) } else { u32::from(!dirs) }).sum()
    }

    pub fn vol_info(&self, vol: NodeId) -> VolInfo {
        let used_blocks = (self.used(vol) / u64::from(ALLOC_BLOCK)).min(u64::from(TOTAL_BLOCKS)) as u32;
        VolInfo {
            vref: self.vref_of(vol), name: super::bytes_of(self.tree.name(vol)), attrib: if self.tree.is_writable(vol) { 0 } else { VATRB_LOCKED },
            files_in_root: self.tree.children(vol).iter().filter(|&&c| !self.tree.is_dir(c)).count() as u16,
            total_blocks: TOTAL_BLOCKS as u16, free_blocks: (TOTAL_BLOCKS - used_blocks) as u16,
            file_count: self.count(vol, false), dir_count: self.count(vol, true), created: self.tree.get(vol).created_ms.max(0) as u32,
        }
    }

    /// Select a volume for GetVolInfo the way ioVolIndex says: > 0 the nth mounted volume; < 0 by name (falling back to the
    /// vRefNum); 0 by vRefNum alone.
    pub fn pick_volume(&self, index: i16, vref: i16, name: Option<&[u8]>) -> Result<NodeId, i16> {
        if index > 0 { return self.tree.children(ROOT).get(index as usize - 1).copied().ok_or(err::NSV_ERR); }
        if index < 0 { if let Some(n) = name.filter(|n| !n.is_empty()) {
            let mut full = n.to_vec(); // ioNamePtr names a VOLUME here, with or without its colon
            if !full.contains(&b':') { full.push(b':'); }
            return self.locate(vref, 0, Some(&full)).map(|l| l.vol);
        } }
        self.volume_of_vref(vref)
    }

    /// SetVol: make the directory `loc` names the default.
    pub fn set_default(&mut self, loc: &Located) -> Result<(), i16> {
        let node = loc.node.ok_or(err::DIR_NF_ERR)?;
        if !self.tree.is_dir(node) { return Err(err::DIR_NF_ERR); }
        self.def_vol = loc.vol;
        self.def_dir = node;
        Ok(())
    }

    /// GetVol: the default volume's name and the vRefNum to give back (the volume's, or a working directory's).
    pub fn default_vref(&mut self) -> (i16, Vec<u8>) {
        let (v, d) = (self.def_vol, self.def_dir);
        (self.wd_for(v, d), super::bytes_of(self.tree.name(v)))
    }

    /// The `index`th open fork (1-based) for GetFCBInfo.
    pub fn nth_open(&self, index: u32) -> Option<i16> { self.open_forks().map(|(&r, _)| r).nth(index as usize - 1) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_volume_directory_and_volume_info() {
        let mut fm = FileMgr::new();
        let d = fm.create_dir(&fm.locate(0, 0, Some(b":Work")).unwrap()).unwrap();
        let f = fm.create_file(&fm.locate(0, 0, Some(b":Work:big")).unwrap()).unwrap();
        fm.tree.file_data_mut(f).unwrap().resize(1025, 0);
        assert_eq!(fm.default_vref().0, -1, "the default is the volume root");
        fm.set_default(&fm.locate(0, 0, Some(b":Work:")).unwrap()).unwrap();
        let (wd, name) = fm.default_vref();
        assert_eq!(name, b"Macintosh HD");
        assert!(wd < -0x3000, "a subdirectory default is a working directory");
        assert_eq!(fm.wd(wd), Some((fm.def_vol, d)));
        assert_eq!(fm.locate(0, 0, Some(b"big")).unwrap().node, Some(f), "names resolve in the default directory");
        let i = fm.vol_info(fm.def_vol);
        assert_eq!((i.vref, i.dir_count, i.file_count, i.files_in_root, i.attrib), (-1, 1, 1, 0, 0));
        assert_eq!(i.free_blocks as u32, TOTAL_BLOCKS - 3, "1025 bytes = 3 allocation blocks");
        // negative controls
        assert_eq!(fm.set_default(&fm.locate(0, 0, Some(b"big")).unwrap()), Err(err::DIR_NF_ERR), "a file cannot be the default directory");
        let cd = fm.add_volume("CD", false);
        assert_eq!(fm.vol_info(cd).attrib, VATRB_LOCKED);
        assert_eq!(fm.pick_volume(2, 0, None), Ok(cd));
        assert_eq!(fm.pick_volume(3, 0, None), Err(err::NSV_ERR));
        assert_eq!(fm.pick_volume(-1, 0, Some(b"CD")), Ok(cd));
        assert_eq!(fm.pick_volume(0, -2, None), Ok(cd));
    }
}
