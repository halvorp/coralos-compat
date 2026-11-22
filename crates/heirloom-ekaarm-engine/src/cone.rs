//! The UI-framework natives of the application start: the `CCoeEnv` singleton and `BaseConstructL`, which reads the
//! application's resources off the drive and opens its window through the UI seam (`ui.rs`). Facts are from the S60 5th
//! edition SDK headers (`epoc32/include`, EPL), each with header and line.
use alloc::format;
use alloc::string::String;
use heirloom_fmt_rsc::{Reader, RscFile};
use heirloom_ui_abi::{CallbackToken, GuestRect, WindowModel, WindowRole};
use crate::drive::Drive;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// Bytes allocated (zeroed, vtable pointer 0) for the `CEikonEnv` singleton. NOT a computed `sizeof`: CEikonEnv derives CCoeEnv
/// (coemain.h:~395-467, itself CActive-derived with four mixin bases and RWsSession/RFs/TWsEvent members) and adds ~60 fields
/// (eikenv.h:~270-318). The fields this engine writes are named where they are written (COEENV_I_APPUI here, COEENV_I_SCREEN in
/// screen.rs, both read off the app's own inline code); the rest stay zero, so the block is a generous upper bound, not a sizeof.
/// DELETE WHEN a guest reads a field past 0x400 or one the header places differently (then size it from the header).
pub const CEIKON_ENV_BYTES: u32 = 0x400;
/// `CCoeAppUi::iCoeEnv`: the first data member after the CBase vptr (coeaui.h:226).
pub const APPUI_I_COE_ENV: u32 = 4;
/// `CCoeEnv::iAppUi` (coemain.h:203): CCoeEnv derives CActive (vptr, `iStatus` 8, `iLink` 12, `iSpare` = 28 bytes, e32base.h:1700-1710) then
/// MObjectProvider (vptr + reserved = 8 bytes, coemop.h:167), so the first data member is at 36. Confirmed against the app's own inline
/// `CCoeEnv::AppUi()`: `ldr r0,[appui,#4]; ldr r0,[r0,#0x24]`.
pub const COEENV_I_APPUI: u32 = 0x24;
/// The S60 5th edition portrait application rectangle (nHD, 360x640): the window the framework gives an application.
pub const S60_5TH_APP_RECT: GuestRect = GuestRect { left: 0, top: 0, width: 360, height: 640 };

/// `RHandleBase::iHandle` of an `RApaLsSession` after `Connect()`: the placeholder session (see the native).
pub const APA_SESSION_PLACEHOLDER_HANDLE: u32 = 1;

/// The caption the framework shows for the application whose UID3 is `uid3`: the registration resource names the localisable
/// resource file and id (appinfo.rh:83-92 `APP_REGISTRATION_INFO`: reserved LONG, LLINK, `app_file`, `attributes`,
/// `localisable_resource_file`, `localisable_resource_id`), and that resource's `caption_and_icon.caption` is the caption
/// (appinfo.rh:176-202 `LOCALISABLE_APP_INFO`, `CAPTION_AND_ICON_INFO`). Every failure is a message naming what was missing.
pub fn app_caption(drive: &Drive, uid3: u32) -> Result<String, String> {
    let reg = crate::resource::registration(drive, uid3)?;
    if reg.localisable_file.is_empty() {
        return Err(format!("{}: the registration names no localisable resource file, so there is no caption to read", reg.path));
    }
    let id = reg.localisable_id;
    let loc_path = crate::resource::resolve_rsc_path(drive, &reg.localisable_file);
    let loc_bytes = drive.read(&loc_path).map_err(|e| format!("localisable resource file {loc_path} (from {}): {e:?}", reg.path))?;
    let loc = RscFile::parse(loc_bytes).map_err(|e| format!("{loc_path}: {e:?}"))?;
    let info = loc.resource(id).map_err(|e| format!("{loc_path}: resource {id:#x}: {e:?}"))?;
    let mut r = Reader::new(&info);
    let field = |e| format!("{loc_path}: LOCALISABLE_APP_INFO {id:#x}: {e:?}");
    r.long().map_err(field)?;
    r.long().map_err(field)?;
    let _short = r.ltext().map_err(field)?;
    r.long().map_err(field)?;
    r.long().map_err(field)?;
    let caption = r.ltext().map_err(field)?;
    if caption.is_empty() {
        return Err(format!("{loc_path}: resource {id:#x} has an empty caption"));
    }
    Ok(caption)
}

