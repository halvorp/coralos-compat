//! `MUI_MakeObjectA`: the ready-made objects of the MUIO_ collection, built as the same object trees MUI builds (Zune's
//! mui_makeobject.c is the recipe list; mui.h the parameter list of each type). Each recipe is a class plus a tag list, so the
//! role a made object has is the role its class contract gives it ([`super::roles`]); nothing here knows what it will look like.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::*;
use super::object::{self, create, set_text};
use super::methods;
use crate::boopsi::Ext;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;

/// intuition/intuition.h menu flags and gadtools.h NewMenu (20 bytes: type 0, label 2, comm key 6, flags 10, mutual exclude 12, user data 16;
/// the layout gt_menus.rs reads).
/// mui.h: `MUIO_CoolButton = 111` is followed by `MUIO_ImageButton` (the generated ids stop at MUIO_NumericButton).
const MUIO_ImageButton: u32 = 112;
const NM_END: u8 = 0;
const NM_TITLE: u8 = 1;
const NM_ITEM: u8 = 2;
const NM_SUB: u8 = 3;
const NM_SIZE: u32 = 20;
const NM_BARLABEL: u32 = 0xFFFF_FFFF;
const MENUENABLED: u32 = 0x0001;
const ITEMENABLED: u32 = 0x0010;
const CHECKIT: u32 = 0x0001;
const MENUTOGGLE: u32 = 0x0008;
const CHECKED: u32 = 0x0100;
const COMMSEQ_STRING: u32 = 0x0004;
const NM_ITEMDISABLED: u32 = ITEMENABLED;
const NM_MENUDISABLED: u32 = MENUENABLED;
const MAX_MENU_ENTRIES: u32 = 4096;

/// The `_x` hotkey of a label (lower-cased), 0 when it has none (mui_makeobject.c get_control_char).
fn control_char(label: &str) -> u32 { label.split_once('_').and_then(|(_, r)| r.chars().next()).map_or(0, |c| c.to_ascii_lowercase() as u32) }

fn param<U: GuestUi>(a: &Amiga<U>, params: u32, i: u32) -> Result<u32, Refusal> { a.mem.read_u32(params + 4 * i).map_err(Refusal::mem("MUI_MakeObjectA parameters")) }

fn label_text<U: GuestUi>(a: &Amiga<U>, ptr: u32) -> Result<String, Refusal> { if ptr == 0 { Ok(String::new()) } else { a.mem.read_cstr(ptr).map_err(Refusal::mem("MUI_MakeObjectA label")) } }

/// `class` with `tags` plus the control character and cycle chain the recipes add for a labelled control.
fn control<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, label: u32, mut tags: Vec<(u32, u32)>, chain: bool) -> Result<u32, Refusal> {
    let cc = control_char(&label_text(a, label)?);
    if cc != 0 { tags.push((MUIA_ControlChar, cc)); }
    if chain { tags.push((MUIA_CycleChain, 1)); }
    create(a, kind, &tags)
}

