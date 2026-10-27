//! The objects a list box owns besides its model: the item drawer (`CFormattedCellListBoxItemDrawer`, eikfrlb.h:40) and the cell
//! data it draws with (`CFormattedCellListBoxData`, eikfrlbd.h), the view (`CListBoxView`) and the scroll bar frame
//! (`CEikScrollBarFrame`, eikscrlb.h). The host draws the list as one List control (`listproject.rs`) and keeps its own scroll bar, so the
//! sub-cell geometry, fonts and scroll bar settings the guest sets are RECORDED here (readable by name in `p.log` and the host state)
//! and drawn nowhere.
//! DELETE WHEN the list is drawn from the guest's cell layout (a sub-cell geometry reaches the host), or the scroll bar frame's
//! type/visibility reaches the host's scroll bar.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

/// `CListItemDrawer::iData` (eiklbi.h:114): vptr 0, iItemCellSize 4, iMarkGutter 12, iMarkColumnWidth 16, iViewRect 20, iDrawMark 36,
/// iGc 40, six TRgb 44..68, iMarkColor 68, then iData.
pub const DRAWER_I_DATA: u32 = 72;
const DRAWER_SIZE: u32 = 128;
/// `CListItemDrawer`'s virtuals (eiklbi.h): ~ 0,1, Extension_ 2, DrawItem 3, ClearRect 4, ItemWidthInPixels 5, MinimumCellSize 6, SetGc 7,
/// SetMarkColumnWidth 8, SetMarkGutter 9, SetItemCellSize 10 (CONFIRMED: FileBrowser calls `[[drawer]+40](drawer, TSize&)` at 0x4031a0),
/// SetViewRect 11, ... Reserved_1: 20 slots, the rest unnamed until called.
const DRAWER_SLOTS: usize = 24;
const DRAWER_NAMED: &[(usize, &str)] = &[(10, "_ZN15CListItemDrawer15SetItemCellSizeERK5TSize")];
const FONT_SPEC_BYTES: u32 = 72;
const SKIN_IFACE_SLOTS: usize = 12;

/// What the guest set for one sub-cell of a formatted cell list (the `SetSubCell*` calls, eikfrlbd.h).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubCell {
    pub size: Option<(i32, i32)>,
    pub position: Option<(i32, i32)>,
    pub icon_size: Option<(i32, i32)>,
    pub margins: Option<[i32; 4]>,
    pub font: Option<u32>,
    pub baseline: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Default)]
pub struct CellData {
    pub subcells: BTreeMap<i32, SubCell>,
    pub icon_array: u32,
    pub marquee: Option<bool>,
    pub separator_line: Option<u32>,
    /// The list's skin background context (a `CAknsBasicBackgroundControlContext` this engine made; its rectangle is recorded).
    pub skin_context: u32,
}

/// The scroll bar frame the guest configured (`CreateScrollBarFrameL`, `SetScrollBarVisibilityL`, ...): recorded.
#[derive(Debug, Clone, Default)]
pub struct ScrollFrame {
    pub vertical_type: Option<u32>,
    pub visibility: Option<(u32, u32)>,
    pub double_span: Option<(u32, u32)>,
}

impl Process {
    /// An engine-built plain object (`CBase`-derived: destructors then nothing the guest calls): `size` zero bytes, its vtable's
    /// destructors named `_ZN<len><class>D1Ev` / `D0Ev`.
    pub(crate) fn plain_object(&mut self, class: &str, size: u32) -> Result<u32, String> {
        self.typed_object(class, size, 3, &[])
    }

    /// The same with a table of `total` slots, of which `named` (slot, mangled name) are confirmed; the rest refuse as `<class>::vslotN`.
    pub(crate) fn typed_object(&mut self, class: &str, size: u32, total: usize, named: &[(usize, &str)]) -> Result<u32, String> {
        let o = self.alloc_zeroed(size).ok_or_else(|| format!("no guest heap for a {class}"))?;
        let (d1, d0) = (format!("_ZN{}{class}D1Ev", class.len()), format!("_ZN{}{class}D0Ev", class.len()));
        let mut all: alloc::vec::Vec<(usize, &str)> = alloc::vec![(0, d1.as_str()), (1, d0.as_str()), (2, "_ZN5CBase10Extension_EjRPvS0_")];
        all.extend_from_slice(named);
        let vptr = self.build_vtable(class, &slots_with_gaps(total, &all))?;
        self.wr(o, vptr)?;
        Ok(o)
    }

