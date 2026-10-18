# A program's own subclass of Group.mui through MUI_CreateCustomClass, written for this repo (MIT). The dispatcher handles OM_NEW (calls
# DoSuperMethod, then stores 'MYMY' in its 8-byte instance data), OM_GET of MYATTR, and a method of its own (0x80070010: stores its
# argument); everything else is DoSuperMethod into the NATIVE Group.mui. Results (longs) from offset 8, read by tests/muicustom.rs.
        .equ    MYATTR, 0x80070001
        .equ    MYMETH, 0x80070010
        .equ    MUIA_Group_Horiz, 0x8042536b
        .equ    MUIA_UserData, 0x80420313
        .equ    MUIM_Notify, 0x8042c9cb
        .text
        bra.w   start
        .long   0
res:    .space  128
tagsn:  .long   MUIA_Group_Horiz, 1, MYATTR, 5, 0, 0
tagsu:  .long   MUIA_UserData, 22, 0, 0
gname:  .asciz  "Group.mui"
tname:  .asciz  "Text.mui"
iname:  .asciz  "intuition.library"
mname:  .asciz  "muimaster.library"
        .align  2
# dispatcher: a0 = class, a1 = message, a2 = object
disp:   move.l  (%a1),%d0
        cmp.l   #0x101,%d0
        beq.s   dnew
        cmp.l   #0x104,%d0
        beq.s   dget
        cmp.l   #MYMETH,%d0
        beq.s   dmeth
dsuper: move.l  24(%a0),%a0             | cl_Super
        move.l  8(%a0),%a3              | its h_Entry
        jmp     (%a3)
dnew:   movem.l %a0-%a1,-(%sp)
        bsr.s   dsuper
        movem.l (%sp)+,%a0-%a1
        tst.l   %d0
        beq.s   dout
        move.l  %d0,%a2
        bsr.s   inst
        move.l  #0x4D594D59,(%a4)       | 'MYMY'
        move.l  4(%a1),%a5              | copy MYATTR from the tag list into the instance
sloop:  move.l  (%a5),%d1
        beq.s   dout
        cmp.l   #MYATTR,%d1
        bne.s   snext
        move.l  4(%a5),4(%a4)
snext:  addq.l  #8,%a5
        bra.s   sloop
dout:   rts
dget:   cmp.l   #MYATTR,4(%a1)
        bne.s   dsuper
        bsr.s   inst
        move.l  8(%a1),%a5
        move.l  4(%a4),(%a5)
        moveq   #1,%d0
        rts
dmeth:  lea     res(%pc),%a5
        move.l  4(%a1),28(%a5)          | r7 = the method's argument
        addq.l  #1,32(%a5)              | r8 = how many times it ran
        moveq   #1,%d0
        rts
inst:   moveq   #0,%d1                  | a4 = INST_DATA(cl, obj)
        move.w  32(%a0),%d1
        lea     (%a2,%d1.l),%a4
        rts
start:  move.l  4.w,%a6
        lea     mname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)               | OpenLibrary muimaster
        move.l  %d0,%a5
        lea     iname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,%a4                 | a4 = IntuitionBase
        move.l  %a5,%a6
        sub.l   %a0,%a0                 | base
        lea     gname(%pc),%a1          | supername
        sub.l   %a2,%a2                 | supermcc
        moveq   #8,%d0                  | datasize
        lea     disp(%pc),%a3           | dispatcher
        jsr     -108(%a6)               | MUI_CreateCustomClass(NULL, "Group.mui", NULL, 8, disp)
        lea     res(%pc),%a3
        move.l  %d0,0(%a3)              | r0 = mcc
        move.l  %d0,%a2
        move.l  24(%a2),%a2             | mcc_Class
        move.l  %a2,4(%a3)              | r1 = the class
        move.l  %a4,%a6
        move.l  %a2,%a0
        sub.l   %a1,%a1
        lea     tagsn(%pc),%a2
        jsr     -636(%a6)               | NewObjectA(cl, NULL, {Horiz 1, MYATTR 5})
        move.l  %d0,8(%a3)              | r2 = the object
        move.l  %d0,%a5
        move.l  -4(%a5),12(%a3)         | r3 = o_Class (the program's class)
        move.l  #MYATTR,%d0
        move.l  %a5,%a0
        lea     16(%a3),%a1             | r4 = MYATTR (5, the guest's OM_GET)
        jsr     -654(%a6)
        move.l  #MUIA_Group_Horiz,%d0
        move.l  %a5,%a0
        lea     20(%a3),%a1             | r5 = Horiz (1, answered by the NATIVE Group.mui through DoSuperMethod)
        jsr     -654(%a6)
        move.l  4(%a3),%a0
        moveq   #0,%d1
        move.w  32(%a0),%d1             | cl_InstOffset
        move.l  (%a5,%d1.l),36(%a3)     | r9 = 'MYMY' (the instance data rootclass's block holds)
        sub.l   %a0,%a0
        lea     tname(%pc),%a1
        sub.l   %a2,%a2
        jsr     -636(%a6)               | NewObjectA(NULL, "Text.mui", NULL): a plain MUI object
        move.l  %d0,24(%a3)             | r6 = the text object
        move.l  %d0,%a2
        lea     -32(%sp),%sp            | DoMethod(text, MUIM_Notify, UserData, 22, custom, 2, MYMETH, 55)
        move.l  #MUIM_Notify,(%sp)
        move.l  #MUIA_UserData,4(%sp)
        move.l  #22,8(%sp)
        move.l  %a5,12(%sp)
        move.l  #2,16(%sp)
        move.l  #MYMETH,20(%sp)
        move.l  #55,24(%sp)
        move.l  -4(%a2),%a0
        move.l  %sp,%a1
        move.l  8(%a0),%a3
        jsr     (%a3)                   | (the text object's class is the native Text.mui: its h_Entry is the trap slot)
        lea     32(%sp),%sp
        lea     res(%pc),%a3
        move.l  %a2,%a0
        lea     tagsu(%pc),%a1
        jsr     -648(%a6)               | SetAttrsA(text, UserData = 22): the notification calls the program's dispatcher
        move.l  %a5,%a0
        jsr     -642(%a6)               | DisposeObject(custom): OM_DISPOSE through the program's dispatcher into the native Group
        move.l  %a2,%a0
        jsr     -642(%a6)
        move.l  #1,56(%a3)              | r14 = end marker
        rts
