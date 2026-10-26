//! The File Manager as a PowerPC program calls it: the C-callable routines of `Files.h` (`FSOpen`, `FSRead`, `HOpen`,
//! `GetVol`, `Create` ...), which the Universal Interfaces implement as glue that fills a parameter block and traps.
//!
//! The 68k engine already implements the TRAPS (`heirloom_mac68k_engine::fm::traps`: `_Open`, `_Read`, `_GetFPos` ... on a parameter
//! block in guest memory, the HFS forms selected by the trap word's bit 9). This module is only the glue: it builds the block
//! in a scratch pointer, runs the SAME handler through [`Mac::os_call`], and copies the outputs back to the caller's pointers,
//! so a PowerPC program and a 68k program see one volume, one set of refnums and one set of error codes.
//! Field offsets are `heirloom_mac68k_engine::fm::pb` (Inside Macintosh: Files, "Parameter blocks"); argument orders are `Files.h`.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::{String, ToString};
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::fm::pb::*;
use heirloom_mac68k_engine::mixed::{trap_opcode, OsSig, Reg};

/// The scratch parameter block is the largest File Manager block (HFileInfo / DirInfo, 108 bytes) rounded up.
const PB_SIZE: u32 = 128;
/// The HFS bit of a File Manager trap word (`_HOpen` = `_Open` | HFS): selects the hierarchical parameter-block form.
const HFS: u16 = 0x0200;
/// `fsAtMark`: ioPosMode for FSRead/FSWrite (Files.h: fsAtMark 0, fsFromStart 1, fsFromLEOF 2, fsFromMark 3).
const FS_AT_MARK: i16 = 0;
const RF_NUM_ERR: i16 = -51;
static PB_SIG: OsSig = OsSig { name: "ParamBlock", args: &[Reg::A0], result: Some(Reg::D0) };

/// A Files.h routine this engine serves, named by its InterfaceLib export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCall {
    FSOpen, OpenRF, FSClose, FSRead, FSWrite, GetEOF, SetEOF, GetFPos, SetFPos, FlushVol, GetVol, SetVol, Create, FSDelete,
    HOpen, HOpenDF, HOpenRF, HCreate, HDelete, GetVRefNum,
}

impl FileCall {
    pub fn by_name(name: &str) -> Option<FileCall> {
        use FileCall::*;
        Some(match name {
            "FSOpen" => FSOpen, "OpenRF" => OpenRF, "FSClose" => FSClose, "FSRead" => FSRead, "FSWrite" => FSWrite, "GetEOF" => GetEOF,
            "SetEOF" => SetEOF, "GetFPos" => GetFPos, "SetFPos" => SetFPos, "FlushVol" => FlushVol, "GetVol" => GetVol, "SetVol" => SetVol,
            "Create" => Create, "FSDelete" => FSDelete, "HOpen" => HOpen, "HOpenDF" => HOpenDF, "HOpenRF" => HOpenRF, "HCreate" => HCreate,
            "HDelete" => HDelete, "GetVRefNum" => GetVRefNum,
            _ => return None,
        })
    }
}

fn pb_failed(call: &str, what: &str) -> String { format!("{call}: parameter block or caller pointer outside guest RAM ({what})") }

