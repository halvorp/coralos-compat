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
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
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
