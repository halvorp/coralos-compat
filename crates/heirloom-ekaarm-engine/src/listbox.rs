//! `CAknDoubleGraphicStyleListBox` (aknlists.h:623) and the `CEikListBox` (eiklbx.h:127) it is built on: the list FileBrowser shows.
//! The guest allocates the object (`new (ELeave)`, 0x104 bytes) and the library constructor builds it, so this engine writes the
//! vtables and the fields the SDK declares, at the SDK's offsets: `CCoeControl` 0..52 (control.rs), `CAknControl::iSpare[6]` (akncontrol.h),
//! `CEikBorderedControl::iBorder` + `iSpare[2]` (eikbctrl.h) up to 88, the `MEikScrollBarObserver` base at 88 (confirmed by the guest's
//! own import of the `_ZThn88_` thunk), then `CEikListBox`'s members from 92 (eiklbx.h:1449-1486).
//!
//! Virtual slots: 0..43 are `CCoeControl`'s (confirmed in control.rs); the list box's own are named only once a guest call has been
//! seen to land on them. Confirmed so far: 6 `SetContainerWindowL`, 8 `ActivateL`, 59 `ConstructL(const CCoeControl*, TInt)` (the
//! guest's calls at 0x402a36, 0x402a80, 0x402a48 in FileBrowser's container `ConstructL`).
use alloc::format;
use alloc::string::String;
use crate::control::{CTRL_I_POSITION, CTRL_I_SIZE};
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;


pub const LB_I_SCROLL_OBSERVER: u32 = 88; // MEikScrollBarObserver base (eiklbx.h:127 `public MEikScrollBarObserver`)
pub const LB_I_FLAGS: u32 = 92; // eiklbx.h:1451 TInt iListBoxFlags
pub const LB_I_VIEW: u32 = 96; // :1454 CListBoxView* iView
pub const LB_I_DRAWER: u32 = 100; // :1457 CListItemDrawer* iItemDrawer
pub const LB_I_MODEL: u32 = 104; // :1460 MListBoxModel* iModel (the MListBoxModel base of the model: model + 4)
pub const LB_I_ITEM_HEIGHT: u32 = 108; // :1463 TInt iItemHeight
pub const LB_I_SB_FRAME: u32 = 112; // :1466 CEikScrollBarFrame* iSBFrame
pub const LB_I_SB_OWNED: u32 = 116; // :1469 TScrollBarOwnerShip iSBFrameOwned
pub const LB_I_OBSERVER: u32 = 128; // :1481 MEikListBoxObserver* iListBoxObserver
/// `CListBoxView` (eiklbv.h:147-160): vptr 0, iFlags 4, iItemDrawer 8, iModel 12, iDataWidth 16, iTopItemIndex 20, iBottomItemIndex 24,
/// iHScrollOffset 28, iCurrentItemIndex 32, iItemHeight 36, iWin 40, iGroupWin 44, iGc 48, iViewRect 52.
pub const VIEW_I_DRAWER: u32 = 8;
pub const VIEW_I_MODEL: u32 = 12;
pub const VIEW_I_ITEM_HEIGHT: u32 = 36;
pub const VIEW_I_VIEW_RECT: u32 = 52;
/// `CListItemDrawer` (eiklbi.h:101-114): iItemCellSize 4 (iWidth, iHeight), iViewRect 20.
pub const DRAWER_I_CELL_SIZE: u32 = 4;
pub const DRAWER_I_VIEW_RECT: u32 = 20;
const CLASS: &str = "CAknDoubleGraphicStyleListBox";
/// Slots past CCoeControl's 44 that the table has (the CEikListBox, CEikTextListBox and CEikFormattedCellListBox virtuals).
const TAIL: usize = 40;
const OBSERVER_VTABLE: &[Option<&str>] = &[Some("_ZThn88_N11CEikListBox18HandleScrollEventLEP13CEikScrollBar15TEikScrollEvent")];
/// The overrides of CCoeControl's slots (slot numbers are CCoeControl's) and the confirmed own slot.
const OVERRIDES: &[(usize, &str)] = &[
    (3, "_ZN11CEikListBox14OfferKeyEventLERK9TKeyEvent10TEventCode"),
    (5, "_ZN11CEikListBox9SetDimmedEi"),
    (6, "_ZN11CEikListBox19SetContainerWindowLERK11CCoeControl"),
    (7, "_ZN24CEikFormattedCellListBox22ConstructFromResourceLER15TResourceReader"),
    (8, "_ZN11CEikListBox9ActivateLEv"),
    (14, "_ZN29CAknDoubleGraphicStyleListBox11MinimumSizeEv"),
    (15, "_ZN24CEikFormattedCellListBox20HandleResourceChangeEi"),
    (16, "_ZNK24CEikFormattedCellListBox16GetColorUseListLER9CArrayFixI12TCoeColorUseE"),
    (18, "_ZNK11CEikListBox17InputCapabilitiesEv"),
    (24, "_ZN29CAknDoubleGraphicStyleListBox19HandlePointerEventLERK13TPointerEvent"),
    (26, "_ZN24CEikFormattedCellListBox12FocusChangedE8TDrawNow"),
    (27, "_ZN29CAknDoubleGraphicStyleListBox11SizeChangedEv"),
    (29, "_ZNK23AknListBoxLinesTemplateI24CEikFormattedCellListBoxE22CountComponentControlsEv"),
    (30, "_ZNK23AknListBoxLinesTemplateI24CEikFormattedCellListBoxE16ComponentControlEi"),
    (39, "_ZN24CEikFormattedCellListBox15MopSupplyObjectE8TTypeUid"),
    (41, "_ZNK23AknListBoxLinesTemplateI24CEikFormattedCellListBoxE4DrawERK5TRect"),
    (42, "_ZNK15CEikTextListBox19WriteInternalStateLER12RWriteStream"),
    (59, "_ZN24CEikFormattedCellListBox10ConstructLEPK11CCoeControli"),
];

