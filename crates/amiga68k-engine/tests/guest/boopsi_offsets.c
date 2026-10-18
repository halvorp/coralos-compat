/* Struct layouts the BOOPSI engine hands to programs, transcribed from the AROS 'compiler/include' field lists
 * (intuition/classes.h, utility/hooks.h, intuition/intuition.h [Gadget, ExtGadget, Image], intuition/cghooks.h,
 * intuition/gadgetclass.h, intuition/classusr.h) as plain C so m68k gcc computes the m68k ABI offsets. Interface facts
 * only, no AROS code. ICData is AROS's private intuition_intern.h struct (the IC part of a gadgetclass instance).
 *   tests/guest/boopsi_offsets.sh writes boopsi_offsets.txt, which tests/boopsi_layout.rs compares to src/boopsi_ids.rs. */
#include <stddef.h>
typedef short WORD; typedef unsigned short UWORD; typedef signed char BYTE; typedef unsigned char UBYTE;
typedef int LONG; typedef unsigned int ULONG; typedef void *APTR;
struct MinNode { struct MinNode *mln_Succ, *mln_Pred; };
struct Hook { struct MinNode h_MinNode; APTR h_Entry, h_SubEntry, h_Data; };
struct IClass { struct Hook cl_Dispatcher; ULONG cl_Reserved; struct IClass *cl_Super; char *cl_ID; UWORD cl_InstOffset, cl_InstSize;
  APTR cl_UserData; ULONG cl_SubclassCount, cl_ObjectCount, cl_Flags, cl_ObjectSize; APTR cl_MemoryPool; };
struct _Object { struct MinNode o_Node; struct IClass *o_Class; };
struct Gadget { struct Gadget *NextGadget; WORD LeftEdge, TopEdge, Width, Height; UWORD Flags, Activation, GadgetType; APTR GadgetRender, SelectRender;
  void *GadgetText; LONG MutualExclude; APTR SpecialInfo; UWORD GadgetID; APTR UserData; };
struct ExtGadget { struct ExtGadget *NextGadget; WORD LeftEdge, TopEdge, Width, Height; UWORD Flags, Activation, GadgetType; APTR GadgetRender, SelectRender;
  void *GadgetText; LONG MutualExclude; APTR SpecialInfo; UWORD GadgetID; APTR UserData; ULONG MoreFlags; WORD BoundsLeftEdge, BoundsTopEdge, BoundsWidth, BoundsHeight; };
struct ICData { void *ic_Target; void *ic_Mapping; void *ic_CloneTags; ULONG ic_LoopCounter; };
struct GadgetData { struct ExtGadget eg; struct ICData IC; };
struct Image { WORD LeftEdge, TopEdge, Width, Height, Depth; UWORD *ImageData; UBYTE PlanePick, PlaneOnOff; struct Image *NextImage; };
struct IBox { WORD Left, Top, Width, Height; };
struct GadgetInfo { void *gi_Screen, *gi_Window, *gi_Requester, *gi_RastPort, *gi_Layer; struct IBox gi_Domain; struct { UBYTE DetailPen, BlockPen; } gi_Pens;
  void *gi_DrInfo; ULONG gi_Reserved[6]; };
struct opSet { ULONG MethodID; void *ops_AttrList; void *ops_GInfo; };
struct opGet { ULONG MethodID; ULONG opg_AttrID; ULONG *opg_Storage; };
struct opUpdate { ULONG MethodID; void *opu_AttrList; void *opu_GInfo; ULONG opu_Flags; };
struct gpHitTest { ULONG MethodID; struct GadgetInfo *gpht_GInfo; struct { WORD X, Y; } gpht_Mouse; };
struct gpRender { ULONG MethodID; struct GadgetInfo *gpr_GInfo; void *gpr_RPort; LONG gpr_Redraw; };
struct gpInput { ULONG MethodID; struct GadgetInfo *gpi_GInfo; void *gpi_IEvent; LONG *gpi_Termination; struct { WORD X, Y; } gpi_Mouse; void *gpi_TabletData; };
struct gpGoInactive { ULONG MethodID; struct GadgetInfo *gpgi_GInfo; ULONG gpgi_Abort; };
struct gpLayout { ULONG MethodID; struct GadgetInfo *gpl_GInfo; ULONG gpl_Initial; };
struct InputEvent { struct InputEvent *ie_NextEvent; UBYTE ie_Class, ie_SubClass; UWORD ie_Code, ie_Qualifier; union { struct { WORD ie_x, ie_y; } ie_xy; APTR ie_addr; } ie_position;
  struct { ULONG tv_secs, tv_micro; } ie_TimeStamp; };
