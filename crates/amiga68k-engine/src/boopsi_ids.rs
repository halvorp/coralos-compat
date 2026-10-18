//! Guest struct offsets and numeric ids of the BOOPSI classes (intuition/classes.h, classusr.h, gadgetclass.h,
//! imageclass.h, icclass.h, cghooks.h). Offsets come from `tests/guest/boopsi_offsets.c` compiled with
//! m68k-linux-gnu-gcc -m68020 -mno-align-int over the AROS field lists (interface facts only); `tests/boopsi_layout.rs`
//! compares every constant here to that output. Tag/method values are the AROS header values.
use crate::tags::TAG_USER;

// struct IClass (60 bytes): cl_Dispatcher is a struct Hook at offset 0, so a class pointer IS its hook.
pub const CL_SIZE: u32 = 60;
pub const CL_H_ENTRY: u32 = 8;
pub const CL_H_DATA: u32 = 16;
pub const CL_SUPER: u32 = 24;
pub const CL_ID: u32 = 28;
pub const CL_INST_OFFSET: u32 = 32;
pub const CL_INST_SIZE: u32 = 34;
pub const CL_USER_DATA: u32 = 36;
pub const CL_SUBCLASS_COUNT: u32 = 40;
pub const CL_OBJECT_COUNT: u32 = 44;
pub const CL_FLAGS: u32 = 48;
pub const CL_OBJECT_SIZE: u32 = 52;
pub const CLF_INLIST: u32 = 1;
/// `struct _Object` (MinNode + o_Class) precedes every object pointer.
pub const OBJ_HEADER: u32 = 12;
pub const OBJ_CLASS: u32 = 8;

// struct Gadget / ExtGadget / GadgetData (ExtGadget + ICData, the gadgetclass instance).
pub const GAD_NEXT: u32 = 0;
pub const GAD_LEFT: u32 = 4;
pub const GAD_TOP: u32 = 6;
pub const GAD_WIDTH: u32 = 8;
pub const GAD_HEIGHT: u32 = 10;
pub const GAD_FLAGS: u32 = 12;
pub const GAD_ACTIVATION: u32 = 14;
pub const GAD_TYPE: u32 = 16;
pub const GAD_RENDER: u32 = 18;
pub const GAD_SELECT: u32 = 22;
pub const GAD_TEXT: u32 = 26;
pub const GAD_MUTUAL: u32 = 30;
pub const GAD_SPECIAL: u32 = 34;
pub const GAD_ID: u32 = 38;
pub const GAD_USER: u32 = 40;
pub const EGAD_MORE_FLAGS: u32 = 44;
pub const EGAD_BOUNDS: u32 = 48;
pub const GDATA_SIZE: u32 = 72;
pub const GDATA_IC: u32 = 56;
pub const IC_TARGET: u32 = 0;
pub const IC_MAPPING: u32 = 4;
// struct Image.
pub const IMG_LEFT: u32 = 0;
pub const IMG_TOP: u32 = 2;
pub const IMG_WIDTH: u32 = 4;
pub const IMG_HEIGHT: u32 = 6;
pub const IMG_DEPTH: u32 = 8;
pub const IMG_DATA: u32 = 10;
pub const IMG_PLANE_PICK: u32 = 14;
pub const IMG_PLANE_ONOFF: u32 = 15;
pub const IMG_SIZE: u32 = 20;
// struct GadgetInfo.
pub const GI_SIZE: u32 = 58;
pub const GI_SCREEN: u32 = 0;
pub const GI_WINDOW: u32 = 4;
pub const GI_RASTPORT: u32 = 12;
pub const GI_LAYER: u32 = 16;
pub const GI_DOMAIN: u32 = 20;
pub const GI_PENS: u32 = 28;
pub const GI_DRINFO: u32 = 30;
// Message structs.
pub const OPSET_SIZE: u32 = 12;
pub const OPGET_SIZE: u32 = 12;
pub const OPSET_GINFO: u32 = 8;
pub const GPR_SIZE: u32 = 16;
pub const GPR_RPORT: u32 = 8;
pub const GPR_REDRAW: u32 = 12;
pub const GPHT_SIZE: u32 = 12;
pub const GPHT_MOUSE: u32 = 8;
pub const GPI_SIZE: u32 = 24;
pub const GPI_IEVENT: u32 = 8;
pub const GPI_TERMINATION: u32 = 12;
pub const GPI_MOUSE: u32 = 16;
pub const GPGI_SIZE: u32 = 12;
pub const GPGI_ABORT: u32 = 8;
pub const GPL_SIZE: u32 = 12;
pub const GPL_INITIAL: u32 = 8;

