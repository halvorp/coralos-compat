# test.library: a minimal disk library (RTF_AUTOINIT) used by tests/disklib.rs. Written for this repo (MIT).
# Position-independent except for the pointers the RomTag/tables need; those are listed in testlib.relocs.
        .text
start:  moveq   #-1,%d0   | a library run as a program must fail
        rts
romtag: .word   0x4afc   | rt_MatchWord
        .long   romtag   | rt_MatchTag
        .long   endskip   | rt_EndSkip
        .byte   0x80, 1, 9, 0   | rt_Flags (AUTOINIT), rt_Version, rt_Type (NT_LIBRARY), rt_Pri
        .long   libname   | rt_Name
        .long   idstring   | rt_IdString
        .long   inittab   | rt_Init
endskip:
libname:  .asciz "test.library"
idstring: .asciz "test.library 3.7 (18.10.2026)"
dosname:  .asciz "dos.library"
        .align  2
inittab:
        .long   42   | dataSize: struct Library (34) + seglist (4) + dos base (4)
        .long   functable
        .long   datainit
        .long   initroutine
functable:
        .long   libopen, libclose, libexpunge, libreserved, func1, -1
datainit:
        .byte   0xe0, 0x00   | INITBYTE ln_Type = NT_LIBRARY
        .word   8
        .byte   9, 0
        .byte   0xc0, 0x00   | INITLONG ln_Name
        .word   10
        .long   libname
        .byte   0xe0, 0x00   | INITBYTE lib_Flags = LIBF_CHANGED|LIBF_SUMUSED
        .word   14
        .byte   6, 0
        .byte   0xd0, 0x00   | INITWORD lib_Version = 3
        .word   20
        .word   3
        .byte   0xd0, 0x00   | INITWORD lib_Revision = 7
        .word   22
        .word   7
        .byte   0xc0, 0x00   | INITLONG lib_IdString
        .word   24
        .long   idstring
        .word   0   | end of table
initroutine:   | d0 = library, a0 = seglist, a6 = ExecBase
        movem.l %d2/%a2,-(%sp)
        move.l  %d0,%d2
        move.l  %d0,%a2
        move.l  %a0,34(%a2)   | remember the seglist
        moveq   #77,%d3   | trash callee-saved registers on purpose:
        sub.l   %a3,%a3   | the engine must give the caller's back
        lea     dosname(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)   | OpenLibrary("dos.library", 0): a bridged call from inside the guest call
        move.l  %d0,38(%a2)
        move.l  %d2,%d0   | the library
        movem.l (%sp)+,%d2/%a2
        rts
libopen:
        addq.w  #1,32(%a6)   | lib_OpenCnt
        move.l  %a6,%d0
        rts
libclose:
        subq.w  #1,32(%a6)
        moveq   #0,%d0
        rts
libexpunge:
        moveq   #0,%d0
        rts
libreserved:
        moveq   #0,%d0
        rts
func1:   | LVO -30: d0 = d0 + d1
        add.l   %d1,%d0
        rts