impl Process {
    /// The guest `CEikonEnv` singleton, allocated on first use.
    pub(crate) fn coe_env(&mut self) -> Result<u32, String> {
        if let Some(e) = self.host.env {
            return Ok(e);
        }
        let e = self.alloc_zeroed(CEIKON_ENV_BYTES).ok_or_else(|| String::from("no guest heap for the CEikonEnv singleton"))?;
        self.host.env = Some(e);
        self.make_screen_device(e)?;
        let fs_handle = self.fs_connect();
        // CCoeEnv::ConstructL connects the environment's file server session (coemain.h:204).
        self.mem.vm.write_u32(e.wrapping_add(crate::fs::COEENV_I_FS_SESSION), fs_handle).map_err(|_| String::from("the CEikonEnv is unwritable"))?;
        // CEikonEnv::InitColorSchemeL (EIKENV.CPP:2966): the colour list the app's inline `ColorList()` reads.
        let colors = self.make_color_list()?;
        self.wr(e.wrapping_add(crate::colorlist::EIKENV_I_COLOR_LIST), colors)?;
        // CCoeEnv is the window server's event active object: in the scheduler's queue with its first event request out (`wsevent.rs`).
        self.env_as_active_object(e)?;
        Ok(e)
    }

    /// `CAknAppUi::BaseConstructL` / `CAknViewAppUi::BaseConstructL(TInt aAppUiFlags)` (aknappui.h, aknviewappui.h): bind the
    /// AppUi to the environment (`iCoeEnv`) and open the application window titled with the caption its resources give.
    fn base_construct_l(&mut self, name: &str) -> Result<(), String> {
        let appui = self.cpu.regs[0];
        let env = self.coe_env()?;
        if self.host.window.is_some() {
            return Err(String::from("a second BaseConstructL in one process: one application window is all this engine models"));
        }
        let drive = self.host.drive.as_ref().ok_or_else(|| String::from("no drive: the application's resource files cannot be read, so there is no caption (run with --drive)"))?;
        let caption = app_caption(drive, self.uid3)?;
        let ui = self.host.ui.as_mut().ok_or_else(|| String::from("no GuestUi: there is nowhere to open the application window"))?;
        self.mem.vm.write_u32(appui.wrapping_add(APPUI_I_COE_ENV), env).map_err(|_| format!("the AppUi at {appui:#x} is unwritable"))?;
        // CCoeEnv::SetAppUi (coemain.h:45): the environment's current AppUi, which the app's inline `CCoeEnv::AppUi()` reads.
        self.mem.vm.write_u32(env.wrapping_add(COEENV_I_APPUI), appui).map_err(|_| format!("the CEikonEnv at {env:#x} is unwritable"))?;
        let model = WindowModel { title: caption.clone(), rect: S60_5TH_APP_RECT, role: WindowRole::Document, closable: true, resizable: false, token: CallbackToken(appui as u64) };
        let win = ui.open_window(model).map_err(|e| format!("the UI refused the window: {e}"))?;
        self.host.window = Some(win);
        self.log.push(format!("[cone] {name}: window \"{caption}\" {}x{}", S60_5TH_APP_RECT.width, S60_5TH_APP_RECT.height));
        Ok(())
    }
}

