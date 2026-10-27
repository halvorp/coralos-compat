//! The Avkon shell the engine owns: the `MEikAppUiFactory` (eikaufty.h) `CEikonEnv::AppUiFactory()` returns, and the real objects
//! behind it: the status pane (`CEikStatusPaneBase`, eikspane.h) with its title pane (`CAknTitlePane`, akntitle.h) and navigation
//! container (`CAknNavigationControlContainer`, aknnavi.h), the menu bar and the CBA. Each is a guest object with an engine vtable;
//! what the guest sets on them is host state here. Nothing is painted by the engine.
//! Projection: the title pane's text becomes the window title through the GuestUi seam (only what the guest sets). Menu items and
//! CBA labels are recorded; DELETE WHEN slice 4 projects them as native kit menus and buttons.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_fmt_rsc::Reader;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// Status pane ids (avkon.hrh:887-889).
pub const PANE_CONTEXT: u32 = 0x1000_59BB;
pub const PANE_TITLE: u32 = 0x1000_59BC;
pub const PANE_NAVI: u32 = 0x1000_59BD;

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub command: u32,
    pub cascade: u32,
    pub flags: u32,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct MenuTitle {
    pub text: String,
    pub pane: u32,
    pub items: Vec<MenuItem>,
}

#[derive(Debug, Clone)]
pub struct CbaButton {
    pub id: u16,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct ShellState {
    pub factory: Option<u32>,
    pub status_pane: Option<u32>,
    pub title_pane: Option<u32>,
    pub navi: Option<u32>,
    pub cba: Option<u32>,
    pub menu_bar: Option<u32>,
    /// The layout resource the guest switched the status pane to (`SwitchLayoutL`).
    pub layout: Option<u32>,
    pub title_text: Option<String>,
    /// The menu of the last view constructed: titles with their items.
    pub menu: Vec<MenuTitle>,
    pub cba_buttons: Vec<CbaButton>,
}

// ---- MEikAppUiFactory's primary vtable (eikaufty.h): MObjectProvider's four slots, then the factory's virtuals in declaration order.
// StatusPane 20 is CONFIRMED by FileBrowser's `ldr r1,[vtbl,#0x50]`.
const FACTORY_METHODS: &[(usize, &str)] = &[
    (5, "ClientRect"), (13, "MenuBar"), (16, "ToolBar"), (18, "ToolBand"), (19, "TitleBand"), (20, "StatusPane"), (21, "Cba"),
];
const FACTORY_SLOTS: usize = 27;
/// CEikStatusPaneBase (eikspane.h): ~ 0,1, Extension_ 2, SwitchLayoutL 3 (CONFIRMED: the app calls `[vtbl,#0xc]`), MakeVisible 4,
/// SetDimmed 5, SetFaded 6, HandleResourceChange 7, OkToChangeStatusPaneNow 8, then the protected pure CreateModelL 9 and IsApp 10.
const STATUS_PANE_SLOTS: &[(usize, &str)] = &[
    (0, "_ZN14CEikStatusPaneD1Ev"), (1, "_ZN14CEikStatusPaneD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZN18CEikStatusPaneBase12SwitchLayoutLEi"), (4, "_ZN18CEikStatusPaneBase11MakeVisibleEi"), (5, "_ZN18CEikStatusPaneBase9SetDimmedEi"),
    (6, "_ZN18CEikStatusPaneBase8SetFadedEi"), (7, "_ZN18CEikStatusPaneBase20HandleResourceChangeEi"), (8, "_ZN18CEikStatusPaneBase21OkToChangeStatusPaneNowEv"),
];

// Resource fields are packed: a WORD or LONG is NOT aligned (a CBA_BUTTON's id WORD follows its version BYTE at an odd offset); only
// an LTEXT body is 2-aligned, with a pad byte (`Reader::ltext` does that).
fn rd_word(r: &mut Reader) -> Result<u16, String> { r.word().map_err(|e| format!("{e:?}")) }
fn rd_long(r: &mut Reader) -> Result<u32, String> { r.long().map_err(|e| format!("{e:?}")) }
fn rd_text(r: &mut Reader) -> Result<String, String> { r.ltext().map_err(|e| format!("{e:?}")) }

/// `MENU_PANE` (eikon.rh:97): `items[]` (a WORD count, then `MENU_ITEM`s: command, cascade, flags, txt, extratxt, bmpfile, bmpid,
/// bmpmask, extension), then `extension`.
fn parse_menu_pane(bytes: &[u8]) -> Result<Vec<MenuItem>, String> {
    let mut r = Reader::new(bytes);
    let n = rd_word(&mut r)?;
    let mut items = Vec::new();
    for _ in 0..n {
        let command = rd_long(&mut r)?;
        let cascade = rd_long(&mut r)?;
        let flags = rd_long(&mut r)?;
        let text = rd_text(&mut r)?;
        let _extra = rd_text(&mut r)?;
        let _bmp = rd_text(&mut r)?;
        rd_word(&mut r)?;
        rd_word(&mut r)?;
        rd_long(&mut r)?;
        items.push(MenuItem { command, cascade, flags, text });
    }
    Ok(items)
}

impl Process {
    /// `MENU_BAR` (eikon.rh:116): `titles[]` (a WORD count, then `MENU_TITLE`s: menu_pane, txt, flags, bmpfile, bmpid, bmpmask,
    /// extension), then `extension`; each title's pane is read.
    pub(crate) fn read_menu_bar(&mut self, id: u32) -> Result<Vec<MenuTitle>, String> {
        let bytes = self.resource(id)?;
        let mut r = Reader::new(&bytes);
        let n = rd_word(&mut r)?;
        let mut titles = Vec::new();
        for _ in 0..n {
            let pane = rd_long(&mut r)?;
            let text = rd_text(&mut r)?;
            rd_long(&mut r)?;
            let _bmp = rd_text(&mut r)?;
            rd_word(&mut r)?;
            rd_word(&mut r)?;
            rd_long(&mut r)?;
            let items = parse_menu_pane(&self.resource(pane)?).map_err(|e| format!("MENU_PANE {pane:#x}: {e}"))?;
            titles.push(MenuTitle { text, pane, items });
        }
        Ok(titles)
    }

