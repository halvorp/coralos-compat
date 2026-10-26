| GadTools fixture program (assembled with m68k-linux-gnu-as -m68020; tools/asm-fixture.sh turns it into gtmenu.bin +
| gtmenu.rel, which the tests wrap as a one-hunk executable with heirloom-fmt-hunk's writer).
|
| What it does: CreateContext, four gadgets (BUTTON "Go", CHECKBOX "Check" checked, STRING "abc" max 8, SLIDER 1..9
| level 3), CreateMenusA (two menus: Project{Open^O, Save^S, bar, Quit^Q}, Options{Verbose toggle, Mode A/B mutually
| exclusive, More > Sub One}), LayoutMenusA, OpenWindowTagList (IDCMP MENUPICK|GADGETUP|CLOSEWINDOW) with the gadget
| list, SetMenuStrip, SetWindowTitles, ModifyIDCMP, ActivateWindow, WindowToFront, RefreshGList, OffMenu(Save). Then it
| loops WaitPort / GT_GetIMsg / GT_ReplyIMsg: every message is recorded; a MENUPICK is resolved with ItemAddress and
| followed down its NextSelect chain, and a menu item whose GTMENUITEM_USERDATA is 'QUIT' (or CLOSEWINDOW) ends the loop.
| On the way out it reads the gadgets back with GT_GetGadgetAttrsA, then frees everything it made and exits.
|
| Result block (hunk base + 4, big-endian longs): +0 event count; +4.. eight 16-byte records {class, code, iaddress or
| ItemAddress, GTMENUITEM_USERDATA}; +132 checkbox Gadget.Flags; +136 GTCB_Checked read back; +140 GTSL_Level read back;
| +144 GTST_String pointer read back; +148 GTIN_Number-free marker 0xC0DE once the program reached its cleanup.
	.text
	.globl _start
_start:
	bra.w	main
results:
	.space	160
	.even

	.set	QUIT, 0x51554954
	.set	NOSUB_MENUNUM_SAVE, 0xF820
	.macro	NM type, label, key, flags, excl, ud
	.byte	\type, 0
	.long	\label
	.long	\key
	.short	\flags
	.long	\excl
	.long	\ud
	.endm
	.macro	MKGAD kind, ng, tags, slot
	moveq	#\kind, %d0
	move.l	%a3, %a0
	lea	\ng, %a1
	lea	\tags, %a2
	jsr	-30(%a6)
	move.l	%d0, %a3
	move.l	%d0, \slot(%a5)
	beq	fail
	.endm

main:
	link	%a5, #-64
	move.l	4.w, %a6
	lea	iname, %a1
	moveq	#0, %d0
	jsr	-552(%a6)		| OpenLibrary intuition
	move.l	%d0, -24(%a5)
	beq	fail
	lea	gname, %a1
	moveq	#0, %d0
	jsr	-552(%a6)		| OpenLibrary gadtools
	move.l	%d0, -28(%a5)
	beq	fail
	move.l	-28(%a5), %a6
	suba.l	%a0, %a0
	suba.l	%a1, %a1
	jsr	-126(%a6)		| GetVisualInfoA
	move.l	%d0, -16(%a5)
	clr.l	-4(%a5)
	lea	-4(%a5), %a0
	jsr	-114(%a6)		| CreateContext
	move.l	%d0, %a3
	MKGAD	1, ng_go, tags_none, -36
	MKGAD	2, ng_chk, tags_chk, -40
	MKGAD	12, ng_str, tags_str, -44
	MKGAD	11, ng_sld, tags_sld, -48
	lea	newmenu, %a0
	lea	menutags, %a1
	jsr	-48(%a6)		| CreateMenusA
	move.l	%d0, -12(%a5)
	beq	fail
	move.l	-12(%a5), %a0
	move.l	-16(%a5), %a1
	suba.l	%a2, %a2
	jsr	-66(%a6)		| LayoutMenusA
	move.l	-4(%a5), wintags_gadgets+4
	move.l	-24(%a5), %a6
	suba.l	%a0, %a0
	lea	wintags, %a1
	jsr	-606(%a6)		| OpenWindowTagList
	move.l	%d0, -8(%a5)
	beq	fail
	move.l	%d0, %a0
	move.l	-12(%a5), %a1
	jsr	-264(%a6)		| SetMenuStrip
	move.l	-8(%a5), %a0
	lea	newtitle, %a1
	move.l	#-1, %a2
	jsr	-276(%a6)		| SetWindowTitles
	move.l	-8(%a5), %a0
	move.l	#0x340, %d0
	jsr	-150(%a6)		| ModifyIDCMP (same flags)
	move.l	-8(%a5), %a0
	jsr	-450(%a6)		| ActivateWindow
	move.l	-8(%a5), %a0
	jsr	-312(%a6)		| WindowToFront
	move.l	-4(%a5), %a0
	move.l	-8(%a5), %a1
	suba.l	%a2, %a2
	moveq	#-1, %d0
	jsr	-432(%a6)		| RefreshGList
	move.l	-8(%a5), %a0
	move.l	#NOSUB_MENUNUM_SAVE, %d0
	jsr	-180(%a6)		| OffMenu(Save)

loop:
	move.l	4.w, %a6
	move.l	-8(%a5), %a0
	move.l	86(%a0), %a0
	jsr	-384(%a6)		| WaitPort(UserPort)
next:
	move.l	-28(%a5), %a6
	move.l	-8(%a5), %a0
	move.l	86(%a0), %a0
	jsr	-72(%a6)		| GT_GetIMsg
	tst.l	%d0
	beq	loop
	move.l	%d0, %a4
	move.l	20(%a4), %d2		| Class
	moveq	#0, %d3
	move.w	24(%a4), %d3		| Code
	move.l	28(%a4), %d4		| IAddress
	moveq	#0, %d5
	move.l	%a4, %a1
	jsr	-78(%a6)		| GT_ReplyIMsg
	cmp.l	#0x200, %d2
	beq	done
	cmp.l	#0x100, %d2
	beq	menupick
	bsr	record			| a GADGETUP
	bra	next
menupick:
	cmp.w	#0xFFFF, %d3
	beq	next
	move.l	-24(%a5), %a6
	move.l	-12(%a5), %a0
	moveq	#0, %d0
	move.w	%d3, %d0
	jsr	-144(%a6)		| ItemAddress
	move.l	%d0, %d4
	move.l	%d0, %a0
	move.l	34(%a0), %d5		| GTMENUITEM_USERDATA
	bsr	record
	cmp.l	#QUIT, %d5
	beq	done
	move.l	%d4, %a0
	moveq	#0, %d3
	move.w	32(%a0), %d3		| NextSelect
	bra	menupick

record:
	lea	results, %a0
	move.l	(%a0), %d0
	cmp.l	#8, %d0
	bhs.s	rec_full
	lsl.l	#4, %d0
	lea	4(%a0,%d0.l), %a1
	move.l	%d2, (%a1)+
	move.l	%d3, (%a1)+
	move.l	%d4, (%a1)+
	move.l	%d5, (%a1)+
	addq.l	#1, (%a0)
rec_full:
	rts

done:
	move.l	-28(%a5), %a6
	move.l	-40(%a5), %a0
	moveq	#0, %d0
	move.w	12(%a0), %d0
	move.l	%d0, results+132
	move.l	-40(%a5), %a0
	move.l	-8(%a5), %a1
	suba.l	%a2, %a2
	lea	get_chk, %a3
	jsr	-174(%a6)		| GT_GetGadgetAttrsA checkbox
	move.l	-44(%a5), %a0
	move.l	-8(%a5), %a1
	suba.l	%a2, %a2
	lea	get_str, %a3
	jsr	-174(%a6)		| ... string
	move.l	-48(%a5), %a0
	move.l	-8(%a5), %a1
	suba.l	%a2, %a2
	lea	get_sld, %a3
	jsr	-174(%a6)		| ... slider
	move.l	#0xC0DE, results+148
	move.l	-24(%a5), %a6
	move.l	-8(%a5), %a0
	jsr	-54(%a6)		| ClearMenuStrip
	move.l	-8(%a5), %a0
	jsr	-72(%a6)		| CloseWindow
	move.l	-28(%a5), %a6
	move.l	-12(%a5), %a0
	jsr	-54(%a6)		| FreeMenus
	move.l	-4(%a5), %a0
	jsr	-36(%a6)		| FreeGadgets
	move.l	-16(%a5), %a0
	jsr	-132(%a6)		| FreeVisualInfo
	move.l	4.w, %a6
	move.l	-28(%a5), %a1
	jsr	-414(%a6)		| CloseLibrary gadtools
	move.l	-24(%a5), %a1
	jsr	-414(%a6)		| CloseLibrary intuition
	unlk	%a5
	moveq	#0, %d0
	rts
fail:
	unlk	%a5
	moveq	#20, %d0
	rts

	.even
iname:	.asciz	"intuition.library"
gname:	.asciz	"gadtools.library"
wtitle:	.asciz	"GadTools fixture"
newtitle: .asciz "Renamed"
s_go:	.asciz	"Go"
s_chk:	.asciz	"Check"
s_abc:	.asciz	"abc"
s_proj:	.asciz	"Project"
s_open:	.asciz	"Open..."
s_save:	.asciz	"Save"
s_quit:	.asciz	"Quit"
s_opts:	.asciz	"Options"
s_verb:	.asciz	"Verbose"
s_ma:	.asciz	"Mode A"
s_mb:	.asciz	"Mode B"
s_more:	.asciz	"More"
s_sub1:	.asciz	"Sub One"
k_o:	.asciz	"O"
k_s:	.asciz	"S"
k_q:	.asciz	"Q"
	.even
ng_go:	.short	10, 10, 60, 14
	.long	s_go, 0
	.short	1
	.long	0, 0, 0
	.short	0
ng_chk:	.short	10, 30, 26, 11
	.long	s_chk, 0
	.short	2
	.long	0, 0, 0
	.short	0
ng_str:	.short	10, 50, 100, 14
	.long	0, 0
	.short	3
	.long	0, 0, 0
	.short	0
ng_sld:	.short	10, 70, 100, 12
	.long	0, 0
	.short	4
	.long	0, 0, 0
	.short	0
tags_none: .long 0
tags_chk: .long 0x80080004, 1, 0
tags_str: .long 0x8008002D, s_abc, 0x8008002E, 8, 0
tags_sld: .long 0x80080026, 1, 0x80080027, 9, 0x80080028, 3, 0
get_chk: .long 0x80080004, results+136, 0
get_str: .long 0x8008002D, results+144, 0
get_sld: .long 0x80080028, results+140, 0
menutags: .long 0x8008003E, 1, 0
wintags:
	.long 0x8000006E, wtitle
	.long 0x80000066, 320
	.long 0x80000067, 160
	.long 0x8000006A, 0x340
	.long 0x80000082, 1
	.long 0x80000084, 1
wintags_gadgets:
	.long 0x8000006C, 0
	.long 0
newmenu:
	NM	1, s_proj, 0, 0, 0, 0
	NM	2, s_open, k_o, 0, 0, 1
	NM	2, s_save, k_s, 0, 0, 2
	NM	2, -1, 0, 0, 0, 0
	NM	2, s_quit, k_q, 0, 0, QUIT
	NM	1, s_opts, 0, 0, 0, 0
	NM	2, s_verb, 0, 0x0109, 0, 3		| CHECKIT|MENUTOGGLE|CHECKED
	NM	2, s_ma, 0, 0x0101, 4, 4		| CHECKIT|CHECKED, excludes item 2
	NM	2, s_mb, 0, 0x0001, 2, 5		| CHECKIT, excludes item 1
	NM	2, s_more, 0, 0, 0, 6
	NM	3, s_sub1, 0, 0, 0, 7
	NM	0, 0, 0, 0, 0, 0
