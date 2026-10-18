| QuickDraw rect/point utilities from assembly (flat program): the Pascal frames of SetRect, InsetRect, OffsetRect,
| SectRect, PtInRect, EmptyRect, AddPt. Results at 0x3000.. .
|   m68k-linux-gnu-as -m68020 recttest.s -o recttest.o && m68k-linux-gnu-objcopy -O binary -j .text recttest.o recttest.bin
        .include "traps.inc"
        .equ _SetRect,    0xA8A7
        .equ _OffsetRect, 0xA8A8
        .equ _InsetRect,  0xA8A9
        .equ _SectRect,   0xA8AA
        .equ _PtInRect,   0xA8AD
        .equ _EmptyRect,  0xA8AE
        .equ _AddPt,      0xA87E
        .text
start:  pea     0x3000                 | r = (left 20, top 10, right 220, bottom 110) -> memory top,left,bottom,right
        move.w  #20,-(%sp)
        move.w  #10,-(%sp)
        move.w  #220,-(%sp)
        move.w  #110,-(%sp)
        .short  _SetRect
        pea     0x3000
        move.w  #5,-(%sp)              | dh
        move.w  #7,-(%sp)              | dv
        .short  _InsetRect
        pea     0x3000
        move.w  #1,-(%sp)
        move.w  #2,-(%sp)
        .short  _OffsetRect
        pea     0x3010                 | s = (50,100,500,500)
        move.w  #100,-(%sp)
        move.w  #50,-(%sp)
        move.w  #500,-(%sp)
        move.w  #500,-(%sp)
        .short  _SetRect
        clr.w   -(%sp)
        pea     0x3000
        pea     0x3010
        pea     0x3020
        .short  _SectRect
        move.w  (%sp)+,0x3030          | TRUE
        clr.w   -(%sp)
        move.l  #(60<<16)+130,-(%sp)   | point (v 60, h 130)
        pea     0x3000
        .short  _PtInRect
        move.w  (%sp)+,0x3032          | inside r
        clr.w   -(%sp)
        move.l  #(500<<16)+130,-(%sp)  | point (v 500, h 130)
        pea     0x3000
        .short  _PtInRect
        move.w  (%sp)+,0x3034          | outside
        clr.w   -(%sp)
        pea     0x3040                 | an all-zero rect
        .short  _EmptyRect
        move.w  (%sp)+,0x3036
        move.l  #(3<<16)+4,0x3050      | pt (v 3, h 4)
        move.l  #(10<<16)+20,-(%sp)    | src
        pea     0x3050
        .short  _AddPt
        .short  _ExitToShell
