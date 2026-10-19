//! The attributes each MUI class accepts, and how each is held. Names and numbers are MUI's own interface (libraries/mui.h,
//! see ids_gen.rs); which class owns which attribute is Zune's class headers (AROS workbench/libs/muimaster/classes/*.h).
//! An attribute is looked up in the object's class, then in every ancestor's; one in no table is a NAMED refusal.
//!
//! `Ty::Num` is a plain stored longword (also pointers the engine never dereferences, hooks, object pointers, flags).
//! An attribute that is only appearance or placement advice (the weights, the inner spacing, the window's position) is stored
//! and read back exactly like any other; the projection ([`super::roles`], [`super::layout`]) reads the ones it honours.
use super::classes::Kind;
use super::ids_gen::*;

/// How an attribute is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    /// A longword, stored as given.
    Num,
    /// A C string, copied when it is set.
    Str,
    /// A child object: the object takes ownership (only at creation, or through its add method).
    Adopt,
    /// A C string, or a small flag standing in for one (`MUIA_List_Title` TRUE = ask the display hook): below 0x100 it is the flag.
    StrOrFlag,
    /// A NULL-terminated array of C strings (Cycle/Radio entries, Register titles), copied when it is set.
    StrArr,
}
use Ty::*;

const NOTIFY: &[(u32, Ty)] = &[(MUIA_UserData, Num), (MUIA_ObjectID, Num), (MUIA_NoNotify, Num), (MUIA_Version, Num), (MUIA_Revision, Num),
    (MUIA_ApplicationObject, Num), (MUIA_AppMessage, Num), (MUIA_HelpLine, Num), (MUIA_HelpNode, Num), (MUIA_Parent, Num)];
const AREA: &[(u32, Ty)] = &[(MUIA_Background, Num), (MUIA_BottomEdge, Num), (MUIA_ContextMenu, Num), (MUIA_ContextMenuTrigger, Num),
    (MUIA_ControlChar, Num), (MUIA_CycleChain, Num), (MUIA_Disabled, Num), (MUIA_Draggable, Num), (MUIA_Dropable, Num), (MUIA_ExportID, Num),
    (MUIA_FillArea, Num), (MUIA_FixHeight, Num), (MUIA_FixHeightTxt, Str), (MUIA_FixWidth, Num), (MUIA_FixWidthTxt, Str), (MUIA_Font, Num),
    (MUIA_Frame, Num), (MUIA_FrameDynamic, Num), (MUIA_FramePhantomHoriz, Num), (MUIA_FrameTitle, Str), (MUIA_FrameVisible, Num),
    (MUIA_Height, Num), (MUIA_HorizDisappear, Num), (MUIA_HorizWeight, Num), (MUIA_InnerBottom, Num), (MUIA_InnerLeft, Num),
    (MUIA_InnerRight, Num), (MUIA_InnerTop, Num), (MUIA_InputMode, Num), (MUIA_LeftEdge, Num), (MUIA_MaxHeight, Num), (MUIA_MaxWidth, Num),
    (MUIA_Pressed, Num), (MUIA_RightEdge, Num), (MUIA_Selected, Num), (MUIA_ShortHelp, Str), (MUIA_ShowMe, Num), (MUIA_ShowSelState, Num),
    (MUIA_Timer, Num), (MUIA_TopEdge, Num), (MUIA_VertDisappear, Num), (MUIA_VertWeight, Num), (MUIA_Weight, Num), (MUIA_Width, Num),
    (MUIA_Window, Num), (MUIA_WindowObject, Num)];
// family.c OM_NEW: MUIA_Group_Child is accepted as MUIA_Family_Child (the `Child` macro builds menus with it).
const FAMILY: &[(u32, Ty)] = &[(MUIA_Family_Child, Adopt), (MUIA_Group_Child, Adopt), (MUIA_Family_List, Num)];
const APPLICATION: &[(u32, Ty)] = &[(MUIA_Application_Title, Str), (MUIA_Application_Version, Str), (MUIA_Application_Author, Str),
    (MUIA_Application_Copyright, Str), (MUIA_Application_Description, Str), (MUIA_Application_Base, Str), (MUIA_Application_HelpFile, Str),
    (MUIA_Application_Menustrip, Adopt), (MUIA_Application_Window, Adopt), (MUIA_Application_MenuAction, Num), (MUIA_Application_SingleTask, Num),
    (MUIA_Application_DiskObject, Num), (MUIA_Application_BrokerPri, Num), (MUIA_Application_UseCommodities, Num), (MUIA_Application_UseRexx, Num),
    (MUIA_Application_UsedClasses, Num), (MUIA_Application_Active, Num), (MUIA_Application_Iconified, Num), (MUIA_Application_Sleep, Num),
    (MUIA_Application_DoubleStart, Num), (MUIA_Application_ForceQuit, Num), (MUIA_Application_Broker, Num), (MUIA_Application_BrokerPort, Num),
    (MUIA_Application_BrokerHook, Num), (MUIA_Application_Commands, Num), (MUIA_Application_RexxHook, Num), (MUIA_Application_RexxMsg, Num),
    (MUIA_Application_RexxString, Str), (MUIA_Application_DropObject, Num), (MUIA_Application_MenuHelp, Num), (MUIA_Application_Menu, Num),
    (MUIA_Application_WindowList, Num), (MUIA_Application_Version_Extra, Str)];
