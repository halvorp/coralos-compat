| CODE 1 of the QuickDraw oracle application: draws a fixed set of primitives, EACH IN ITS OWN 40x40 CELL, into its window
| on the first update event, so a pixel difference against an independent implementation (systemless, run as an external
| oracle) localises to one primitive. Window content is 320x280 = 8 columns x 7 rows of cells:
|   row 0  a pen-mode line pair per cell, patCopy..notPatBic (modes 8..15) over a 50% gray cell
|   row 1  FrameRect(pen 1), PaintRect, InvertRect over gray, EraseRect inside black, FillRect ltGray, FillRect dkGray,
|          FrameRect with a 3x3 pen, PaintRect with PenPat gray
|   row 2  FrameOval, PaintOval, InvertOval over gray, EraseOval inside black, FrameRoundRect, PaintRoundRect, a capsule
|          (FrameRoundRect with oval = rect), FillOval gray
|   row 3  regions: A=rect, B=oval, PaintRgn(A), PaintRgn(B), Union, Diff(A,B), Sect, Xor, FrameRgn(Union, 2x2 pen),
|          InvertRgn(Union) over gray   (Cells: A, B, A+B, A-B, A*B, A^B, frame, invert)
|   row 4  CopyBits of a 32x32 offscreen "F" bitmap: srcCopy, srcOr over gray, srcXor over gray, srcBic over black,
|          notSrcCopy, srcCopy through an oval mask region, srcCopy shrunk 32x32 -> 24x36, srcCopy grown 16x16 -> 36x36
|   row 6  line geometry on white (black pen): slopes, pen sizes, directions
|   row 5  four DrawString slots 80 px wide in the system font (12 pt, 12 pt, 12 pt, 9 pt); after each string a 4 px tick
|          is drawn below the baseline at the pen's x, so the pen advance is visible in the pixels
| Window content origin on screen: (left 61, top 43): not a multiple of 8, so pattern and bit-shift alignment is exercised.
| A5 world: QuickDraw globals fill A5-206..A5; the application's own variables are below them.
|   m68k-linux-gnu-as -m68020 oracle.s -o oracle.o && m68k-linux-gnu-objcopy -O binary -j .text oracle.o oracle.bin
        .include "traps.inc"
        .equ WINDOW, -210
        .equ BM,     -240              | BitMap: baseAddr 0, rowBytes 4, bounds 6
        .equ RGNA,   -244
        .equ RGNB,   -248
        .equ RGNC,   -252
        .equ RGND,   -256
        .equ EVENT,  -280              | EventRecord
        .equ OFFPORT, -400             | the offscreen GrafPort (108 bytes)
        .equ BLACK,  -20               | qd.black, qd.gray, qd.ltGray, qd.dkGray as offsets from A5
        .equ GRAY,   -28
        .equ LTGRAY, -36
        .equ DKGRAY, -44
        .equ _NewPtrClr, 0xA31E

        .macro RECT t,l,b,r
        move.w  #\r,-(%sp)
        move.w  #\b,-(%sp)
        move.w  #\l,-(%sp)
        move.w  #\t,-(%sp)
        .endm
        .macro RCALL trap,t,l,b,r          | trap(@rect)
        RECT    \t,\l,\b,\r
        pea     (%sp)
        .short  \trap
        lea     8(%sp),%sp
        .endm
        .macro RCALLW trap,t,l,b,r,w1,w2   | trap(@rect, w1, w2)
        RECT    \t,\l,\b,\r
        pea     (%sp)
        move.w  #\w1,-(%sp)
        move.w  #\w2,-(%sp)
        .short  \trap
        lea     8(%sp),%sp
        .endm
        .macro RCALLP trap,t,l,b,r,pat     | trap(@rect, @pattern)
        RECT    \t,\l,\b,\r
        pea     (%sp)
        pea     \pat(%a5)
        .short  \trap
        lea     8(%sp),%sp
        .endm
        .macro PENMODE m
        move.w  #\m,-(%sp)
        .short  _PenMode
        .endm
        .macro PENSIZE w,h
        move.w  #\w,-(%sp)
        move.w  #\h,-(%sp)
        .short  _PenSize
        .endm
        .macro PENPAT pat
        pea     \pat(%a5)
        .short  _PenPat
        .endm
        .macro MOVETO h,v
        move.w  #\h,-(%sp)
        move.w  #\v,-(%sp)
        .short  _MoveTo
        .endm
        .macro LINETO h,v
        move.w  #\h,-(%sp)
        move.w  #\v,-(%sp)
        .short  _LineTo
        .endm
        .macro CELL c,r
        .set    CX,(\c)*40
        .set    CY,(\r)*40
        .endm
        .macro CELLGRAY                    | the whole cell gray (a background that makes the transfer mode visible)
        RCALLP  _FillRect,CY,CX,CY+40,CX+40,GRAY
        .endm
        .macro CELLBLACK
        RCALL   _PaintRect,CY,CX,CY+40,CX+40
        .endm
        .macro RGN3 trap,a,b,c             | trap(a, b, c) on region handles
        move.l  \a(%a5),-(%sp)
        move.l  \b(%a5),-(%sp)
        move.l  \c(%a5),-(%sp)
        .short  \trap
        .endm
        .macro RGN1 trap,a                 | trap(a)
        move.l  \a(%a5),-(%sp)
        .short  \trap
        .endm
        .macro SHIFTC                      | RGNC offset into the current cell
        move.l  RGNC(%a5),-(%sp)
        move.w  #CX,-(%sp)
        move.w  #CY,-(%sp)
        .short  _OffsetRgn
        .endm
        .macro NEWRGN var
        clr.l   -(%sp)
        .short  _NewRgn
        move.l  (%sp)+,\var(%a5)
        .endm

        .text