// Methods.
pub const OM_NEW: u32 = 0x101;
pub const OM_DISPOSE: u32 = 0x102;
pub const OM_SET: u32 = 0x103;
pub const OM_GET: u32 = 0x104;
pub const OM_ADDTAIL: u32 = 0x105;
pub const OM_REMOVE: u32 = 0x106;
pub const OM_NOTIFY: u32 = 0x107;
pub const OM_UPDATE: u32 = 0x108;
pub const OM_ADDMEMBER: u32 = 0x109;
pub const OM_REMMEMBER: u32 = 0x10A;
pub const GM_HITTEST: u32 = 0;
pub const GM_RENDER: u32 = 1;
pub const GM_GOACTIVE: u32 = 2;
pub const GM_HANDLEINPUT: u32 = 3;
pub const GM_GOINACTIVE: u32 = 4;
pub const GM_HELPTEST: u32 = 5;
pub const GM_LAYOUT: u32 = 6;
pub const GM_DOMAIN: u32 = 7;
pub const IM_DRAW: u32 = 0x202;
pub const IM_HITTEST: u32 = 0x203;
pub const IM_ERASE: u32 = 0x204;
pub const GMR_GADGETHIT: u32 = 4;
pub const GMR_NOREUSE: u32 = 2;
pub const GREDRAW_REDRAW: u32 = 1;

// Gadget flag bits.
pub const GFLG_GADGHIGHBITS: u32 = 3;
pub const GFLG_GADGHIMAGE: u32 = 2;
pub const GFLG_GADGIMAGE: u32 = 1 << 2;
pub const GFLG_RELBOTTOM: u32 = 1 << 3;
pub const GFLG_RELRIGHT: u32 = 1 << 4;
pub const GFLG_RELWIDTH: u32 = 1 << 5;
pub const GFLG_RELHEIGHT: u32 = 1 << 6;
pub const GFLG_SELECTED: u32 = 1 << 7;
pub const GFLG_DISABLED: u32 = 1 << 8;
pub const GFLG_TABCYCLE: u32 = 1 << 9;
pub const GFLG_LABELSTRING: u32 = 1 << 12;
pub const GFLG_LABELIMAGE: u32 = 1 << 13;
pub const GFLG_LABELMASK: u32 = 0x3000;
pub const GFLG_RELSPECIAL: u32 = 1 << 14;
pub const GFLG_EXTENDED: u32 = 1 << 15;
pub const GACT_RELVERIFY: u32 = 1;
pub const GACT_IMMEDIATE: u32 = 2;
pub const GACT_ENDGADGET: u32 = 4;
pub const GACT_FOLLOWMOUSE: u32 = 8;
pub const GACT_RIGHTBORDER: u32 = 0x10;
pub const GACT_LEFTBORDER: u32 = 0x20;
pub const GACT_TOPBORDER: u32 = 0x40;
pub const GACT_BOTTOMBORDER: u32 = 0x80;
pub const GACT_TOGGLESELECT: u32 = 0x100;
pub const GTYP_SYSTYPEMASK: u32 = 0xF0;
pub const GTYP_GZZGADGET: u32 = 1 << 13;
pub const GTYP_SYSGADGET: u32 = 1 << 15;
pub const GTYP_CUSTOMGADGET: u32 = 5;
pub const GTYP_GTYPEMASK: u32 = 7;
pub const GMORE_BOUNDS: u32 = 1;
pub const GMORE_GADGETHELP: u32 = 2;
pub const GMORE_BOOPSIGADGET: u32 = 8;

