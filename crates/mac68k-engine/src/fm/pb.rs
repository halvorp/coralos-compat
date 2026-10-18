//! Parameter-block field offsets, from Inside Macintosh: Files ("Parameter blocks"; the same layouts as Files.h) and
//! the `FSSpec`. All blocks share a 24-byte header; the rest depends on the call. Offsets are checked against the sizes
//! the structures document (IOParam ends at 50, FileParam at 80, HFileInfo/DirInfo at 108, VolumeParam at 66).
use retro_guest_mem::{GuestMem, MemError};

// the common header (ParamBlockRec)
pub const IO_COMPLETION: u32 = 12;
pub const IO_RESULT: u32 = 16;
pub const IO_NAME_PTR: u32 = 18;
pub const IO_VREF_NUM: u32 = 22;
// IOParam (Open, Close, Read, Write, GetEOF ...)
pub const IO_REF_NUM: u32 = 24;
pub const IO_PERMSSN: u32 = 27;
pub const IO_MISC: u32 = 28;
pub const IO_BUFFER: u32 = 32;
pub const IO_REQ_COUNT: u32 = 36;
pub const IO_ACT_COUNT: u32 = 40;
pub const IO_POS_MODE: u32 = 44;
pub const IO_POS_OFFSET: u32 = 46;
// FileParam / HFileInfo / DirInfo (GetFileInfo, GetCatInfo ...)
pub const IO_FDIR_INDEX: u32 = 28;
pub const IO_FL_ATTRIB: u32 = 30;
pub const IO_AC_USER: u32 = 31;
pub const IO_FL_FNDR_INFO: u32 = 32; // FInfo / DInfo, 16 bytes
pub const IO_DIR_ID: u32 = 48; // HFileInfo.ioDirID, DirInfo.ioDrDirID, every HFS call's directory; a file's ioFlNum
pub const IO_FL_ST_BLK: u32 = 52;
pub const IO_DR_NM_FLS: u32 = 52; // DirInfo: items in the directory
pub const IO_FL_LG_LEN: u32 = 54;
pub const IO_FL_PY_LEN: u32 = 58;
pub const IO_FL_RST_BLK: u32 = 62;
pub const IO_FL_RLG_LEN: u32 = 64;
pub const IO_FL_RPY_LEN: u32 = 68;
pub const IO_FL_CR_DAT: u32 = 72;
pub const IO_FL_MD_DAT: u32 = 76;
pub const IO_FL_BK_DAT: u32 = 80;
pub const IO_FL_PAR_ID: u32 = 100;
pub const IO_FL_CLP_SIZ: u32 = 104;
// VolumeParam / HVolumeParam (GetVolInfo)
pub const IO_VOL_INDEX: u32 = 28;
pub const IO_VCR_DATE: u32 = 30;
pub const IO_VLS_BK_UP: u32 = 34;
pub const IO_VATRB: u32 = 38;
pub const IO_VNM_FLS: u32 = 40;
pub const IO_VDIR_ST: u32 = 42;
pub const IO_VBL_LN: u32 = 44;
pub const IO_VNM_AL_BLKS: u32 = 46;
pub const IO_VAL_BLK_SIZ: u32 = 48;
pub const IO_VCLP_SIZ: u32 = 52;
pub const IO_AL_BL_ST: u32 = 56;
pub const IO_VNXT_FNUM: u32 = 58;
pub const IO_VFR_BLK: u32 = 62;
pub const IO_VSIG_WORD: u32 = 64;
pub const IO_VFIL_CNT: u32 = 82;
pub const IO_VDIR_CNT: u32 = 86;
// WDPBRec (OpenWD, CloseWD, GetWDInfo)
pub const IO_WD_INDEX: u32 = 26;
pub const IO_WD_PROC_ID: u32 = 28;
pub const IO_WD_VREF_NUM: u32 = 32;
pub const IO_WD_DIR_ID: u32 = 48;
// FCBPBRec (GetFCBInfo)
pub const IO_FCB_INDX: u32 = 28;
pub const IO_FCB_FL_NM: u32 = 34;
pub const IO_FCB_FLAGS: u32 = 38;
pub const IO_FCB_ST_BLK: u32 = 40;
pub const IO_FCB_EOF: u32 = 42;
pub const IO_FCB_PLEN: u32 = 46;
pub const IO_FCB_CR_PS: u32 = 50;
pub const IO_FCB_VREF_NUM: u32 = 54;
pub const IO_FCB_CLP_SIZ: u32 = 56;
pub const IO_FCB_PAR_ID: u32 = 60;
// CMovePBRec (CatMove)
pub const IO_NEW_NAME: u32 = 28;
pub const IO_NEW_DIR_ID: u32 = 36;
// FSSpec
pub const FSS_VREF: u32 = 0;
pub const FSS_PAR_ID: u32 = 2;
pub const FSS_NAME: u32 = 6;
pub const FSSPEC_SIZE: u32 = 70;

/// ioFlAttrib bits.
pub const ATTR_LOCKED: u8 = 0x01;
pub const ATTR_RSRC_OPEN: u8 = 0x04;
pub const ATTR_DIR: u8 = 0x10;
pub const ATTR_OPEN: u8 = 0x80;

/// A parameter block in guest memory.
#[derive(Clone, Copy)]
pub struct Pb(pub u32);

impl Pb {
    pub fn u8(self, mem: &GuestMem, off: u32) -> Result<u8, MemError> { mem.read_u8(self.0.wrapping_add(off)) }
    pub fn i16(self, mem: &GuestMem, off: u32) -> Result<i16, MemError> { mem.read_i16(self.0.wrapping_add(off)) }
    pub fn u32(self, mem: &GuestMem, off: u32) -> Result<u32, MemError> { mem.read_u32(self.0.wrapping_add(off)) }
    pub fn put_u8(self, mem: &mut GuestMem, off: u32, v: u8) -> Result<(), MemError> { mem.write_u8(self.0.wrapping_add(off), v) }
    pub fn put_i16(self, mem: &mut GuestMem, off: u32, v: i16) -> Result<(), MemError> { mem.write_u16(self.0.wrapping_add(off), v as u16) }
    pub fn put_u32(self, mem: &mut GuestMem, off: u32, v: u32) -> Result<(), MemError> { mem.write_u32(self.0.wrapping_add(off), v) }
    pub fn put_bytes(self, mem: &mut GuestMem, off: u32, b: &[u8]) -> Result<(), MemError> { mem.write_bytes(self.0.wrapping_add(off), b) }
    pub fn bytes(self, mem: &GuestMem, off: u32, n: u32) -> Result<alloc::vec::Vec<u8>, MemError> {
        let s = mem.span(self.0.wrapping_add(off), n)?;
        Ok(mem.bytes(s)?.to_vec())
    }
}