const WINDOW: &[(u32, Ty)] = &[(MUIA_Window_Title, Str), (MUIA_Window_ScreenTitle, Str), (MUIA_Window_ID, Num), (MUIA_Window_RootObject, Adopt),
    (MUIA_Window_Menustrip, Adopt), (MUIA_Window_Open, Num), (MUIA_Window_CloseRequest, Num), (MUIA_Window_Activate, Num),
    (MUIA_Window_AppWindow, Num), (MUIA_Window_ActiveObject, Num), (MUIA_Window_DefaultObject, Num), (MUIA_Window_LeftEdge, Num),
    (MUIA_Window_TopEdge, Num), (MUIA_Window_Width, Num), (MUIA_Window_Height, Num), (MUIA_Window_AltLeftEdge, Num), (MUIA_Window_AltTopEdge, Num),
    (MUIA_Window_AltWidth, Num), (MUIA_Window_AltHeight, Num), (MUIA_Window_Backdrop, Num), (MUIA_Window_Borderless, Num),
    (MUIA_Window_CloseGadget, Num), (MUIA_Window_DepthGadget, Num), (MUIA_Window_DragBar, Num), (MUIA_Window_SizeGadget, Num),
    (MUIA_Window_SizeRight, Num), (MUIA_Window_NoMenus, Num), (MUIA_Window_Sleep, Num), (MUIA_Window_Screen, Num), (MUIA_Window_PublicScreen, Str),
    (MUIA_Window_RefWindow, Num), (MUIA_Window_IsSubWindow, Num), (MUIA_Window_DisableKeys, Num), (MUIA_Window_MouseObject, Num),
    (MUIA_Window_NeedsMouseObject, Num), (MUIA_Window_FancyDrawing, Num), (MUIA_Window_InputEvent, Str), (MUIA_Window_MenuAction, Num),
    (MUIA_Window_Menu, Num), (MUIA_Window_Window, Num), (MUIA_Window_BackfillHook, Num), (MUIA_Window_UseBottomBorderScroller, Num),
    (MUIA_Window_UseLeftBorderScroller, Num), (MUIA_Window_UseRightBorderScroller, Num)];
const MENU: &[(u32, Ty)] = &[(MUIA_Menu_Title, Str), (MUIA_Menu_Enabled, Num)];
const MENUSTRIP: &[(u32, Ty)] = &[(MUIA_Menustrip_Enabled, Num)];
const MENUITEM: &[(u32, Ty)] = &[(MUIA_Menuitem_Title, Str), (MUIA_Menuitem_Shortcut, Str), (MUIA_Menuitem_Checked, Num), (MUIA_Menuitem_Checkit, Num),
    (MUIA_Menuitem_CommandString, Num), (MUIA_Menuitem_Enabled, Num), (MUIA_Menuitem_Exclude, Num), (MUIA_Menuitem_Toggle, Num), (MUIA_Menuitem_Trigger, Num)];
const RECTANGLE: &[(u32, Ty)] = &[(MUIA_Rectangle_BarTitle, Str), (MUIA_Rectangle_HBar, Num), (MUIA_Rectangle_VBar, Num)];
const BALANCE: &[(u32, Ty)] = &[(MUIA_Balance_Quiet, Num)];
const IMAGE: &[(u32, Ty)] = &[(MUIA_Image_Spec, Num), (MUIA_Image_FontMatch, Num), (MUIA_Image_FontMatchHeight, Num), (MUIA_Image_FontMatchWidth, Num),
    (MUIA_Image_FreeHoriz, Num), (MUIA_Image_FreeVert, Num), (MUIA_Image_OldImage, Num), (MUIA_Image_State, Num)];
