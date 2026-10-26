/* Struct layouts graphics.library hands to programs, transcribed from the AROS 'compiler/include' field lists
 * (graphics/rastport.h, gfx.h, text.h, regions.h, clip.h, view.h; intuition/screens.h, intuition.h) as plain C
 * (exec types spelled out) so m68k gcc computes the m68k ABI offsets. Interface facts only; no AROS code.
 *   m68k-linux-gnu-gcc -m68020 -mno-align-int -S -o gfx_offsets.s gfx_offsets.c ; tests/guest/gfx_offsets.sh
 * writes gfx_offsets.txt (NAME VALUE lines) which tests/gfx_layout.rs compares to src/graphics/layout.rs. */
#include <stddef.h>
typedef short WORD; typedef unsigned short UWORD; typedef signed char BYTE; typedef unsigned char UBYTE;
typedef int LONG; typedef unsigned int ULONG; typedef void *APTR;
struct Node { struct Node *ln_Succ, *ln_Pred; UBYTE ln_Type; BYTE ln_Pri; char *ln_Name; };
struct MinNode { struct MinNode *mln_Succ, *mln_Pred; };
struct MinList { struct MinNode *mlh_Head, *mlh_Tail, *mlh_TailPred; };
struct MsgPort { struct Node mp_Node; UBYTE mp_Flags, mp_SigBit; void *mp_SigTask; struct { struct Node *lh_Head, *lh_Tail, *lh_TailPred; UBYTE lh_Type, l_pad; } mp_MsgList; };
struct Message { struct Node mn_Node; struct MsgPort *mn_ReplyPort; UWORD mn_Length; };
struct SignalSemaphore { struct Node ss_Link; WORD ss_NestCount; struct MinList ss_WaitQueue; struct { struct MinNode a; void *b; } ss_MultipleLink; void *ss_Owner; WORD ss_QueueCount; };
struct Rectangle { WORD MinX, MinY, MaxX, MaxY; };
struct RastPort { void *Layer; void *BitMap; const UWORD *AreaPtrn; void *TmpRas; void *AreaInfo; void *GelsInfo;
  UBYTE Mask; BYTE FgPen, BgPen, AOlPen, DrawMode, AreaPtSz, linpatcnt, dummy; UWORD Flags, LinePtrn; WORD cp_x, cp_y; UBYTE minterms[8];
  WORD PenWidth, PenHeight; void *Font; UBYTE AlgoStyle, TxFlags; UWORD TxHeight, TxWidth, TxBaseline; WORD TxSpacing; APTR *RP_User;
  APTR RP_Extra; ULONG longreserved[6]; UBYTE reserved[2]; };
struct BitMap { UWORD BytesPerRow, Rows; UBYTE Flags, Depth; UWORD pad; UBYTE *Planes[8]; };
struct TextFont { struct Message tf_Message; UWORD tf_YSize; UBYTE tf_Style, tf_Flags; UWORD tf_XSize, tf_Baseline, tf_BoldSmear, tf_Accessors;
  UBYTE tf_LoChar, tf_HiChar; APTR tf_CharData; UWORD tf_Modulo; APTR tf_CharLoc, tf_CharSpace, tf_CharKern; };
struct TextAttr { char *ta_Name; UWORD ta_YSize; UBYTE ta_Style, ta_Flags; };
struct TextExtent { UWORD te_Width, te_Height; struct Rectangle te_Extent; };
struct RegionRectangle { struct RegionRectangle *Next, *Prev; struct Rectangle bounds; };
struct Region { struct Rectangle bounds; struct RegionRectangle *RegionRectangle; };
struct Layer { struct Layer *front, *back; void *ClipRect; struct RastPort *rp; struct Rectangle bounds; struct Layer *parent;
  UWORD priority, Flags; void *SuperBitMap, *SuperClipRect; APTR Window; WORD Scroll_X, Scroll_Y; void *cr, *cr2, *crnew, *SuperSaveClipRects, *_cliprects;
  void *LayerInfo; struct SignalSemaphore Lock; void *BackFill; struct Region *VisibleRegion, *ClipRegion, *saveClipRects; WORD Width, Height;
  struct Region *shape, *shaperegion, *visibleshape; UWORD nesting; UBYTE SuperSaveClipRectCounter, visible; UBYTE reserved2[2]; struct Region *DamageList; };