#define O(n, s, f) const int n = offsetof(struct s, f);
#define S(n, s) const int n = sizeof(struct s);
S(CL_SIZE, IClass) O(CL_H_ENTRY, IClass, cl_Dispatcher.h_Entry) O(CL_H_SUBENTRY, IClass, cl_Dispatcher.h_SubEntry) O(CL_H_DATA, IClass, cl_Dispatcher.h_Data)
O(CL_RESERVED, IClass, cl_Reserved) O(CL_SUPER, IClass, cl_Super) O(CL_ID, IClass, cl_ID) O(CL_INST_OFFSET, IClass, cl_InstOffset) O(CL_INST_SIZE, IClass, cl_InstSize)
O(CL_USER_DATA, IClass, cl_UserData) O(CL_SUBCLASS_COUNT, IClass, cl_SubclassCount) O(CL_OBJECT_COUNT, IClass, cl_ObjectCount) O(CL_FLAGS, IClass, cl_Flags)
O(CL_OBJECT_SIZE, IClass, cl_ObjectSize) O(CL_MEMORY_POOL, IClass, cl_MemoryPool)
S(OBJ_HEADER, _Object) O(OBJ_CLASS, _Object, o_Class)
S(GAD_SIZE, Gadget) O(GAD_NEXT, Gadget, NextGadget) O(GAD_LEFT, Gadget, LeftEdge) O(GAD_TOP, Gadget, TopEdge) O(GAD_WIDTH, Gadget, Width) O(GAD_HEIGHT, Gadget, Height)
O(GAD_FLAGS, Gadget, Flags) O(GAD_ACTIVATION, Gadget, Activation) O(GAD_TYPE, Gadget, GadgetType) O(GAD_RENDER, Gadget, GadgetRender) O(GAD_SELECT, Gadget, SelectRender)
O(GAD_TEXT, Gadget, GadgetText) O(GAD_MUTUAL, Gadget, MutualExclude) O(GAD_SPECIAL, Gadget, SpecialInfo) O(GAD_ID, Gadget, GadgetID) O(GAD_USER, Gadget, UserData)
S(EGAD_SIZE, ExtGadget) O(EGAD_MORE_FLAGS, ExtGadget, MoreFlags) O(EGAD_BOUNDS, ExtGadget, BoundsLeftEdge)
S(GDATA_SIZE, GadgetData) O(GDATA_IC, GadgetData, IC) O(IC_TARGET, ICData, ic_Target) O(IC_MAPPING, ICData, ic_Mapping)
S(IMG_SIZE, Image) O(IMG_LEFT, Image, LeftEdge) O(IMG_TOP, Image, TopEdge) O(IMG_WIDTH, Image, Width) O(IMG_HEIGHT, Image, Height) O(IMG_DEPTH, Image, Depth)
O(IMG_DATA, Image, ImageData) O(IMG_PLANE_PICK, Image, PlanePick) O(IMG_PLANE_ONOFF, Image, PlaneOnOff) O(IMG_NEXT, Image, NextImage)
S(GI_SIZE, GadgetInfo) O(GI_SCREEN, GadgetInfo, gi_Screen) O(GI_WINDOW, GadgetInfo, gi_Window) O(GI_REQUESTER, GadgetInfo, gi_Requester) O(GI_RASTPORT, GadgetInfo, gi_RastPort)
O(GI_LAYER, GadgetInfo, gi_Layer) O(GI_DOMAIN, GadgetInfo, gi_Domain) O(GI_PENS, GadgetInfo, gi_Pens) O(GI_DRINFO, GadgetInfo, gi_DrInfo)
O(OPSET_GINFO, opSet, ops_GInfo) O(OPGET_STORAGE, opGet, opg_Storage) O(OPUPDATE_FLAGS, opUpdate, opu_Flags)
S(GPR_SIZE, gpRender) O(GPR_RPORT, gpRender, gpr_RPort) O(GPR_REDRAW, gpRender, gpr_Redraw)
S(GPI_SIZE, gpInput) O(GPI_IEVENT, gpInput, gpi_IEvent) O(GPI_TERMINATION, gpInput, gpi_Termination) O(GPI_MOUSE, gpInput, gpi_Mouse) O(GPI_TABLET, gpInput, gpi_TabletData)
S(GPHT_SIZE, gpHitTest) O(GPHT_MOUSE, gpHitTest, gpht_Mouse)
S(GPGI_SIZE, gpGoInactive) O(GPGI_ABORT, gpGoInactive, gpgi_Abort) S(GPL_SIZE, gpLayout) O(GPL_INITIAL, gpLayout, gpl_Initial)
S(IE_SIZE, InputEvent) O(IE_CLASS, InputEvent, ie_Class) O(IE_CODE, InputEvent, ie_Code) O(IE_QUALIFIER, InputEvent, ie_Qualifier) O(IE_X, InputEvent, ie_position.ie_xy.ie_x)
O(IE_TIME, InputEvent, ie_TimeStamp)
