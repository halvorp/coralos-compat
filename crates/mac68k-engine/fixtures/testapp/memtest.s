| Memory Manager from assembly (flat program at CODE_BASE): OS traps with the REGISTER convention. Results are stored
| at 0x3000.. for the test to read.
|   m68k-linux-gnu-as -m68020 memtest.s -o memtest.o && m68k-linux-gnu-objcopy -O binary -j .text memtest.o memtest.bin
        .include "traps.inc"
        .text
start:  moveq   #16,%d0
        .short  _NewHandle
        move.l  %a0,0x3000             | the handle
        move.w  %d0,0x3004             | OSErr
        move.l  %a0,%a3
        move.l  (%a3),%a1
        move.l  #0xCAFEBABE,(%a1)      | write through the double dereference
        move.l  %a3,%a0
        .short  _HLock
        beq.s   1f                     | the trap set Z from D0 = noErr
        bra.s   2f
1:      move.w  #1,0x3042
2:      move.l  %a3,%a0
        .short  _HGetState
        move.l  %d0,0x3008             | 0x80 = locked
        move.l  %a3,%a0
        move.l  #4096,%d0
        .short  _SetHandleSize         | locked: refused
        move.w  %d0,0x300C
        move.l  %a3,%a0
        .short  _HUnlock
        move.l  %a3,%a0
        move.l  #4096,%d0
        .short  _SetHandleSize
        move.w  %d0,0x300E
        move.l  (%a3),%a1
        move.l  (%a1),0x3010           | contents followed the block
        move.l  %a3,%a0
        .short  _GetHandleSize
        move.l  %d0,0x3014
        move.l  %a3,%a0
        .short  _DisposeHandle
        move.w  %d0,0x3018
        move.l  %a3,%a0
        .short  _GetHandleSize         | a disposed handle: memWZErr
        move.w  %d0,0x301A
        move.w  0x220,0x301C           | MemError is a read of the MemErr global
        moveq   #32,%d0
        .short  _NewPtrClear
        move.l  %a0,%a4
        move.l  %a4,0x3020
        lea     src(%pc),%a0
        move.l  %a4,%a1
        moveq   #8,%d0
        .short  _BlockMove
        move.l  (%a4),0x3028
        move.l  4(%a4),0x302C
        move.l  %a4,%a0
        .short  _GetPtrSize
        move.l  %d0,0x3030
        move.l  %a4,%a0
        .short  _DisposePtr
        move.w  %d0,0x3034
        move.l  #0x7FFFFFFF,%d0
        .short  _NewHandle             | far too large
        bne.s   3f                     | the trap set NE from D0 = memFullErr
        bra.s   4f
3:      move.w  #1,0x3040
4:      move.l  %a0,0x3050             | NIL on failure
        moveq   #16,%d0                | dirty a block, free it, take it back plain and then cleared
        .short  _NewHandle
        move.l  %a0,%a3
        move.l  (%a3),%a1
        move.l  #0xEEEEEEEE,(%a1)
        move.l  %a3,%a0
        .short  _DisposeHandle
        moveq   #16,%d0
        .short  _NewHandle
        move.l  %a0,%a3
        move.l  (%a3),%a1
        move.l  (%a1),0x3048           | plain NewHandle: the old contents are still there
        move.l  #0xEEEEEEEE,(%a1)
        move.l  %a3,%a0
        .short  _DisposeHandle
        moveq   #16,%d0
        .short  _NewHandleClear
        move.l  (%a0),%a1
        move.l  (%a1),0x3044           | NewHandleClear (bit 9 of the trap word): zeroed
        .short  _ExitToShell
src:    .ascii  "ABCDEFGH"
