# A guest BOOPSI gadget class whose dispatcher answers GM_RENDER by filling its box in pen 2 on the RastPort it is
# handed (gpr_RPort). The program opens a borderless window, makes one gadget (GA_Left 10, GA_Top 5, 20x10), AddGList's it,
# and RefreshGList's it when `doref` (offset 4) is non-zero. Written for this repo (MIT). Results from offset 8:
# r0 window, r1 gadget, r2 AddGList position, r3 Window.FirstGadget, r4 RemoveGList position (second run only).
        .equ    GA_LEFT, 0x80030001
        .equ    GA_TOP, 0x80030003
        .equ    GA_WIDTH, 0x80030005
        .equ    GA_HEIGHT, 0x80030007
        .text
        bra.w   start
doref:  .long   1
res:    .space  64
gbase:  .long   0
tags:   .long   GA_LEFT, 10, GA_TOP, 5, GA_WIDTH, 20, GA_HEIGHT, 10, 0, 0
nw:     .short  20,10,200,100
        .byte   0,1
        .long   0
        .long   0x800
        .long   0,0,0,0,0
        .short  80,40,640,400
        .short  1
iname:  .asciz  "intuition.library"
gname:  .asciz  "graphics.library"
cname:  .asciz  "rectclass"
sname:  .asciz  "gadgetclass"
        .align  2
disp:   cmp.l   #1,(%a1)
        beq.s   render
        move.l  24(%a0),%a0
        move.l  8(%a0),%a3
        jmp     (%a3)
render: move.l  8(%a1),%a3              | gpr_RPort
        move.l  gbase(%pc),%a6
        move.l  %a3,%a1
        moveq   #2,%d0
        jsr     -342(%a6)               | SetAPen(2)
        move.l  %a3,%a1
        moveq   #0,%d0
        move.w  4(%a2),%d0
        moveq   #0,%d1
        move.w  6(%a2),%d1
        moveq   #0,%d2
        move.w  8(%a2),%d2
        add.l   %d0,%d2
        subq.l  #1,%d2
        moveq   #0,%d3
        move.w  10(%a2),%d3
        add.l   %d1,%d3
        subq.l  #1,%d3
        jsr     -306(%a6)               | RectFill(left, top, left+w-1, top+h-1)
        moveq   #1,%d0
        rts
start:  move.l  4.w,%a6
        lea     gname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        lea     gbase(%pc),%a0
        move.l  %d0,(%a0)
        lea     iname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,%a6
        lea     res(%pc),%a3
        lea     nw(%pc),%a0
        jsr     -204(%a6)               | OpenWindow
        move.l  %d0,%a4
        move.l  %d0,0(%a3)
        lea     cname(%pc),%a0
        lea     sname(%pc),%a1
        sub.l   %a2,%a2
        moveq   #0,%d0
        moveq   #0,%d1
        jsr     -678(%a6)               | MakeClass("rectclass","gadgetclass",0,0,0)
        move.l  %d0,%a5
        lea     disp(%pc),%a0
        move.l  %a0,8(%a5)
        move.l  %a5,%a0
        jsr     -684(%a6)               | AddClass
        sub.l   %a0,%a0
        lea     cname(%pc),%a1
        lea     tags(%pc),%a2
        jsr     -636(%a6)               | NewObjectA
        move.l  %d0,4(%a3)
        move.l  %d0,%a1                 | AddGList(window, gadget, -1, 1, NULL)
        move.l  %a4,%a0
        moveq   #-1,%d0
        moveq   #1,%d1
        sub.l   %a2,%a2
        jsr     -438(%a6)
        move.l  %d0,8(%a3)
        move.l  62(%a4),12(%a3)         | Window.FirstGadget
        tst.l   doref(%pc)
        beq.s   norefresh
        move.l  4(%a3),%a0              | RefreshGList(gadget, window, NULL, 1)
        move.l  %a4,%a1
        sub.l   %a2,%a2
        moveq   #1,%d0
        jsr     -432(%a6)
norefresh:
        move.l  %a4,%a0                 | RemoveGList(window, gadget, 1)
        move.l  4(%a3),%a1
        moveq   #1,%d0
        jsr     -444(%a6)
        move.l  %d0,16(%a3)
        move.l  62(%a4),20(%a3)         | FirstGadget after removal
        move.l  #1,24(%a3)
        move.l  4.w,%a6
        moveq   #0,%d0
        bset    #5,%d0
        jsr     -318(%a6)               | Wait on a signal nobody raises: the window stays up
        rts
