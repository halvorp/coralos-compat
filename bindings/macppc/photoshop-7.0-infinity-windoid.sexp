; Window-definition binding (coralos doctrine HEIRLOOM-CUSTOM-WINDOW-NEEDS-A-BINDING; heirloom-bind `wdef`): DATA, read by the host
; (System/Heirloom/Bindings/macppc/) and handed to the engine; no code. It names ONE build of one window definition by what its code
; IS, and what the window it draws MEANS.
;
; The definition: WDEF 128 "Infinity Windoid 3.0" of Adobe Photoshop 7.0 (Mac OS 9 PowerPC; the 7.0 Tryout build), an accelerated
; resource (a RoutineDescriptor with a relative PowerPC record and a 14980-byte PEF container) that the program selects with procID
; 2048..2063 (128 * 16 + variation) for its floating tool, options, and palette windows. It implements classic floating windoid
; chrome (title bar, drag region, close box, zoom/collapse, optional grow box) while the content region holds the palette controls.
; Its meaning is therefore a floating utility window: titled, movable, closable, and resizable.
(window-definition
  (id "photoshop-7.0-infinity-windoid")
  (fragment-code (length 13624) (fnv1a64 "0x82249bfcce08650e") (entry-offset 0))
  (meaning (role utility) (titled true) (movable true) (closable true) (resizable true)))
