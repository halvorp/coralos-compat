# Buffered I/O, DateStamp, Info and assigns from a guest program. Results at offset 4 (longwords). Written for this repo (MIT).
        .text
        bra.w   start
res:    .space  128
buf:    .space  64
ds:     .space  16
fib:    .space  264
info:   .space  40
s_ramt: .asciz  "RAM:tf"
s_ram:  .asciz  "RAM:"
s_sys:  .asciz  "SYS:"
s_dos:  .asciz  "dos.library"
s_xyz:  .asciz  "xyz\n"
s_wk:   .asciz  "WK:"
s_wk2:  .asciz  "WK:f"
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
start:  move.l  4.w,%a6
        lea     s_dos(%pc),%a1
        moveq   #0,%d0
        jsr     -552(%a6)
        move.l  %d0,%a5
        lea     res(%pc),%a4
        STR     s_ramt
        move.l  #1006,%d2
        DOS     -30                     | Open(RAM:t, NEWFILE)
        move.l  %d0,%d7
        move.l  %d7,%d1
        moveq   #97,%d2
        DOS     -312                    | FPutC 'a'
        REC     0                       | [0] 97
        move.l  %d7,%d1
        moveq   #10,%d2
        DOS     -312                    | FPutC '\n'
        move.l  %d7,%d1
        lea     s_xyz(%pc),%a0
        move.l  %a0,%d2
        moveq   #1,%d3
        moveq   #4,%d4
        DOS     -330                    | FWrite(xyz\n, 1, 4)
        REC     1                       | [1] 4
        move.l  %d7,%d1
        DOS     -36
        STR     s_ramt
        move.l  #1005,%d2
        DOS     -30                     | Open OLDFILE
        move.l  %d0,%d7
        move.l  %d7,%d1
        lea     buf(%pc),%a0
        move.l  %a0,%d2
        moveq   #32,%d3
        DOS     -336                    | FGets -> "a\n"
        REC     2                       | [2] buf
        move.l  %d7,%d1
        DOS     -306                    | FGetC -> 'x'
        REC     3                       | [3] 120
        move.l  %d7,%d1
        moveq   #120,%d2
        DOS     -318                    | UnGetC
        move.l  %d7,%d1
        DOS     -306                    | FGetC again -> 'x'
        REC     4                       | [4] 120
        move.l  %d7,%d1
        lea     buf(%pc),%a0
        move.l  %a0,%d2
        moveq   #1,%d3
        moveq   #8,%d4
        DOS     -324                    | FRead 8 blocks of 1 -> only "yz\n" left = 3
        REC     5                       | [5] 3
        move.l  %d7,%d1
        DOS     -306                    | FGetC at EOF -> -1
        REC     6                       | [6] -1
        move.l  %d7,%d1
        DOS     -36
        lea     ds(%pc),%a0
        move.l  %a0,%d1
        DOS     -192                    | DateStamp
        move.l  ds(%pc),%d0
        REC     7                       | [7] day
        STR     s_ramt
        moveq   #-2,%d2
        DOS     -84
        move.l  %d0,%d6
        move.l  %d6,%d1
        lea     fib(%pc),%a0
        move.l  %a0,%d2
        DOS     -102                    | Examine
        lea     fib(%pc),%a0
        move.l  132(%a0),%d0
        REC     8                       | [8] fib_Date.ds_Days
        move.l  %d6,%d1
        DOS     -90
        STR     s_ram
        moveq   #-2,%d2
        DOS     -84
        move.l  %d0,%d6
        move.l  %d6,%d1
        lea     info(%pc),%a0
        move.l  %a0,%d2
        DOS     -114                    | Info(RAM:)
        lea     info(%pc),%a0
        move.l  8(%a0),%d0
        REC     9                       | [9] 82 (validated)
        move.l  20(%a0),%d0
        REC     10                      | [10] 512
        move.l  %d6,%d1
        DOS     -90
        STR     s_sys
        moveq   #-2,%d2
        DOS     -84
        move.l  %d0,%d6
        move.l  %d6,%d1
        lea     info(%pc),%a0
        move.l  %a0,%d2
        DOS     -114                    | Info(SYS:)
        lea     info(%pc),%a0
        move.l  8(%a0),%d0
        REC     11                      | [11] 80 (write protected)
        STR     s_ram                   | AssignLock("WK", Lock(RAM:)) then Open(WK:f, NEWFILE)
        moveq   #-2,%d2
        DOS     -84
        move.l  %d0,%d2
        lea     s_wk(%pc),%a0
        move.l  %a0,%d1
        DOS     -612
        STR     s_wk2
        move.l  #1006,%d2
        DOS     -30
        REC     12                      | [12] nonzero
        move.l  %d0,%d1
        DOS     -36
        rts