const GROUP: &[(u32, Ty)] = &[(MUIA_Group_Child, Adopt), (MUIA_Group_Columns, Num), (MUIA_Group_Rows, Num), (MUIA_Group_Horiz, Num),
    (MUIA_Group_SameWidth, Num), (MUIA_Group_SameHeight, Num), (MUIA_Group_SameSize, Num), (MUIA_Group_Spacing, Num), (MUIA_Group_HorizSpacing, Num),
    (MUIA_Group_VertSpacing, Num), (MUIA_Group_PageMode, Num), (MUIA_Group_ActivePage, Num), (MUIA_Group_Forward, Num), (MUIA_Group_LayoutHook, Num),
    (MUIA_Group_ChildList, Num)];
const REGISTER: &[(u32, Ty)] = &[(MUIA_Register_Titles, StrArr), (MUIA_Register_Frame, Num)];
const VIRTGROUP: &[(u32, Ty)] = &[(MUIA_Virtgroup_Left, Num), (MUIA_Virtgroup_Top, Num), (MUIA_Virtgroup_Width, Num), (MUIA_Virtgroup_Height, Num),
    (MUIA_Virtgroup_Input, Num)];
const SCROLLGROUP: &[(u32, Ty)] = &[(MUIA_Scrollgroup_Contents, Adopt), (MUIA_Scrollgroup_FreeHoriz, Num), (MUIA_Scrollgroup_FreeVert, Num),
    (MUIA_Scrollgroup_HorizBar, Num), (MUIA_Scrollgroup_VertBar, Num), (MUIA_Scrollgroup_UseWinBorder, Num)];
const TEXT: &[(u32, Ty)] = &[(MUIA_Text_Contents, Str), (MUIA_Text_PreParse, Str), (MUIA_Text_SetMax, Num), (MUIA_Text_SetMin, Num),
    (MUIA_Text_SetVMax, Num), (MUIA_Text_HiChar, Num), (MUIA_Text_ControlChar, Num), (MUIA_Text_Copy, Num), (MUIA_Text_HiCharIdx, Num)];
const STRING: &[(u32, Ty)] = &[(MUIA_String_Contents, Str), (MUIA_String_Accept, Str), (MUIA_String_Reject, Str), (MUIA_String_Acknowledge, Str),
    (MUIA_String_MaxLen, Num), (MUIA_String_Integer, Num), (MUIA_String_Secret, Num), (MUIA_String_AdvanceOnCR, Num), (MUIA_String_AttachedList, Num),
    (MUIA_String_BufferPos, Num), (MUIA_String_DisplayPos, Num), (MUIA_String_EditHook, Num), (MUIA_String_Format, Num), (MUIA_String_LonelyEditHook, Num)];
const CYCLE: &[(u32, Ty)] = &[(MUIA_Cycle_Active, Num), (MUIA_Cycle_Entries, StrArr)];
const RADIO: &[(u32, Ty)] = &[(MUIA_Radio_Active, Num), (MUIA_Radio_Entries, StrArr)];
const NUMERIC: &[(u32, Ty)] = &[(MUIA_Numeric_Value, Num), (MUIA_Numeric_Min, Num), (MUIA_Numeric_Max, Num), (MUIA_Numeric_Default, Num),
    (MUIA_Numeric_Format, Str), (MUIA_Numeric_Reverse, Num), (MUIA_Numeric_RevLeftRight, Num), (MUIA_Numeric_RevUpDown, Num),
    (MUIA_Numeric_CheckAllSizes, Num)];
const SLIDER: &[(u32, Ty)] = &[(MUIA_Slider_Horiz, Num), (MUIA_Slider_Quiet, Num)];
const GAUGE: &[(u32, Ty)] = &[(MUIA_Gauge_Current, Num), (MUIA_Gauge_Max, Num), (MUIA_Gauge_Horiz, Num), (MUIA_Gauge_Divide, Num),
    (MUIA_Gauge_InfoText, Str), (MUIA_Gauge_InfoRate, Num), (MUIA_Gauge_FullFrame, Num), (MUIA_Gauge_EmptyImage, Num), (MUIA_Gauge_FullImage, Num)];
const PROP: &[(u32, Ty)] = &[(MUIA_Prop_First, Num), (MUIA_Prop_Entries, Num), (MUIA_Prop_Visible, Num), (MUIA_Prop_Horiz, Num),
    (MUIA_Prop_Slider, Num), (MUIA_Prop_UseWinBorder, Num)];
