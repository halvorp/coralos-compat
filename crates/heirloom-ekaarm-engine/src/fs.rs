//! The file server session (`RFs`, f32file.h) natives. One `RFs` is the environment's own (`CCoeEnv::iFsSession`, a connected
//! session the guest uses through `FsSession()`); the file system behind it is the drive (`drive.rs`).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::FsSessionState;

/// `CCoeEnv::iFsSession` (coemain.h:204): the RFs object inside the environment, after `iAppUi` (0x24); the app passes `env + 0x28` as the `RFs` of its `FsSession()` calls (observed: CreatePrivatePath's r0).
pub const COEENV_I_FS_SESSION: u32 = 0x28;

const KERR_NONE: i32 = 0; // e32err.h
const KERR_ALREADY_EXISTS: i32 = -11; // e32err.h
const KERR_NOT_READY: i32 = -18; // e32err.h
const KERR_ARGUMENT: i32 = -6; // e32err.h
const ATT_LOCAL: u32 = 0x01; // e32const.h:1021 KDriveAttLocal
const ATT_ROM: u32 = 0x02; // :1032 KDriveAttRom
const ATT_INTERNAL: u32 = 0x10; // :1066 KDriveAttInternal
const MEDIA_ATT_WRITE_PROTECTED: u32 = 0x08; // :1218 KMediaAttWriteProtected
const MEDIA_FLASH: u32 = 6; // e32const.h:989 TMediaType EMediaFlash
const MEDIA_ROM: u32 = 7; // EMediaRom
/// `TVolumeInfo` (f32file.h:1454): `iDrive` (TDriveInfo: iType, iBattery, iDriveAtt, iMediaAtt, iReserved: 20 bytes), `iUniqueID` 20, `iSize` 24,
/// `iFree` 32, `iName` (a `TBufC<KMaxFileName>`, 4 + 512 bytes) 40.
const VOL_I_SIZE: u32 = 24;
const VOL_I_FREE: u32 = 32;
const VOL_NAME_END: u32 = 40 + 4 + 2 * 0x100;

/// The attributes of a drive the confined drive answers to (`Drive::LETTERS`): C: is local and internal, Z: is ROM.
fn drive_att(letter: char) -> u32 { if letter == 'Z' { ATT_ROM | ATT_INTERNAL } else { ATT_LOCAL | ATT_INTERNAL } }

impl Process {
    /// Open a new file server session and return its handle (`RHandleBase::iHandle`, e32std.h:4100, word 0): non-zero, never reused.
    pub(crate) fn fs_connect(&mut self) -> u32 {
        let objs = &mut self.nat.objs;
        objs.next_fs_handle += 1;
        objs.fs_sessions.insert(objs.next_fs_handle, FsSessionState::default());
        objs.next_fs_handle
    }

    /// The state of the session the `RFs` at `addr` holds, if it is connected.
    pub(crate) fn fs_session(&mut self, addr: u32) -> Option<&mut FsSessionState> {
        let handle = self.mem.vm.read_u32(addr).ok()?;
        self.nat.objs.fs_sessions.get_mut(&handle)
    }
}

/// Offset of `iNameBuf` (`TBuf<256>`, 520 bytes) inside `TParse` (544 = 0x220 bytes total in `f32file.h`).
const TPARSE_NAMEBUF_OFF: u32 = 0x18;
const TPARSE_MAX_LEN: u32 = 256;

fn tparse_slice(units: &[u16], part: &str) -> (usize, usize) {
    let n = units.len();
    let drive_end = if n >= 2 && units[1] == b':' as u16 { 2 } else { 0 };
    let last_slash = units[drive_end..].iter().rposition(|&c| c == b'\\' as u16).map(|i| drive_end + i + 1);
    let path_end = last_slash.unwrap_or(drive_end);
    let dot = units[path_end..].iter().rposition(|&c| c == b'.' as u16).map(|i| path_end + i);
    let name_end = dot.unwrap_or(n);
    match part {
        "drive" => (0, drive_end),
        "path" => (drive_end, path_end),
        "drive_and_path" => (0, path_end),
        "name" => (path_end, name_end),
        "ext" => (name_end, n),
        "name_and_ext" => (path_end, n),
        _ => (0, n),
    }
}

fn tparse_write_ptrc(p: &mut Process, sret: u32, parse_this: u32, part: &str) -> Result<(), String> {
    let buf_addr = parse_this.wrapping_add(TPARSE_NAMEBUF_OFF);
    let units = crate::desc::read16(p, buf_addr)?;
    let (start, end) = tparse_slice(&units, part);
    let len = (end - start) as u32;
    let ptr = buf_addr.wrapping_add(8).wrapping_add(start as u32 * 2);
    let w0 = (crate::desc::DES_PTR_C << crate::desc::KSHIFT_DES_TYPE) | len;
    p.mem.vm.write_u32(sret, w0).map_err(|_| format!("TPtrC16 at {sret:#x} is unwritable"))?;
    p.mem.vm.write_u32(sret.wrapping_add(4), ptr).map_err(|_| format!("TPtrC16 at {sret:#x}+4 is unwritable"))?;
    Ok(())
}

