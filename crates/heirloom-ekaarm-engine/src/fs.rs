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

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED efsrv {name}: {m}"));
    Some(match name {
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