const SCROLLBAR: &[(u32, Ty)] = &[(MUIA_Scrollbar_Type, Num)];
const LIST: &[(u32, Ty)] = &[(MUIA_List_Active, Num), (MUIA_List_Quiet, Num), (MUIA_List_Format, Str), (MUIA_List_Title, StrOrFlag),
    (MUIA_List_AutoVisible, Num), (MUIA_List_AdjustWidth, Num), (MUIA_List_AdjustHeight, Num), (MUIA_List_MinLineHeight, Num),
    (MUIA_List_ConstructHook, Num), (MUIA_List_DestructHook, Num), (MUIA_List_DisplayHook, Num), (MUIA_List_CompareHook, Num),
    (MUIA_List_MultiTestHook, Num), (MUIA_List_Pool, Num), (MUIA_List_PoolPuddleSize, Num), (MUIA_List_PoolThreshSize, Num),
    (MUIA_List_InsertPosition, Num), (MUIA_List_DragSortable, Num), (MUIA_List_ShowDropMarks, Num), (MUIA_List_DropMark, Num),
    (MUIA_List_SourceArray, Num), (MUIA_List_Entries, Num), (MUIA_List_First, Num), (MUIA_List_Visible, Num)];
const LISTVIEW: &[(u32, Ty)] = &[(MUIA_Listview_List, Adopt), (MUIA_Listview_DoubleClick, Num), (MUIA_Listview_MultiSelect, Num),
    (MUIA_Listview_ScrollerPos, Num), (MUIA_Listview_Input, Num), (MUIA_Listview_ClickColumn, Num), (MUIA_Listview_DefClickColumn, Num),
    (MUIA_Listview_SelectChange, Num), (MUIA_Listview_DragType, Num)];
const POPSTRING: &[(u32, Ty)] = &[(MUIA_Popstring_String, Adopt), (MUIA_Popstring_Button, Adopt), (MUIA_Popstring_Toggle, Num),
    (MUIA_Popstring_OpenHook, Num), (MUIA_Popstring_CloseHook, Num)];
const POPASL: &[(u32, Ty)] = &[(MUIA_Popasl_Type, Num), (MUIA_Popasl_StartHook, Num), (MUIA_Popasl_StopHook, Num), (MUIA_Popasl_Active, Num)];
const POPOBJECT: &[(u32, Ty)] = &[(MUIA_Popobject_Object, Adopt), (MUIA_Popobject_ObjStrHook, Num), (MUIA_Popobject_StrObjHook, Num),
    (MUIA_Popobject_Follow, Num), (MUIA_Popobject_Light, Num), (MUIA_Popobject_Volatile, Num), (MUIA_Popobject_WindowHook, Num)];

// Zune's image classes (bitmap.h, chunkyimage.h, bodychunk.h, imagedisplay.h). The data they hold is the program's (pixel and palette pointers are
// never dereferenced here); their pixels are NOT drawn (docs/MUI-NATIVE.md "Not done"), they take their place in the layout as Areas.
const BITMAP: &[(u32, Ty)] = &[(MUIA_Bitmap_Alpha, Num), (MUIA_Bitmap_Bitmap, Num), (MUIA_Bitmap_Height, Num), (MUIA_Bitmap_MappingTable, Num),
    (MUIA_Bitmap_Precision, Num), (MUIA_Bitmap_RemappedBitmap, Num), (MUIA_Bitmap_SourceColors, Num), (MUIA_Bitmap_Transparent, Num),
    (MUIA_Bitmap_UseFriend, Num), (MUIA_Bitmap_Width, Num)];
const CHUNKYIMAGE: &[(u32, Ty)] = &[(MUIA_ChunkyImage_Pixels, Num), (MUIA_ChunkyImage_Palette, Num), (MUIA_ChunkyImage_NumColors, Num), (MUIA_ChunkyImage_Modulo, Num)];
const BODYCHUNK: &[(u32, Ty)] = &[(MUIA_Bodychunk_Body, Num), (MUIA_Bodychunk_Compression, Num), (MUIA_Bodychunk_Depth, Num), (MUIA_Bodychunk_Masking, Num)];
const IMAGEDISPLAY: &[(u32, Ty)] = &[(MUIA_Imagedisplay_Spec, Num), (MUIA_Imagedisplay_UseDefSize, Num), (MUIA_Imagedisplay_FreeHoriz, Num), (MUIA_Imagedisplay_FreeVert, Num)];

