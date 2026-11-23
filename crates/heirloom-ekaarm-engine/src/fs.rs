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
    /// On Symbian OS 9.x (data caging), a process's initial session path is `C:\private\<uid3>\`.
    pub(crate) fn fs_connect(&mut self) -> u32 {
        let session_path = if self.uid3 != 0 {
            format!("C:\\private\\{:08x}\\", self.uid3)
        } else {
            String::from("C:\\")
        };
        let objs = &mut self.nat.objs;
        objs.next_fs_handle += 1;
        objs.fs_sessions.insert(objs.next_fs_handle, FsSessionState { session_path });
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

const TPARSE_KIND_OFF: u32 = 4;
const TPARSE_KIND_PTR: u32 = 0x5054_5243; // 'PTRC' tag distinguishing TParsePtr/TParsePtrC from TParse

fn tparse_desc_addr(p: &Process, parse_this: u32) -> Result<u32, String> {
    if p.rd(parse_this.wrapping_add(TPARSE_KIND_OFF))? == TPARSE_KIND_PTR {
        p.rd(parse_this.wrapping_add(TPARSE_NAMEBUF_OFF))
    } else {
        Ok(parse_this.wrapping_add(TPARSE_NAMEBUF_OFF))
    }
}

fn tparse_write_ptrc(p: &mut Process, sret: u32, parse_this: u32, part: &str) -> Result<(), String> {
    let desc_addr = tparse_desc_addr(p, parse_this)?;
    let view = crate::desc::locate(p, desc_addr)?;
    let units = crate::desc::read16(p, desc_addr)?;
    let (start, end) = tparse_slice(&units, part);
    let len = (end - start) as u32;
    let ptr = view.data.wrapping_add(start as u32 * 2);
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
        // void RHandleBase::Close() (e32std.h): release any tracked session handle and zero iHandle.
        "_ZN11RHandleBase5CloseEv" => {
            if let Ok(handle) = p.mem.vm.read_u32(this) {
                if handle != 0 {
                    p.nat.objs.fs_sessions.remove(&handle);
                    let _ = p.mem.vm.write_u32(this, 0);
                }
            }
            NativeOutcome::Return
        }
        // TParse::TParse(): initialise the embedded TBuf<256> at +TPARSE_NAMEBUF_OFF.
        "_ZN6TParseC1Ev" | "_ZN6TParseC2Ev" => {
            let buf = this.wrapping_add(TPARSE_NAMEBUF_OFF);
            let ok = p.mem.vm.write_u32(this.wrapping_add(TPARSE_KIND_OFF), 0).is_ok()
                && p.mem.vm.write_u32(buf, crate::desc::DES_BUF << crate::desc::KSHIFT_DES_TYPE).is_ok()
                && p.mem.vm.write_u32(buf.wrapping_add(4), TPARSE_MAX_LEN).is_ok();
            if ok { NativeOutcome::Return } else { refuse(format!("TParse at {this:#x} is unwritable")) }
        }
        // TParsePtrC(const TDesC16& aName) / TParsePtr(TDes16& aName) (f32file.h):
        "_ZN10TParsePtrCC1ERK7TDesC16" | "_ZN10TParsePtrCC2ERK7TDesC16"
        | "_ZN9TParsePtrC1ER6TDes16" | "_ZN9TParsePtrC2ER6TDes16" => {
            let ok = p.mem.vm.write_u32(this.wrapping_add(TPARSE_KIND_OFF), TPARSE_KIND_PTR).is_ok()
                && p.mem.vm.write_u32(this.wrapping_add(TPARSE_NAMEBUF_OFF), r1).is_ok();
            if ok { p.cpu.regs[0] = this; NativeOutcome::Return } else { refuse(format!("TParsePtr at {this:#x} is unwritable")) }
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
        // const TDesC16& TParseBase::FullName() const: returns pointer to embedded TBuf<256> or wrapped descriptor.
        "_ZNK10TParseBase8FullNameEv" => match tparse_desc_addr(p, this) {
            Ok(a) => { p.cpu.regs[0] = a; NativeOutcome::Return }
            Err(m) => refuse(m),
        },
        // TPtrC16 TParseBase::{Drive,Path,DriveAndPath,Name,Ext,NameAndExt}() const: ARM EABI sret in r0, this in r1.
        "_ZNK10TParseBase5DriveEv" => match tparse_write_ptrc(p, this, r1, "drive") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase4PathEv" => match tparse_write_ptrc(p, this, r1, "path") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase12DriveAndPathEv" => match tparse_write_ptrc(p, this, r1, "drive_and_path") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase4NameEv" => match tparse_write_ptrc(p, this, r1, "name") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase3ExtEv" => match tparse_write_ptrc(p, this, r1, "ext") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        "_ZNK10TParseBase10NameAndExtEv" => match tparse_write_ptrc(p, this, r1, "name_and_ext") { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        // TInt RFs::PrivatePath(TDes16& aPath) (f32file.h): writes `\private\<uid3:08x>\` into aPath.
        "_ZN3RFs11PrivatePathER6TDes16" => {
            if p.fs_session(this).is_none() {
                return Some(refuse(format!("the session at {this:#x} is not connected")));
            }
            let path = format!("\\private\\{:08x}\\", p.uid3);
            let units: alloc::vec::Vec<u16> = path.encode_utf16().collect();
            match crate::desc::write16(p, r1, &units) {
                Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // TBool BaflUtils::FolderExists(RFs& aFs, const TDesC16& aFolderName) (bautils.h)
        "_ZN9BaflUtils12FolderExistsER3RFsRK7TDesC16" => {
            let exists = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).ok().is_some_and(|path| {
                p.host.drive.as_ref().is_some_and(|d| d.list(&path).is_ok() || d.list_rom(&path).is_some())
            });
            p.cpu.regs[0] = u32::from(exists);
            NativeOutcome::Return
        }
        // TBool BaflUtils::FileExists(const RFs& aFs, const TDesC16& aFileName) (bautils.h)
        "_ZN9BaflUtils10FileExistsERK3RFsRK7TDesC16" => {
            let exists = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).ok().is_some_and(|path| {
                p.host.drive.as_ref().is_some_and(|d| d.read_owned(&path).is_ok())
            });
            p.cpu.regs[0] = u32::from(exists);
            NativeOutcome::Return
        }
        // void BaflUtils::EnsurePathExistsL(RFs& aFs, const TDesC16& aFileName) (bautils.h)
        "_ZN9BaflUtils17EnsurePathExistsLER3RFsRK7TDesC16" => {
            let res = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).and_then(|path| {
                let dir = match path.rfind(['\\', '/']) {
                    Some(i) => &path[..i],
                    None => "",
                };
                if !dir.is_empty() {
                    if let Some(d) = p.host.drive.as_mut() {
                        d.make_dirs(dir).map_err(|e| format!("make_dirs({dir}): {e:?}"))?;
                    }
                }
                Ok(())
            });
            match res { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) }
        }
        // TInt RFs::GetDir(const TDesC16& aName, TUint aEntryAttMask, TUint aEntrySortKey, CDir*& anEntryList) const (f32file.h):
        // TInt RFs::GetDir(const TDesC16& aName, TUint aEntryAttMask, TUint aEntrySortKey, CDir*& anEntryList, CDir*& aDirList) const:
        // 5th arg `anEntryList` is on the ARM stack at [sp + 0]; 6th arg `aDirList` (when present) is at [sp + 4].
        "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDir" | "_ZNK3RFs6GetDirERK7TDesC16jjRP4CDirS5_" => {
            if p.fs_session(this).is_none() {
                return Some(refuse(format!("the session at {this:#x} is not connected")));
            }
            let two_lists = name.ends_with("S5_");
            let entry_out_addr = match p.rd(p.cpu.regs[13]) {
                Ok(a) => a,
                Err(m) => return Some(refuse(format!("stack arg anEntryList: {m}"))),
            };
            let dir_out_addr = if two_lists {
                match p.rd(p.cpu.regs[13] + 4) {
                    Ok(a) => Some(a),
                    Err(m) => return Some(refuse(format!("stack arg aDirList: {m}"))),
                }
            } else {
                None
            };
            let res = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).and_then(|path| {
                let dir_path = match path.rfind(['\\', '/']) {
                    Some(i) => &path[..i],
                    None => path.as_str(),
                };
                let all: alloc::vec::Vec<crate::objects::DirEntry> = p.host.drive.as_ref()
                    .and_then(|d| {
                        d.list_entries(dir_path).ok().map(|v| {
                            v.into_iter()
                                .map(|(name, is_dir, size)| crate::objects::DirEntry { name, is_dir, size })
                                .collect()
                        }).or_else(|| {
                            d.list_rom(dir_path).map(|v| {
                                v.into_iter()
                                    .map(|name| crate::objects::DirEntry { name, is_dir: false, size: 0 })
                                    .collect()
                            })
                        })
                    })
                    .unwrap_or_default();
                if let Some(d_out) = dir_out_addr {
                    let (dirs, files): (alloc::vec::Vec<_>, alloc::vec::Vec<_>) =
                        all.into_iter().partition(|e| e.is_dir);
                    let fdir = p.plain_object("CDir", 0x10)?;
                    p.obj_set(fdir, crate::objects::HostObj::Dir(files))?;
                    p.wr(entry_out_addr, fdir)?;
                    let ddir = p.plain_object("CDir", 0x10)?;
                    p.obj_set(ddir, crate::objects::HostObj::Dir(dirs))?;
                    p.wr(d_out, ddir)?;
                } else {
                    let cdir = p.plain_object("CDir", 0x10)?;
                    p.obj_set(cdir, crate::objects::HostObj::Dir(all))?;
                    p.wr(entry_out_addr, cdir)?;
                }
                Ok(())
            });
            match res {
                Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // TInt CDir::Count() const (f32file.h)
        "_ZNK4CDir5CountEv" => match p.obj_get(this) {
            Some(crate::objects::HostObj::Dir(entries)) => {
                p.cpu.regs[0] = entries.len() as u32;
                NativeOutcome::Return
            }
            _ => refuse(format!("{this:#x} is not a CDir this engine made")),
        },
        // const TEntry& CDir::operator[](TInt anIndex) const (f32file.h):
        // TEntry layout (ARM EABI): iAtt(0), iSize(4), iModified(8, 8B), iType(16, 12B), iName(28, TBufC<256> = 516B).
        "_ZNK4CDirixEi" => {
            const TENTRY_SIZE: u32 = 28 + 4 + 256 * 2;
            const TENTRY_NAME_OFF: u32 = 28;
            const KENTRY_ATT_DIR: u32 = 0x10;
            let idx = r1 as i32;
            let entry = match p.obj_get(this) {
                Some(crate::objects::HostObj::Dir(entries)) if idx >= 0 && (idx as usize) < entries.len() => {
                    entries[idx as usize].clone()
                }
                Some(crate::objects::HostObj::Dir(entries)) => {
                    return Some(refuse(format!("CDir index {idx} out of bounds (count {})", entries.len())));
                }
                _ => return Some(refuse(format!("{this:#x} is not a CDir this engine made"))),
            };
            let res = (|| -> Result<u32, String> {
                let cell = p.alloc_zeroed(TENTRY_SIZE).ok_or_else(|| String::from("out of heap for TEntry"))?;
                if entry.is_dir {
                    p.wr(cell, KENTRY_ATT_DIR)?;
                }
                p.wr(cell + 4, entry.size)?;
                let units: alloc::vec::Vec<u16> = entry.name.encode_utf16().take(256).collect();
                p.wr(cell + TENTRY_NAME_OFF, units.len() as u32)?; // TBufC16 (type 0)
                for (i, u) in units.iter().enumerate() {
                    p.mem.vm.write_u16(cell + TENTRY_NAME_OFF + 4 + i as u32 * 2, *u)
                        .map_err(|_| String::from("TEntry::iName unwritable"))?;
                }
                Ok(cell)
            })();
            match res {
                Ok(cell) => { p.cpu.regs[0] = cell; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // CDir::~CDir() (f32file.h)
        "_ZN4CDirD1Ev" | "_ZN4CDirD0Ev" => {
            p.obj_remove(this);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(this); }
            }
            p.cpu.regs[0] = this;
            NativeOutcome::Return
        }
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
        // static TInt RFs::DriveToChar(TInt aDrive, TChar& aChar): 0..26 -> 'A'..'Z', else KErrArgument.
        "_ZN3RFs11DriveToCharEiR5TChar" => match u8::try_from(this).ok().filter(|&d| d < 26) {
            Some(d) => match p.wr(r1, u32::from(b'A' + d)) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) },
            None => { p.cpu.regs[0] = KERR_ARGUMENT as u32; NativeOutcome::Return }
        },
        // TInt RFs::Drive(TDriveInfo& aInfo, TInt aDrive = KDefaultDrive) const (f32file.h): fills the 20-byte TDriveInfo.
        "_ZNK3RFs5DriveER10TDriveInfoi" => {
            let letter = u8::try_from(r2).ok().filter(|&d| d < 26).map(|d| (b'A' + d) as char).filter(|l| crate::drive::Drive::LETTERS.contains(l));
            let (media, d_att, m_att) = match letter {
                Some('Z') => (MEDIA_ROM, drive_att('Z'), MEDIA_ATT_WRITE_PROTECTED),
                Some(l) => (MEDIA_FLASH, drive_att(l), 0),
                None => (0, 0, 0), // EMediaNotPresent
            };
            let fields = [(0u32, media), (4, 0), (8, d_att), (12, m_att), (16, 0)];
            match fields.iter().try_for_each(|(o, v)| p.wr(r1 + o, *v)) { Ok(()) => { p.cpu.regs[0] = KERR_NONE as u32; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        // TVolumeInfo::TVolumeInfo(): zeroed (the name is an empty TBufC).
        "_ZN11TVolumeInfoC1Ev" => match (0..VOL_NAME_END / 4).try_for_each(|i| p.wr(this + i * 4, 0)) { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) },
        // TInt RFs::Volume(TVolumeInfo& aVol, TInt aDrive = KDefaultDrive) const (f32file.h:1876): the volume of a drive the confined drive answers
        // to. Z: is read-only ROM (write-protected, 0 free); C: is writable internal flash with an in-memory overlay (writable, 64 MiB free).
        // A drive it does not answer to has no media (KErrNotReady).
        "_ZNK3RFs6VolumeER11TVolumeInfoi" => {
            let Some(letter) = u8::try_from(r2).ok().filter(|d| *d < 26).map(|d| (b'A' + d) as char).filter(|l| crate::drive::Drive::LETTERS.contains(l)) else {
                p.cpu.regs[0] = KERR_NOT_READY as u32;
                return Some(NativeOutcome::Return);
            };
            let used = p.host.drive.as_ref().map_or(0, |d| d.used_bytes());
            let (media, media_att, free): (u32, u32, u64) = if letter == 'Z' {
                (MEDIA_ROM, MEDIA_ATT_WRITE_PROTECTED, 0)
            } else {
                (MEDIA_FLASH, 0, 64 * 1024 * 1024)
            };
            let size = used.saturating_add(free);
            let fields = [
                (0, media),
                (4, 0),
                (8, drive_att(letter)),
                (12, media_att),
                (VOL_I_SIZE, size as u32),
                (VOL_I_SIZE + 4, (size >> 32) as u32),
                (VOL_I_FREE, free as u32),
                (VOL_I_FREE + 4, (free >> 32) as u32),
            ];
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
        // TInt RFile::Open(RFs& aFs, const TDesC16& aName, TUint aFileMode)
        "_ZN5RFile4OpenER3RFsRK7TDesC16j" => {
            let Ok(units) = crate::desc::read16(p, r2) else { return Some(refuse(format!("bad filename at {r2:#x}"))) };
            let raw = crate::desc::to_utf8(&units);
            let session_prefix = p.fs_session(r1).map(|s| s.session_path.clone()).unwrap_or_default();
            let full = resolve_drive_path(&raw, &session_prefix);
            let priv_dir = format!("C:\\private\\{:08x}\\", p.uid3);
            let data = p.host.drive.as_ref().and_then(|d| {
                d.read(&full).ok()
                    .or_else(|| {
                        let tail = raw.rsplit(['\\', '/']).next().unwrap_or(&raw);
                        d.read(&format!("{priv_dir}{tail}")).ok()
                    })
            }).map(|b| b.to_vec());
            match data {
                Some(bytes) => {
                    p.nat.objs.next_file_handle += 1;
                    let h = p.nat.objs.next_file_handle;
                    p.nat.objs.open_files.insert(h, crate::objects::OpenFile { path: full, data: bytes, pos: 0 });
                    let _ = p.mem.vm.write_u32(this, h);
                    p.cpu.regs[0] = KERR_NONE as u32;
                }
                None => {
                    let _ = p.mem.vm.write_u32(this, 0);
                    p.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
                }
            }
            NativeOutcome::Return
        }
        // TInt RFile::Create / Replace (in-memory writable file handle)
        "_ZN5RFile6CreateER3RFsRK7TDesC16j" | "_ZN5RFile7ReplaceER3RFsRK7TDesC16j" => {
            let Ok(units) = crate::desc::read16(p, r2) else { return Some(refuse(format!("bad filename at {r2:#x}"))) };
            let raw = crate::desc::to_utf8(&units);
            let session_prefix = p.fs_session(r1).map(|s| s.session_path.clone()).unwrap_or_default();
            let full = resolve_drive_path(&raw, &session_prefix);
            p.nat.objs.next_file_handle += 1;
            let h = p.nat.objs.next_file_handle;
            p.nat.objs.open_files.insert(h, crate::objects::OpenFile { path: full, data: alloc::vec::Vec::new(), pos: 0 });
            let _ = p.mem.vm.write_u32(this, h);
            p.cpu.regs[0] = KERR_NONE as u32;
            NativeOutcome::Return
        }
        // TInt RFile::Size(TInt& aSize) const
        "_ZNK5RFile4SizeERi" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            if let Some(f) = p.nat.objs.open_files.get(&h) {
                let _ = p.wr(r1, f.data.len() as u32);
                p.cpu.regs[0] = KERR_NONE as u32;
            } else {
                p.cpu.regs[0] = (-8i32) as u32; // KErrBadHandle
            }
            NativeOutcome::Return
        }
        // TInt RFile::SetSize(TInt aSize)
        "_ZN5RFile7SetSizeEi" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            let new_size = r1 as i32;
            if new_size < 0 {
                p.cpu.regs[0] = (-6i32) as u32; // KErrArgument
            } else if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                f.data.resize(new_size as usize, 0);
                if f.pos > f.data.len() { f.pos = f.data.len(); }
                p.cpu.regs[0] = KERR_NONE as u32;
            } else {
                p.cpu.regs[0] = (-8i32) as u32; // KErrBadHandle
            }
            NativeOutcome::Return
        }
        // TInt RFile::Read(TDes8& aDes) const / Read(TDes8& aDes, TInt aLength) const
        "_ZNK5RFile4ReadER5TDes8" | "_ZNK5RFile4ReadER5TDes8i" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            let max_len = crate::desc::locate(p, r1).ok().and_then(|v| v.max).unwrap_or(0) as usize;
            let want = if name.ends_with("8i") { (r2 as i32).max(0) as usize } else { max_len }.min(max_len);
            let chunk = if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                let avail = f.data.len().saturating_sub(f.pos);
                let n = want.min(avail);
                let c = f.data[f.pos..f.pos + n].to_vec();
                f.pos += n;
                Some(c)
            } else {
                None
            };
            match chunk {
                Some(bytes) => {
                    let _ = crate::desc::write8(p, r1, &bytes);
                    p.cpu.regs[0] = KERR_NONE as u32;
                }
                None => { p.cpu.regs[0] = (-8i32) as u32; }
            }
            NativeOutcome::Return
        }
        // TInt RFile::Read(TInt aPos, TDes8& aDes) const / Read(TInt aPos, TDes8& aDes, TInt aLength) const
        "_ZNK5RFile4ReadEiR5TDes8" | "_ZNK5RFile4ReadEiR5TDes8i" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            let pos = (r1 as i32).max(0) as usize;
            let max_len = crate::desc::locate(p, r2).ok().and_then(|v| v.max).unwrap_or(0) as usize;
            let want = if name.ends_with("8i") { (p.cpu.regs[3] as i32).max(0) as usize } else { max_len }.min(max_len);
            let chunk = if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                let p0 = pos.min(f.data.len());
                let n = want.min(f.data.len().saturating_sub(p0));
                f.pos = p0 + n;
                Some(f.data[p0..p0 + n].to_vec())
            } else {
                None
            };
            match chunk {
                Some(bytes) => {
                    let _ = crate::desc::write8(p, r2, &bytes);
                    p.cpu.regs[0] = KERR_NONE as u32;
                }
                None => { p.cpu.regs[0] = (-8i32) as u32; }
            }
            NativeOutcome::Return
        }
        // TInt RFile::Seek(TSeek aMode, TInt& aPos) const
        "_ZNK5RFile4SeekE5TSeekRi" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            let off = p.rd(r2).unwrap_or(0) as i32;
            if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                let new_pos = match r1 {
                    0 => off.max(0) as usize, // ESeekStart
                    1 => (f.pos as i64 + off as i64).max(0) as usize, // ESeekCurrent
                    2 => (f.data.len() as i64 + off as i64).max(0) as usize, // ESeekEnd
                    _ => f.pos,
                }.min(f.data.len());
                f.pos = new_pos;
                let _ = p.wr(r2, new_pos as u32);
                p.cpu.regs[0] = KERR_NONE as u32;
            } else {
                p.cpu.regs[0] = (-8i32) as u32;
            }
            NativeOutcome::Return
        }
        // TInt RFile::Write(const TDesC8& aDes)
        "_ZN5RFile5WriteERK6TDesC8" => {
            let h = p.mem.vm.read_u32(this).unwrap_or(0);
            let bytes = crate::desc::read8(p, r1).unwrap_or_default();
            if let Some(f) = p.nat.objs.open_files.get_mut(&h) {
                if f.pos + bytes.len() > f.data.len() {
                    f.data.resize(f.pos + bytes.len(), 0);
                }
                f.data[f.pos..f.pos + bytes.len()].copy_from_slice(&bytes);
                f.pos += bytes.len();
                p.cpu.regs[0] = KERR_NONE as u32;
            } else {
                p.cpu.regs[0] = (-8i32) as u32;
            }
            NativeOutcome::Return
        }
        // void RFile::Close()
        "_ZN5RFile5CloseEv" => {
            if let Ok(h) = p.mem.vm.read_u32(this) { p.nat.objs.open_files.remove(&h); }
            let _ = p.mem.vm.write_u32(this, 0);
            NativeOutcome::Return
        }
        // TFindFile (f32file.h): search across drives; returns KErrNotFound when no matching file exists.
        "_ZN9TFindFileC1ER3RFs" | "_ZN9TFindFileC2ER3RFs" => {
            let _ = p.wr(this, r1);
            p.cpu.regs[0] = this;
            NativeOutcome::Return
        }
        "_ZN9TFindFile13FindWildByDirERK7TDesC16S2_RP4CDir" => {
            let _ = p.wr(r3, 0);
            p.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        "_ZN9TFindFile8FindWildERP4CDir" => {
            let _ = p.wr(r1, 0);
            p.cpu.regs[0] = (-1i32) as u32; // KErrNotFound
            NativeOutcome::Return
        }
        _ => return None,
    })
}

pub(crate) fn resolve_drive_path(raw: &str, session_path: &str) -> String {
    let b = raw.as_bytes();
    if b.len() >= 2 && b[1] == b':' {
        return String::from(raw);
    }
    if raw.starts_with('\\') || raw.starts_with('/') {
        let drive = if session_path.len() >= 2 && session_path.as_bytes()[1] == b':' {
            &session_path[..2]
        } else {
            "C:"
        };
        return format!("{drive}{raw}");
    }
    if !session_path.is_empty() {
        let sep = if session_path.ends_with('\\') { "" } else { "\\" };
        return format!("{session_path}{sep}{raw}");
    }
    format!("C:\\{raw}")
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
