//! Open forks: Open, Close, Read, Write, EOF and the file mark (Inside Macintosh: Files, "Accessing files").
//! Pure over [`FileMgr`]: bytes in and out, OSErr codes; the trap glue puts them in parameter blocks.
use super::names::Located;
use super::{err, FileMgr, OpenFork, Perm};
use alloc::vec::Vec;
use heirloom_volume::{Kind, NodeId};

/// ioPermssn (Files.h).
pub const FS_CUR_PERM: u8 = 0;
pub const FS_RD_PERM: u8 = 1;
pub const FS_WR_PERM: u8 = 2;
pub const FS_RD_WR_PERM: u8 = 3;
pub const FS_RD_WR_SH_PERM: u8 = 4;
/// ioPosMode low bits.
pub const FS_AT_MARK: u16 = 0;
pub const FS_FROM_START: u16 = 1;
pub const FS_FROM_LEOF: u16 = 2;
pub const FS_FROM_MARK: u16 = 3;
/// ioPosMode bit 7 = newline mode; the newline character is the mode's high byte.
const NEWLINE_BIT: u16 = 0x80;

/// What a Read produced: the bytes, and the result (eofErr when the file ended first).
#[derive(Debug, PartialEq, Eq)]
pub struct ReadOut { pub data: Vec<u8>, pub result: i16 }

impl FileMgr {
    fn fork_bytes(&self, f: &OpenFork) -> &[u8] {
        if f.rsrc { &self.tree.get(f.node).rsrc } else { self.tree.file_data(f.node).unwrap_or(&[]) }
    }
    fn fork_bytes_mut(&mut self, f: &OpenFork) -> &mut Vec<u8> {
        if f.rsrc { &mut self.tree.get_mut(f.node).rsrc } else { self.tree.file_data_mut(f.node).expect("an open fork is a file") }
    }
    fn fork_of(&self, refnum: i16) -> Result<OpenFork, i16> { self.forks.get(&refnum).cloned().ok_or(err::RF_NUM_ERR) }

    /// Open the data (or resource) fork of the file `loc` names. `perm` is ioPermssn.
    pub fn open(&mut self, loc: &Located, rsrc: bool, perm: u8) -> Result<i16, i16> {
        let node = loc.node.ok_or(err::FNF_ERR)?;
        if loc.leaf.is_none() || !matches!(self.tree.get(node).kind, Kind::File(_)) { return Err(err::FNF_ERR); }
        let locked = self.tree.get(node).attrs & 1 != 0;
        let vol_writable = self.tree.is_writable(node);
        let can_write = vol_writable && !locked;
        let want = match perm {
            FS_CUR_PERM => Perm { read: true, write: can_write },
            FS_RD_PERM => Perm { read: true, write: false },
            FS_WR_PERM => Perm { read: false, write: true },
            FS_RD_WR_PERM | FS_RD_WR_SH_PERM => Perm { read: true, write: true },
            _ => return Err(err::PARAM_ERR),
        };
        if want.write {
            if !vol_writable { return Err(err::W_PR_ERR); }
            if locked { return Err(err::PERM_ERR); }
            if perm != FS_RD_WR_SH_PERM && self.forks.values().any(|f| f.node == node && f.rsrc == rsrc && f.perm.write) { return Err(err::OP_WR_ERR); }
        }
        let refnum = self.alloc_ref()?;
        self.forks.insert(refnum, OpenFork { node, rsrc, perm: want, mark: 0 });
        Ok(refnum)
    }

    /// Even, positive reference numbers, as a real FCB index gives them.
    fn alloc_ref(&mut self) -> Result<i16, i16> {
        for _ in 0..0x4000 {
            self.next_ref = if self.next_ref >= 0x3FFE { 2 } else { self.next_ref + 2 };
            let r = self.next_ref.max(2);
            // the running application's own resource fork already holds an FCB under its refnum: never hand that number out
            if r != crate::resmgr::APP_REF_NUM && !self.forks.contains_key(&r) { return Ok(r); }
        }
        Err(err::TMFO_ERR)
    }

    /// The running application's own file: it lives on the default volume under its name (an existing file of that name is
    /// kept as it is) and its resource fork is OPEN, read-only, under `refnum` (the Resource Manager's number for it), so the
    /// program can find itself the way a real one does: `CurApName`, `PBGetFCBInfo(CurApRefNum)`, `OpenRF` of its own name.
    pub fn install_app(&mut self, name: &str, data: &[u8], rsrc: &[u8], file_type: [u8; 4], creator: [u8; 4], refnum: i16) -> Result<(), i16> {
        let dir = self.def_dir;
        let node = match self.tree.find_child(dir, name) {
            Some(n) => n,
            None => {
                let n = self.tree.put_file(dir, name, data.to_vec()).map_err(super::os_err)?;
                let node = self.tree.get_mut(n);
                node.rsrc = rsrc.to_vec();
                node.finfo.file_type = file_type;
                node.finfo.creator = creator;
                n
            }
        };
        self.forks.insert(refnum, OpenFork { node, rsrc: true, perm: Perm { read: true, write: false }, mark: 0 });
        Ok(())
    }