main:   pea     -4(%a5)                | &qd.thePort
        .short  _InitGraf
        .short  _InitFonts
        .short  _InitWindows
        .short  _InitMenus
        .short  _TEInit
        clr.l   -(%sp)
        .short  _InitDialogs
        .short  _InitCursor
        | ---- the offscreen 32x32 1-bit "F" bitmap
        move.l  #128,%d0
        .short  _NewPtrClr
        move.l  %a0,BM(%a5)
        move.w  #4,BM+4(%a5)
        clr.w   BM+6(%a5)
        clr.w   BM+8(%a5)
        move.w  #32,BM+10(%a5)
        move.w  #32,BM+12(%a5)
        pea     OFFPORT(%a5)
        .short  _OpenPort
        pea     BM(%a5)
        .short  _SetPBits
        RCALL   _EraseRect,0,0,32,32
        RCALL   _PaintRect,2,2,6,28        | the F: top bar, stem, middle bar
        RCALL   _PaintRect,2,2,30,6
        RCALL   _PaintRect,13,2,17,20
        RCALL   _FrameOval,18,16,31,31     | an oval in the lower right, and a diagonal
        MOVETO  20,20
        LINETO  30,30
        | ---- the window (WIND 128, content 320x240 at global top 43 left 61)
        clr.l   -(%sp)
        move.w  #128,-(%sp)
        clr.l   -(%sp)
        move.l  #-1,-(%sp)
        .short  _GetNewWindow
        move.l  (%sp)+,WINDOW(%a5)
        move.l  WINDOW(%a5),-(%sp)
        .short  _SetPort
        | ---- the two regions A (a rect) and B (an oval), both built with OpenRgn/CloseRgn, in cell-local coordinates
        NEWRGN  RGNA
        .short  _OpenRgn
        RCALL   _FrameRect,4,4,28,24
        RGN1    _CloseRgn,RGNA
        NEWRGN  RGNB
        .short  _OpenRgn
        RCALL   _FrameOval,14,12,38,38
        RGN1    _CloseRgn,RGNB
        NEWRGN  RGNC
loop:   clr.w   -(%sp)                 | GetNextEvent(everyEvent, @ev)
        move.w  #0xFFFF,-(%sp)
        pea     EVENT(%a5)
        .short  _GetNextEvent
        move.w  (%sp)+,%d0
        move.w  EVENT(%a5),%d1
        cmp.w   #6,%d1                 | updateEvt
        beq.s   update
        cmp.w   #3,%d1                 | keyDown
        beq.s   quit
        bra.s   loop
quit:   .short  _ExitToShell

