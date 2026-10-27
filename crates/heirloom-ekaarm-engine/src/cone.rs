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
    let loc_path = format!("{}.rsc", reg.localisable_file);
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

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    Some(match name {
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
        // RWsSession::ComputeMode(TComputeMode) (w32std.h:1616): a hint that lets the window server boost the client's priority
        // when its window group gains focus (w32std.h:1468-1473). One thread, no scheduler priority to move: nothing to do.
        "_ZN10RWsSession11ComputeModeENS_12TComputeModeE" => NativeOutcome::Return,
        _ => return None,
    })
}