    pub fn close(&mut self, refnum: i16) -> Result<(), i16> { self.forks.remove(&refnum).map(|_| ()).ok_or(err::RF_NUM_ERR) }
    pub fn eof(&self, refnum: i16) -> Result<u32, i16> { let f = self.fork_of(refnum)?; Ok(self.fork_bytes(&f).len() as u32) }
    pub fn mark(&self, refnum: i16) -> Result<u32, i16> { self.fork_of(refnum).map(|f| f.mark) }
    pub fn fork_node(&self, refnum: i16) -> Result<(NodeId, bool), i16> { self.fork_of(refnum).map(|f| (f.node, f.rsrc)) }

    /// Where a transfer starts: (ioPosMode, ioPosOffset) against the mark and the logical EOF.
    fn start_pos(f: &OpenFork, len: u32, mode: u16, off: i32) -> Result<u32, i16> {
        let base = match mode & 3 { FS_AT_MARK => i64::from(f.mark), FS_FROM_START => 0, FS_FROM_LEOF => i64::from(len), _ => i64::from(f.mark) } ;
        let off = if mode & 3 == FS_AT_MARK { 0 } else { i64::from(off) };
        let p = base + off;
        if p < 0 || p > i64::from(u32::MAX) { Err(err::POS_ERR) } else { Ok(p as u32) }
    }

    /// Read up to `count` bytes. Short because the file ended: `result` is eofErr and the mark is at the end.
    pub fn read(&mut self, refnum: i16, mode: u16, off: i32, count: u32) -> Result<ReadOut, i16> {
        let f = self.fork_of(refnum)?;
        if !f.perm.read { return Err(err::PERM_ERR); }
        let len = self.fork_bytes(&f).len() as u32;
        let start = Self::start_pos(&f, len, mode, off)?;
        let newline = (mode & NEWLINE_BIT != 0).then(|| (mode >> 8) as u8);
        let data = self.fork_bytes(&f);
        let avail = len.saturating_sub(start).min(count) as usize;
        let mut got = &data[(start.min(len)) as usize..start.min(len) as usize + avail];
        if let Some(nl) = newline { if let Some(i) = got.iter().position(|&b| b == nl) { got = &got[..=i]; } }
        let out = got.to_vec();
        let end = start.min(len) + out.len() as u32;
        let hit_nl = newline.is_some_and(|nl| out.last() == Some(&nl));
        self.forks.get_mut(&refnum).expect("checked").mark = end;
        let result = if (out.len() as u32) < count && !hit_nl { err::EOF_ERR } else { err::NO_ERR };
        Ok(ReadOut { data: out, result })
    }

    /// Write `data` at the position; the file grows (zero-filled if the position is past the end).
    pub fn write(&mut self, refnum: i16, mode: u16, off: i32, data: &[u8]) -> Result<u32, i16> {
        let f = self.fork_of(refnum)?;
        if !f.perm.write { return Err(err::WR_PERM_ERR); }
        let len = self.fork_bytes(&f).len() as u32;
        let start = Self::start_pos(&f, len, mode, off)?;
        let end = start.checked_add(data.len() as u32).ok_or(err::DSK_FUL_ERR)?;
        let buf = self.fork_bytes_mut(&f);
        if buf.len() < end as usize { buf.resize(end as usize, 0); }
        buf[start as usize..end as usize].copy_from_slice(data);
        self.touch(f.node);
        self.forks.get_mut(&refnum).expect("checked").mark = end;
        Ok(data.len() as u32)
    }

    /// SetEOF: grow (zero-filled) or truncate; a mark past the new end moves to it.
    pub fn set_eof(&mut self, refnum: i16, new_len: u32) -> Result<(), i16> {
        let f = self.fork_of(refnum)?;
        if !f.perm.write { return Err(err::WR_PERM_ERR); }
        self.fork_bytes_mut(&f).resize(new_len as usize, 0);
        self.touch(f.node);
        let m = self.forks.get_mut(&refnum).expect("checked");
        m.mark = m.mark.min(new_len);
        Ok(())
    }