impl Process {
    /// `CAknDoubleGraphicStyleListBox::CAknDoubleGraphicStyleListBox()`: the whole constructor chain on the guest's block.
    fn list_construct(&mut self, this: u32) -> Result<(), String> {
        self.attach_control(this, CLASS, TAIL, OVERRIDES)?;
        let observer = self.build_vtable("MEikScrollBarObserver", OBSERVER_VTABLE)?;
        self.wr(this + LB_I_SCROLL_OBSERVER, observer)?;
        self.list_state(this).map(|_| ())
    }

    /// `CEikFormattedCellListBox::ConstructL(const CCoeControl* aParent, TInt aFlags)`: the model, the item drawer and the view.
    /// (The real one calls the virtual `CreateModelL` / `CreateItemDrawerL` / `CreateViewL`; the engine builds them directly.)
    fn list_construct_l(&mut self, this: u32, flags: u32) -> Result<(), String> {
        let model = self.make_text_model()?;
        let drawer = self.make_cell_drawer()?;
        let view = self.plain_object("CListBoxView", 0x90)?;
        self.wr(view + VIEW_I_DRAWER, drawer)?;
        self.wr(view + VIEW_I_MODEL, model + 4)?;
        self.wr(this + LB_I_FLAGS, flags)?;
        self.wr(this + LB_I_MODEL, model + 4)?;
        self.wr(this + LB_I_DRAWER, drawer)?;
        self.wr(this + LB_I_VIEW, view)
    }

    /// `CEikListBox::CreateScrollBarFrameL(TBool aPreAlloc)`: the list owns a scroll bar frame.
    fn list_create_scroll_frame(&mut self, this: u32) -> Result<u32, String> {
        let f = self.plain_object("CEikScrollBarFrame", 0x40)?;
        self.obj_set(f, HostObj::ScrollFrame(Default::default()))?;
        self.wr(this + LB_I_SB_FRAME, f)?;
        self.wr(this + LB_I_SB_OWNED, 0)?; // TScrollBarOwnerShip: EOwnedExternally = 0 is the default; the list owns it below
        Ok(f)
    }