update: move.l  EVENT+2(%a5),%a3
        move.l  %a3,-(%sp)
        .short  _SetPort
        move.l  %a3,-(%sp)
        .short  _BeginUpDate
        lea     16(%a3),%a0
        move.l  %a0,-(%sp)
        .short  _EraseRect
        | ================= row 0: pen modes
        .set    MODE,8
        .rept   8
        CELL    MODE-8,0
        .short  _PenNormal
        CELLGRAY
        PENSIZE 3,2
        PENMODE MODE
        PENPAT  DKGRAY
        MOVETO  CX+4,CY+4
        LINETO  CX+36,CY+30
        PENSIZE 1,1
        PENPAT  BLACK
        MOVETO  CX+8,CY+36
        LINETO  CX+12,CY+4
        .set    MODE,MODE+1
        .endr
        .short  _PenNormal
        | ================= row 1: rectangles
        CELL    0,1
        RCALL   _FrameRect,CY+4,CX+4,CY+30,CX+35
        CELL    1,1
        RCALL   _PaintRect,CY+4,CX+4,CY+30,CX+35
        CELL    2,1
        CELLGRAY
        RCALL   _InverRect,CY+5,CX+5,CY+31,CX+36
        CELL    3,1
        CELLBLACK
        RCALL   _EraseRect,CY+5,CX+6,CY+30,CX+33
        CELL    4,1
        RCALLP  _FillRect,CY+3,CX+3,CY+37,CX+37,LTGRAY
        CELL    5,1
        RCALLP  _FillRect,CY+3,CX+3,CY+37,CX+37,DKGRAY
        CELL    6,1
        PENSIZE 3,3
        RCALL   _FrameRect,CY+4,CX+4,CY+30,CX+35
        .short  _PenNormal
        CELL    7,1
        PENPAT  GRAY
        RCALL   _PaintRect,CY+4,CX+4,CY+31,CX+35
        .short  _PenNormal
        | ================= row 2: ovals and round rects
        CELL    0,2
        RCALL   _FrameOval,CY+4,CX+4,CY+30,CX+35
        CELL    1,2
        RCALL   _PaintOval,CY+4,CX+4,CY+31,CX+35
        CELL    2,2
        CELLGRAY
        RCALL   _InvertOval,CY+5,CX+5,CY+36,CX+30
        CELL    3,2
        CELLBLACK
        RCALL   _EraseOval,CY+3,CX+4,CY+36,CX+37
        CELL    4,2
        RCALLW  _FrameRoundRect,CY+4,CX+4,CY+34,CX+35,16,16
        CELL    5,2
        RCALLW  _PaintRoundRect,CY+4,CX+4,CY+30,CX+35,10,20
        CELL    6,2
        RCALLW  _FrameRoundRect,CY+8,CX+3,CY+30,CX+36,40,40
        CELL    7,2
        RCALLP  _FillOval,CY+4,CX+3,CY+37,CX+36,GRAY
        | ================= row 3: regions (OpenRgn/CloseRgn built, combined, painted)
        CELL    0,3
        RGN3    _UnionRgn,RGNA,RGNA,RGNC       | C = A (copy by union with itself)
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    1,3
        RGN3    _UnionRgn,RGNB,RGNB,RGNC
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    2,3
        RGN3    _UnionRgn,RGNA,RGNB,RGNC
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    3,3
        RGN3    _DiffRgn,RGNA,RGNB,RGNC
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    4,3
        RGN3    _SectRgn,RGNA,RGNB,RGNC
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    5,3
        RGN3    _XOrRgn,RGNA,RGNB,RGNC
        SHIFTC
        RGN1    _PaintRgn,RGNC
        CELL    6,3
        RGN3    _UnionRgn,RGNA,RGNB,RGNC
        SHIFTC
        PENSIZE 2,2
        RGN1    _FrameRgn,RGNC
        .short  _PenNormal
        CELL    7,3
        CELLGRAY
        RGN3    _UnionRgn,RGNA,RGNB,RGNC
        SHIFTC
        RGN1    _InverRgn,RGNC
        | ================= row 4: CopyBits(srcBits, dstBits = &window->portBits, srcRect, dstRect, mode, maskRgn)
        .macro CB sl,st,sr,sb,dt,dl,db,dr,mode,mask
        RECT    \dt,\dl,\db,\dr                  | destination rect (higher address)
        RECT    \st,\sl,\sb,\sr                  | source rect, at (sp)
        move.l  %sp,%a0
        lea     8(%sp),%a1
        pea     BM(%a5)
        pea     2(%a3)                           | &window->portBits
        move.l  %a0,-(%sp)
        move.l  %a1,-(%sp)
        move.w  #\mode,-(%sp)
        move.l  \mask,-(%sp)
        .short  _CopyBits
        lea     16(%sp),%sp
        .endm
        CELL    0,4
        CB      0,0,32,32,CY+4,CX+4,CY+36,CX+36,0,#0
        CELL    1,4
        CELLGRAY
        CB      0,0,32,32,CY+4,CX+5,CY+36,CX+37,1,#0
        CELL    2,4
        CELLGRAY
        CB      0,0,32,32,CY+4,CX+3,CY+36,CX+35,2,#0
        CELL    3,4
        CELLBLACK
        CB      0,0,32,32,CY+4,CX+4,CY+36,CX+36,3,#0
        CELL    4,4
        CB      0,0,32,32,CY+4,CX+7,CY+36,CX+39,4,#0
        CELL    5,4
        NEWRGN  RGND                             | an oval mask region in window coordinates
        .short  _OpenRgn
        RCALL   _FrameOval,CY+6,CX+6,CY+34,CX+34
        RGN1    _CloseRgn,RGND
        CB      0,0,32,32,CY+4,CX+4,CY+36,CX+36,0,RGND(%a5)
        CELL    6,4
        CB      0,0,32,32,CY+2,CX+2,CY+38,CX+26,0,#0
        CELL    7,4
        CB      0,0,16,16,CY+2,CX+2,CY+38,CX+38,0,#0
        | ================= row 6: line geometry on white (black pen, patCopy): slopes, pen sizes, directions
        CELL    0,6
        MOVETO  CX+4,CY+10
        LINETO  CX+36,CY+10
        MOVETO  CX+4,CY+14
        LINETO  CX+4,CY+36
        MOVETO  CX+8,CY+14
        LINETO  CX+36,CY+36
        CELL    1,6
        MOVETO  CX+3,CY+4
        LINETO  CX+37,CY+12
        MOVETO  CX+3,CY+16
        LINETO  CX+37,CY+19
        MOVETO  CX+3,CY+28
        LINETO  CX+37,CY+23
        CELL    2,6
        MOVETO  CX+6,CY+3
        LINETO  CX+10,CY+37
        MOVETO  CX+20,CY+3
        LINETO  CX+17,CY+37
        MOVETO  CX+30,CY+37
        LINETO  CX+34,CY+3
        CELL    3,6
        PENSIZE 3,2
        MOVETO  CX+4,CY+4
        LINETO  CX+35,CY+16
        PENSIZE 2,3
        MOVETO  CX+4,CY+22
        LINETO  CX+34,CY+30
        .short  _PenNormal
        CELL    4,6
        MOVETO  CX+36,CY+4
        LINETO  CX+4,CY+10
        MOVETO  CX+36,CY+36
        LINETO  CX+6,CY+14
        MOVETO  CX+20,CY+36
        LINETO  CX+20,CY+16
        CELL    5,6
        PENSIZE 5,5
        MOVETO  CX+4,CY+4
        LINETO  CX+30,CY+30
        PENSIZE 1,4
        MOVETO  CX+34,CY+4
        LINETO  CX+34,CY+34
        .short  _PenNormal
        CELL    6,6
        PENSIZE 4,1
        MOVETO  CX+3,CY+5
        LINETO  CX+36,CY+5
        PENSIZE 8,8
        MOVETO  CX+5,CY+20
        LINETO  CX+5,CY+20
        .short  _PenNormal
        MOVETO  CX+20,CY+20
        LINETO  CX+20,CY+20
        MOVETO  CX+30,CY+20
        .short  _PenNormal
        CELL    7,6
        MOVETO  CX+4,CY+4
        LINETO  CX+36,CY+36
        MOVETO  CX+36,CY+4
        LINETO  CX+4,CY+36
        MOVETO  CX+4,CY+20
        LINETO  CX+36,CY+20
        | ================= row 5: DrawString in the system font, with a tick under the pen's end point
        .macro TEXT x,y,s,size,mode
        clr.w   -(%sp)
        .short  _TextFont
        move.w  #\size,-(%sp)
        .short  _TextSize
        move.w  #\mode,-(%sp)
        .short  _TextMode
        MOVETO  \x,\y
        pea     \s(%pc)
        .short  _DrawString
        subq.l  #4,%sp                           | GetPen(VAR pt): v then h
        pea     (%sp)
        .short  _GetPen
        move.w  (%sp)+,%d1                       | v
        move.w  (%sp)+,%d2                       | h
        add.w   #6,%d1
        move.w  %d2,-(%sp)
        move.w  %d1,-(%sp)
        .short  _MoveTo
        add.w   #4,%d1
        move.w  %d2,-(%sp)
        move.w  %d1,-(%sp)
        .short  _LineTo
        .endm
        TEXT    4,218,s0,12,1
        TEXT    84,218,s1,12,1
        TEXT    164,218,s2,12,1
        TEXT    244,218,s3,9,1
        move.l  %a3,-(%sp)
        .short  _EndUpDate
        bra     loop

s0:     .byte   5
        .ascii  "Hello"
s1:     .byte   7
        .ascii  "Mqgy,jp"
s2:     .byte   9
        .ascii  "QuickDraw"
s3:     .byte   6
        .ascii  "i.l 1|"
        .align  2
