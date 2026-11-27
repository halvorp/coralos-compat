//! Stateless Symbian `CDictionaryFileStore` (`s32file.h:421`) adapter over `guestsvc_abi::GuestPrefs`.
//!
//! REUSE U6 (CoralOS `docs/heirloom/21-SHARED-SERVICES.md` + component-model C5):
//! - Every committed dictionary stream persists directly in the engine process's native `prefs.service` session
//!   (`Space::Own`, loader-admitted bundle namespace) under `dict/<store_uid3_8hex>/<stream_uid_8hex>` as
//!   `GuestValue::Bytes`.
//! - `DictHandle` keeps ONLY transient in-flight write-stream staging bytes between `AssignL` -> `WriteL*` -> `CommitL`.
//!   On `CommitL`, staged bytes are written through `GuestPrefs::set` and removed from the transient buffer; ZERO
//!   persistent state is held inside the engine.
//! - Without a `(prefs)` grant (`GuestSvcError::NotGranted`), `IsPresentL`, `OpenLC`, `CommitL`, and `RemoveL` fail
//!   cleanly with `KErrPermissionDenied` (-46) / named refusal instead of falling back to private in-engine storage.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestsvc_abi::{GuestSvcError, GuestValue};
use crate::desc;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{DictHandle, HostObj, ReadStreamState};