    /// `CBA` (eikon.rh:335): flags LONG, breadth WORD, related_buttons LLINK, `buttons[]` of `CBA_BUTTON` (version BYTE, id WORD, txt,
    /// bmpfile, bmpid, bmpmask).
    pub(crate) fn read_cba(&mut self, id: u32) -> Result<Vec<CbaButton>, String> {
        let bytes = self.resource(id)?;
        let mut r = Reader::new(&bytes);
        rd_long(&mut r)?;
        rd_word(&mut r)?;
        rd_long(&mut r)?;
        let n = rd_word(&mut r)?;
        let mut out = Vec::new();
        for _ in 0..n {
            r.byte().map_err(|e| format!("{e:?}"))?;
            let id = rd_word(&mut r)?;
            let text = rd_text(&mut r)?;
            let _bmp = rd_text(&mut r)?;
            rd_word(&mut r)?;
            rd_word(&mut r)?;
            out.push(CbaButton { id, text });
        }
        Ok(out)
    }

    fn shell_factory(&mut self) -> Result<u32, String> {
        if let Some(f) = self.nat.shell.factory { return Ok(f); }
        let f = self.alloc_zeroed(8).ok_or_else(|| String::from("no guest heap for the factory"))?;
        let names: Vec<(usize, String)> = FACTORY_METHODS.iter().map(|(i, n)| (*i, format!("CAknAppUiFactory::{n}"))).collect();
        let refs: Vec<(usize, &str)> = names.iter().map(|(i, n)| (*i, n.as_str())).collect();
        let vptr = self.build_vtable("MEikAppUiFactory", &slots_with_gaps(FACTORY_SLOTS, &refs))?;
        self.mem.vm.write_u32(f, vptr).map_err(|_| String::from("factory unwritable"))?;
        self.nat.shell.factory = Some(f);
        Ok(f)
    }

    /// The CBA (`CEikButtonGroupContainer`), made on first need.
    pub(crate) fn shell_cba(&mut self) -> Result<u32, String> {
        if let Some(c) = self.nat.shell.cba { return Ok(c); }
        let c = self.make_control("CEikButtonGroupContainer", 0x80, &[])?;
        self.nat.shell.cba = Some(c);
        Ok(c)
    }

