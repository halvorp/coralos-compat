//! The fixed toolbar of a touch AppUi (`CAknToolbar`, akntoolbar.h). The device profile is touch (`AknLayoutUtils::PenEnabled`), and an
//! application there addresses `CAknAppUi::CurrentFixedToolbar()` without testing it, so the AppUi owns one. Its items are the buttons
//! of the application's own toolbar: `CEikAppUiFactory::ReadAppInfoResourceL` reads the `EIK_APP_INFO` resource (the file's third,
//! `EAppInfoResourceOffset`, uikon.hrh), its extension's `popup_toolbar` (EIKCOLIB.CPP `SEikAppInfoExtension`) and, when that
//! `AVKON_TOOLBAR` has `KAknToolbarFixed` (eikon.hrh:1332), keeps it as the fixed toolbar. Each item is a guest `CAknButton` that
//! `ControlOrNull(id)` returns; what the guest sets is recorded. Nothing is drawn: DELETE WHEN the host draws a toolbar (then give it
//! the items' states: text, help text, icons).
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_rsc::Reader;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::ToolbarState;

const TOOLBAR_BYTES: u32 = 0x40;
const VTABLE: &[(usize, &str)] = &[(0, "_ZN11CAknToolbarD1Ev"), (1, "_ZN11CAknToolbarD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")];

/// `EAppInfoResourceOffset` (uikon.hrh `TEikAppResourceFileStandardOffset`: signature 1, name 2, app info 3).
const APP_INFO_RESOURCE: u32 = 3;
/// `EIK_APP_INFO` (eikon.rh; the factory's `SEikAppInfo`): hotkeys, menubar, toolbar, toolband, cba, status_pane, extension: 7 LLINKs.
const APP_INFO_EXTENSION_INDEX: usize = 6;
/// `KAknToolbarFixed` (eikon.hrh:1332).
const KAKN_TOOLBAR_FIXED: u32 = 0x80;
/// `EAknCtButton` (avkon.hrh:554): the only control type a toolbar item of this engine may be.
const EAKN_CT_BUTTON: u16 = 0x168a;
/// `CAknButton`'s first own virtual, `SetCurrentState(TInt aStateIndex, TBool aDrawNow)` (aknbutton.h:689): slot 45 after CCoeControl's 44
/// and CAknControl's `ExtensionInterface` (44). CONFIRMED by FileBrowser's `ldr r3,[vtbl+0x80+0x34]` called with (0, 1).
const BUTTON_OVERRIDES: &[(usize, &str)] = &[(44, "_ZN11CAknControl18ExtensionInterfaceE4TUid"), (45, "_ZN10CAknButton15SetCurrentStateEii")];

/// The item ids of an `AVKON_TOOLBAR` (eikon.rh:1160): version BYTE, lines BYTE, flags LONG, softkeys LLINK, `items[]` of `TBAR_CTRL`
/// (type WORD, id WORD, flags WORD, length WORD, then the control: `AVKON_BUTTON` = flags WORD, state WORD, `states[]` of
/// `AVKON_BUTTON_STATE`, extension LLINK), extension LLINK. Returns (flags, ids).
fn parse_toolbar(bytes: &[u8]) -> Result<(u32, Vec<u32>), String> {
    let mut r = Reader::new(bytes);
    let e = |e: heirloom_fmt_rsc::RscError| format!("AVKON_TOOLBAR: {e:?}");
    r.byte().map_err(e)?;
    r.byte().map_err(e)?;
    let flags = r.long().map_err(e)?;
    r.long().map_err(e)?;
    let mut ids = Vec::new();
    for _ in 0..r.word().map_err(e)? {
        let (ty, id) = (r.word().map_err(e)?, r.word().map_err(e)?);
        for _ in 0..2 { r.word().map_err(e)?; } // flags, length
        if ty != EAKN_CT_BUTTON {
            return Err(format!("AVKON_TOOLBAR item {id} has control type {ty:#x}, not EAknCtButton: only buttons are built"));
        }
        for _ in 0..2 { r.word().map_err(e)?; } // the button's flags, state
        for _ in 0..r.word().map_err(e)? {
            r.word().map_err(e)?; // state flags
            for _ in 0..3 { r.ltext().map_err(e)?; } // txt, helptxt, bmpfile
            for _ in 0..8 { r.word().map_err(e)?; } // the four bitmap / mask pairs
            r.long().map_err(e)?; // state extension
        }
        r.long().map_err(e)?; // button extension
        ids.push(u32::from(id));
    }
    Ok((flags, ids))
}

