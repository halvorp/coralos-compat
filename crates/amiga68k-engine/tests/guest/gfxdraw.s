# A guest program that opens a window and draws into its RastPort: a filled rectangle, then "AB" at (40,30) in
# pen 1 on pen 3. Written for this repo (MIT). Offset 4: the draw mode for the text (patched by the tests);
# offset 8: the window's flags; 12..: results (RPort, Window).
        .text
        bra.w   start
mode:   .long   1
wflags: .long   0x800                   | WFLG_BORDERLESS: the RPort origin is the window's top-left
res:    .space  16
nw:     .short  20,10,200,100           | Left, Top, Width, Height
        .byte   0,1
        .long   0                       | IDCMPFlags
nwfl:   .long   0                       | Flags (patched from wflags)
        .long   0,0,0,0,0
        .short  80,40,640,400
        .short  1
iname:  .asciz  "intuition.library"
gname:  .asciz  "graphics.library"
txt:    .asciz  "AB"
        .align  2
start:  move.l  4.w,%a6
        lea     iname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)               | OpenLibrary intuition
        move.l  %d0,%d7
        lea     gname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)               | OpenLibrary graphics
        move.l  %d0,%d6
        lea     nwfl(%pc),%a0
        move.l  wflags(%pc),(%a0)
        move.l  %d7,%a6
        lea     nw(%pc),%a0
        jsr     -204(%a6)               | OpenWindow
        move.l  %d0,%a5
        lea     res(%pc),%a3
        move.l  %a5,4(%a3)
        move.l  50(%a5),%a4             | Window.RPort
        move.l  %a4,(%a3)
        move.l  %d6,%a6
        move.l  %a4,%a1
        moveq   #2,%d0
        jsr     -342(%a6)               | SetAPen(2)
        move.l  %a4,%a1
        moveq   #10,%d0
        moveq   #10,%d1
        moveq   #29,%d2
        moveq   #19,%d3
        jsr     -306(%a6)               | RectFill(10,10,29,19)
        move.l  %a4,%a1
        moveq   #1,%d0
        jsr     -342(%a6)               | SetAPen(1)
        move.l  %a4,%a1
        moveq   #3,%d0
        jsr     -348(%a6)               | SetBPen(3)
        move.l  %a4,%a1
        move.l  mode(%pc),%d0
        jsr     -354(%a6)               | SetDrMd(mode)
        move.l  %a4,%a1
        moveq   #40,%d0
        moveq   #30,%d1
        jsr     -240(%a6)               | Move(40,30)
        move.l  %a4,%a1
        lea     txt(%pc),%a0
        moveq   #2,%d0
        jsr     -60(%a6)                | Text("AB", 2)
        move.l  4.w,%a6
        move.l  #0x1000,%d0
        jsr     -318(%a6)               | Wait(SIGBREAKF_CTRL_C): the frame is published while blocked
        moveq   #0,%d0
        rts
