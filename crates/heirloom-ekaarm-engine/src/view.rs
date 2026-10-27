//! `CAknView` natives (aknview.h): the Avkon view the application derives its screens from.
//!
//! Layout (ARM EABI, vptr first, bases in declaration order `CBase, MCoeView, MEikMenuObserver, MObjectProvider`):
//! the imports `_ZThn4_NK8CAknView6ViewIdEv`, `_ZThn12_N8CAknView15ProcessCommandLEi` and `_ZThn24_N8CAknView15MopSupplyObject...`
//! confirm the three secondary bases at 4, 12 and 24.
use alloc::format;
use alloc::string::String;
use heirloom_fmt_rsc::Reader;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::{HostObj, ViewState};

pub const VIEW_MCOE_VIEW: u32 = 4; // MCoeView: vptr + iMCoeView_Reserved1 (coeview.h:43) = 8 bytes
pub const VIEW_MEIK_MENU_OBSERVER: u32 = 12; // vptr, MEikCommandObserver reserved (eikcmobs.h), iMEikMenuObserver_Spare1 (eikmobs.h:68) = 12 bytes
pub const VIEW_MOBJECT_PROVIDER: u32 = 24; // vptr + iMObjectProvider_Reserved1 (coemop.h:167) = 8 bytes
pub const VIEW_I_COE_ENV: u32 = 32; // aknview.h:~line "CCoeEnv* iCoeEnv"
pub const VIEW_I_APP_UI: u32 = 36; // CAknViewAppUi* iAppUi
/// `SAknViewInfo iViewInfo` (aknview.h): iVersion (TUint8) at 40, three bytes padding, then four TInt.
pub const VIEW_I_VIEW_INFO: u32 = 40;
pub const VIEWINFO_VERSION: u32 = 40;
pub const VIEWINFO_HOT_KEYS: u32 = 44;
pub const VIEWINFO_MENU: u32 = 48;
pub const VIEWINFO_CBA: u32 = 52;
pub const VIEWINFO_TOOLBAR: u32 = 56;
pub const VIEWINFO_EXTENSION: u32 = 60;
pub const VIEW_I_MENU: u32 = 64; // CEikMenuBar*
pub const VIEW_I_CBA: u32 = 68; // CEikButtonGroupContainer*
pub const VIEW_I_FLAGS: u32 = 72; // TBitFlags
pub const VIEW_I_EXTENSION: u32 = 76; // CAknViewExtension*
pub const CAKN_VIEW_SIZE: u32 = 80;

impl Process {
    fn write_words(&mut self, base: u32, words: &[(u32, u32)]) -> Result<(), String> {
        for &(off, v) in words {
            self.mem.vm.write_u32(base.wrapping_add(off), v).map_err(|_| format!("the object at {base:#x} is unwritable (offset {off:#x})"))?;
        }
        Ok(())
    }

    /// `CAknView::CAknView()`: the base part zeroed (the derived constructor writes the vptrs), `iCoeEnv` = the environment.
    fn view_construct(&mut self, this: u32) -> Result<(), String> {
        let env = self.coe_env()?;
        for off in (4..CAKN_VIEW_SIZE).step_by(4) {
            self.write_words(this, &[(off, 0)])?;
        }
        self.write_words(this, &[(VIEW_I_COE_ENV, env)])?;
        self.obj_set(this, HostObj::View(ViewState::default()))
    }

    /// `CAknView::BaseConstructL(TInt aResId)`: read the view's `AVKON_VIEW` resource into `iViewInfo`. The menu bar and the
    /// button group it names are NOT constructed here.
    // DELETE WHEN a view's menu bar or CBA is shown on the host (then build `iMenu` / `iCba` from `iViewInfo.iMenu` / `.iCba`).
    fn view_base_construct(&mut self, this: u32, res_id: u32) -> Result<(), String> {
        // iAppUi = the environment's AppUi (`iCoeEnv->AppUi()`, coemain.h:55), the view's owner.
        let env = self.coe_env()?;
        let appui = self.mem.vm.read_u32(env.wrapping_add(crate::cone::COEENV_I_APPUI)).map_err(|_| format!("the CEikonEnv at {env:#x} is unreadable"))?;
        self.write_words(this, &[(VIEW_I_APP_UI, appui)])?;
        let info = if res_id == 0 { None } else { Some(self.app_resource(res_id)?) };
        let (mut version, mut hot, mut menu, mut cba, mut toolbar, mut ext) = (0, 0, 0, 0, 0, 0);
        if let Some(bytes) = &info {
            let mut r = Reader::new(bytes);
            let field = |e| format!("AVKON_VIEW resource {res_id:#x}: {e:?}");
            // AVKON_VIEW (avkon.rh, not in the SDK include tree): version BYTE, hotkeys, menubar, cba, toolbar LLINKs; the extension
            // LLINK only when the resource has it (SAknViewInfo, aknview.h, lists the same six fields).
            version = r.byte().map_err(field)? as u32;
            hot = r.long().map_err(field)?;
            menu = r.long().map_err(field)?;
            cba = r.long().map_err(field)?;
            toolbar = r.long().map_err(field)?;
            if r.at + 4 <= bytes.len() {
                ext = r.long().map_err(field)?;
            }
        }
        self.write_words(this, &[(VIEWINFO_VERSION, version), (VIEWINFO_HOT_KEYS, hot), (VIEWINFO_MENU, menu), (VIEWINFO_CBA, cba), (VIEWINFO_TOOLBAR, toolbar), (VIEWINFO_EXTENSION, ext)])?;
        if let Some(HostObj::View(v)) = self.obj_get(this) {
            v.resource_id = res_id;
        }
        self.log.push(format!("[view] CAknView::BaseConstructL({res_id:#x}) on {this:#x}: menu {menu:#x} cba {cba:#x}"));
        Ok(())
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    let r1 = p.cpu.regs[1];
    let done = |p: &mut Process, r: Result<(), String>| match r {
        Ok(()) => { p.cpu.regs[0] = this; NativeOutcome::Return }
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}")),
    };
    Some(match name {
        "_ZN8CAknViewC1Ev" | "_ZN8CAknViewC2Ev" => { let r = p.view_construct(this); done(p, r) }
        // The destructor owns nothing yet (no menu bar or CBA is built, see BaseConstructL).
        "_ZN8CAknViewD1Ev" | "_ZN8CAknViewD2Ev" => done(p, Ok(())),
        "_ZN8CAknView14BaseConstructLEi" => { let r = p.view_base_construct(this, r1); done(p, r) }
        _ => return None,
    })
}
