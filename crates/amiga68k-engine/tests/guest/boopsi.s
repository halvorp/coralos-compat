# A guest-defined BOOPSI class, written for this repo (MIT): "myclass", a subclass of gadgetclass with an 8-byte
# instance whose dispatcher handles OM_NEW / OM_SET / OM_GET itself and does DoSuperMethod for everything else.
# Results (longs) from offset 8: see the r* comments; tests/boopsi_guest.rs reads them.
        .equ    MYATTR, 0x80070001
        .equ    GA_LEFT, 0x80030001
        .equ    GA_TOP, 0x80030003
        .equ    GA_WIDTH, 0x80030005
        .text
        bra.w   start
        .long   0
res:    .space  128
tags1:  .long   GA_LEFT, 10, GA_WIDTH, 50, MYATTR, 5, 0, 0
tags2:  .long   MYATTR, 77, GA_TOP, 9, 0, 0
iname:  .asciz  "intuition.library"
cname:  .asciz  "myclass"
sname:  .asciz  "gadgetclass"
unk:    .asciz  "noSuchClass"
        .align  2
# dispatcher: a0 = class, a1 = message, a2 = object
disp:   move.l  (%a1),%d0
        cmp.l   #0x101,%d0
        beq.s   dnew
        cmp.l   #0x103,%d0
        beq.s   dset
        cmp.l   #0x104,%d0
        beq.s   dget
dsuper: move.l  24(%a0),%a0             | cl_Super
        move.l  8(%a0),%a3              | its h_Entry
        jmp     (%a3)                   | DoSuperMethodA (tail call)
dnew:   movem.l %a0-%a1,-(%sp)
        bsr.s   dsuper
        movem.l (%sp)+,%a0-%a1
        tst.l   %d0
        beq.s   dout
        move.l  %d0,%a2
        bsr.s   inst
        move.l  #0x4D594D59,(%a4)       | 'MYMY'
        bsr.s   store
dout:   rts
dset:   movem.l %a0-%a1,-(%sp)
        bsr.s   inst
        bsr.s   store
        movem.l (%sp)+,%a0-%a1
        bra.s   dsuper
dget:   cmp.l   #MYATTR,4(%a1)
        bne.s   dsuper
        bsr.s   inst
        move.l  8(%a1),%a5
        move.l  4(%a4),(%a5)
        moveq   #1,%d0
        rts
inst:   moveq   #0,%d1                  | a4 = INST_DATA(cl, obj)
        move.w  32(%a0),%d1
        lea     (%a2,%d1.l),%a4
        rts
store:  move.l  4(%a1),%a5              | copy MYATTR from the tag list (msg+4) into the instance
sloop:  move.l  (%a5),%d1
        beq.s   sdone
        cmp.l   #MYATTR,%d1
        bne.s   snext
        move.l  4(%a5),4(%a4)
snext:  addq.l  #8,%a5
        bra.s   sloop
sdone:  rts
start:  move.l  4.w,%a6
        lea     iname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)               | OpenLibrary intuition
        move.l  %d0,%a6
        lea     res(%pc),%a3
        lea     cname(%pc),%a0
        lea     sname(%pc),%a1
        sub.l   %a2,%a2
        moveq   #8,%d0
        moveq   #0,%d1
        jsr     -678(%a6)               | MakeClass("myclass","gadgetclass",0,8,0)
        move.l  %d0,%a5
        move.l  %d0,0(%a3)              | r0 = class
        lea     disp(%pc),%a0
        move.l  %a0,8(%a5)              | cl_Dispatcher.h_Entry
        lea     cname(%pc),%a0
        jsr     -672(%a6)               | FindClass before AddClass
        move.l  %d0,4(%a3)              | r1 = NULL (private until AddClass)
        move.l  %a5,%a0
        jsr     -684(%a6)               | AddClass
        lea     cname(%pc),%a0
        jsr     -672(%a6)
        move.l  %d0,8(%a3)              | r2 = class
        sub.l   %a0,%a0
        lea     cname(%pc),%a1
        lea     tags1(%pc),%a2
        jsr     -636(%a6)               | NewObjectA(0,"myclass",tags1)
        move.l  %d0,12(%a3)             | r3 = object
        move.l  %d0,%a4
        move.l  #MYATTR,%d0
        move.l  %a4,%a0
        lea     16(%a3),%a1             | r4 = MYATTR value (5), r5 = GetAttr result
        jsr     -654(%a6)
        move.l  %d0,20(%a3)
        move.l  #GA_LEFT,%d0
        move.l  %a4,%a0
        lea     24(%a3),%a1             | r6 = GA_Left (10, via the native gadgetclass), r7 = result
        jsr     -654(%a6)
        move.l  %d0,28(%a3)
        move.l  %a4,%a0
        lea     tags2(%pc),%a1
        jsr     -648(%a6)               | SetAttrsA(MYATTR=77, GA_Top=9)
        move.l  %d0,32(%a3)             | r8 = OM_SET result
        move.l  #MYATTR,%d0
        move.l  %a4,%a0
        lea     36(%a3),%a1             | r9 = 77
        jsr     -654(%a6)
        move.l  #GA_TOP,%d0
        move.l  %a4,%a0
        lea     40(%a3),%a1             | r10 = 9
        jsr     -654(%a6)
        move.l  -4(%a4),44(%a3)         | r11 = o_Class
        move.l  44(%a5),48(%a3)         | r12 = cl_ObjectCount (1)
        moveq   #0,%d1
        move.w  32(%a5),%d1
        move.l  %d1,52(%a3)             | r13 = cl_InstOffset
        move.w  34(%a5),%d1
        move.l  %d1,56(%a3)             | r14 = cl_InstSize
        move.w  12(%a4),%d1
        move.l  %d1,60(%a3)             | r15 = Gadget.Flags
        move.w  16(%a4),%d1
        move.l  %d1,64(%a3)             | r16 = Gadget.GadgetType
        move.l  30(%a4),68(%a3)         | r17 = Gadget.MutualExclude
        move.l  72(%a4),72(%a3)         | r18 = 'MYMY' (instance data right after the 72-byte gadget)
        move.l  %a5,%a0
        jsr     -714(%a6)               | FreeClass with a live object
        move.l  %d0,76(%a3)             | r19 = FALSE
        lea     cname(%pc),%a0
        jsr     -672(%a6)
        move.l  %d0,80(%a3)             | r20 = NULL (FreeClass removed it from the public list)
        sub.l   %a0,%a0
        lea     cname(%pc),%a1
        sub.l   %a2,%a2
        jsr     -636(%a6)               | NewObjectA by the now-unlisted name
        move.l  %d0,84(%a3)             | r21 = NULL
        move.l  %a4,%a0
        jsr     -642(%a6)               | DisposeObject
        move.l  44(%a5),88(%a3)         | r22 = cl_ObjectCount (0)
        move.l  %a5,%a0
        jsr     -714(%a6)               | FreeClass, now free
        move.l  %d0,92(%a3)             | r23 = TRUE
        sub.l   %a0,%a0
        lea     unk(%pc),%a1
        sub.l   %a2,%a2
        jsr     -636(%a6)
        move.l  %d0,96(%a3)             | r24 = NULL (unknown class)
        move.l  #1,100(%a3)             | r25 = done marker
        rts