fn tparse_combine(name: &[u16], related: Option<&[u16]>, default: Option<&[u16]>) -> alloc::vec::Vec<u16> {
    let pick = |part: &str| -> alloc::vec::Vec<u16> {
        for src in [Some(name), related, default].into_iter().flatten() {
            let (s, e) = tparse_slice(src, part);
            if e > s { return src[s..e].to_vec(); }
        }
        alloc::vec::Vec::new()
    };
    let mut out = pick("drive");
    out.extend(pick("path"));
    out.extend(pick("name"));
    out.extend(pick("ext"));
    out
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1, r2, r3) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED efsrv {name}: {m}"));
    Some(match name {
        // TParse::TParse(): initialise the embedded TBuf<256> at +TPARSE_NAMEBUF_OFF.
        "_ZN6TParseC1Ev" | "_ZN6TParseC2Ev" => {
            let buf = this.wrapping_add(TPARSE_NAMEBUF_OFF);
            let ok = p.mem.vm.write_u32(buf, crate::desc::DES_BUF << crate::desc::KSHIFT_DES_TYPE).is_ok()
                && p.mem.vm.write_u32(buf.wrapping_add(4), TPARSE_MAX_LEN).is_ok();
            if ok { NativeOutcome::Return } else { refuse(format!("TParse at {this:#x} is unwritable")) }
        }
        // TInt TParse::Set(const TDesC16& aName, const TDesC16* aRelated, const TDesC16* aDefault) & SetNoWild
        "_ZN6TParse3SetERK7TDesC16PS1_S3_" | "_ZN6TParse9SetNoWildERK7TDesC16PS1_S3_" => {
            let r = (|| -> Result<(), String> {
                let n = crate::desc::read16(p, r1)?;
                let rel = if r2 != 0 { Some(crate::desc::read16(p, r2)?) } else { None };
                let def = if r3 != 0 { Some(crate::desc::read16(p, r3)?) } else { None };
                let combined = tparse_combine(&n, rel.as_deref(), def.as_deref());
                crate::desc::write16(p, this.wrapping_add(TPARSE_NAMEBUF_OFF), &combined)
            })();
            match r {
                Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // const TDesC16& TParseBase::FullName() const: returns pointer to embedded TBuf<256>.
        "_ZNK10TParseBase8FullNameEv" => {
            p.cpu.regs[0] = this.wrapping_add(TPARSE_NAMEBUF_OFF);
            NativeOutcome::Return
        }
        // TPtrC16 TParseBase::{Drive,Path,DriveAndPath,Name,Ext,NameAndExt}() const: ARM EABI sret in r0, this in r1.
        "_ZNK10TParseBase5DriveEv" => match tparse_write_ptrc(p, this, r1, "drive") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase4PathEv" => match tparse_write_ptrc(p, this, r1, "path") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase12DriveAndPathEv" => match tparse_write_ptrc(p, this, r1, "drive_and_path") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase4NameEv" => match tparse_write_ptrc(p, this, r1, "name") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase3ExtEv" => match tparse_write_ptrc(p, this, r1, "ext") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase10NameAndExtEv" => match tparse_write_ptrc(p, this, r1, "name_and_ext") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        // TInt RFs::CreatePrivatePath(TDriveNumber aDrive) (f32file.h): creates `\private\<secure id>\` on the drive. The secure id of an
        // application is its UID3. KErrAlreadyExists when it is there. The directory is made in the in-memory drive only.
        "_ZN3RFs17CreatePrivatePathEi" => {
            if p.fs_session(this).is_none() {
                return Some(refuse(format!("the session at {this:#x} is not connected")));
            }
            let path = format!("\\private\\{:08x}", p.uid3);
            let Some(drive) = p.host.drive.as_mut() else { return Some(refuse(String::from("no drive"))) };
            match drive.make_dirs(&path) {
                Ok(created) => { p.cpu.regs[0] = (if created { KERR_NONE } else { KERR_ALREADY_EXISTS }) as u32; NativeOutcome::Return }
                Err(e) => refuse(format!("{path} on drive {r1}: {e:?}")),
            }
        }
        // TInt RFs::SetSessionToPrivate(TInt aDrive) (f32file.h): the session path becomes the process's private directory on the drive.
        "_ZN3RFs19SetSessionToPrivateEi" => {
            let path = format!("{}:\\private\\{:08x}\\", (b'A' + r1 as u8) as char, p.uid3);
            match p.fs_session(this) {
                Some(s) => { s.session_path = path; p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return }
                None => refuse(format!("the session at {this:#x} is not connected")),
            }
        }
        // TInt RFs::DriveList(TDriveList& aList [, TUint aFlags]) const (f32file.h:1873): a TBuf8<26>, one byte per drive A..Z: KDriveAbsent (0) or
        // the drive's attributes (e32const.h). The drives are the ones the confined drive answers to (`Drive::LETTERS`): C: is local and
        // internal (KDriveAttLocal | KDriveAttInternal), Z: is ROM (KDriveAttRom | KDriveAttInternal). The flags only widen what is listed
        // (KDriveAttAll); every drive here is listed either way.
        "_ZNK3RFs9DriveListER5TBuf8ILi26EE" | "_ZNK3RFs9DriveListER5TBuf8ILi26EEj" => {
            if p.fs_session(this).is_none() { return Some(refuse(format!("the session at {this:#x} is not connected"))); }
            let mut list = [0u8; 26];
            for l in crate::drive::Drive::LETTERS { list[(l as u8 - b'A') as usize] = drive_att(l) as u8; }
            match crate::desc::write8(p, r1, &list) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        // static TInt RFs::CharToDrive(TChar aChar, TInt& aDrive): A-Z or a-z is 0-25; any other character is KErrArgument.
        "_ZN3RFs11CharToDriveE5TCharRi" => match u8::try_from(this).ok().filter(u8::is_ascii_alphabetic) {
            Some(c) => match p.wr(r1, u32::from(c.to_ascii_uppercase() - b'A')) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) },
            None => { p.cpu.regs[0] = KERR_ARGUMENT as u32; NativeOutcome::Return }
        },
        // TVolumeInfo::TVolumeInfo(): zeroed (the name is an empty TBufC).
        "_ZN11TVolumeInfoC1Ev" => match (0..VOL_NAME_END / 4).try_for_each(|i| p.wr(this + i * 4, 0)) { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        // TInt RFs::Volume(TVolumeInfo& aVol, TInt aDrive = KDefaultDrive) const (f32file.h:1876): the volume of a drive the confined drive answers
        // to. Its media type is the device profile's (C: flash, Z: ROM); the drive is read-only, so the media is write protected and nothing is free;
        // the size is the bytes of the files in it; no serial number (0) and no label (empty) are known. A drive it does not answer to has no
        // media (KErrNotReady). DELETE WHEN the drive carries a volume label and serial from the install it was read from.
        "_ZNK3RFs6VolumeER11TVolumeInfoi" => {
            let Some(letter) = u8::try_from(r2).ok().filter(|d| *d < 26).map(|d| (b'A' + d) as char).filter(|l| crate::drive::Drive::LETTERS.contains(l)) else {
                p.cpu.regs[0] = KERR_NOT_READY as u32;
                return Some(NativeOutcome::Return);
            };
            let used = p.host.drive.as_ref().map_or(0, |d| d.used_bytes());
            let fields = [(0, if letter == 'Z' { MEDIA_ROM } else { MEDIA_FLASH }), (4, 0), (8, drive_att(letter)), (12, MEDIA_ATT_WRITE_PROTECTED), (VOL_I_SIZE, used as u32), (VOL_I_SIZE + 4, (used >> 32) as u32), (VOL_I_FREE, 0), (VOL_I_FREE + 4, 0)];
            match fields.iter().try_for_each(|(o, v)| p.wr(r1 + o, *v)) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        // TInt RFs::GetDriveName(TInt aDrive, TDes& aDriveName) const: the user-set name of a drive; none is set (empty).
        "_ZNK3RFs12GetDriveNameEiR6TDes16" => match crate::desc::write16(p, r2, &[]) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) },
        // TInt RFs::Connect(TInt aMessageSlots = -1): a new session; its handle is written into the RFs.
        "_ZN3RFs7ConnectEi" => {
            let h = p.fs_connect();
            match p.mem.vm.write_u32(this, h) {
                Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return }
                Err(_) => refuse(format!("the RFs at {this:#x} is unwritable")),
            }
        }
        // void RFs::Close(): the session ends.
        "_ZN3RFs5CloseEv" => {
            if let Ok(h) = p.mem.vm.read_u32(this) { p.nat.objs.fs_sessions.remove(&h); }
            match p.mem.vm.write_u32(this, 0) { Ok(()) => NativeOutcome::Return, Err(_) => refuse(format!("the RFs at {this:#x} is unwritable")) }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(s: &str) -> alloc::vec::Vec<u16> { s.encode_utf16().collect() }
    fn s16(u: &[u16]) -> String { String::from_utf16_lossy(u) }

    #[test]
    fn tparse_splits_and_combines_symbian_paths() {
        let p = u16s("c:\\sys\\bin\\putty.exe");
        let part = |k| { let (a, b) = tparse_slice(&p, k); s16(&p[a..b]) };
        assert_eq!(part("drive"), "c:");
        assert_eq!(part("path"), "\\sys\\bin\\");
        assert_eq!(part("drive_and_path"), "c:\\sys\\bin\\");
        assert_eq!(part("name"), "putty");
        assert_eq!(part("ext"), ".exe");
        assert_eq!(part("name_and_ext"), "putty.exe");
        // Combine name with related + default fallback
        let comb = tparse_combine(&u16s("notes"), Some(&u16s("\\docs\\")), Some(&u16s("d:\\fallback\\x.txt")));
        assert_eq!(s16(&comb), "d:\\docs\\notes.txt");
        // NEGATIVE CONTROL: path with no drive and no extension returns empty drive/ext slices.
        let bare = u16s("readme");
        assert_eq!(tparse_slice(&bare, "drive"), (0, 0));
        assert_eq!(tparse_slice(&bare, "ext"), (6, 6));
    }
}
