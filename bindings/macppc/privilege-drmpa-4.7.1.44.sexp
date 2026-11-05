; Window-definition binding (coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING; heirloom-bind `wdef`): DATA, read by the host
; (System/Heirloom/Bindings/macppc/) and handed to the engine; no code. It names ONE build of one window definition by what its code
; IS, and what the window it draws MEANS.
;
; The definition: the WindowDefUPP the "drmpa" dialog library (Aladdin Privilege 4.7.1.44, a PowerPlant-built Carbon library that any
; program protected by that licensing system loads) passes to CreateCustomWindow (kWindowDefProcPtr) for its tryout / licence dialogs.
; Read from its code (MacWindows.h message numbers): kWindowMsgGetFeatures answers kWindowCanGetWindowRegion |
; kWindowDefSupportsColorGrafPort (0x40000012: no title bar, no grow, no zoom, no collapse); kWindowMsgGetRegion answers the
; structure region = the port's global rectangle grown by one pixel and the content region = that rectangle; kWindowMsgDraw frames
; the port rectangle grown by one pixel (FrameRect: a one-pixel border, nothing else); kWindowMsgHitTest answers wInContent for every
; point the Window Manager asks about (no drag region: the user cannot move it by its frame); Initialize / CleanUp / CalculateShape do
; nothing. Its meaning is therefore a plain bordered dialog box: no title, not movable, not closable, not resizable.
(window-definition
  (id "privilege-drmpa-4.7.1.44")
  (fragment-code (length 439244) (fnv1a64 "0xcc2b69a9d5527ccc") (entry-offset 388208))
  (meaning (role dialog) (titled false) (movable false) (closable false) (resizable false)))