/// The attributes `kind` itself defines (ancestors' are looked up separately).
fn own(kind: Kind) -> &'static [(u32, Ty)] {
    match kind {
        Kind::Notify => NOTIFY, Kind::Family => FAMILY, Kind::Application => APPLICATION, Kind::Window => WINDOW,
        Kind::Menustrip => MENUSTRIP, Kind::Menu => MENU, Kind::Menuitem => MENUITEM, Kind::Area => AREA, Kind::Rectangle => RECTANGLE,
        Kind::Balance => BALANCE, Kind::Image => IMAGE, Kind::Group => GROUP, Kind::Virtgroup => VIRTGROUP, Kind::Scrollgroup => SCROLLGROUP,
        Kind::Register => REGISTER, Kind::Text => TEXT, Kind::String => STRING, Kind::Cycle => CYCLE, Kind::Radio => RADIO,
        Kind::Numeric => NUMERIC, Kind::Slider => SLIDER, Kind::Gauge => GAUGE, Kind::Prop => PROP, Kind::Scrollbar => SCROLLBAR,
        Kind::List => LIST, Kind::Listview => LISTVIEW, Kind::Popstring => POPSTRING, Kind::Popasl => POPASL, Kind::Popobject => POPOBJECT,
        Kind::Bitmap => BITMAP, Kind::ChunkyImage => CHUNKYIMAGE, Kind::Bodychunk => BODYCHUNK, Kind::Imagedisplay => IMAGEDISPLAY,
    }
}

/// How `kind` holds attribute `attr`, or `None` when its class tree does not define it.
pub fn lookup(kind: Kind, attr: u32) -> Option<Ty> {
    // Popasl hands every ASL tag it is given to the requester it opens (asl/asl.h ASL_Dummy range): held as given.
    if kind == Kind::Popasl && (0x8008_0000..0x8009_0000).contains(&attr) { return Some(Num); }
    // Scrollbar.mui builds a Prop and forwards the Prop attributes to it (Zune scrollbar.c), so it answers for them itself.
    if kind == Kind::Scrollbar { if let Some(&(_, t)) = PROP.iter().find(|e| e.0 == attr) { return Some(t); } }
    let mut k = Some(kind);
    while let Some(x) = k {
        if let Some(&(_, t)) = own(x).iter().find(|e| e.0 == attr) { return Some(t); }
        k = x.sup();
    }
    None
}

/// `attr`'s MUIA_ name for a refusal message (the number when the header has none).
pub fn name(attr: u32) -> alloc::string::String {
    match super::names::ATTR_NAMES.binary_search_by_key(&attr, |e| e.0) {
        Ok(i) => super::names::ATTR_NAMES[i].1.into(),
        Err(_) => alloc::format!("{attr:#010x}"),
    }
}

/// Is `attr` an attribute some MUI class defines (it has a name in MUI's headers)?
pub fn is_mui_attribute(attr: u32) -> bool { super::names::ATTR_NAMES.binary_search_by_key(&attr, |e| e.0).is_ok() }

/// `id`'s MUIM_ name for a refusal message (the number when the header has none).
pub fn method_name(id: u32) -> alloc::string::String {
    match super::names::METHOD_NAMES.binary_search_by_key(&id, |e| e.0) {
        Ok(i) => super::names::METHOD_NAMES[i].1.into(),
        Err(_) => alloc::format!("method {id:#010x}"),
    }
}

/// A Group subclass that is not a plain container still takes the Group attributes (Zune: Cycle < Group ...); this is the
/// list of the ones whose children come from tags other than MUIA_Group_Child.
pub fn adopts(kind: Kind, attr: u32) -> bool { lookup(kind, attr) == Some(Adopt) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookups_walk_the_class_tree_and_unknown_attributes_say_so() {
        assert_eq!(lookup(Kind::Slider, MUIA_Numeric_Value), Some(Num), "Slider < Numeric");
        assert_eq!(lookup(Kind::Slider, MUIA_Frame), Some(Num), "Numeric < Area");
        assert_eq!(lookup(Kind::Cycle, MUIA_Group_Child), Some(Adopt), "Cycle < Group in Zune");
        assert_eq!(lookup(Kind::Text, MUIA_Group_Columns), None, "Text is no Group");
        assert_eq!(name(MUIA_Window_Open), "MUIA_Window_Open");
        assert_eq!(name(0x8042_ffff), "0x8042ffff");
    }
}
