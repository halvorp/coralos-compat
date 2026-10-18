//! MUI attribute / method ids and the per-kind attribute tables. Values are interface facts from the MUI 3.8
//! developer kit's `libraries/mui.h` (the header amimon-mui was compiled against; Zune's mui.h agrees).
use crate::boopsi::Kind;

// Methods.
pub const MUIM_NOTIFY: u32 = 0x8042_c9cb;
pub const MUIM_SET: u32 = 0x8042_549a;
pub const MUIM_APP_NEW_INPUT: u32 = 0x8042_3ba6;
pub const MUIM_APP_RETURN_ID: u32 = 0x8042_76ef;
pub const MUIM_APP_OPEN_CONFIG_WINDOW: u32 = 0x8042_99ba;
/// MUIM_Notify's special values.
pub const MUIV_TRIGGER_VALUE: u32 = 0x4989_3131;
pub const MUIV_NOT_TRIGGER_VALUE: u32 = 0x4989_3133;
pub const MUIV_EVERY_TIME: u32 = 0x4989_3131;
pub const MUIV_NOTIFY_SELF: u32 = 1;
pub const MUIV_NOTIFY_WINDOW: u32 = 2;
pub const MUIV_NOTIFY_APPLICATION: u32 = 3;
pub const MUIV_NOTIFY_PARENT: u32 = 4;

// Attributes.
pub const MUIA_USER_DATA: u32 = 0x8042_0313;
pub const MUIA_FAMILY_CHILD: u32 = 0x8042_c696;
pub const MUIA_APP_AUTHOR: u32 = 0x8042_4842;
pub const MUIA_APP_BASE: u32 = 0x8042_e07a;
pub const MUIA_APP_COPYRIGHT: u32 = 0x8042_ef4d;
pub const MUIA_APP_DESCRIPTION: u32 = 0x8042_1fc6;
pub const MUIA_APP_MENU_ACTION: u32 = 0x8042_8961;
pub const MUIA_APP_MENUSTRIP: u32 = 0x8042_52d9;
pub const MUIA_APP_TITLE: u32 = 0x8042_81b8;
pub const MUIA_APP_VERSION: u32 = 0x8042_b33f;
pub const MUIA_APP_WINDOW: u32 = 0x8042_bfe0;
pub const MUIA_WIN_CLOSE_REQUEST: u32 = 0x8042_e86e;
pub const MUIA_WIN_ID: u32 = 0x8042_01bd;
pub const MUIA_WIN_OPEN: u32 = 0x8042_8aa0;
pub const MUIA_WIN_ROOT_OBJECT: u32 = 0x8042_cba5;
pub const MUIA_WIN_TITLE: u32 = 0x8042_ad3d;
pub const MUIA_BACKGROUND: u32 = 0x8042_545b;
pub const MUIA_FRAME: u32 = 0x8042_ac64;
pub const MUIA_TEXT_CONTENTS: u32 = 0x8042_f8dc;
pub const MUIA_GROUP_CHILD: u32 = 0x8042_26e6;
pub const MUIA_GROUP_COLUMNS: u32 = 0x8042_f416;
pub const MUIA_GROUP_HORIZ: u32 = 0x8042_536b;
pub const MUIA_MENU_TITLE: u32 = 0x8042_a0e3;
pub const MUIA_MENUITEM_SHORTCUT: u32 = 0x8042_2030;
pub const MUIA_MENUITEM_TITLE: u32 = 0x8042_18be;

/// MUI_MakeObject types.
pub const MUIO_LABEL: u32 = 1;
/// MUIV_Application_ReturnID_Quit.
pub const RETURN_ID_QUIT: u32 = u32::MAX;

