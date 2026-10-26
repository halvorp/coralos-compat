# A guest program that opens test.library, calls it, closes it, and records what it saw in `results` (offset 4).
        .text
        bra.w   start
results: .space 64
libname: .asciz "test.library"
nosuch:  .asciz "nosuch.library"
        .align  2
start:  move.l  4.w,%a6
        move.l  #0xd2d2d2d2,%d2
        move.l  #0xd3d3d3d3,%d3
        move.l  #0xa3a3a3a3,%a3
        lea     libname(%pc),%a1
        moveq   #2,%d0
        jsr     -552(%a6)   | OpenLibrary("test.library", 2)
        lea     results(%pc),%a4
        move.l  %d0,(%a4)   | [0] library base
        move.l  %d2,4(%a4)   | [1] d2 survived
        move.l  %d3,8(%a4)   | [2] d3 survived
        move.l  %a3,12(%a4)   | [3] a3 survived
        move.l  %a6,16(%a4)   | [4] a6 is ExecBase again
        move.l  %d0,%d0
        beq.s   done
        move.l  %d0,%a5
        moveq   #5,%d0
        moveq   #6,%d1
        move.l  %a5,%a6
        jsr     -30(%a6)   | the library's own function: 5 + 6
        move.l  %d0,20(%a4)   | [5]
        move.l  34(%a5),24(%a4)   | [6] seglist the init routine saw
        move.l  38(%a5),28(%a4)   | [7] dos.library base the init routine opened
        moveq   #0,%d0
        move.w  20(%a5),%d0
        move.l  %d0,32(%a4)   | [8] lib_Version
        moveq   #0,%d0
        move.w  32(%a5),%d0
        move.l  %d0,36(%a4)   | [9] lib_OpenCnt while open
        move.l  4.w,%a6
        move.l  %a5,%a1
        jsr     -414(%a6)   | CloseLibrary
        moveq   #0,%d0
        move.w  32(%a5),%d0
        move.l  %d0,40(%a4)   | [10] lib_OpenCnt after close
        lea     libname(%pc),%a1
        move.l  #99,%d0
        jsr     -552(%a6)   | OpenLibrary("test.library", 99): too old
        move.l  %d0,44(%a4)   | [11] must be NULL
        lea     nosuch(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,48(%a4)   | [12] must be NULL
        lea     libname(%pc),%a1
        moveq   #3,%d0
        jsr     -552(%a6)   | second open of the loaded library
        move.l  %d0,52(%a4)   | [13] the same base as [0]
done:   rts