impl Process {
    /// The application's toolbar resource id: `EIK_APP_INFO`'s extension's `popup_toolbar` (0 = the application names none).
    fn app_toolbar_resource(&mut self) -> Result<u32, String> {
        let info = self.app_resource(APP_INFO_RESOURCE)?;
        let mut r = Reader::new(&info);
        let mut ext = 0;
        for _ in 0..=APP_INFO_EXTENSION_INDEX { ext = r.long().map_err(|e| format!("EIK_APP_INFO: {e:?}"))?; }
        if ext == 0 { return Ok(0); }
        let ext = self.resource(ext)?;
        let mut r = Reader::new(&ext);
        r.long().map_err(|e| format!("EIK_APP_INFO_EXT: {e:?}"))?; // touch_pane
        r.long().map_err(|e| format!("EIK_APP_INFO_EXT: {e:?}")) // popup_toolbar
    }

    fn fixed_toolbar(&mut self, appui: u32) -> Result<u32, String> {
        if let Some(t) = self.appui_state(appui)?.toolbar {
            return Ok(t);
        }
        let t = self.alloc_zeroed(TOOLBAR_BYTES).ok_or_else(|| String::from("no guest heap for the toolbar"))?;
        let vptr = self.build_vtable("CAknToolbar", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(t, vptr).map_err(|_| String::from("toolbar unwritable"))?;
        let mut items = Vec::new();
        let res = self.app_toolbar_resource()?;
        if res != 0 {
            let (flags, ids) = parse_toolbar(&self.resource(res)?)?;
            if flags & KAKN_TOOLBAR_FIXED == 0 {
                return Err(format!("the application's toolbar {res:#x} is a popup toolbar (flags {flags:#x}): no popup toolbar is built, and a fixed one would be the default r_aknpriv_toolbar, which is not read"));
            }
            for id in ids {
                items.push((id, self.make_control("CAknButton", 0x100, BUTTON_OVERRIDES)?));
            }
        } else {
            return Err(String::from("the application names no toolbar: the default fixed toolbar (r_aknpriv_toolbar) is not read"));
        }
        self.control_state(t)?.toolbar = Some(ToolbarState { visible: true, items, ..Default::default() });
        self.appui_state(appui)?.toolbar = Some(t);
        Ok(t)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED eikcoctl {name}: {m}"));
    if name == "_ZNK9CAknAppUi19CurrentFixedToolbarEv" {
        return Some(match p.fixed_toolbar(this) { Ok(t) => { p.cpu.regs[0] = t; NativeOutcome::Return } Err(m) => refuse(m) });
    }
    // void CAknButton::SetCurrentState(TInt aStateIndex, TBool aDrawNow): nothing is drawn, and the one state each item has is current.
    // DELETE WHEN the host draws a toolbar.
    if name == "_ZN10CAknButton15SetCurrentStateEii" { return Some(NativeOutcome::Return); }
    if !name.contains("11CAknToolbar") && !name.contains("CAknToolbar") { return None; }
    let st = match p.control_state(this) { Ok(c) => match c.toolbar.as_mut() { Some(t) => t, None => return Some(refuse(format!("{this:#x} is not a toolbar"))) }, Err(m) => return Some(refuse(m)) };
    Some(match name {
        "_ZN11CAknToolbar18SetToolbarObserverEP19MAknToolbarObserver" => { st.observer = r1; NativeOutcome::Return }
        "_ZN11CAknToolbar20SetToolbarVisibilityEi" => { st.visible = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar15DisableToolbarLEi" => { st.disabled = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar14SetWithSlidingEi" => { st.sliding = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar21SetCloseOnAllCommandsEi" => { st.close_on_all = r1 != 0; NativeOutcome::Return }
        // HideItem(TInt aCommandId, TBool aHide, TBool aDraw): nothing is drawn, so hiding is not shown. DELETE WHEN the host draws a toolbar.
        "_ZN11CAknToolbar8HideItemEiii" => NativeOutcome::Return,
        // CCoeControl* ControlOrNull(TInt aId) const: the item's button, or NULL when the toolbar has no item of that id.
        "_ZNK11CAknToolbar13ControlOrNullEi" => { p.cpu.regs[0] = st.items.iter().find(|(id, _)| *id == r1).map_or(0, |(_, c)| *c); NativeOutcome::Return }
        // TInt EventModifiers() const: no pointer event is in progress.
        "_ZNK11CAknToolbar14EventModifiersEv" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        _ => return None,
    })
}
