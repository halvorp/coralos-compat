; Window-definition binding (coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING; heirloom-bind `wdef`): DATA, read by the host
; (System/Heirloom/Bindings/macppc/) and handed to the engine; no code. It names ONE build of one window definition by what its code
; IS, and what the window it draws MEANS.
;
; The definition: WDEF 200 "EmptyWDEF" of Adobe Photoshop 7.0 (Mac OS 9 PowerPC; the 7.0 Tryout build), an accelerated resource (a
; RoutineDescriptor with a relative PowerPC record and a 1828-byte PEF container) that the program selects with procID 3200 for
; the windows it draws entirely itself. Read from its code (classic WDEF messages, `long def(short varCode, WindowRef w, short msg,
; long param)`): wDraw (0) draws nothing; wHit (1) answers wInContent when the point is in the structure region, else wNoHit (no drag,
; grow, go-away or zoom part: the program moves the window itself); wCalcRgns (2) sets BOTH the structure and the content region to the
; port rectangle in global coordinates (no frame at all); wNew (3) clears spareFlag (no zoom box) and keeps a 16-byte handle in
; dataHandle; wDispose (4) frees it; every other message (wGrow, wDrawGIcon, the Mac OS 8.5 messages) does nothing and answers 0.
; Its meaning is therefore a frameless utility window: everything in it is the program's content; no title, not movable by a frame,
; not closable, not resizable.
(window-definition
  (id "photoshop-7.0-emptywdef")
  (fragment-code (length 1312) (fnv1a64 "0xb194c5872806ba7f") (entry-offset 400))
  (meaning (role utility) (titled false) (movable false) (closable false) (resizable false)))
