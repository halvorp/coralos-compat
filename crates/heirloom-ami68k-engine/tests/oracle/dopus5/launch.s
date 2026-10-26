| Oracle launcher (PC-relative only). AROS's m68k ROM ships no disk libraries (no asl/diskfont/locale/datatypes/
| commodities/rexxsyslib/amigaguide .library files are available here), but DirectoryOpus 5 refuses to start without them.
| So: 1. add STUB libraries (every vector `moveq #0,d0; rts`; LibOpen returns the base; locale GetCatalogStr returns its default string)
|     2. AssignAdd LIBS: DOpus5:Libs   3. LoadSeg + RunCommand DOpus5:DirectoryOpus, then Wait forever.
	.text
	.globl _start
_start:
	move.l	4.w,%a6
	lea	rexxn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	asln(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	dfn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	dtn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	cxn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	agn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	mstn(%pc),%a0
	moveq	#60,%d2
	bsr.w	mklib
	lea	mdbn(%pc),%a0
	moveq	#60,%d2
	bsr.w	mklib
	lea	mdtn(%pc),%a0
	moveq	#60,%d2
	bsr.w	mklib
	lea	locn(%pc),%a0
	moveq	#120,%d2
	bsr.w	mklib
	lea	stubgcs(%pc),%a0
	move.l	%a0,-70(%a3)		| locale -72 GetCatalogStr: JMP abs.l target
	| --- dos
	lea	dosn(%pc),%a1
	moveq	#0,%d0
	jsr	-552(%a6)		| OpenLibrary
	move.l	%d0,%a6
	beq.s	out
	lea	libs(%pc),%a0
	move.l	%a0,%d1
	moveq	#-2,%d2			| ACCESS_READ
	jsr	-84(%a6)		| Lock
	move.l	%d0,%d2
	beq.s	out
	lea	libn(%pc),%a0
	move.l	%a0,%d1
	jsr	-630(%a6)		| AssignAdd(LIBS, lock)
	lea	progd(%pc),%a0
	move.l	%a0,%d1
	moveq	#-2,%d2
	jsr	-84(%a6)		| Lock("DOpus5:")
	move.l	%d0,%d1
	jsr	-594(%a6)		| SetProgramDir: PROGDIR: = DOpus5:
	lea	cmd(%pc),%a0
	move.l	%a0,%d1
	jsr	-150(%a6)		| LoadSeg
	move.l	%d0,%d1
	beq.s	fail
	move.l	%d0,%d7
	move.l	#16384,%d2		| stack
	lea	nul(%pc),%a0
	move.l	%a0,%d3			| args
	moveq	#1,%d4			| arglen (just "\n")
	jsr	-504(%a6)		| RunCommand
	move.l	%d0,%d6
	bra.s	out
fail:	jsr	-132(%a6)		| IoErr
	move.l	%d0,%d5
out:	move.l	4.w,%a6
	move.l	#0x1000,%d0
	jsr	-318(%a6)		| Wait(CTRL_C) forever
	bra.s	out
dosn:	.asciz	"dos.library"
libs:	.asciz	"DOpus5:Libs"
libn:	.asciz	"LIBS"
progd:	.asciz	"DOpus5:"
cmd:	.asciz	"DOpus5:DirectoryOpus"
mklib:	| a6=exec a0=name d2=vectors -> a3=base
	move.l	%a0,%a4
	move.l	%d2,%d3
	move.l	%d2,%d0
	lsl.l	#2,%d0
	addq.l	#8,%d0
	move.l	#0x10001,%d1
	jsr	-198(%a6)		| AllocMem (vector array)
	move.l	%d0,%a2
	lea	stub(%pc),%a0
	move.l	%a2,%a1
	subq.l	#1,%d3
1:	move.l	%a0,(%a1)+
	dbra	%d3,1b
	moveq	#-1,%d0
	move.l	%d0,(%a1)
	lea	stubo(%pc),%a0
	move.l	%a0,(%a2)		| vector 1 = LibOpen returns the base
	move.l	%a2,%a0
	sub.l	%a1,%a1
	sub.l	%a2,%a2
	move.l	#256,%d0
	moveq	#0,%d1
	jsr	-84(%a6)		| MakeLibrary(funcInit, NULL, NULL, 256, 0)
	move.l	%d0,%a1
	move.l	%d0,%a3
	move.b	#9,8(%a1)		| NT_LIBRARY
	move.l	%a4,10(%a1)		| ln_Name
	move.b	#6,14(%a1)		| LIBF_SUMUSED|LIBF_CHANGED
	move.w	#44,20(%a1)		| lib_Version
	jsr	-396(%a6)		| AddLibrary
	rts
stub:	moveq	#0,%d0
	rts
stubo:	move.l	%a6,%d0
	rts
stubgcs: move.l	%a1,%d0
	rts
rexxn:	.asciz	"rexxsyslib.library"
asln:	.asciz	"asl.library"
dfn:	.asciz	"diskfont.library"
dtn:	.asciz	"datatypes.library"
cxn:	.asciz	"commodities.library"
agn:	.asciz	"amigaguide.library"
locn:	.asciz	"locale.library"
mstn:	.asciz	"mathieeesingtrans.library"
mdbn:	.asciz	"mathieeedoubbas.library"
mdtn:	.asciz	"mathieeedoubtrans.library"
nul:	.byte	10,0
	.even