/// MUI_MakeObjectA(type, params): the object, or the named refusal.
pub fn make<U: GuestUi>(a: &mut Amiga<U>, ty: u32, params: u32) -> Result<u32, Refusal> {
    let p = |a: &Amiga<U>, i: u32| param(a, params, i);
    match ty {
        MUIO_Label => {
            let (label, flags) = (p(a, 0)?, p(a, 1)?);
            let mut t = alloc::vec![(MUIA_FramePhantomHoriz, 1), (MUIA_Weight, 0), (MUIA_Text_Contents, label)];
            if flags & 0xFF != 0 { t.push((MUIA_Text_HiChar, flags & 0xFF)); } else { t.push((MUIA_Text_HiCharIdx, '_' as u32)); }
            if flags & MUIO_Label_SingleFrame != 0 { t.push((MUIA_Frame, MUIV_Frame_ImageButton)); } else if flags & MUIO_Label_DoubleFrame != 0 { t.push((MUIA_Frame, MUIV_Frame_String)); }
            if flags & MUIO_Label_FreeVert != 0 { t.push((MUIA_Text_SetVMax, 0)); }
            let obj = create(a, Kind::Text, &t)?;
            let align = if flags & MUIO_Label_LeftAligned != 0 { "\u{1b}l" } else if flags & MUIO_Label_Centered != 0 { "\u{1b}c" } else { "\u{1b}r" };
            set_text(a, obj, MUIA_Text_PreParse, align.into());
            if let Some(o) = a.objects.map.get_mut(&obj) { o.ext = Ext::Text { label: true }; }
            Ok(obj)
        }
        MUIO_Button => {
            let label = p(a, 0)?;
            let obj = control(a, Kind::Text, label, alloc::vec![(MUIA_Frame, MUIV_Frame_Button), (MUIA_Text_HiCharIdx, '_' as u32), (MUIA_Text_Contents, label),
                (MUIA_InputMode, MUIV_InputMode_RelVerify), (MUIA_Background, MUII_ButtonBack)], true)?;
            set_text(a, obj, MUIA_Text_PreParse, "\u{1b}c".into());
            Ok(obj)
        }
        MUIO_ImageButton => {
            // INTERNAL_ImageButton: a button with a picture when the image file exists, otherwise SimpleButton(label).
            let image = p(a, 1)?;
            let path = label_text(a, image)?;
            let cwd = crate::fs::cur_dir(a);
            if a.fs.vol.resolve(cwd, &path).is_ok() { return Err(Refusal(format!("MUIO_ImageButton: \"{path}\" exists, and a picture button (a RelVerify HGroup) has no native role yet"))); }
            make(a, MUIO_Button, params)
        }
        MUIO_Checkmark => { let label = p(a, 0)?; control(a, Kind::Image, label, alloc::vec![(MUIA_Frame, MUIV_Frame_ImageButton), (MUIA_Background, MUII_ButtonBack),
            (MUIA_Image_Spec, MUII_CheckMark), (MUIA_InputMode, MUIV_InputMode_Toggle), (MUIA_Image_FreeVert, 1), (MUIA_ShowSelState, 0)], true) }
        MUIO_Cycle => { let (label, entries) = (p(a, 0)?, p(a, 1)?); control(a, Kind::Cycle, label, alloc::vec![(MUIA_Frame, MUIV_Frame_Button), (MUIA_Cycle_Entries, entries)], true) }
        MUIO_Radio => { let (label, entries) = (p(a, 0)?, p(a, 1)?); control(a, Kind::Radio, label, alloc::vec![(MUIA_Radio_Entries, entries)], false) }
        MUIO_Slider => { let (label, lo, hi) = (p(a, 0)?, p(a, 1)?, p(a, 2)?); control(a, Kind::Slider, label, alloc::vec![(MUIA_Numeric_Min, lo), (MUIA_Numeric_Max, hi)], false) }
        MUIO_String => { let (label, max) = (p(a, 0)?, p(a, 1)?); control(a, Kind::String, label, alloc::vec![(MUIA_Frame, MUIV_Frame_String), (MUIA_String_MaxLen, max)], false) }
        MUIO_PopButton => { let spec = p(a, 0)?; create(a, Kind::Image, &[(MUIA_Frame, MUIV_Frame_ImageButton), (MUIA_Background, MUII_ButtonBack), (MUIA_Image_Spec, spec),
            (MUIA_InputMode, MUIV_InputMode_RelVerify), (MUIA_Image_FreeVert, 1), (MUIA_Image_FreeHoriz, 0)]) }
        MUIO_HSpace => { let n = p(a, 0)?; create(a, Kind::Rectangle, &[(MUIA_VertWeight, 0), (MUIA_FixWidth, n)]) }
        MUIO_VSpace => { let n = p(a, 0)?; create(a, Kind::Rectangle, &[(MUIA_HorizWeight, 0), (MUIA_FixHeight, n)]) }
        MUIO_HBar => { let n = p(a, 0)?; create(a, Kind::Rectangle, &[(MUIA_Rectangle_HBar, 1), (MUIA_FixHeight, n)]) }
        MUIO_VBar => { let n = p(a, 0)?; create(a, Kind::Rectangle, &[(MUIA_Rectangle_VBar, 1), (MUIA_FixWidth, n)]) }
        MUIO_BarTitle => { let t = p(a, 0)?; create(a, Kind::Rectangle, &[(MUIA_Rectangle_HBar, 1), (MUIA_Rectangle_BarTitle, t), (MUIA_FixHeight, 1)]) }
        MUIO_Menuitem => {
            let (title, short, flags, data) = (p(a, 0)?, p(a, 1)?, p(a, 2)?, p(a, 3)?);
            create(a, Kind::Menuitem, &[(MUIA_Menuitem_Title, title), (MUIA_Menuitem_Shortcut, short), (MUIA_Menuitem_Checkit, (flags & CHECKIT != 0) as u32),
                (MUIA_Menuitem_Checked, (flags & CHECKED != 0) as u32), (MUIA_Menuitem_Toggle, (flags & MENUTOGGLE != 0) as u32),
                (MUIA_Menuitem_Enabled, (flags & NM_ITEMDISABLED == 0) as u32), (MUIA_UserData, data)])
        }
        MUIO_MenustripNM => menustrip(a, p(a, 0)?, p(a, 1)?),
        other => Err(Refusal(format!("MUI_MakeObjectA: object type {other} is not implemented"))),
    }
}

