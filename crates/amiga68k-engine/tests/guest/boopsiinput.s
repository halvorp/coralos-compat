# A guest BOOPSI gadget class driven by pointer input: GM_HITTEST says "hit" only for the left 15 pixels of the gadget,
# GM_GOACTIVE keeps the pointer (GMR_MEACTIVE), GM_HANDLEINPUT finishes on the button-up (*gpi_Termination = 123,
# GMR_VERIFY|GMR_NOREUSE), GM_GOINACTIVE is counted. The program then reads the IDCMP_GADGETUP message. Written for this
# repo (MIT). Counters from offset 8: c0 hittest, c1 goactive, c2 handleinput, c3 goinactive, c4 last hit mouse X;
# then the message: c5 Class, c6 Code, c7 IAddress, c8 gadget, c9 done marker.
        .equ    GA_LEFT, 0x80030001
        .equ    GA_TOP, 0x80030003
        .equ    GA_WIDTH, 0x80030005
        .equ    GA_HEIGHT, 0x80030007
        .equ    GA_RELVERIFY, 0x80030016
        .text
        bra.w   start
        .long   0
cnt:    .space  64
tags:   .long   GA_LEFT, 10, GA_TOP, 5, GA_WIDTH, 20, GA_HEIGHT, 10, GA_RELVERIFY, 1, 0, 0
nw:     .short  20,10,200,100
        .byte   0,1
        .long   0x40                    | IDCMP_GADGETUP
        .long   0x800
        .long   0,0,0,0,0
        .short  80,40,640,400
        .short  1
iname:  .asciz  "intuition.library"
cname:  .asciz  "btnclass"
sname:  .asciz  "gadgetclass"
        .align  2
disp:   move.l  (%a1),%d0
        lea     cnt(%pc),%a3
        cmp.l   #0,%d0
        beq.s   hit
        cmp.l   #2,%d0
        beq.s   goact
        cmp.l   #3,%d0
        beq.s   handle
        cmp.l   #4,%d0
        beq.s   goinact
super:  move.l  24(%a0),%a0
        move.l  8(%a0),%a3
        jmp     (%a3)
hit:    addq.l  #1,(%a3)
        moveq   #0,%d1
        move.w  8(%a1),%d1              | gpht_Mouse.X
        move.l  %d1,16(%a3)
        moveq   #0,%d0
        cmp.w   #15,%d1
        bge.s   hitret
        moveq   #4,%d0                  | GMR_GADGETHIT
hitret: rts
goact:  addq.l  #1,4(%a3)
        moveq   #0,%d0                  | GMR_MEACTIVE
        rts
handle: addq.l  #1,8(%a3)
        move.l  8(%a1),%a4              | gpi_IEvent
        moveq   #0,%d0
        cmp.w   #0xE8,6(%a4)            | IECODE_LBUTTON | IECODE_UP_PREFIX
        bne.s   hdone
        move.l  12(%a1),%a4             | gpi_Termination
        move.l  #123,(%a4)
        moveq   #10,%d0                 | GMR_NOREUSE | GMR_VERIFY
hdone:  rts
goinact: addq.l #1,12(%a3)
        moveq   #0,%d0
        rts
start:  move.l  4.w,%a6
        lea     iname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,%a6
        lea     cnt(%pc),%a3
        lea     nw(%pc),%a0
        jsr     -204(%a6)               | OpenWindow
        move.l  %d0,%a4
        lea     cname(%pc),%a0
        lea     sname(%pc),%a1
        sub.l   %a2,%a2
        moveq   #0,%d0
        moveq   #0,%d1
        jsr     -678(%a6)
        move.l  %d0,%a5
        lea     disp(%pc),%a0
        move.l  %a0,8(%a5)
        move.l  %a5,%a0
        jsr     -684(%a6)
        sub.l   %a0,%a0
        lea     cname(%pc),%a1
        lea     tags(%pc),%a2
        jsr     -636(%a6)               | NewObjectA
        move.l  %d0,32(%a3)
        move.l  %d0,%a1
        move.l  %a4,%a0
        moveq   #-1,%d0
        moveq   #1,%d1
        sub.l   %a2,%a2
        jsr     -438(%a6)               | AddGList
        move.l  4.w,%a6
        move.l  86(%a4),%a2             | Window.UserPort
        move.l  %a2,%a0
        jsr     -384(%a6)               | WaitPort
        move.l  %a2,%a0
        jsr     -372(%a6)               | GetMsg
        move.l  %d0,%a1
        move.l  20(%a1),20(%a3)         | Class
        moveq   #0,%d1
        move.w  24(%a1),%d1
        move.l  %d1,24(%a3)             | Code
        move.l  28(%a1),28(%a3)         | IAddress
        jsr     -378(%a6)               | ReplyMsg
        move.l  #1,36(%a3)
        rts