/// How an attribute is stored.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ty { Num, Str, Adopt }
pub struct Def { pub id: u32, pub name: &'static str, pub ty: Ty }
const fn d(id: u32, name: &'static str, ty: Ty) -> Def { Def { id, name, ty } }

const COMMON: &[Def] = &[d(MUIA_USER_DATA, "MUIA_UserData", Ty::Num)];
const APP: &[Def] = &[d(MUIA_APP_TITLE, "MUIA_Application_Title", Ty::Str), d(MUIA_APP_VERSION, "MUIA_Application_Version", Ty::Str),
    d(MUIA_APP_AUTHOR, "MUIA_Application_Author", Ty::Str), d(MUIA_APP_COPYRIGHT, "MUIA_Application_Copyright", Ty::Str),
    d(MUIA_APP_DESCRIPTION, "MUIA_Application_Description", Ty::Str), d(MUIA_APP_BASE, "MUIA_Application_Base", Ty::Str),
    d(MUIA_APP_MENUSTRIP, "MUIA_Application_Menustrip", Ty::Adopt), d(MUIA_APP_WINDOW, "MUIA_Application_Window", Ty::Adopt),
    d(MUIA_APP_MENU_ACTION, "MUIA_Application_MenuAction", Ty::Num)];
const WIN: &[Def] = &[d(MUIA_WIN_TITLE, "MUIA_Window_Title", Ty::Str), d(MUIA_WIN_ID, "MUIA_Window_ID", Ty::Num),
    d(MUIA_WIN_ROOT_OBJECT, "MUIA_Window_RootObject", Ty::Adopt), d(MUIA_WIN_OPEN, "MUIA_Window_Open", Ty::Num),
    d(MUIA_WIN_CLOSE_REQUEST, "MUIA_Window_CloseRequest", Ty::Num)];
const GROUP: &[Def] = &[d(MUIA_GROUP_CHILD, "MUIA_Group_Child", Ty::Adopt), d(MUIA_GROUP_COLUMNS, "MUIA_Group_Columns", Ty::Num),
    d(MUIA_GROUP_HORIZ, "MUIA_Group_Horiz", Ty::Num)];
const TEXT: &[Def] = &[d(MUIA_TEXT_CONTENTS, "MUIA_Text_Contents", Ty::Str), d(MUIA_FRAME, "MUIA_Frame", Ty::Num),
    d(MUIA_BACKGROUND, "MUIA_Background", Ty::Num)];
const FAMILY: &[Def] = &[d(MUIA_FAMILY_CHILD, "MUIA_Family_Child", Ty::Adopt)];
const MENU: &[Def] = &[d(MUIA_MENU_TITLE, "MUIA_Menu_Title", Ty::Str), d(MUIA_FAMILY_CHILD, "MUIA_Family_Child", Ty::Adopt)];
const MENUITEM: &[Def] = &[d(MUIA_MENUITEM_TITLE, "MUIA_Menuitem_Title", Ty::Str), d(MUIA_MENUITEM_SHORTCUT, "MUIA_Menuitem_Shortcut", Ty::Str),
    d(MUIA_FAMILY_CHILD, "MUIA_Family_Child", Ty::Adopt)];

/// The attribute `attr` of `kind`, if the engine implements it.
pub fn lookup(kind: Kind, attr: u32) -> Option<&'static Def> {
    let own = match kind {
        Kind::Application => APP, Kind::Window => WIN, Kind::Group => GROUP, Kind::Text => TEXT,
        Kind::Menustrip => FAMILY, Kind::Menu => MENU, Kind::Menuitem => MENUITEM,
    };
    own.iter().chain(COMMON).find(|x| x.id == attr)
}

/// A name for a refusal message.
pub fn attr_name(attr: u32) -> alloc::string::String {
    for t in [APP, WIN, GROUP, TEXT, MENU, MENUITEM, COMMON] { if let Some(x) = t.iter().find(|x| x.id == attr) { return x.name.into(); } }
    alloc::format!("{attr:#010x}")
}

pub fn method_name(id: u32) -> alloc::string::String {
    match id {
        MUIM_NOTIFY => "MUIM_Notify".into(), MUIM_SET => "MUIM_Set".into(), MUIM_APP_NEW_INPUT => "MUIM_Application_NewInput".into(),
        MUIM_APP_RETURN_ID => "MUIM_Application_ReturnID".into(), MUIM_APP_OPEN_CONFIG_WINDOW => "MUIM_Application_OpenConfigWindow".into(),
        _ => alloc::format!("method {id:#010x}"),
    }
}