/// Read the raw resource bytes held by a `TResourceReader` built by `CCoeEnv::CreateResourceReaderLC` (`+0 = ptr, +4 = end`).
pub(crate) fn read_resource_reader(p: &Process, reader_addr: u32) -> Result<alloc::vec::Vec<u8>, String> {
    let start = p.mem.vm.read_u32(reader_addr).map_err(|_| format!("TResourceReader at {reader_addr:#x} is unreadable"))?;
    let end = p.mem.vm.read_u32(reader_addr.wrapping_add(4)).map_err(|_| format!("TResourceReader at {reader_addr:#x}+4 is unreadable"))?;
    if end < start || end - start > 0x10_0000 {
        return Err(format!("TResourceReader at {reader_addr:#x} has invalid span {start:#x}..{end:#x}"));
    }
    let mut buf = alloc::vec![0u8; (end - start) as usize];
    if !buf.is_empty() {
        p.mem.vm.read_bytes(start, &mut buf).map_err(|_| format!("TResourceReader buffer at {start:#x} is unreadable"))?;
    }
    Ok(buf)
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    Some(match name {
        // void CCoeEnv::CreateResourceReaderLC(TResourceReader& aReader, TInt aResourceId) const (coemain.h):
        // allocates a guest buffer with the resource bytes, pushes it on the cleanup stack, and points aReader at it.
        "_ZNK7CCoeEnv22CreateResourceReaderLCER15TResourceReaderi" => {
            let (reader, res_id) = (p.cpu.regs[1], p.cpu.regs[2]);
            let bytes = match p.resource(res_id) {
                Ok(b) => b,
                Err(m) => return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}"))),
            };
            let len = bytes.len() as u32;
            let Some(buf) = p.alloc_zeroed(len.max(1)) else {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: out of heap for resource {res_id:#x}")));
            };
            if !bytes.is_empty() && p.mem.vm.write_bytes(buf, &bytes).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: buffer at {buf:#x} is unwritable")));
            }
            if p.mem.vm.write_u32(reader, buf).is_err() || p.mem.vm.write_u32(reader.wrapping_add(4), buf.wrapping_add(len)).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: TResourceReader at {reader:#x} is unwritable")));
            }
            p.nat.cleanup.push(crate::cleanup::CleanupItem::Any(buf));
            NativeOutcome::Return
        }
        // CDesC16ArrayFlat* CCoeEnv::ReadDesC16ArrayResourceL(TInt aResourceId) (coemain.h):
        // reads an ARRAY of LTEXT items from the resource into a newly allocated CDesC16ArrayFlat.
        "_ZN7CCoeEnv24ReadDesC16ArrayResourceLEi" => {
            let res_id = p.cpu.regs[1];
            let res = (|| -> Result<u32, String> {
                let bytes = p.resource(res_id)?;
                let mut r = heirloom_fmt_rsc::Reader::new(&bytes);
                let count = r.word().map_err(|e| format!("resource {res_id:#x}: {e:?}"))? as usize;
                let mut items = alloc::vec::Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(r.ltext().map_err(|e| format!("resource {res_id:#x}: {e:?}"))?);
                }
                let arr = p.alloc_zeroed(crate::listmodel::DESARR_SIZE)
                    .ok_or_else(|| String::from("out of heap for CDesC16ArrayFlat"))?;
                p.desarr_construct(arr, (count as u32).max(4))?;
                for item in &items {
                    let units: alloc::vec::Vec<u16> = item.encode_utf16().collect();
                    p.desarr_append_units(arr, &units)?;
                }
                Ok(arr)
            })();
            match res {
                Ok(arr) => { p.cpu.regs[0] = arr; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
            }
        }
        // void CCoeEnv::ReadResourceAsDes16(TDes16& aDes, TInt aResourceId) const (coemain.h):
        "_ZNK7CCoeEnv19ReadResourceAsDes16ER6TDes16i" | "_ZN7CCoeEnv19ReadResourceAsDes16ER6TDes16i" => {
            let (des, res_id) = (p.cpu.regs[1], p.cpu.regs[2]);
            let r = p.resource(res_id).and_then(|bytes| {
                let text = heirloom_fmt_rsc::Reader::new(&bytes).rest_utf16().map_err(|e| format!("resource {res_id:#x}: {e:?}"))?;
                let units: alloc::vec::Vec<u16> = text.encode_utf16().collect();
                crate::desc::write16(p, des, &units)
            });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")) }
        }
        // static HBufC16* StringLoader::LoadLC(TInt aResourceId, CCoeEnv* aLoaderEnv) / LoadL (StringLoader.h),
        // and HBufC16* CCoeEnv::AllocReadResourceAsDes16LC(TInt aResourceId) const / AllocReadResourceAsDes16L (coemain.h):
        // reads a TBUF resource into a freshly allocated HBufC16 (pushing onto CleanupStack for *LC).
        "_ZN12StringLoader6LoadLCEiP7CCoeEnv" | "_ZN12StringLoader5LoadLEiP7CCoeEnv"
        | "_ZNK7CCoeEnv26AllocReadResourceAsDes16LCEi" | "_ZNK7CCoeEnv25AllocReadResourceAsDes16LEi"
        | "_ZNK7CCoeEnv19AllocReadResourceLCEi" | "_ZNK7CCoeEnv18AllocReadResourceLEi" => {
            let is_member = name.starts_with("_ZNK7CCoeEnv");
            let res_id = if is_member { p.cpu.regs[1] } else { p.cpu.regs[0] };
            let push = name.contains("LCE");
            let res = p.resource(res_id).and_then(|bytes| {
                let text = heirloom_fmt_rsc::Reader::new(&bytes).rest_utf16().map_err(|e| format!("resource {res_id:#x}: {e:?}"))?;
                let units: alloc::vec::Vec<u16> = text.encode_utf16().collect();
                let hbuf = p.alloc_zeroed(4 + units.len() as u32 * 2)
                    .ok_or_else(|| String::from("out of heap for HBufC16"))?;
                p.wr(hbuf, units.len() as u32)?; // DES_BUF_C (type 0)
                for (i, u) in units.iter().enumerate() {
                    p.mem.vm.write_u16(hbuf + 4 + i as u32 * 2, *u).map_err(|_| String::from("HBufC16 unwritable"))?;
                }
                if push {
                    p.nat.cleanup.push(crate::cleanup::CleanupItem::Any(hbuf));
                }
                Ok(hbuf)
            });
            match res {
                Ok(hbuf) => { p.cpu.regs[0] = hbuf; NativeOutcome::Return }
                Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
            }
        }
        // static void CEikonEnv::InfoWinL(const TDesC16& aFirstLine, const TDesC16& aSecondLine) (eikenv.h):
        // non-blocking informational banner; static overload takes (r0, r1), const member overload takes (this, r1, r2).
        "_ZN9CEikonEnv8InfoWinLERK7TDesC16S2_" | "_ZNK9CEikonEnv8InfoWinLERK7TDesC16S2_" => {
            let (a1, a2) = if name.starts_with("_ZNK") {
                (p.cpu.regs[1], p.cpu.regs[2])
            } else {
                (p.cpu.regs[0], p.cpu.regs[1])
            };
            let l1 = crate::desc::read16(p, a1).map(|u| crate::desc::to_utf8(&u)).unwrap_or_default();
            let l2 = crate::desc::read16(p, a2).map(|u| crate::desc::to_utf8(&u)).unwrap_or_default();
            p.log.push(format!("[eikcore] InfoWinL \"{l1}\" / \"{l2}\""));
            NativeOutcome::Return
        }
        // CCoeEnv::Static(): the process-wide environment (coemain.h `static inline CCoeEnv* Static()`), the singleton.
        "_ZN7CCoeEnv6StaticEv" => match p.coe_env() {
            Ok(e) => { p.cpu.regs[0] = e; NativeOutcome::Return }
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone/182 {name}: {m}")),
        },
        "_ZN9CAknAppUi14BaseConstructLEi" | "_ZN13CAknViewAppUi14BaseConstructLEi" => match p.base_construct_l(name) {
            Ok(()) => NativeOutcome::Return,
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}")),
        },
        // RApaLsSession::Connect() (apgcli.h:123): connect to the application-architecture server. This engine has none; the
        // session gets a non-zero handle (RHandleBase::iHandle, word 0, e32std.h:4100) naming that placeholder and KErrNone is
        // returned. Every method on the session is its own import and is refused by name, so nothing runs against the placeholder.
        // DELETE WHEN a session method is the next refusal of a corpus app (then the apparc client talks to a real registry).
        "_ZN13RApaLsSession7ConnectEv" => {
            let this = p.cpu.regs[0];
            if p.mem.vm.write_u32(this, APA_SESSION_PLACEHOLDER_HANDLE).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED apgrfx/25 {name}: session object at {this:#x} is unwritable")));
            }
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // RApaLsSession::Close() (apgcli.h): release the session handle.
        "_ZN13RApaLsSession5CloseEv" => {
            let this = p.cpu.regs[0];
            if p.mem.vm.write_u32(this, 0).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED apgrfx {name}: session object at {this:#x} is unwritable")));
            }
            NativeOutcome::Return
        }
        // TInt RApaLsSession::ApplicationLanguage(TUid aAppUid, TLanguage& aLanguage) const (apgcli.h):
        // report the engine's default locale (ELangEnglish = 1, e32lang.h).
        "_ZNK13RApaLsSession19ApplicationLanguageE4TUidR9TLanguage" => {
            let out_lang = p.cpu.regs[2];
            if p.mem.vm.write_u32(out_lang, 1).is_err() {
                return Some(NativeOutcome::Refused(format!("NEXT-NEEDED apgrfx {name}: language out-param at {out_lang:#x} is unwritable")));
            }
            p.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // RWsSession::ComputeMode(TComputeMode) (w32std.h:1616): a hint that lets the window server boost the client's priority
        // when its window group gains focus (w32std.h:1468-1473). One thread, no scheduler priority to move: nothing to do.
        "_ZN10RWsSession11ComputeModeENS_12TComputeModeE" => NativeOutcome::Return,
        _ => return None,
    })
}