/// A Menustrip from a NewMenu array (mui_makeobject.c CreateMenuString): titles become Menus, items Menuitems, subitems nest.
fn menustrip<U: GuestUi>(a: &mut Amiga<U>, nm: u32, flags: u32) -> Result<u32, Refusal> {
    let strip = create(a, Kind::Menustrip, &[])?;
    match build_menus(a, strip, nm, flags) {
        Ok(()) => Ok(strip),
        Err(e) => { let _ = object::dispose(a, strip); Err(e) }
    }
}

fn build_menus<U: GuestUi>(a: &mut Amiga<U>, strip: u32, nm: u32, flags: u32) -> Result<(), Refusal> {
    let (mut menu, mut item): (Option<u32>, Option<u32>) = (None, None);
    for i in 0..MAX_MENU_ENTRIES {
        let at = nm + NM_SIZE * i;
        let ty = a.mem.read_u8(at).map_err(Refusal::mem("NewMenu.nm_Type"))?;
        if ty == NM_END { return Ok(()); }
        let (label, comm, fl) = (a.mem.read_u32(at + 2).map_err(Refusal::mem("NewMenu.nm_Label"))?, a.mem.read_u32(at + 6).map_err(Refusal::mem("NewMenu.nm_CommKey"))?, a.mem.read_u16(at + 10).map_err(Refusal::mem("NewMenu.nm_Flags"))? as u32);
        let (excl, user) = (a.mem.read_u32(at + 12).map_err(Refusal::mem("NewMenu.nm_MutualExclude"))?, a.mem.read_u32(at + 16).map_err(Refusal::mem("NewMenu.nm_UserData"))?);
        match ty {
            NM_TITLE => {
                let m = create(a, Kind::Menu, &[(MUIA_Menu_Title, label), (MUIA_UserData, user), (MUIA_Menu_Enabled, (fl & NM_MENUDISABLED == 0) as u32)])?;
                methods::method(a, strip, MUIM_Family_AddTail, &[m])?;
                (menu, item) = (Some(m), None);
            }
            NM_ITEM | NM_SUB => {
                // flags & MUIO_MenustripNM_CommandKeyCheck: a one-character label is a command key: "O\0Open".
                let (mut label, mut comm) = (label, comm);
                if flags & MUIO_MenustripNM_CommandKeyCheck != 0 && label != NM_BARLABEL && label != 0 && a.mem.read_u8(label + 1).map_err(Refusal::mem("NewMenu label"))? == 0 { comm = label; label += 2; }
                let it = create(a, Kind::Menuitem, &[(MUIA_Menuitem_Title, if label == NM_BARLABEL { 0 } else { label }), (MUIA_Menuitem_Shortcut, if comm == NM_BARLABEL { 0 } else { comm }), (MUIA_UserData, user),
                    (MUIA_Menuitem_Exclude, excl), (MUIA_Menuitem_Checkit, (fl & CHECKIT != 0) as u32), (MUIA_Menuitem_Checked, (fl & CHECKED != 0) as u32),
                    (MUIA_Menuitem_Toggle, (fl & MENUTOGGLE != 0) as u32), (MUIA_Menuitem_CommandString, (fl & COMMSEQ_STRING != 0) as u32), (MUIA_Menuitem_Enabled, (fl & NM_ITEMDISABLED == 0) as u32)])?;
                if label == NM_BARLABEL { set_text(a, it, MUIA_Menuitem_Title, "-".into()); }
                match (ty, menu, item) {
                    (NM_ITEM, Some(m), _) => { methods::method(a, m, MUIM_Family_AddTail, &[it])?; item = Some(it); }
                    (NM_SUB, _, Some(parent)) => { methods::method(a, parent, MUIM_Family_AddTail, &[it])?; }
                    _ => { let _ = object::dispose(a, it); }
                }
            }
            _ => {}
        }
    }
    Err(Refusal(format!("MUIO_MenustripNM: no NM_END within {MAX_MENU_ENTRIES} entries")))
}