    fn shell_status_pane(&mut self) -> Result<u32, String> {
        if let Some(s) = self.nat.shell.status_pane { return Ok(s); }
        let s = self.alloc_zeroed(0x40).ok_or_else(|| String::from("no guest heap for the status pane"))?;
        let vptr = self.build_vtable("CEikStatusPane", &slots_with_gaps(11, STATUS_PANE_SLOTS))?;
        self.mem.vm.write_u32(s, vptr).map_err(|_| String::from("status pane unwritable"))?;
        self.nat.shell.status_pane = Some(s);
        Ok(s)
    }

    fn shell_pane_control(&mut self, uid: u32) -> Result<u32, String> {
        match uid {
            PANE_TITLE => {
                if let Some(t) = self.nat.shell.title_pane { return Ok(t); }
                let t = self.make_control("CAknTitlePane", 0x80, &[])?;
                self.nat.shell.title_pane = Some(t);
                Ok(t)
            }
            PANE_NAVI => {
                if let Some(n) = self.nat.shell.navi { return Ok(n); }
                let n = self.make_control("CAknNavigationControlContainer", 0x80, &[])?;
                self.nat.shell.navi = Some(n);
                Ok(n)
            }
            other => Err(format!("status pane id {other:#x} has no control implemented")),
        }
    }

    /// The title pane's text: kept, and shown as the window title through the GuestUi seam.
    fn shell_set_title(&mut self, text: String) -> Result<(), String> {
        self.nat.shell.title_text = Some(text.clone());
        let win = self.host.window.ok_or_else(|| String::from("no application window yet"))?;
        let ui = self.host.ui.as_mut().ok_or_else(|| String::from("no GuestUi"))?;
        ui.set_window_title(win, text).map_err(|e| format!("the UI refused the title: {e}"))
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    let ret = |p: &mut Process, r: Result<u32, String>| match r { Ok(v) => { p.cpu.regs[0] = v; NativeOutcome::Return } Err(m) => refuse(m) };
    Some(match name {
        // MEikAppUiFactory* CEikonEnv::AppUiFactory() const
        "_ZNK9CEikonEnv12AppUiFactoryEv" => { let r = p.shell_factory(); ret(p, r) }
        "CAknAppUiFactory::Cba" => { let r = p.shell_cba(); ret(p, r) }
        // CEikMenuBar* CAknView::MenuBar() const: iMenu.
        "_ZNK8CAknView7MenuBarEv" => ret(p, p.mem.vm.read_u32(r0 + crate::view::VIEW_I_MENU).map_err(|_| format!("the view at {r0:#x} is unreadable"))),
        // MEikAppUiFactory::StatusPane() const
        "CAknAppUiFactory::StatusPane" => { let r = p.shell_status_pane(); ret(p, r) }
        // CCoeControl* CEikStatusPaneBase::ControlL(TPaneId) const: the TUid is one word.
        "_ZNK18CEikStatusPaneBase8ControlLE4TUid" => { let r = p.shell_pane_control(r1); ret(p, r) }
        // TInt CEikStatusPaneBase::CurrentLayoutResId() const
        "_ZNK18CEikStatusPaneBase18CurrentLayoutResIdEv" => match p.nat.shell.layout {
            Some(l) => { p.cpu.regs[0] = l; NativeOutcome::Return }
            None => refuse(String::from("no layout was switched to, and the initial layout is the application's core resource, which is not read")),
        },
        // void CEikStatusPaneBase::SwitchLayoutL(TInt aLayoutResourceId): which panes exist and where is the layout resource's; only the id is kept.
        "_ZN18CEikStatusPaneBase12SwitchLayoutLEi" => { p.nat.shell.layout = Some(r1); NativeOutcome::Return }
        "_ZN18CEikStatusPaneBase11MakeVisibleEi" | "_ZN18CEikStatusPaneBase9SetDimmedEi" | "_ZN18CEikStatusPaneBase8SetFadedEi" => NativeOutcome::Return,
        // CAknTitlePane::SetTextL(const TDesC&) and (const TDesC&, TBool aScroll)
        "_ZN13CAknTitlePane8SetTextLERK7TDesC16" | "_ZN13CAknTitlePane8SetTextLERK7TDesC16i" => {
            let r = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).and_then(|t| p.shell_set_title(t)).map(|_| r0);
            ret(p, r)
        }
        _ => return None,
    })
}