impl<U: GuestUi> MacPpc<U> {
    /// The scratch block, allocated once and zeroed per call.
    fn scratch_pb(&mut self) -> Result<u32, String> {
        let p = match self.rt.fm_pb {
            Some(p) => p,
            None => {
                let p = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, PB_SIZE, true).map_err(|e| format!("no memory for the File Manager parameter block ({e})"))?;
                self.rt.fm_pb = Some(p);
                p
            }
        };
        self.mac.mem.zero(p, PB_SIZE).map_err(|_| "the File Manager parameter block left guest RAM".to_string())?;
        Ok(p)
    }

    /// Run the trap named `trap` (HFS form when `hfs`) on the block at `pb`; the OSErr it returned.
    fn run_pb(&mut self, trap: &str, hfs: bool, pb: u32) -> Result<i16, String> {
        let op = trap_opcode(trap).ok_or_else(|| format!("{trap} has no trap word"))? | if hfs { HFS } else { 0 };
        let r = self.mac.os_call(op, &PB_SIG, &[pb]).map_err(|e| format!("{e}"))?;
        if r.blocked { return Err(format!("{trap}: the handler tried to block the caller")); }
        Ok(r.value as u16 as i16)
    }

    /// One File Manager routine: `a` = the PowerPC arguments (r3..), the result is the OSErr to put in r3.
    pub(crate) fn file_call(&mut self, c: FileCall, a: &[u32]) -> Result<i16, String> {
        use FileCall::*;
        let name = alloc::format!("{c:?}");
        let pb = self.scratch_pb()?;
        let g = |what: &str| pb_failed(&name, what);
        let p = Pb(pb);
        let (n_in, hfs, trap) = match c {
            FSOpen => (3, false, "Open"), OpenRF => (3, false, "OpenRF"), HOpen | HOpenDF => (5, true, "Open"), HOpenRF => (5, true, "OpenRF"),
            FSClose => (1, false, "Close"), FSRead => (3, false, "Read"), FSWrite => (3, false, "Write"),
            GetEOF => (2, false, "GetEOF"), SetEOF => (2, false, "SetEOF"), GetFPos => (2, false, "GetFPos"), SetFPos => (3, false, "SetFPos"),
            FlushVol => (2, false, "FlushVol"), GetVol => (2, false, "GetVol"), SetVol => (2, false, "SetVol"),
            Create => (4, false, "Create"), HCreate => (5, true, "Create"), FSDelete => (2, false, "Delete"), HDelete => (3, true, "Delete"),
            GetVRefNum => (2, false, ""),
        };
        if a.len() < n_in { return Err(format!("{name}: {n_in} arguments needed, {} given", a.len())); }
        let m = &mut self.mac;
        // inputs
        let put = (|| -> Result<(), heirloom_mem_flat32::MemError> {
            match c {
                FSOpen | OpenRF => { p.put_u32(&mut m.mem, IO_NAME_PTR, a[0])?; p.put_i16(&mut m.mem, IO_VREF_NUM, a[1] as i16)?; }
                HOpen | HOpenDF | HOpenRF => {
                    p.put_i16(&mut m.mem, IO_VREF_NUM, a[0] as i16)?; p.put_u32(&mut m.mem, IO_DIR_ID, a[1])?;
                    p.put_u32(&mut m.mem, IO_NAME_PTR, a[2])?; p.put_u8(&mut m.mem, IO_PERMSSN, a[3] as u8)?;
                }
                FSClose | GetEOF | GetFPos => p.put_i16(&mut m.mem, IO_REF_NUM, a[0] as i16)?,
                SetEOF => { p.put_i16(&mut m.mem, IO_REF_NUM, a[0] as i16)?; p.put_u32(&mut m.mem, IO_MISC, a[1])?; }
                FSRead | FSWrite => {
                    p.put_i16(&mut m.mem, IO_REF_NUM, a[0] as i16)?; let want = m.mem.read_u32(a[1])?; p.put_u32(&mut m.mem, IO_REQ_COUNT, want)?;
                    p.put_u32(&mut m.mem, IO_BUFFER, a[2])?; p.put_i16(&mut m.mem, IO_POS_MODE, FS_AT_MARK)?;
                }
                SetFPos => { p.put_i16(&mut m.mem, IO_REF_NUM, a[0] as i16)?; p.put_i16(&mut m.mem, IO_POS_MODE, a[1] as i16)?; p.put_u32(&mut m.mem, IO_POS_OFFSET, a[2])?; }
                FlushVol | SetVol | Create | FSDelete => { p.put_u32(&mut m.mem, IO_NAME_PTR, a[0])?; p.put_i16(&mut m.mem, IO_VREF_NUM, a[1] as i16)?; }
                GetVol => p.put_u32(&mut m.mem, IO_NAME_PTR, a[0])?,
                HCreate | HDelete => { p.put_i16(&mut m.mem, IO_VREF_NUM, a[0] as i16)?; p.put_u32(&mut m.mem, IO_DIR_ID, a[1])?; p.put_u32(&mut m.mem, IO_NAME_PTR, a[2])?; }
                GetVRefNum => {}
            }
            Ok(())
        })();
        put.map_err(|_| g("input"))?;
        if c == GetVRefNum {
            // Files.h: GetVRefNum(fileRefNum, *vRefNum) is a lookup in the file control block: the volume the open fork lives on.
            let refnum = a[0] as i16;
            let Ok((node, _)) = self.mac.fm.fork_node(refnum) else { return Ok(RF_NUM_ERR) };
            let vref = self.mac.fm.vref_of(self.mac.fm.tree.volume_of(node));
            self.mac.mem.write_u16(a[1], vref as u16).map_err(|_| g("vRefNum"))?;
            return Ok(0);
        }
        let mut res = self.run_pb(trap, hfs, pb)?;
        // Create / HCreate set the Finder type and creator afterwards (Files.h: the glue is PBCreate then PBSetFInfo).
        if matches!(c, Create | HCreate) && res == 0 {
            let (creator, ftype) = if c == Create { (a[2], a[3]) } else { (a[3], a[4]) };
            let (np, vref, dir) = (p.u32(&self.mac.mem, IO_NAME_PTR).map_err(|_| g("name"))?, p.i16(&self.mac.mem, IO_VREF_NUM).map_err(|_| g("vref"))?, p.u32(&self.mac.mem, IO_DIR_ID).map_err(|_| g("dir"))?);
            self.mac.mem.zero(pb, PB_SIZE).map_err(|_| g("clear"))?;
            let ok = p.put_u32(&mut self.mac.mem, IO_NAME_PTR, np).and_then(|_| p.put_i16(&mut self.mac.mem, IO_VREF_NUM, vref)).and_then(|_| p.put_u32(&mut self.mac.mem, IO_DIR_ID, dir));
            ok.map_err(|_| g("info"))?;
            res = self.run_pb("GetFileInfo", hfs, pb)?;
            if res == 0 {
                // an HFS GetFInfo answers the file number at offset 48, where the caller's ioDirID was
                p.put_u32(&mut self.mac.mem, IO_DIR_ID, dir).map_err(|_| g("dirID"))?;
                p.put_u32(&mut self.mac.mem, IO_FL_FNDR_INFO, ftype).and_then(|_| p.put_u32(&mut self.mac.mem, IO_FL_FNDR_INFO + 4, creator)).map_err(|_| g("FInfo"))?;
                res = self.run_pb("SetFileInfo", hfs, pb)?;
            }
        }
        // outputs: the counts of a short read or write are valid with eofErr, so they are copied whatever the result
        let out = (|| -> Result<(), heirloom_mem_flat32::MemError> {
            let m = &mut self.mac;
            match c {
                FSRead | FSWrite => { let n = p.u32(&m.mem, IO_ACT_COUNT)?; m.mem.write_u32(a[1], n)?; }
                FSOpen | OpenRF | HOpen | HOpenDF | HOpenRF if res == 0 => { let r = p.i16(&m.mem, IO_REF_NUM)?; m.mem.write_u16(a[if hfs { 4 } else { 2 }], r as u16)?; }
                GetEOF if res == 0 => { let v = p.u32(&m.mem, IO_MISC)?; m.mem.write_u32(a[1], v)?; }
                GetFPos if res == 0 => { let v = p.u32(&m.mem, IO_POS_OFFSET)?; m.mem.write_u32(a[1], v)?; }
                GetVol if res == 0 => { let v = p.i16(&m.mem, IO_VREF_NUM)?; m.mem.write_u16(a[1], v as u16)?; }
                _ => {}
            }
            Ok(())
        })();
        out.map_err(|_| g("output"))?;
        Ok(res)
    }
}
