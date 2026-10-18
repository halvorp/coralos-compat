# A guest program exercising dos.library on the volume; it records what it saw in `res` (offset 4, longwords).
# Written for this repo (MIT). Rebuilt by build.sh. Volume layout expected (see tests/dos_files.rs).
        .text
        bra.w   start
res:    .space  256                     | 64 longword results
fib:    .space  264                     | FileInfoBlock (260), longword aligned by the .align below
buf:    .space  64
name:   .space  128
s_foo:  .asciz  "LIBS:foo.library"
s_nope: .asciz  "LIBS:nope"
s_libs: .asciz  "LIBS:"
s_rel:  .asciz  "foo.library"
s_ramt: .asciz  "RAM:test"
s_sysn: .asciz  "LIBS:new"
s_dos:  .asciz  "dos.library"
s_path: .asciz  "xxx:yyy/zzz"
s_hello: .asciz "hello"
        .align  2

.macro  DOS lvo
        move.l  %a5,%a6
        jsr     \lvo(%a6)
.endm
.macro  STR label
        lea     \label(%pc),%a0
        move.l  %a0,%d1
.endm
.macro  REC idx
        move.l  %d0,\idx*4(%a4)
.endm
.macro  IOERR idx
        DOS -132
        move.l  %d0,\idx*4(%a4)
.endm

start:  move.l  4.w,%a6
        lea     s_dos(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,%a5
        lea     res(%pc),%a4
        lea     fib(%pc),%a3
        move.l  %a3,%d0                 | align the fib up to 4 (the hunk is longword aligned; fib is at +264)
        addq.l  #3,%d0
        and.l   #-4,%d0
        move.l  %d0,%a3

        STR     s_foo                   | [0] Lock(LIBS:foo.library, ACCESS_READ)
        moveq   #-2,%d2
        DOS     -84
        REC     0
        move.l  %d0,%d7
        STR     s_nope                  | [1] Lock(LIBS:nope) = 0, [2] IoErr
        moveq   #-2,%d2
        DOS     -84
        REC     1
        IOERR   2
        move.l  %d7,%d1                 | [3] Examine(lock, fib); [4] size; [5] type
        move.l  %a3,%d2
        DOS     -102
        REC     3
        move.l  124(%a3),4*4(%a4)
        move.l  4(%a3),5*4(%a4)
        move.l  %d7,%d1
        DOS     -90                     | UnLock
        STR     s_libs                  | dir = Lock(LIBS:)
        moveq   #-2,%d2
        DOS     -84
        move.l  %d0,%d6
        move.l  %d6,%d1
        move.l  %a3,%d2
        DOS     -102                    | Examine(dir)
        moveq   #0,%d5
exloop: move.l  %d6,%d1
        move.l  %a3,%d2
        DOS     -108                    | ExNext
        tst.l   %d0
        beq.s   exdone
        addq.l  #1,%d5
        bra.s   exloop
exdone: move.l  %d5,6*4(%a4)            | [6] entries in LIBS:
        IOERR   7                       | [7] ERROR_NO_MORE_ENTRIES
        move.l  %d6,%d1                 | CurrentDir(dir)
        DOS     -126
        STR     s_rel                   | [8] Open("foo.library", MODE_OLDFILE) relative to the new current dir
        move.l  #1005,%d2
        DOS     -30
        REC     8
        move.l  %d0,%d7
        move.l  %d7,%d1                 | [9] Read 10 -> 3
        lea     buf(%pc),%a0
        move.l  %a0,%d2
        moveq   #10,%d3
        DOS     -42
        REC     9
        move.l  %d7,%d1                 | [10] Seek(1, BEGINNING) returns the old position (3)
        moveq   #1,%d2
        moveq   #-1,%d3
        DOS     -66
        REC     10
        move.l  %d7,%d1                 | [11] Read 10 -> 2
        lea     buf(%pc),%a0
        move.l  %a0,%d2
        moveq   #10,%d3
        DOS     -42
        REC     11
        move.l  %d7,%d1                 | [12] Write to an OLDFILE handle fails -1, [13] IoErr 223
        lea     s_hello(%pc),%a0
        move.l  %a0,%d2
        moveq   #5,%d3
        DOS     -48
        REC     12
        IOERR   13
        move.l  %d7,%d1
        DOS     -36                     | Close
        STR     s_ramt                  | RAM:test: write hello
        move.l  #1006,%d2
        DOS     -30
        move.l  %d0,%d7
        move.l  %d7,%d1
        lea     s_hello(%pc),%a0
        move.l  %a0,%d2
        moveq   #5,%d3
        DOS     -48
        REC     14                      | [14] 5
        move.l  %d7,%d1
        DOS     -36
        STR     s_ramt                  | read it back
        move.l  #1005,%d2
        DOS     -30
        move.l  %d0,%d7
        move.l  %d7,%d1
        lea     buf(%pc),%a0
        move.l  %a0,%d2
        moveq   #10,%d3
        DOS     -42
        REC     15                      | [15] 5
        move.l  %d7,%d1
        DOS     -36
        STR     s_ramt
        DOS     -72                     | [16] DeleteFile
        REC     16
        STR     s_ramt                  | [17] Open again = 0, [18] IoErr 205
        move.l  #1005,%d2
        DOS     -30
        REC     17
        IOERR   18
        STR     s_sysn                  | [19] Open(LIBS:new, NEWFILE) on read-only SYS: = 0, [20] IoErr 214
        move.l  #1006,%d2
        DOS     -30
        REC     19
        IOERR   20
        move.l  %d6,%d1                 | [21] NameFromLock(dir) -> name
        lea     name(%pc),%a0
        move.l  %a0,%d2
        moveq   #127,%d3
        DOS     -402
        REC     21
        lea     s_path(%pc),%a0         | [22] FilePart offset, [23] PathPart offset
        move.l  %a0,%d1
        DOS     -870
        sub.l   %a0,%d0
        REC     22
        lea     s_path(%pc),%a0
        move.l  %a0,%d1
        DOS     -876
        sub.l   %a0,%d0
        REC     23
        move.l  %d6,%d1                 | restore cwd to the boot root, release the dir lock
        moveq   #0,%d1
        DOS     -126
        move.l  %d6,%d1
        DOS     -90
        rts