// Tags.
pub const IA: u32 = TAG_USER + 0x20000;
pub const IA_LEFT: u32 = IA + 1;
pub const IA_TOP: u32 = IA + 2;
pub const IA_WIDTH: u32 = IA + 3;
pub const IA_HEIGHT: u32 = IA + 4;
pub const IA_FGPEN: u32 = IA + 5;
pub const IA_BGPEN: u32 = IA + 6;
pub const IA_DATA: u32 = IA + 7;
pub const GA: u32 = TAG_USER + 0x30000;
pub const GA_LEFT: u32 = GA + 1;
pub const GA_REL_RIGHT: u32 = GA + 2;
pub const GA_TOP: u32 = GA + 3;
pub const GA_REL_BOTTOM: u32 = GA + 4;
pub const GA_WIDTH: u32 = GA + 5;
pub const GA_REL_WIDTH: u32 = GA + 6;
pub const GA_HEIGHT: u32 = GA + 7;
pub const GA_REL_HEIGHT: u32 = GA + 8;
pub const GA_TEXT: u32 = GA + 9;
pub const GA_IMAGE: u32 = GA + 10;
pub const GA_BORDER: u32 = GA + 11;
pub const GA_SELECT_RENDER: u32 = GA + 12;
pub const GA_HIGHLIGHT: u32 = GA + 13;
pub const GA_DISABLED: u32 = GA + 14;
pub const GA_GZZ_GADGET: u32 = GA + 15;
pub const GA_ID: u32 = GA + 16;
pub const GA_USER_DATA: u32 = GA + 17;
pub const GA_SPECIAL_INFO: u32 = GA + 18;
pub const GA_SELECTED: u32 = GA + 19;
pub const GA_END_GADGET: u32 = GA + 20;
pub const GA_IMMEDIATE: u32 = GA + 21;
pub const GA_REL_VERIFY: u32 = GA + 22;
pub const GA_FOLLOW_MOUSE: u32 = GA + 23;
pub const GA_RIGHT_BORDER: u32 = GA + 24;
pub const GA_LEFT_BORDER: u32 = GA + 25;
pub const GA_TOP_BORDER: u32 = GA + 26;
pub const GA_BOTTOM_BORDER: u32 = GA + 27;
pub const GA_TOGGLE_SELECT: u32 = GA + 28;
pub const GA_SYS_GADGET: u32 = GA + 29;
pub const GA_SYS_GTYPE: u32 = GA + 30;
pub const GA_PREVIOUS: u32 = GA + 31;
pub const GA_NEXT: u32 = GA + 32;
pub const GA_INTUI_TEXT: u32 = GA + 34;
pub const GA_LABEL_IMAGE: u32 = GA + 35;
pub const GA_TAB_CYCLE: u32 = GA + 36;
pub const GA_GADGET_HELP: u32 = GA + 37;
pub const GA_BOUNDS: u32 = GA + 38;
pub const GA_REL_SPECIAL: u32 = GA + 39;
pub const ICA_TARGET: u32 = TAG_USER + 0x40000 + 1;
pub const ICA_MAP: u32 = TAG_USER + 0x40000 + 2;

/// strgclass instance (`StrGData`): `struct StringInfo`: Buffer 0, UndoBuffer 4, BufferPos 8, MaxChars 10, DispPos 12, UndoPos 14,
/// NumChars 16, DispCount 18, CLeft 20, CTop 22, Extension 24, LongInt 28, AltKeyMap 32 = 36 bytes.
pub const FRAME_DATA: u32 = 10; // type, HOffset, VOffset, Recessed, EdgesOnly (UWORDs)
pub const PROP_DATA: u32 = 36; // PropInfo 22 + pad 2 + Total, Visible, Top (u32 each)
pub const STRG_DATA: u32 = 76; // StringInfo 36 + StringExtend 36 + flags 4