/// Bytes of the guest block: a vptr and room (the guest reads none of it).
const DICT_STORE_BYTES: u32 = 16;
/// ~CDictionaryFileStore 0 (complete), 1 (deleting), `CBase::Extension_` 2 (e32base.h:48-56).
const VTABLE: &[(usize, &str)] = &[
    (0, "_ZN20CDictionaryFileStoreD1Ev"),
    (1, "_ZN20CDictionaryFileStoreD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
];

const KERR_NONE: i32 = 0;
const KERR_NOT_FOUND: i32 = -1;
const KERR_ARGUMENT: i32 = -6;
const KERR_SERVER_TERMINATED: i32 = -15;
const KERR_OVERFLOW: i32 = -22;
const KERR_EOF: i32 = -25;
const KERR_PERMISSION_DENIED: i32 = -46;

fn dict_key(store_id: &str, stream_uid: u32) -> String {
    format!("dict/{store_id}/{stream_uid:08x}")
}

fn svc_err_to_symbian(err: GuestSvcError) -> i32 {
    match err {
        GuestSvcError::NotGranted => KERR_PERMISSION_DENIED,
        GuestSvcError::TooLarge => KERR_OVERFLOW,
        GuestSvcError::BadName | GuestSvcError::Refused(_) => KERR_ARGUMENT,
        GuestSvcError::Lost | GuestSvcError::Stalled | GuestSvcError::Unreadable => KERR_SERVER_TERMINATED,
    }
}

fn leave_or_refuse(p: &mut Process, name: &str, code: i32, detail: String) -> NativeOutcome {
    if !p.exception_descriptors.is_empty() {
        crate::ehabi::leave(p, code)
    } else {
        NativeOutcome::Refused(format!("NEXT-NEEDED estor {name}: {detail} (symbian err {code})"))
    }
}

/// A file name the guest gave, resolved against the session path when it has no directory (f32file.h `RFs::SetSessionPath`).
fn resolve(session_path: &str, name: &str) -> String {
    let b = name.as_bytes();
    let has_drive = b.len() >= 2 && b[1] == b':';
    if has_drive || name.starts_with('\\') {
        String::from(name)
    } else {
        format!("{session_path}{name}")
    }
}

impl Process {
    fn dict_open_handle(&mut self, store_id: String, path: String) -> Result<u32, String> {
        let obj = self.alloc_zeroed(DICT_STORE_BYTES).ok_or_else(|| String::from("no guest heap"))?;
        let vptr = self.build_vtable("CDictionaryFileStore", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(obj, vptr).map_err(|_| String::from("the store object is unwritable"))?;
        self.obj_set(
            obj,
            HostObj::Dict(DictHandle {
                store_id,
                path,
                pending_writes: Default::default(),
            }),
        )?;
        Ok(obj)
    }

    fn dict_open(&mut self, fs: u32, name_desc: u32, uid3: u32) -> Result<u32, String> {
        let name = desc::to_utf8(&desc::read16(self, name_desc)?);
        let session = self
            .fs_session(fs)
            .map(|s| s.session_path.clone())
            .ok_or_else(|| format!("the RFs at {fs:#x} is not connected"))?;
        let path = resolve(&session, &name);
        let drive = self.host.drive.as_mut().ok_or_else(|| String::from("no drive"))?;
        // A pre-existing non-empty binary Symbian permanent file store is a foreign format we do not parse;
        // an empty/absent file is a fresh store whose streams live in CoralOS prefs.service.
        if drive.exists(&path) {
            let size = drive.file_len(&path).unwrap_or(0) as usize;
            if size != 0 {
                return Err(format!("{path} exists with {size} bytes: reading an existing dictionary store is not implemented"));
            }
        } else {
            drive.put_file(&path, Vec::new()).map_err(|e| format!("{path}: {e:?}"))?;
        }
        let store_id = format!("{:08x}", if uid3 != 0 { uid3 } else { self.uid3 });
        self.dict_open_handle(store_id, path)
    }

    /// Flush one pending write stream `(store_addr, uid)` to `prefs.service`, clearing its transient buffer.
    fn dict_commit_stream(&mut self, store_addr: u32, stream_uid: u32) -> Result<(), (i32, String)> {
        let (store_id, payload) = match self.obj_get(store_addr) {
            Some(HostObj::Dict(d)) => {
                let id = d.store_id.clone();
                let data = d.pending_writes.remove(&stream_uid);
                (id, data)
            }
            _ => return Err((KERR_ARGUMENT, format!("{store_addr:#x} is not a live dictionary store"))),
        };
        let Some(bytes) = payload else {
            return Ok(());
        };
        let Some(prefs) = self.prefs_host.as_mut() else {
            return Err((KERR_PERMISSION_DENIED, String::from("not-granted: no prefs service attached")));
        };
        let key = dict_key(&store_id, stream_uid);
        prefs
            .set(&key, &GuestValue::Bytes(bytes))
            .map_err(|e| (svc_err_to_symbian(e), format!("prefs set {key}: {}", e.name())))
    }

    /// Flush all pending write streams on `store_addr` to `prefs.service`, clearing all transient buffers.
    fn dict_commit_all(&mut self, store_addr: u32) -> Result<(), (i32, String)> {
        let (store_id, pending) = match self.obj_get(store_addr) {
            Some(HostObj::Dict(d)) => (d.store_id.clone(), core::mem::take(&mut d.pending_writes)),
            _ => return Err((KERR_ARGUMENT, format!("{store_addr:#x} is not a live dictionary store"))),
        };
        if pending.is_empty() {
            return Ok(());
        }
        let Some(prefs) = self.prefs_host.as_mut() else {
            return Err((KERR_PERMISSION_DENIED, String::from("not-granted: no prefs service attached")));
        };
        for (stream_uid, bytes) in pending {
            let key = dict_key(&store_id, stream_uid);
            prefs
                .set(&key, &GuestValue::Bytes(bytes))
                .map_err(|e| (svc_err_to_symbian(e), format!("prefs set {key}: {}", e.name())))?;
        }
        Ok(())
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED estor {name}: {m}"));
    Some(match name {
        // CDictionaryStore* CEikApplication::OpenIniFileLC(RFs& aFs) const (eikapp.h)
        "_ZNK15CEikApplication13OpenIniFileLCER3RFs" => {
            if p.fs_session(r1).is_none() {
                return Some(refuse(format!("the RFs at {r1:#x} is not connected")));
            }
            let store_id = format!("{:08x}", p.uid3);
            let path = format!("c:\\private\\{:08x}\\app.ini", p.uid3);
            match p.dict_open_handle(store_id, path) {
                Ok(obj) => {
                    p.nat.cleanup.push(crate::cleanup::CleanupItem::Base(obj));
                    p.cpu.regs[0] = obj;
                    NativeOutcome::Return
                }
                Err(m) => refuse(m),
            }
        }
        // static CDictionaryFileStore* OpenLC(RFs&, const TDesC& aName, TUid aUid3)
        "_ZN20CDictionaryFileStore6OpenLCER3RFsRK7TDesC164TUid" => match p.dict_open(r0, r1, r2) {
            Ok(obj) => {
                p.nat.cleanup.push(crate::cleanup::CleanupItem::Base(obj));
                p.cpu.regs[0] = obj;
                NativeOutcome::Return
            }
            Err(m) => refuse(m),
        },
        // TBool CDictionaryStore::IsPresentL(TUid) const: consults prefs.service directly.
        "_ZNK16CDictionaryStore10IsPresentLE4TUid" => {
            let store_id = match p.obj_get(r0) {
                Some(HostObj::Dict(d)) => d.store_id.clone(),
                _ => return Some(refuse(format!("{r0:#x} is not a live dictionary store"))),
            };
            let key = dict_key(&store_id, r1);
            if let Some(prefs) = p.prefs_host.as_mut() {
                match prefs.get(&key) {
                    Ok(Some(_)) => {
                        p.cpu.regs[0] = 1;
                        NativeOutcome::Return
                    }
                    Ok(None) => {
                        p.cpu.regs[0] = 0;
                        NativeOutcome::Return
                    }
                    Err(e) => leave_or_refuse(p, name, svc_err_to_symbian(e), format!("prefs get {key}: {}", e.name())),
                }
            } else {
                p.cpu.regs[0] = 0;
                NativeOutcome::Return
            }
        }
        // void CDictionaryStore::RemoveL(TUid) / void CDictionaryStore::Remove(TUid)
        "_ZN16CDictionaryStore7RemoveLE4TUid" | "_ZN16CDictionaryStore6RemoveE4TUid" => {
            let store_id = match p.obj_get(r0) {
                Some(HostObj::Dict(d)) => {
                    d.pending_writes.remove(&r1);
                    d.store_id.clone()
                }
                _ => return Some(refuse(format!("{r0:#x} is not a live dictionary store"))),
            };
            let key = dict_key(&store_id, r1);
            let Some(prefs) = p.prefs_host.as_mut() else {
                return Some(leave_or_refuse(p, name, KERR_PERMISSION_DENIED, String::from("not-granted: no prefs service attached")));
            };
            match prefs.delete(&key) {
                Ok(()) => NativeOutcome::Return,
                Err(e) => leave_or_refuse(p, name, svc_err_to_symbian(e), format!("prefs delete {key}: {}", e.name())),
            }
        }
        // void RDictionaryWriteStream::AssignLC(CDictionaryStore& aStore, TUid aUid) / AssignL
        "_ZN22RDictionaryWriteStream8AssignLCER16CDictionaryStore4TUid"
        | "_ZN22RDictionaryWriteStream7AssignLER16CDictionaryStore4TUid" => {
            let _ = p.wr(r0, r1); // store ptr
            let _ = p.wr(r0.wrapping_add(4), r2); // stream uid
            if let Some(HostObj::Dict(d)) = p.obj_get(r1) {
                d.pending_writes.insert(r2, Vec::new());
            } else {
                return Some(refuse(format!("{r1:#x} is not a live dictionary store")));
            }
            if name.contains("AssignLC") {
                p.nat.cleanup.push(crate::cleanup::CleanupItem::Any(0));
            }
            NativeOutcome::Return
        }
        // void RWriteStream::WriteL(const TDesC8& aDes)
        "_ZN12RWriteStream6WriteLERK6TDesC8" => {
            let store = p.rd(r0).unwrap_or(0);
            let uid = p.rd(r0.wrapping_add(4)).unwrap_or(0);
            let bytes = match desc::read8(p, r1) {
                Ok(b) => b,
                Err(m) => return Some(refuse(m)),
            };
            if let Some(HostObj::Dict(d)) = p.obj_get(store) {
                d.pending_writes.entry(uid).or_default().extend_from_slice(&bytes);
                NativeOutcome::Return
            } else {
                refuse(format!("{store:#x} is not a live dictionary store"))
            }
        }
        // void RWriteStream::WriteInt32L(TInt32) / WriteUint32L(TUint32)
        "_ZN12RWriteStream11WriteInt32LEl" | "_ZN12RWriteStream12WriteUint32LEm" => {
            let store = p.rd(r0).unwrap_or(0);
            let uid = p.rd(r0.wrapping_add(4)).unwrap_or(0);
            if let Some(HostObj::Dict(d)) = p.obj_get(store) {
                d.pending_writes.entry(uid).or_default().extend_from_slice(&r1.to_le_bytes());
                NativeOutcome::Return
            } else {
                refuse(format!("{store:#x} is not a live dictionary store"))
            }
        }
        // void RWriteStream::CommitL()
        "_ZN12RWriteStream7CommitLEv" => {
            let store = p.rd(r0).unwrap_or(0);
            let uid = p.rd(r0.wrapping_add(4)).unwrap_or(0);
            match p.dict_commit_stream(store, uid) {
                Ok(()) => NativeOutcome::Return,
                Err((code, msg)) => leave_or_refuse(p, name, code, msg),
            }
        }
        // void RWriteStream::Close()
        "_ZN12RWriteStream5CloseEv" => NativeOutcome::Return,
        // void CDictionaryStore::CommitL()
        "_ZN16CDictionaryStore7CommitLEv" => match p.dict_commit_all(r0) {
            Ok(()) => NativeOutcome::Return,
            Err((code, msg)) => leave_or_refuse(p, name, code, msg),
        },
        // TInt CDictionaryStore::Commit()
        "_ZN16CDictionaryStore6CommitEv" => match p.dict_commit_all(r0) {
            Ok(()) => {
                p.cpu.regs[0] = KERR_NONE as u32;
                NativeOutcome::Return
            }
            Err((code, _)) => {
                p.cpu.regs[0] = code as u32;
                NativeOutcome::Return
            }
        },
        // void RDictionaryReadStream::OpenLC(const CDictionaryStore& aStore, TUid aUid) / OpenL
        "_ZN21RDictionaryReadStream6OpenLCERK16CDictionaryStore4TUid"
        | "_ZN21RDictionaryReadStream5OpenLERK16CDictionaryStore4TUid" => {
            let store_id = match p.obj_get(r1) {
                Some(HostObj::Dict(d)) => d.store_id.clone(),
                _ => return Some(refuse(format!("{r1:#x} is not a live dictionary store"))),
            };
            let key = dict_key(&store_id, r2);
            let Some(prefs) = p.prefs_host.as_mut() else {
                return Some(leave_or_refuse(p, name, KERR_PERMISSION_DENIED, String::from("not-granted: no prefs service attached")));
            };
            match prefs.get(&key) {
                Ok(Some(GuestValue::Bytes(data))) => {
                    p.nat.objs.next_read_stream_handle = p.nat.objs.next_read_stream_handle.wrapping_add(1).max(1);
                    let handle = p.nat.objs.next_read_stream_handle;
                    p.nat.objs.read_streams.insert(handle, ReadStreamState { data, pos: 0 });
                    let _ = p.wr(r0, handle);
                    if name.contains("OpenLC") {
                        p.nat.cleanup.push(crate::cleanup::CleanupItem::Any(0));
                    }
                    NativeOutcome::Return
                }
                Ok(Some(_)) => leave_or_refuse(p, name, KERR_ARGUMENT, format!("prefs key {key} is not Bytes")),
                Ok(None) => leave_or_refuse(p, name, KERR_NOT_FOUND, format!("dictionary stream {key} not found")),
                Err(e) => leave_or_refuse(p, name, svc_err_to_symbian(e), format!("prefs get {key}: {}", e.name())),
            }
        }
        // void RReadStream::ReadL(TDes8& aDes) / ReadL(TDes8& aDes, TInt aLength)
        "_ZN11RReadStream5ReadLER5TDes8" | "_ZN11RReadStream5ReadLER5TDes8i" => {
            let handle = p.rd(r0).unwrap_or(0);
            let want_len = if name.ends_with("TDes8i") {
                Some((r2 as i32).max(0) as usize)
            } else {
                None
            };
            let max = match desc::locate(p, r1) {
                Ok(v) => match v.max {
                    Some(m) => m as usize,
                    None => return Some(refuse(format!("descriptor at {r1:#x} is not modifiable"))),
                },
                Err(m) => return Some(refuse(m)),
            };
            let slice_out = match p.nat.objs.read_streams.get_mut(&handle) {
                Some(st) => {
                    let avail = st.data.len().saturating_sub(st.pos);
                    let take = match want_len {
                        Some(n) if n > avail => return Some(leave_or_refuse(p, name, KERR_EOF, String::from("read past end of stream"))),
                        Some(n) => n.min(max),
                        None => avail.min(max),
                    };
                    let out = st.data[st.pos..st.pos + take].to_vec();
                    st.pos += take;
                    out
                }
                None => return Some(refuse(format!("RReadStream handle {handle} is not open"))),
            };
            match desc::write8(p, r1, &slice_out) {
                Ok(()) => NativeOutcome::Return,
                Err(m) => refuse(m),
            }
        }
        // TInt32 RReadStream::ReadInt32L() / TUint32 RReadStream::ReadUint32L()
        "_ZN11RReadStream10ReadInt32LEv" | "_ZN11RReadStream11ReadUint32LEv" => {
            let handle = p.rd(r0).unwrap_or(0);
            match p.nat.objs.read_streams.get_mut(&handle) {
                Some(st) if st.pos + 4 <= st.data.len() => {
                    let v = u32::from_le_bytes(st.data[st.pos..st.pos + 4].try_into().unwrap());
                    st.pos += 4;
                    p.cpu.regs[0] = v;
                    NativeOutcome::Return
                }
                Some(_) => leave_or_refuse(p, name, KERR_EOF, String::from("read past end of stream")),
                None => refuse(format!("RReadStream handle {handle} is not open")),
            }
        }
        // void RReadStream::Close() / Release()
        "_ZN11RReadStream5CloseEv" | "_ZN11RReadStream7ReleaseEv" => {
            let handle = p.rd(r0).unwrap_or(0);
            p.nat.objs.read_streams.remove(&handle);
            let _ = p.wr(r0, 0);
            NativeOutcome::Return
        }
        // The destructors: the deleting one (D0) also frees the block.
        "_ZN20CDictionaryFileStoreD1Ev" | "_ZN20CDictionaryFileStoreD0Ev" => {
            p.obj_remove(r0);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() {
                    let _ = h.free(r0);
                }
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
