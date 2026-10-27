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
/// (eikenv.h:~270-318); no field is read or written by this engine yet, so the block is a generous upper bound and a guest read
/// through a field gets zero. DELETE WHEN a measured call reads a CEikonEnv field (then size it from the header and name the offset).
pub const CEIKON_ENV_BYTES: u32 = 0x400;
/// `CCoeAppUi::iCoeEnv`: the first data member after the CBase vptr (coeaui.h:226).
pub const APPUI_I_COE_ENV: u32 = 4;
/// The S60 5th edition portrait application rectangle (nHD, 360x640): the window the framework gives an application.
pub const S60_5TH_APP_RECT: GuestRect = GuestRect { left: 0, top: 0, width: 360, height: 640 };

/// `RHandleBase::iHandle` of an `RApaLsSession` after `Connect()`: the placeholder session (see the native).
pub const APA_SESSION_PLACEHOLDER_HANDLE: u32 = 1;

const REG_DIR: &str = "\\private\\10003a3f\\import\\apps";

/// The caption the framework shows for the application whose UID3 is `uid3`: the registration resource names the localisable
/// resource file and id (appinfo.rh:83-92 `APP_REGISTRATION_INFO`: reserved LONG, LLINK, `app_file`, `attributes`,
/// `localisable_resource_file`, `localisable_resource_id`), and that resource's `caption_and_icon.caption` is the caption
/// (appinfo.rh:176-202 `LOCALISABLE_APP_INFO`, `CAPTION_AND_ICON_INFO`). Every failure is a message naming what was missing.
pub fn app_caption(drive: &Drive, uid3: u32) -> Result<String, String> {
    let names = drive.list(REG_DIR).map_err(|e| format!("no registration directory {REG_DIR} on the drive ({e:?})"))?;
    for name in names.iter().filter(|n| n.to_ascii_lowercase().ends_with("_reg.rsc")) {
        let path = format!("{REG_DIR}\\{name}");
        let bytes = drive.read(&path).map_err(|e| format!("{path}: {e:?}"))?;
        let reg = RscFile::parse(bytes).map_err(|e| format!("{path}: {e:?}"))?;
        if reg.uid3 != uid3 {
            continue;
        }
        let res = reg.resource(1).map_err(|e| format!("{path}: resource 1: {e:?}"))?;
        let mut r = Reader::new(&res);
        let field = |e| format!("{path}: APP_REGISTRATION_INFO: {e:?}");
        r.long().map_err(field)?;
        r.long().map_err(field)?;
        let _app_file = r.ltext().map_err(field)?;
        r.long().map_err(field)?; // attributes
        let file = r.ltext().map_err(field)?;
        let id = r.long().map_err(field)?;
        if file.is_empty() {
            return Err(format!("{path}: the registration names no localisable resource file, so there is no caption to read"));
        }
        let loc_path = format!("{file}.rsc");
        let loc_bytes = drive.read(&loc_path).map_err(|e| format!("localisable resource file {loc_path} (from {path}): {e:?}"))?;
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
        return Ok(caption);
    }
    Err(format!("no *_reg.rsc under {REG_DIR} has UID3 {uid3:#010x}"))
}

impl Process {
    /// The guest `CEikonEnv` singleton, allocated on first use.
    fn coe_env(&mut self) -> Result<u32, String> {
        if let Some(e) = self.host.env {
            return Ok(e);
        }
        let e = self.alloc_zeroed(CEIKON_ENV_BYTES).ok_or_else(|| String::from("no guest heap for the CEikonEnv singleton"))?;
        self.host.env = Some(e);
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
