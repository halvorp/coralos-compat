//! graphics.library VideoControl(a0 = ColorMap, a1 = tags) -> 0, or 1 when a tag is not a videocontrol tag (rom/graphics/videocontrol.c, read for
//! behaviour; tags in graphics/videocontrol.h). The ColorMap is the guest's own struct (graphics/view.h): the SET / CLR tags write its fields and the
//! GET tags rewrite the TAG ITEM in place (ti_Tag becomes the matching SET / CLR tag, ti_Data the value), which is how a program reads it back.
//! Offsets of struct ColorMap (m68k -mno-align-int, 52 bytes = `CM_SIZE`): Flags 0, Type 1, Count 2, ColorTable 4, cm_vpe 8, LowColorBits 12,
//! TransparencyPlane 16, SpriteResolution 17, SpriteResDefault 18, AuxFlags 19, cm_vp 20, NormalDisplayInfo 24, CoerceDisplayInfo 28, cm_batch_items 32,
//! VPModeID 36, PalExtra 40, SpriteBase_Even 44, SpriteBase_Odd 46, Bp_0_base 48, Bp_1_base 50. Nothing here reaches the (single, fixed) display: the
//! chroma-key / sprite / dual-playfield tags are stored or answered as AROS does (its TODO ones answer 0 and store nothing). Not done: GfxAssociate of a
//! ViewPortExtra (there is no hash chain of ViewPorts here), VTAG_IMMEDIATE's driver update (the result word is cleared, as AROS does).
use alloc::format;
use alloc::string::String;
use crate::{machine::Amiga, utility, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

const CM_FLAGS: u32 = 0;
const CM_VPE: u32 = 8;
const CM_SPRITE_RES: u32 = 17;
const CM_AUX_FLAGS: u32 = 19;
const CM_VP: u32 = 20;
const CM_NORMAL_DISP: u32 = 24;
const CM_COERCE_DISP: u32 = 28;
const CM_BATCH_ITEMS: u32 = 32;
const CM_VPMODEID: u32 = 36;
const CM_SPRITE_EVEN: u32 = 44;
const CM_SPRITE_ODD: u32 = 46;
const CM_BP0: u32 = 48;
const CM_BP1: u32 = 50;
// ColorMap.Flags / AuxFlags (view.h:189-195, 256-263)
const BORDER_BLANKING: u8 = 1 << 2;
const BORDER_NOTRANS: u8 = 1 << 3;
const VIDEOCONTROL_BATCH: u8 = 1 << 4;
const USER_COPPER_CLIP: u8 = 1 << 5;
const BORDERSPRITES: u8 = 1 << 6;
const FULLPALETTE: u8 = 1 << 0;
const NO_INTERMED_UPDATE: u8 = 1 << 1;
const NO_COLOR_LOAD: u8 = 1 << 2;
const DUALPF_DISABLE: u8 = 1 << 3;
const INVALID_ID: u32 = 0xFFFF_FFFF;
const VB: u32 = 0x8000_0000; // videocontrol.h tag base

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> { b.named("VideoControl", video_control::<U>) }

fn video_control<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (cm, tags) = (cpu.areg(0), cpu.areg(1));
    if cm == 0 { return Err(Refusal("VideoControl: NULL ColorMap".into())); }
    let m = Refusal::mem("ColorMap");
    let mut st = tags;
    let (mut res, mut immediate, mut n) = (0u32, 0u32, 0u32);
    while let Some((at, tag, data)) = utility::next_item(a, &mut st)? {
        n += 1;
        // helpers over the struct and the tag item
        macro_rules! rd { ($o:expr, L) => { a.mem.read_u32(cm + $o).map_err(&m)? }; ($o:expr, W) => { a.mem.read_u16(cm + $o).map_err(&m)? as u32 }; ($o:expr, B) => { a.mem.read_u8(cm + $o).map_err(&m)? as u32 }; }
        macro_rules! wr { ($o:expr, L, $v:expr) => { a.mem.write_u32(cm + $o, $v).map_err(&m)? }; ($o:expr, W, $v:expr) => { a.mem.write_u16(cm + $o, $v as u16).map_err(&m)? }; ($o:expr, B, $v:expr) => { a.mem.write_u8(cm + $o, $v as u8).map_err(&m)? }; }
        macro_rules! answer { ($t:expr, $d:expr) => {{ a.mem.write_u32(at, $t).map_err(&m)?; a.mem.write_u32(at + 4, $d).map_err(&m)?; }}; }
        macro_rules! flag { ($o:expr, $bit:expr, $set:expr) => {{ let v = rd!($o, B) as u8; wr!($o, B, if $set { v | $bit } else { v & !$bit }); }}; }
        // a GET on a flag answers SET/CLR with TRUE/FALSE (videocontrol.c)
        macro_rules! flag_get { ($o:expr, $bit:expr, $set_tag:expr, $clr_tag:expr) => {{ if rd!($o, B) as u8 & $bit != 0 { answer!($set_tag, 1) } else { answer!($clr_tag, 0) } }}; }
        match tag {
            t if t == VB + 0x0B => { wr!(CM_VP, L, data); }                                   // VTAG_ATTACH_CM_SET
            t if t == VB + 0x1B => { let v = rd!(CM_VP, L); answer!(VB + 0x0B, v) }           // VTAG_ATTACH_CM_GET
            t if t == VB + 0x14 => { wr!(CM_VPE, L, data); }                                  // VTAG_VIEWPORTEXTRA_SET
            t if t == VB + 0x13 => { let v = rd!(CM_VPE, L); answer!(VB + 0x14, v) }          // VTAG_VIEWPORTEXTRA_GET
            t if t == VB + 0x10 => { wr!(CM_NORMAL_DISP, L, data); }
            t if t == VB + 0x0F => { let v = rd!(CM_NORMAL_DISP, L); answer!(VB + 0x10, v) }
            t if t == VB + 0x12 => { wr!(CM_COERCE_DISP, L, data); }
            t if t == VB + 0x11 => { let v = rd!(CM_COERCE_DISP, L); answer!(VB + 0x12, v) }
            t if t == VB + 0x2A => { wr!(CM_BP0, W, data); }                                  // VTAG_PF1_BASE_SET
            t if t == VB + 0x26 => { let v = rd!(CM_BP0, W); answer!(VB + 0x2A, v) }
            t if t == VB + 0x2B => { wr!(CM_BP1, W, data); }
            t if t == VB + 0x27 => { let v = rd!(CM_BP1, W); answer!(VB + 0x2B, v) }
            t if t == VB + 0x2D => { wr!(CM_SPRITE_ODD, W, data); }
            t if t == VB + 0x29 => { let v = rd!(CM_SPRITE_ODD, W); answer!(VB + 0x2D, v) }
            t if t == VB + 0x2C => { wr!(CM_SPRITE_EVEN, W, data); }
            t if t == VB + 0x28 => { let v = rd!(CM_SPRITE_EVEN, W); answer!(VB + 0x2C, v) }
            t if t == VB + 0x2F => flag!(CM_FLAGS, BORDERSPRITES, true),                       // VTAG_BORDERSPRITE_SET
            t if t == VB + 0x30 => flag!(CM_FLAGS, BORDERSPRITES, false),
            t if t == VB + 0x2E => flag_get!(CM_FLAGS, BORDERSPRITES, VB + 0x2F, VB + 0x30),
            t if t == VB + 0x31 => { wr!(CM_SPRITE_RES, B, data); }                           // VTAG_SPRITERESN_SET
            t if t == VB + 0x32 => { let v = rd!(CM_SPRITE_RES, B); answer!(VB + 0x31, v) }
            t if t == VB + 0x33 || t == VB + 0x35 => {}                                        // PFn_TO_SPRITEPRI_SET: AROS "TODO", nothing stored
            t if t == VB + 0x34 => answer!(VB + 0x33, 0),
            t if t == VB + 0x36 => answer!(VB + 0x35, 0),
            t if t == VB + 0x05 => flag!(CM_FLAGS, BORDER_BLANKING, true),                     // VTAG_BORDERBLANK_SET
            t if t == VB + 0x04 => flag!(CM_FLAGS, BORDER_BLANKING, false),
            t if t == VB + 0x17 => flag_get!(CM_FLAGS, BORDER_BLANKING, VB + 0x05, VB + 0x04),
            t if t == VB + 0x07 => flag!(CM_FLAGS, BORDER_NOTRANS, true),                      // VTAG_BORDERNOTRANS_SET
            t if t == VB + 0x06 => flag!(CM_FLAGS, BORDER_NOTRANS, false),
            t if t == VB + 0x18 => flag_get!(CM_FLAGS, BORDER_NOTRANS, VB + 0x07, VB + 0x06),
            // chroma key / bitplane key / chroma pen / chroma plane: AROS stores nothing and answers the CLR form with 0
            t if matches!(t, x if x == VB || x == VB + 1 || x == VB + 2 || x == VB + 3 || x == VB + 8 || x == VB + 9 || x == VB + 0x0A) => {}
            t if t == VB + 0x15 => answer!(VB, 0),
            t if t == VB + 0x16 => answer!(VB + 2, 0),
            t if t == VB + 0x19 => answer!(VB + 8, 0),
            t if t == VB + 0x1A => answer!(VB + 0x0A, 0),
            t if t == VB + 0x37 => immediate = data,                                           // VTAG_IMMEDIATE
            t if t == VB + 0x38 => flag!(CM_AUX_FLAGS, FULLPALETTE, true),
            t if t == VB + 0x3A => flag!(CM_AUX_FLAGS, FULLPALETTE, false),
            t if t == VB + 0x39 => flag_get!(CM_AUX_FLAGS, FULLPALETTE, VB + 0x38, VB + 0x3A),
            t if t == VB + 0x80 => flag!(CM_AUX_FLAGS, NO_INTERMED_UPDATE, data == 0),         // VC_IntermediateCLUpdate
            t if t == VB + 0x81 => { let v = (rd!(CM_AUX_FLAGS, B) as u8 & NO_INTERMED_UPDATE == 0) as u32; a.mem.write_u32(data, v).map_err(&m)?; }
            t if t == VB + 0x82 => flag!(CM_AUX_FLAGS, NO_COLOR_LOAD, data != 0),             // VC_NoColorPaletteLoad
            t if t == VB + 0x83 => { let v = (rd!(CM_AUX_FLAGS, B) as u8 & NO_COLOR_LOAD != 0) as u32; a.mem.write_u32(data, v).map_err(&m)?; }
            t if t == VB + 0x84 => flag!(CM_AUX_FLAGS, DUALPF_DISABLE, data != 0),            // VC_DUALPF_Disable
            t if t == VB + 0x85 => { let v = (rd!(CM_AUX_FLAGS, B) as u8 & DUALPF_DISABLE != 0) as u32; a.mem.write_u32(data, v).map_err(&m)?; }
            t if t == VB + 0x24 => flag!(CM_FLAGS, USER_COPPER_CLIP, true),                    // VTAG_USERCLIP_SET
            t if t == VB + 0x25 => flag!(CM_FLAGS, USER_COPPER_CLIP, false),
            t if t == VB + 0x23 => flag_get!(CM_FLAGS, USER_COPPER_CLIP, VB + 0x24, VB + 0x25),
            t if t == VB + 0x0C => st = data,                                                  // VTAG_NEXTBUF_CM: carry on with another list
            t if t == VB + 0x0E => flag!(CM_FLAGS, VIDEOCONTROL_BATCH, true),                  // VTAG_BATCH_CM_SET
            t if t == VB + 0x0D => flag!(CM_FLAGS, VIDEOCONTROL_BATCH, false),
            t if t == VB + 0x1C => flag_get!(CM_FLAGS, VIDEOCONTROL_BATCH, VB + 0x0E, VB + 0x0D),
            t if t == VB + 0x1E => { wr!(CM_BATCH_ITEMS, L, data); }                           // VTAG_BATCH_ITEMS_SET
            t if t == VB + 0x1F => {}                                                           // VTAG_BATCH_ITEMS_ADD: AROS "TODO"
            t if t == VB + 0x1D => { let v = rd!(CM_BATCH_ITEMS, L); answer!(VB + 0x1E, v) }
            t if t == VB + 0x21 => { wr!(CM_VPMODEID, L, data); }                              // VTAG_VPMODEID_SET
            t if t == VB + 0x22 => { wr!(CM_VPMODEID, L, INVALID_ID); }
            t if t == VB + 0x20 => { let v = rd!(CM_VPMODEID, L); answer!(if v == INVALID_ID { VB + 0x22 } else { VB + 0x21 }, v) }
            _ => res = 1, // not a videocontrol tag (AROS: error 1, the walk continues)
        }
    }
    if immediate != 0 { a.mem.write_u32(immediate, 0).map_err(Refusal::mem("VTAG_IMMEDIATE result"))?; }
    cpu.set_d0(res);
    Ok(format!("[graphics] VideoControl {cm:#x}: {n} tags -> {res}"))
}