    /// The control's rectangle became the view's: `iViewRect` of the view and of the drawer, and the item cell of the drawer is one row wide
    /// (the control's width) and `iItemHeight` high. The host list is told the new rectangle (`list_sync`).
    fn list_size_changed(&mut self, this: u32) -> Result<(), String> {
        let (view, drawer) = (self.rd(this + LB_I_VIEW)?, self.rd(this + LB_I_DRAWER)?);
        if view == 0 || drawer == 0 { return Err(String::from("SizeChanged before ConstructL: the list has no view yet")); }
        let [x, y, w, h] = [CTRL_I_POSITION, CTRL_I_POSITION + 4, CTRL_I_SIZE, CTRL_I_SIZE + 4].map(|o| self.rd(this + o).unwrap_or(0));
        for (i, v) in [x, y, x.wrapping_add(w), y.wrapping_add(h)].iter().enumerate() {
            self.wr(view + VIEW_I_VIEW_RECT + i as u32 * 4, *v)?;
            self.wr(drawer + DRAWER_I_VIEW_RECT + i as u32 * 4, *v)?;
        }
        self.wr(drawer + DRAWER_I_CELL_SIZE, w)?;
        let ih = self.rd(this + LB_I_ITEM_HEIGHT)?;
        self.wr(drawer + DRAWER_I_CELL_SIZE + 4, ih)?;
        self.list_sync(this)
    }

    /// `CEikListBox::SetItemHeightL(TInt aHeight)`: the list's, its view's and its drawer's item height.
    fn list_set_item_height(&mut self, this: u32, h: u32) -> Result<(), String> {
        let (view, drawer) = (self.rd(this + LB_I_VIEW)?, self.rd(this + LB_I_DRAWER)?);
        self.wr(this + LB_I_ITEM_HEIGHT, h)?;
        if view != 0 { self.wr(view + VIEW_I_ITEM_HEIGHT, h)?; }
        if drawer != 0 { self.wr(drawer + DRAWER_I_CELL_SIZE + 4, h)?; }
        Ok(())
    }

