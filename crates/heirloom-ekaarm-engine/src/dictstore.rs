//! `CDictionaryFileStore` (s32file.h:421): the persistent dictionary an application keeps its settings in (UID -> stream). The
//! guest holds a `CBase`-derived object (the destructor is the only virtual it uses); the entries are host state.
use alloc::format;
use alloc::string::String;
use crate::desc;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{DictState, HostObj};

/// Bytes of the guest block: a vptr and room (the guest reads none of it).
const DICT_STORE_BYTES: u32 = 16;
/// ~CDictionaryFileStore 0 (complete), 1 (deleting), `CBase::Extension_` 2 (e32base.h:48-56).
const VTABLE: &[(usize, &str)] = &[(0, "_ZN20CDictionaryFileStoreD1Ev"), (1, "_ZN20CDictionaryFileStoreD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")];

/// A file name the guest gave, resolved against the session path when it has no directory (f32file.h `RFs::SetSessionPath`).
fn resolve(session_path: &str, name: &str) -> String {
    let b = name.as_bytes();
    let has_drive = b.len() >= 2 && b[1] == b':';
    if has_drive || name.starts_with('\\') { String::from(name) } else { format!("{session_path}{name}") }
}

impl Process {
    fn dict_open_empty(&mut self, path: String) -> Result<u32, String> {
        let obj = self.alloc_zeroed(DICT_STORE_BYTES).ok_or_else(|| String::from("no guest heap"))?;
        let vptr = self.build_vtable("CDictionaryFileStore", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(obj, vptr).map_err(|_| String::from("the store object is unwritable"))?;
        self.obj_set(obj, HostObj::Dict(DictState { path, entries: Default::default() }))?;
        Ok(obj)
    }

    fn dict_open(&mut self, fs: u32, name_desc: u32, uid3: u32) -> Result<u32, String> {
        let name = desc::to_utf8(&desc::read16(self, name_desc)?);
        let session = self.fs_session(fs).map(|s| s.session_path.clone()).ok_or_else(|| format!("the RFs at {fs:#x} is not connected"))?;
        let path = resolve(&session, &name);
        let drive = self.host.drive.as_mut().ok_or_else(|| String::from("no drive"))?;
        // An existing file is a Symbian store (a stream dictionary in a permanent file store); this engine does not read that
        // format. A file that is not there is created, empty: `CDictionaryFileStore::OpenLC` creates the store (s32file.h:427).
        // DELETE WHEN a corpus run opens an existing dictionary file (then read the store format).
        if drive.exists(&path) {
            let size = drive.file_len(&path).unwrap_or(0) as usize;
            if size != 0 {
                return Err(format!("{path} exists with {size} bytes: reading an existing dictionary store is not implemented"));
            }
        } else {
            drive.put_file(&path, alloc::vec::Vec::new()).map_err(|e| format!("{path}: {e:?}"))?;
        }
        let _ = uid3; // the store's identifying UID3 (a file-header check on reopen): nothing to check on an empty store
        self.dict_open_empty(path)
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED estor {name}: {m}"));
    Some(match name {
        // CDictionaryStore* CEikApplication::OpenIniFileLC(RFs& aFs) const (eikapp.h): opens the application's private .ini
        // dictionary store and leaves it on the cleanup stack. Empty host dictionary when no binary store reader is present.
        "_ZNK15CEikApplication13OpenIniFileLCER3RFs" => {
            if p.fs_session(r1).is_none() {
                return Some(refuse(format!("the RFs at {r1:#x} is not connected")));
            }
            let path = format!("c:\\private\\{:08x}\\app.ini", p.uid3);
            match p.dict_open_empty(path) {
                Ok(obj) => { p.nat.cleanup.push(crate::cleanup::CleanupItem::Base(obj)); p.cpu.regs[0] = obj; NativeOutcome::Return }
                Err(m) => refuse(m),
            }
        }
        // static CDictionaryFileStore* OpenLC(RFs&, const TDesC& aName, TUid aUid3): the store, left on the cleanup stack.
        "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid" => match p.dict_open(r0, r1, r2) {
            Ok(obj) => { p.nat.cleanup.push(crate::cleanup::CleanupItem::Base(obj)); p.cpu.regs[0] = obj; NativeOutcome::Return }
            Err(m) => refuse(m),
        },
        // TBool CDictionaryStore::IsPresentL(TUid) const: is there a stream for the UID.
        "_ZNK16CDictionaryStore10IsPresentLE4TUid" => match p.obj_get(r0) {
            Some(HostObj::Dict(d)) => { let present = d.entries.contains_key(&r1); p.cpu.regs[0] = present as u32; NativeOutcome::Return }
            _ => refuse(format!("{r0:#x} is not a live dictionary store")),
        },
        // void RDictionaryWriteStream::AssignLC(CDictionaryStore& aStore, TUid aUid)
        "_ZN22RDictionaryWriteStream8AssignLCER16CDictionaryStore4TUid"
        | "_ZN22RDictionaryWriteStream7AssignLER16CDictionaryStore4TUid" => {
            let _ = p.wr(r0, r1); // store ptr
            let _ = p.wr(r0.wrapping_add(4), r2); // uid
            if name.contains("AssignLC") {
                p.nat.cleanup.push(crate::cleanup::CleanupItem::Any(0));
            }
            NativeOutcome::Return
        }
        // void RWriteStream::WriteL(const TDesC8& aDes)
        "_ZN12RWriteStream6WriteLERK6TDesC8" => {
            let store = p.rd(r0).unwrap_or(0);
            let uid = p.rd(r0.wrapping_add(4)).unwrap_or(0);
            let bytes = desc::read8(p, r1).unwrap_or_default();
            if let Some(HostObj::Dict(d)) = p.obj_get(store) {
                d.entries.entry(uid).or_default().extend_from_slice(&bytes);
            }
            NativeOutcome::Return
        }
        "_ZN12RWriteStream7CommitLEv"
        | "_ZN12RWriteStream5CloseEv"
        | "_ZN16CDictionaryStore7CommitLEv"
        | "_ZN16CDictionaryStore6CommitEv" => {
            if name.ends_with("CommitEv") { p.cpu.regs[0] = 0; }
            NativeOutcome::Return
        }
        // The destructors: the deleting one (D0) also frees the block.
        "_ZN20CDictionaryFileStoreD1Ev" | "_ZN20CDictionaryFileStoreD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); }
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
