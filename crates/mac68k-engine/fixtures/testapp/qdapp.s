| A tiny classic Mac application (flat program) that draws its window on every updateEvt with QuickDraw:
| the shape of a real event loop. InitGraf, then GetNextEvent(everyEvent); updateEvt -> SetPort, BeginUpdate, draw, EndUpdate;
| keyDown -> ExitToShell. The window (200x120 at global top 100, left 50) is made by the test. 0x3000 counts updates.
|   m68k-linux-gnu-as -m68020 qdapp.s -o qdapp.o && m68k-linux-gnu-objcopy -O binary -j .text qdapp.o qdapp.bin
        .include "traps.inc"
        .text
start:  pea     0x3F00                 | InitGraf(@qd.thePort)
        .short  _InitGraf
loop:   clr.w   -(%sp)                 | GetNextEvent(everyEvent, @ev)
        move.w  #0xFFFF,-(%sp)
        pea     ev(%pc)
        .short  _GetNextEvent
        move.w  (%sp)+,%d0
        lea     ev(%pc),%a2
        move.w  (%a2),%d1              | what
        cmp.w   #6,%d1                 | updateEvt
        beq.s   update
        cmp.w   #3,%d1                 | keyDown
        beq.s   quit
        bra.s   loop
quit:   .short  _ExitToShell

update: move.l  2(%a2),%a3             | message = the WindowPtr
        move.l  %a3,-(%sp)
        .short  _SetPort
        move.l  %a3,-(%sp)
        .short  _BeginUpDate
        lea     16(%a3),%a0            | EraseRect(@window->portRect)
        move.l  %a0,-(%sp)
        .short  _EraseRect
        move.w  #2,-(%sp)              | PenSize(2,2); FrameRect(r1); PenNormal
        move.w  #2,-(%sp)
        .short  _PenSize
        pea     r1(%pc)
        .short  _FrameRect
        .short  _PenNormal
        pea     r2(%pc)                | PaintRect(r2)
        .short  _PaintRect
        pea     r3(%pc)                | FillRect(r3, qd.gray)
        pea     0x3EE8
        .short  _FillRect
        pea     r4(%pc)                | PaintOval(r4)
        .short  _PaintOval
        pea     r5(%pc)                | FrameRoundRect(r5, 10, 10)
        move.w  #10,-(%sp)
        move.w  #10,-(%sp)
        .short  _FrameRoundRect
        move.w  #140,-(%sp)            | MoveTo(140,10); LineTo(190,40)
        move.w  #10,-(%sp)
        .short  _MoveTo
        move.w  #190,-(%sp)
        move.w  #40,-(%sp)
        .short  _LineTo
        pea     r6(%pc)                | InvertRect(r6): inside the frame's white interior
        .short  _InverRect
        clr.w   -(%sp)                 | TextFont(systemFont); TextSize(12)
        .short  _TextFont
        move.w  #12,-(%sp)
        .short  _TextSize
        move.w  #110,-(%sp)            | MoveTo(110,30); DrawString("QD")
        move.w  #30,-(%sp)
        .short  _MoveTo
        pea     str(%pc)
        .short  _DrawString
        move.l  %a3,-(%sp)
        .short  _EndUpDate
        addq.w  #1,0x3000
        bra     loop

r1:     .short  10,10,50,90            | top, left, bottom, right
r2:     .short  60,10,80,50
r3:     .short  60,60,80,100
r4:     .short  90,10,118,60
r5:     .short  90,70,118,130
r6:     .short  20,20,30,30
str:    .byte   2
        .ascii  "QD"
        .align  2
ev:     .space  16