    fn list_destroy(&mut self, this: u32) {
        self.obj_remove(this);
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    let ret = |p: &mut Process, r: Result<u32, String>| match r { Ok(v) => { p.cpu.regs[0] = v; NativeOutcome::Return } Err(m) => refuse(m) };
    Some(match name {
        "_ZN29CAknDoubleGraphicStyleListBoxC1Ev" | "_ZN29CAknDoubleGraphicStyleListBoxC2Ev" => { let r = p.list_construct(r0); ret(p, r.map(|_| r0)) }
        "_ZN29CAknDoubleGraphicStyleListBoxD1Ev" | "_ZN29CAknDoubleGraphicStyleListBoxD0Ev" => {
            p.list_destroy(r0);
            if name.ends_with("D0Ev") { if let Some(h) = p.heap.as_mut() { let _ = h.free(r0); } }
            NativeOutcome::Return
        }
        // The base class's: CCoeControl's own (the list box adds nothing to it yet).
        "_ZN11CEikListBox19SetContainerWindowLERK11CCoeControl" => return crate::control::native(p, "_ZN11CCoeControl19SetContainerWindowLERKS_"),
        "_ZN24CEikFormattedCellListBox10ConstructLEPK11CCoeControli" => { let r = p.list_construct_l(r0, r2); ret(p, r.map(|_| 0)) }
        "_ZN11CEikListBox9ActivateLEv" => {
            let r = match crate::control::native(p, "_ZN11CCoeControl9ActivateLEv") { Some(NativeOutcome::Return) => Ok(()), Some(NativeOutcome::Refused(m)) => Err(m), _ => Err(String::from("CCoeControl::ActivateL is not implemented")) };
            let r = r.and_then(|_| { p.list_state(r0)?.activated = true; p.list_sync(r0) });
            match r { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) }
        }
        // CEikTextListBox::Model() const returns the CTextListBoxModel: iModel is its MListBoxModel base at +4.
        "_ZNK15CEikTextListBox5ModelEv" | "_ZNK24CEikFormattedCellListBox5ModelEv" => { let r = p.rd(r0 + LB_I_MODEL).map(|m| m - 4); ret(p, r) }
        // CAknDoubleGraphicStyleListBox::SizeChanged(): the real one lays the cells out from the CDL `list_double_graphic_pane` data (item
        // height, sub-cell sizes and positions). Here the control's new rectangle reaches the host and nothing else: the host list has its
        // own row height and no sub-cell geometry. DELETE WHEN the list is drawn from the guest's cell layout (listdata.rs).
        "_ZN29CAknDoubleGraphicStyleListBox11SizeChangedEv" => { let r = p.list_size_changed(r0); ret(p, r.map(|_| 0)) }
        "_ZN11CEikListBox14SetItemHeightLEi" => { let r = p.list_set_item_height(r0, r1); ret(p, r.map(|_| 0)) }
        // CListItemDrawer::ItemCellSize() const: a TSize returned through r0 (r1 = the drawer): iItemCellSize.
        "_ZNK15CListItemDrawer12ItemCellSizeEv" => {
            let r = p.rd(r1 + DRAWER_I_CELL_SIZE).and_then(|w| p.rd(r1 + DRAWER_I_CELL_SIZE + 4).map(|h| (w, h))).and_then(|(w, h)| { p.wr(r0, w)?; p.wr(r0 + 4, h) });
            ret(p, r.map(|_| r0))
        }
        // TInt CountComponentControls() const / CCoeControl* ComponentControl(TInt) const: the list's components are its scroll bars, which are
        // recorded (listdata.rs), not controls. DELETE WHEN the scroll bar frame's bars are controls (then the visible ones).
        "_ZNK23AknListBoxLinesTemplateI24CEikFormattedCellListBoxE22CountComponentControlsEv" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        "_ZNK23AknListBoxLinesTemplateI24CEikFormattedCellListBoxE16ComponentControlEi" => refuse(format!("component control {r1} of a list box with none (E32USER-CBase panic on a device)")),
        // void CEikListBox::ClearSelection(): the marked items go. Nothing can be marked yet (the marking calls are refused by name), so the
        // set is empty and clearing it changes nothing.
        "_ZN11CEikListBox14ClearSelectionEv" => NativeOutcome::Return,
        // CListBoxView* CEikListBox::View() const; CListBoxView::SetListEmptyTextL(const TDesC&): the text shown when the list has no items.
        "_ZNK11CEikListBox4ViewEv" => { let r = p.rd(r0 + LB_I_VIEW); ret(p, r) }
        "_ZN12CListBoxView17SetListEmptyTextLERK7TDesC16" => {
            let r = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).and_then(|t| { p.log.push(format!("[list] empty text: \"{t}\"")); p.obj_set(r0, HostObj::ListView(t)) });
            ret(p, r.map(|_| 0))
        }
        "_ZNK24CEikFormattedCellListBox10ItemDrawerEv" => { let r = p.rd(r0 + LB_I_DRAWER); ret(p, r) }
        "_ZN11CEikListBox21CreateScrollBarFrameLEi" => { let r = p.list_create_scroll_frame(r0); ret(p, r.map(|_| 0)) }
        "_ZN11CEikListBox14ScrollBarFrameEv" => { let r = p.rd(r0 + LB_I_SB_FRAME); ret(p, r) }
        "_ZN11CEikListBox18SetListBoxObserverEP19MEikListBoxObserver" => { let r = p.wr(r0 + LB_I_OBSERVER, r1); ret(p, r.map(|_| 0)) }
        // CEikListBox::HandleItemAdditionL(): the model gained items.
        "_ZN11CEikListBox19HandleItemAdditionLEv" => { let r = p.list_sync(r0); ret(p, r.map(|_| 0)) }
        _ => return None,
    })
}