struct ViewPort { struct ViewPort *Next; void *ColorMap, *DspIns, *SprIns, *ClrIns, *UCopIns; WORD DWidth, DHeight, DxOffset, DyOffset; UWORD Modes; UBYTE SpritePriorities, ExtendedModes; void *RasInfo; };
struct ColorMap { UBYTE Flags, Type; UWORD Count; UWORD *ColorTable; void *cm_vpe; UWORD *LowColorBits; UBYTE TransparencyPlane, SpriteResolution, SpriteResDefault, AuxFlags; void *cm_vp; APTR NormalDisplayInfo, CoerceDisplayInfo; void *cm_batch_items; ULONG VPModeID; void *PalExtra; UWORD SpriteBase_Even, SpriteBase_Odd, Bp_0_base, Bp_1_base; };
struct DrawInfo { UWORD dri_Version, dri_NumPens; UWORD *dri_Pens; void *dri_Font; UWORD dri_Depth; struct { UWORD X, Y; } dri_Resolution; ULONG dri_Flags; void *dri_CheckMark, *dri_AmigaKey; ULONG dri_Reserved[5]; };
struct Screen { struct Screen *NextScreen; void *FirstWindow; WORD LeftEdge, TopEdge, Width, Height, MouseY, MouseX; UWORD Flags; UBYTE *Title, *DefaultTitle;
  BYTE BarHeight, BarVBorder, BarHBorder, MenuVBorder, MenuHBorder; BYTE WBorTop, WBorLeft, WBorRight, WBorBottom; void *Font; struct ViewPort ViewPort; struct RastPort RastPort; struct BitMap BitMap;
  struct { char x[102]; } LayerInfo; void *FirstGadget; UBYTE DetailPen, BlockPen; UWORD SaveColor0; void *BarLayer; UBYTE *ExtData, *UserData; };
struct Window { struct Window *NextWindow; WORD LeftEdge, TopEdge, Width, Height, MouseY, MouseX, MinWidth, MinHeight; UWORD MaxWidth, MaxHeight; ULONG Flags; void *MenuStrip; UBYTE *Title;
  void *FirstRequest, *DMRequest; WORD ReqCount; void *WScreen; struct RastPort *RPort; BYTE BorderLeft, BorderTop, BorderRight, BorderBottom; struct RastPort *BorderRPort; void *FirstGadget;
  struct Window *Parent, *Descendant; UWORD *Pointer; BYTE PtrHeight, PtrWidth, XOffset, YOffset; ULONG IDCMPFlags; struct MsgPort *UserPort, *WindowPort; void *MessageKey; UBYTE DetailPen, BlockPen; void *CheckMark;
  UBYTE *ScreenTitle; WORD GZZMouseX, GZZMouseY, GZZWidth, GZZHeight; UBYTE *ExtData; BYTE *UserData; struct Layer *WLayer; void *IFont; ULONG MoreFlags; };
