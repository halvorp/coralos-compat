| CODE 2 of the test application: SetupMenus (jump-table entry 1).
|   m68k-linux-gnu-as -m68020 menus.s -o menus.o && m68k-linux-gnu-objcopy -O binary -j .text menus.o code2.bin
        .include "traps.inc"
        .text
setup:  clr.l   -(%sp)
        move.w  #128,-(%sp)            | MBAR 128: the Apple and File menus (MENU 128, 129)
        .short  _GetNewMBar
        .short  _SetMenuBar            | the handle GetNewMBar left on the stack is SetMenuBar's argument
        clr.l   -(%sp)
        move.w  #128,-(%sp)
        .short  _GetMenuHandle            | the Apple menu
        move.l  #0x44525652,-(%sp)     | 'DRVR'
        .short  _AppendResMenu            | no desk accessories in this application's fork: adds nothing
        clr.l   -(%sp)
        move.w  #130,-(%sp)
        pea     edit(%pc)
        .short  _NewMenu               | an Edit menu built by hand
        move.l  (%sp)+,%a3
        move.l  %a3,-(%sp)
        pea     items(%pc)
        .short  _AppendMenu
        move.l  %a3,-(%sp)
        clr.w   -(%sp)                 | beforeID 0 = at the end
        .short  _InsertMenu
        .short  _DrawMenuBar
        rts
edit:   .byte   4
        .ascii  "Edit"
items:  .byte   30
        .ascii  "Undo/Z;(-;Cut/X;Copy/C;Paste/V"
