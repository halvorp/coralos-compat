| Oracle program: OpenWindow (IDCMP CLOSEWINDOW|MENUPICK) then Wait(SIGBREAKF_CTRL_C) so the live structures can be
| dumped from the emulator; then CloseWindow, CloseLibrary. PC-relative only (no relocations).
	.text
	.globl _start
_start:
	move.l	4.w,%a6
	lea	iname(%pc),%a1
	moveq	#0,%d0
	jsr	-552(%a6)		| OpenLibrary
	move.l	%d0,%d7
	beq.s	out
	lea	nw(%pc),%a0
	lea	title(%pc),%a1
	move.l	%a1,26(%a0)		| NewWindow.Title
	move.l	%d7,%a6
	jsr	-204(%a6)		| OpenWindow(a0 = NewWindow)
	move.l	%d0,%d6
	beq.s	cl
	move.l	4.w,%a6
	move.l	#0x1000,%d0		| SIGBREAKF_CTRL_C
	jsr	-318(%a6)		| Wait
	move.l	%d7,%a6
	move.l	%d6,%a0
	jsr	-72(%a6)		| CloseWindow
cl:	move.l	4.w,%a6
	move.l	%d7,%a1
	jsr	-414(%a6)		| CloseLibrary
out:	moveq	#0,%d0
	rts
	.even
nw:	.short	40,30,320,120		| Left, Top, Width, Height
	.byte	0,1			| DetailPen, BlockPen
	.long	0x300			| IDCMPFlags: CLOSEWINDOW | MENUPICK
	.long	0x100E			| Flags: DRAGBAR | DEPTHGADGET | CLOSEGADGET | ACTIVATE
	.long	0,0,0,0,0		| FirstGadget, CheckMark, Title (patched), Screen, BitMap
	.short	80,40,640,400		| Min/Max
	.short	1			| Type = WBENCHSCREEN
iname:	.asciz	"intuition.library"
title:	.asciz	"Oracle"
	.even