struct Image { WORD LeftEdge, TopEdge, Width, Height, Depth; UWORD *ImageData; UBYTE PlanePick, PlaneOnOff; struct Image *NextImage; };
struct Border { WORD LeftEdge, TopEdge; UBYTE FrontPen, BackPen, DrawMode; BYTE Count; WORD *XY; struct Border *NextBorder; };
struct IntuiText { UBYTE FrontPen, BackPen, DrawMode; WORD LeftEdge, TopEdge; struct TextAttr *ITextFont; UBYTE *IText; struct IntuiText *NextText; };
#define O(n, s, f) const int n = offsetof(struct s, f);
#define S(n, s) const int n = sizeof(struct s);
S(RP_SIZE, RastPort) O(RP_LAYER, RastPort, Layer) O(RP_BITMAP, RastPort, BitMap) O(RP_AREA_PTRN, RastPort, AreaPtrn) O(RP_TMP_RAS, RastPort, TmpRas) O(RP_AREA_INFO, RastPort, AreaInfo)
O(RP_MASK, RastPort, Mask) O(RP_FG_PEN, RastPort, FgPen) O(RP_BG_PEN, RastPort, BgPen) O(RP_AOL_PEN, RastPort, AOlPen) O(RP_DRAW_MODE, RastPort, DrawMode) O(RP_AREA_PT_SZ, RastPort, AreaPtSz)
O(RP_LINPATCNT, RastPort, linpatcnt) O(RP_FLAGS, RastPort, Flags) O(RP_LINE_PTRN, RastPort, LinePtrn) O(RP_CP_X, RastPort, cp_x) O(RP_CP_Y, RastPort, cp_y) O(RP_MINTERMS, RastPort, minterms)
O(RP_PEN_WIDTH, RastPort, PenWidth) O(RP_PEN_HEIGHT, RastPort, PenHeight) O(RP_FONT, RastPort, Font) O(RP_ALGO_STYLE, RastPort, AlgoStyle) O(RP_TX_FLAGS, RastPort, TxFlags)
O(RP_TX_HEIGHT, RastPort, TxHeight) O(RP_TX_WIDTH, RastPort, TxWidth) O(RP_TX_BASELINE, RastPort, TxBaseline) O(RP_TX_SPACING, RastPort, TxSpacing)
S(BM_SIZE, BitMap) O(BM_BYTES_PER_ROW, BitMap, BytesPerRow) O(BM_ROWS, BitMap, Rows) O(BM_FLAGS, BitMap, Flags) O(BM_DEPTH, BitMap, Depth) O(BM_PLANES, BitMap, Planes)
S(TF_SIZE, TextFont) O(TF_NODE_TYPE, TextFont, tf_Message.mn_Node.ln_Type) O(TF_NODE_NAME, TextFont, tf_Message.mn_Node.ln_Name) O(TF_Y_SIZE, TextFont, tf_YSize) O(TF_STYLE, TextFont, tf_Style) O(TF_FLAGS, TextFont, tf_Flags)
O(TF_X_SIZE, TextFont, tf_XSize) O(TF_BASELINE, TextFont, tf_Baseline) O(TF_BOLD_SMEAR, TextFont, tf_BoldSmear) O(TF_ACCESSORS, TextFont, tf_Accessors) O(TF_LO_CHAR, TextFont, tf_LoChar)
O(TF_HI_CHAR, TextFont, tf_HiChar) O(TF_CHAR_DATA, TextFont, tf_CharData) O(TF_MODULO, TextFont, tf_Modulo) O(TF_CHAR_LOC, TextFont, tf_CharLoc) O(TF_CHAR_SPACE, TextFont, tf_CharSpace) O(TF_CHAR_KERN, TextFont, tf_CharKern)
S(TA_SIZE, TextAttr) O(TA_NAME, TextAttr, ta_Name) O(TA_Y_SIZE, TextAttr, ta_YSize) O(TA_STYLE, TextAttr, ta_Style) O(TA_FLAGS, TextAttr, ta_Flags) S(TE_SIZE, TextExtent)
S(RG_SIZE, Region) O(RG_BOUNDS, Region, bounds) O(RG_FIRST, Region, RegionRectangle) S(RR_SIZE, RegionRectangle) O(RR_NEXT, RegionRectangle, Next) O(RR_PREV, RegionRectangle, Prev) O(RR_BOUNDS, RegionRectangle, bounds)
O(LY_RP, Layer, rp) O(LY_BOUNDS, Layer, bounds) O(LY_FLAGS, Layer, Flags) O(LY_WINDOW, Layer, Window) O(LY_SCROLL_X, Layer, Scroll_X) O(LY_SCROLL_Y, Layer, Scroll_Y) O(LY_CLIP_REGION, Layer, ClipRegion) S(LY_SIZE, Layer)
O(WD_RPORT, Window, RPort) O(WD_BORDER_RPORT, Window, BorderRPort) O(WD_WLAYER, Window, WLayer) O(WD_IFONT, Window, IFont) O(WD_GZZ_WIDTH, Window, GZZWidth) O(WD_GZZ_HEIGHT, Window, GZZHeight)
O(SC_VIEWPORT, Screen, ViewPort) O(SC_RASTPORT, Screen, RastPort) O(SC_BITMAP, Screen, BitMap) O(SC_LAYER_INFO, Screen, LayerInfo)
O(VP_COLORMAP, ViewPort, ColorMap) O(VP_DWIDTH, ViewPort, DWidth) O(VP_DHEIGHT, ViewPort, DHeight) O(VP_MODES, ViewPort, Modes) S(CM_SIZE, ColorMap) O(CM_COUNT, ColorMap, Count) O(CM_COLOR_TABLE, ColorMap, ColorTable)
S(DRI_SIZE, DrawInfo) O(DRI_VERSION, DrawInfo, dri_Version) O(DRI_NUM_PENS, DrawInfo, dri_NumPens) O(DRI_PENS, DrawInfo, dri_Pens) O(DRI_FONT, DrawInfo, dri_Font) O(DRI_DEPTH, DrawInfo, dri_Depth) O(DRI_RESOLUTION, DrawInfo, dri_Resolution)
O(DRI_FLAGS, DrawInfo, dri_Flags) O(DRI_CHECK_MARK, DrawInfo, dri_CheckMark) O(DRI_AMIGA_KEY, DrawInfo, dri_AmigaKey)
O(IM_LEFT, Image, LeftEdge) O(IM_DEPTH, Image, Depth) O(IM_DATA, Image, ImageData) O(IM_PLANE_PICK, Image, PlanePick) O(IM_PLANE_ON_OFF, Image, PlaneOnOff) O(IM_NEXT, Image, NextImage) S(IM_SIZE, Image)
O(BD_LEFT, Border, LeftEdge) O(BD_FRONT_PEN, Border, FrontPen) O(BD_BACK_PEN, Border, BackPen) O(BD_DRAW_MODE, Border, DrawMode) O(BD_COUNT, Border, Count) O(BD_XY, Border, XY) O(BD_NEXT, Border, NextBorder) S(BD_SIZE, Border)
O(IT_FRONT_PEN, IntuiText, FrontPen) O(IT_BACK_PEN, IntuiText, BackPen) O(IT_DRAW_MODE, IntuiText, DrawMode) O(IT_LEFT, IntuiText, LeftEdge) O(IT_TOP, IntuiText, TopEdge) O(IT_FONT, IntuiText, ITextFont) O(IT_TEXT, IntuiText, IText) O(IT_NEXT, IntuiText, NextText) S(IT_SIZE, IntuiText)