    /// The item drawer with its cell data (`CEikFormattedCellListBox::CreateItemDrawerL`).
    pub(crate) fn make_cell_drawer(&mut self) -> Result<u32, String> {
        let data = self.plain_object("CFormattedCellListBoxData", 0x80)?;
        let skin_context = self.plain_object("CAknsBasicBackgroundControlContext", 0x40)?;
        // The `MAknsControlContext` base sits at +4 (FileBrowser converts the returned interface pointer with `- 4`, 0x4024ae).
        let interface = self.build_vtable("MAknsControlContext", &[None; SKIN_IFACE_SLOTS])?;
        self.wr(skin_context + 4, interface)?;
        self.obj_set(skin_context, HostObj::SkinContext([0; 4]))?;
        self.obj_set(data, HostObj::CellData(CellData { skin_context, ..Default::default() }))?;
        let drawer = self.typed_object("CFormattedCellListBoxItemDrawer", DRAWER_SIZE, DRAWER_SLOTS, DRAWER_NAMED)?;
        self.wr(drawer + DRAWER_I_DATA, data)?;
        Ok(drawer)
    }

    /// `AknLayoutUtils::FontFromId(TInt aId, const CFont* aFont)`: the font of logical id `aId`. No glyphs exist in this engine (the host draws the
    /// text), so the font is an opaque guest object per id; a call on it is refused by name, since it has no vtable slots.
    /// DELETE WHEN the guest measures or draws text itself (a `CFont` member is the next refusal).
    fn font_from_id(&mut self, id: u32) -> Result<u32, String> {
        if let Some(f) = self.nat.fonts.get(&id) { return Ok(*f); }
        let f = self.plain_object("CFont", 0x20)?;
        self.nat.fonts.insert(id, f);
        Ok(f)
    }

    fn cell_data(&mut self, this: u32) -> Result<&mut CellData, String> {
        match self.obj_get(this) { Some(HostObj::CellData(c)) => Ok(c), _ => Err(format!("{this:#x} is not a CFormattedCellListBoxData this engine made")) }
    }