    /// SetFPos: a mark past the EOF is put AT the EOF and reported as eofErr.
    pub fn set_mark(&mut self, refnum: i16, mode: u16, off: i32) -> Result<(), i16> {
        let f = self.fork_of(refnum)?;
        let len = self.fork_bytes(&f).len() as u32;
        let p = Self::start_pos(&f, len, mode, off)?;
        let m = self.forks.get_mut(&refnum).expect("checked");
        if p > len { m.mark = len; return Err(err::EOF_ERR); }
        m.mark = p;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fm::names::Located;

    fn file(fm: &mut FileMgr, name: &str, data: &[u8]) -> Located {
        let root = fm.def_vol;
        let f = fm.tree.put_file(root, name, data.to_vec()).unwrap();
        Located { vol: root, parent: root, leaf: Some(name.into()), node: Some(f) }
    }

    #[test]
    fn read_write_eof_and_mark_follow_inside_macintosh() {
        let mut fm = FileMgr::new();
        let loc = file(&mut fm, "f", b"hello world");
        let r = fm.open(&loc, false, FS_RD_WR_PERM).unwrap();
        assert_eq!(r % 2, 0, "refnums are even");
        assert_eq!(fm.read(r, FS_AT_MARK, 0, 5).unwrap(), ReadOut { data: b"hello".to_vec(), result: 0 });
        assert_eq!(fm.mark(r), Ok(5));
        assert_eq!(fm.read(r, FS_FROM_START, 6, 100).unwrap(), ReadOut { data: b"world".to_vec(), result: err::EOF_ERR }, "short read = eofErr + the bytes");
        assert_eq!(fm.mark(r), Ok(11));
        assert_eq!(fm.write(r, FS_FROM_LEOF, 0, b"!!"), Ok(2));
        assert_eq!(fm.eof(r), Ok(13));
        assert_eq!(fm.write(r, FS_FROM_START, 15, b"z"), Ok(1));
        assert_eq!(fm.eof(r), Ok(16));
        assert_eq!(fm.read(r, FS_FROM_START, 13, 3).unwrap().data, [0, 0, b'z'], "a gap is zero-filled");
        fm.set_eof(r, 4).unwrap();
        assert_eq!((fm.eof(r), fm.mark(r)), (Ok(4), Ok(4)), "truncate moves the mark");
        assert_eq!(fm.set_mark(r, FS_FROM_START, 10), Err(err::EOF_ERR));
        assert_eq!(fm.mark(r), Ok(4), "past the end puts the mark AT the end");
        assert_eq!(fm.set_mark(r, FS_FROM_START, -1), Err(err::POS_ERR));
        // negative controls
        assert_eq!(fm.read(r, FS_AT_MARK, 0, 1).unwrap(), ReadOut { data: vec![], result: err::EOF_ERR });
        fm.close(r).unwrap();
        assert_eq!(fm.close(r), Err(err::RF_NUM_ERR));
        assert_eq!(fm.read(r, 0, 0, 1), Err(err::RF_NUM_ERR));
    }

    #[test]
    fn permissions_and_exclusive_write_follow_the_rules() {
        let mut fm = FileMgr::new();
        let loc = file(&mut fm, "p", b"data");
        let rd = fm.open(&loc, false, FS_RD_PERM).unwrap();
        assert_eq!(fm.write(rd, 0, 0, b"x"), Err(err::WR_PERM_ERR));
        let wr = fm.open(&loc, false, FS_WR_PERM).unwrap();
        assert_eq!(fm.read(wr, 0, 0, 1), Err(err::PERM_ERR), "write-only");
        assert_eq!(fm.open(&loc, false, FS_RD_WR_PERM), Err(err::OP_WR_ERR), "a second writer is refused");
        assert!(fm.open(&loc, false, FS_RD_PERM).is_ok(), "negative control: another reader is fine");
        assert!(fm.open(&loc, true, FS_WR_PERM).is_ok(), "the resource fork is a separate fork");
        fm.tree.get_mut(loc.node.unwrap()).attrs = 1;
        assert_eq!(fm.open(&loc, false, FS_WR_PERM), Err(err::PERM_ERR), "a locked file");
        assert_eq!(fm.open(&loc, false, FS_CUR_PERM).map(|r| fm.open_fork(r).unwrap().perm.write), Ok(false), "fsCurPerm falls back to read-only");
        let missing = Located { vol: fm.def_vol, parent: fm.def_vol, leaf: Some("m".into()), node: None };
        assert_eq!(fm.open(&missing, false, 1), Err(err::FNF_ERR));
        assert_eq!(fm.open(&loc, false, 9), Err(err::PARAM_ERR));
    }

    #[test]
    fn newline_mode_stops_after_the_newline_character() {
        let mut fm = FileMgr::new();
        let loc = file(&mut fm, "n", b"line one\rline two");
        let r = fm.open(&loc, false, FS_RD_PERM).unwrap();
        let o = fm.read(r, NEWLINE_BIT | 0x0D00, 0, 100).unwrap();
        assert_eq!(o, ReadOut { data: b"line one\r".to_vec(), result: 0 }, "stops after CR, no error");
        let o = fm.read(r, 0, 0, 100).unwrap();
        assert_eq!(o.data, b"line two", "negative control: plain mode reads on");
    }
}
