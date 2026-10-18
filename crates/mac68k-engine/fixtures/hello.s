| Classic Mac "Hello Mac" fixture, assembled flat (no resource fork) and loaded at CODE_BASE.
|   m68k-linux-gnu-as -m68020 hello.s -o hello.o && m68k-linux-gnu-objcopy -O binary -j .text hello.o hello.bin
| Every Toolbox call is an inline A-line word (Pascal convention: result space, args left to right, then the trap).
        .text
start:  pea     qdglobals(%pc)
        .short  0xA86E                  | _InitGraf(@qd)
        .short  0xA8FE                  | _InitFonts
        .short  0xA912                  | _InitWindows
        .short  0xA930                  | _InitMenus
        .short  0xA9CC                  | _TEInit
        clr.l   -(%sp)
        .short  0xA97B                  | _InitDialogs(NIL)
        .short  0xA850                  | _InitCursor
        clr.l   -(%sp)                  | result: WindowPtr
        clr.l   -(%sp)                  | wStorage = NIL
        pea     rect(%pc)               | boundsRect
        pea     title(%pc)              | title
        move.b  #1,-(%sp)               | visible
        clr.w   -(%sp)                  | documentProc
        move.l  #-1,-(%sp)              | behind = (WindowPtr)-1
        move.b  #1,-(%sp)               | goAwayFlag
        clr.l   -(%sp)                  | refCon
        .short  0xA913                  | _NewWindow
        movea.l (%sp)+,%a3              | the WindowPtr
        move.l  %a3,-(%sp)
        .short  0xA914                  | _DisposeWindow
        .short  0xA9F4                  | _ExitToShell
rect:   .short  40,30,240,350           | top, left, bottom, right
title:  .byte   9
        .ascii  "Hello Mac"
        .align  2
        .space  256                     | the QuickDraw globals live BELOW thePort: 206 bytes (76 private + 130 public)
qdglobals:
        .long   0
