| Resource Manager from assembly (flat program): Pascal-convention Toolbox traps over a fork the test builds
| (TEXT 128 "Greeting" = "hello", TEXT 129 "Second" = "world!"). Results are stored at 0x3000.. for the test.
|   m68k-linux-gnu-as -m68020 restest.s -o restest.o && m68k-linux-gnu-objcopy -O binary -j .text restest.o restest.bin
        .include "traps.inc"
        .equ TEXT, 0x54455854
        .text
start:  clr.l   -(%sp)
        move.l  #TEXT,-(%sp)
        move.w  #128,-(%sp)
        .short  _GetResource
        move.l  (%sp)+,%a3
        move.l  %a3,0x3000             | the handle
        clr.w   -(%sp)
        move.l  #TEXT,-(%sp)
        .short  _CountResources
        move.w  (%sp)+,0x3004
        clr.l   -(%sp)
        move.l  #TEXT,-(%sp)
        pea     name(%pc)
        .short  _GetNamedResource      | "second": a case-insensitive match
        move.l  (%sp)+,0x3008
        clr.l   -(%sp)
        move.l  #TEXT,-(%sp)
        move.w  #999,-(%sp)
        .short  _GetResource           | no such id
        move.l  (%sp)+,0x300C
        clr.w   -(%sp)
        .short  _ResError
        move.w  (%sp)+,0x3010
        clr.l   -(%sp)
        move.l  %a3,-(%sp)
        .short  _SizeRsrc
        move.l  (%sp)+,0x3014
        move.l  (%a3),%a1
        move.l  (%a1),0x3018           | the bytes are in guest memory behind the handle
        clr.l   -(%sp)
        move.l  #TEXT,-(%sp)
        move.w  #128,-(%sp)
        .short  _GetResource           | already loaded: the same handle
        move.l  (%sp)+,0x301C
        clr.l   -(%sp)
        move.l  #TEXT,-(%sp)
        move.w  #2,-(%sp)
        .short  _GetIndResource        | the second TEXT resource
        move.l  (%sp)+,0x3030
        move.l  %a3,-(%sp)
        .short  _ReleaseResource
        clr.w   -(%sp)
        .short  _ResError
        move.w  (%sp)+,0x3020
        move.l  %a3,-(%sp)
        .short  _ReleaseResource       | no longer a resource handle
        clr.w   -(%sp)
        .short  _ResError
        move.w  (%sp)+,0x3022
        .short  _ExitToShell
name:   .byte   6
        .ascii  "second"