    fn scroll_frame(&mut self, this: u32) -> Result<&mut ScrollFrame, String> {
        match self.obj_get(this) { Some(HostObj::ScrollFrame(c)) => Ok(c), _ => Err(format!("{this:#x} is not a CEikScrollBarFrame this engine made")) }
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1, r2, r3) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2] as i32, p.cpu.regs[3] as i32);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    let ret = |p: &mut Process, r: Result<u32, String>| match r { Ok(v) => { p.cpu.regs[0] = v; NativeOutcome::Return } Err(m) => refuse(m) };
    let sub = |p: &mut Process, f: &dyn Fn(&mut SubCell)| { let i = r1 as i32; p.cell_data(r0).map(|c| f(c.subcells.entry(i).or_default())).map(|_| 0) };
    Some(match name {
        // TFontSpec CFont::FontSpecInTwips() const: a class return (r0 = the result's address, r1 = the font). The guest only copies it and
        // sets its iHeight (gdi.h: TTypeface 56 bytes, iHeight at 56, 72 bytes in all, as FileBrowser's own stack frame lays it out). The
        // logical fonts' real specs (typeface, height) are in the layout data this engine does not read; the spec is all zero = unspecified.
        // DELETE WHEN the guest reads the typeface or height of a font it did not set itself.
        "_ZNK5CFont15FontSpecInTwipsEv" => {
            let r = (0..FONT_SPEC_BYTES / 4).try_for_each(|i| p.wr(r0 + i * 4, 0));
            ret(p, r.map(|_| r0))
        }
        "_ZN14AknLayoutUtils10FontFromIdEiPK5CFont" => { let r = p.font_from_id(r0); ret(p, r) }
        // void CListItemDrawer::SetItemCellSize(const TSize&): iItemCellSize (eiklbi.h:101), the drawer's cell (the row's width and height).
        "_ZN15CListItemDrawer15SetItemCellSizeERK5TSize" => {
            let r = p.rd(r1).and_then(|w| p.rd(r1 + 4).map(|h| (w, h))).and_then(|(w, h)| { p.wr(r0 + crate::listbox::DRAWER_I_CELL_SIZE, w)?; p.wr(r0 + crate::listbox::DRAWER_I_CELL_SIZE + 4, h) });
            ret(p, r.map(|_| 0))
        }
        // CFormattedCellListBoxItemDrawer::ColumnData() const / FormattedCellData() const: iData.
        "_ZNK31CFormattedCellListBoxItemDrawer10ColumnDataEv" | "_ZNK31CFormattedCellListBoxItemDrawer17FormattedCellDataEv" => { let r = p.rd(r0 + DRAWER_I_DATA); ret(p, r) }
        // SetSubCellSizeL(TInt aSubCell, TSize), SetSubCellPositionL(TInt, TPoint), SetSubCellIconSize(TInt, TSize): a 2-word value in r2, r3.
        "_ZN25CFormattedCellListBoxData15SetSubCellSizeLEi5TSize" => { let r = sub(p, &|s| s.size = Some((r2, r3))); ret(p, r) }
        "_ZN25CFormattedCellListBoxData19SetSubCellPositionLEi6TPoint" => { let r = sub(p, &|s| s.position = Some((r2, r3))); ret(p, r) }
        "_ZN25CFormattedCellListBoxData18SetSubCellIconSizeEi5TSize" => { let r = sub(p, &|s| s.icon_size = Some((r2, r3))); ret(p, r) }
        // SetSubCellMarginsL(TInt, TMargins): four words (iLeft, iRight, iTop, iBottom, gdi.h): r2, r3, then the first two stack words.
        "_ZN25CFormattedCellListBoxData18SetSubCellMarginsLEi8TMargins" => {
            let sp = p.cpu.regs[13];
            let r = p.rd(sp).and_then(|a| p.rd(sp + 4).map(|b| [r2, r3, a as i32, b as i32])).and_then(|m| sub(p, &|s| s.margins = Some(m)));
            ret(p, r)
        }
        "_ZN25CFormattedCellListBoxData15SetSubCellFontLEiPK5CFont" => { let f = p.cpu.regs[2]; let r = sub(p, &|s| s.font = Some(f)); ret(p, r) }
        "_ZN25CFormattedCellListBoxData22SetSubCellBaselinePosLEii" => { let r = sub(p, &|s| s.baseline = Some((r2, r3))); ret(p, r) }
        "_ZN25CFormattedCellListBoxData12SetIconArrayEP9CArrayPtrI8CGulIconE" => { let r = p.cell_data(r0).map(|c| c.icon_array = r1).map(|_| 0); ret(p, r) }
        // MAknsControlContext* CFormattedCellListBoxData::SkinBackgroundContext() const: the list's background context (the guest calls
        // `SetRect` on it unconditionally). This engine has no skin server and the host draws the list, so the context only records the
        // rectangle the guest gives it. DELETE WHEN AknsUtils::SkinInstance serves a skin (then the context paints the list's background).
        "_ZNK25CFormattedCellListBoxData21SkinBackgroundContextEv" => { let r = p.cell_data(r0).map(|c| c.skin_context + 4); ret(p, r) }
        "_ZN34CAknsBasicBackgroundControlContext7SetRectERK5TRect" => {
            let r = (0..4).try_fold([0i32; 4], |mut a, i| { a[i] = p.rd(r1 + i as u32 * 4)? as i32; Ok::<_, String>(a) }).and_then(|rect| match p.obj_get(r0) {
                Some(HostObj::SkinContext(c)) => { *c = rect; Ok(0) }
                _ => Err(format!("{r0:#x} is not a skin context this engine made")),
            });
            ret(p, r)
        }
        "_ZNK25CFormattedCellListBoxData9IconArrayEv" => { let r = p.cell_data(r0).map(|c| c.icon_array); ret(p, r) }
        "_ZN25CFormattedCellListBoxData14EnableMarqueeLEi" => { let r = p.cell_data(r0).map(|c| c.marquee = Some(r1 != 0)).map(|_| 0); ret(p, r) }
        "_ZN25CFormattedCellListBoxData24SetSeparatorLinePositionE25TAknSeparatorLinePosition" => { let r = p.cell_data(r0).map(|c| c.separator_line = Some(r1)).map(|_| 0); ret(p, r) }
        // CEikScrollBarFrame (eikscrlb.h): recorded.
        "_ZN18CEikScrollBarFrame19SetTypeOfVScrollBarENS_14TScrollBarTypeE" => { let r = p.scroll_frame(r0).map(|c| c.vertical_type = Some(r1)).map(|_| 0); ret(p, r) }
        "_ZN18CEikScrollBarFrame23SetScrollBarVisibilityLENS_20TScrollBarVisibilityES0_" => { let r = p.scroll_frame(r0).map(|c| c.visibility = Some((r1, r2 as u32))).map(|_| 0); ret(p, r) }
        "_ZN18CEikScrollBarFrame27CreateDoubleSpanScrollBarsLEii" => { let r = p.scroll_frame(r0).map(|c| c.double_span = Some((r1, r2 as u32))).map(|_| 0); ret(p, r) }
        _ => return None,
    })
}
