; In-plane chrome binding (coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING; heirloom-bind `plane_chrome`):
; DATA, read by the host (System/Heirloom/Bindings/mac68k/) and handed to the engine; no code.
;
; MacWrite 4.5 (Mac OS 68k): document window formatting ruler (tab wells, 6 lines/inch option, line-spacing
; selector 1x / 1.5x / 2x, paragraph alignment selector left / center / right / justify) and manual bottom-right
; grow box. Keyed strictly by CODE resource ID 1, exact byte length 2530, and FNV-1a 64 digest.
(plane-chrome
  (id "macwrite-4.5-ruler")
  (resource (type "CODE") (id 1) (length 2530) (fnv1a64 "0x73421e483f3e48ef"))
  (band (rect 0 0 487 46) (remap-1bit-pixels true))
  (button (rect 6 26 22 17) (clear-rect 5 25 24 19) (label " ") (icon tab-stop))
  (button (rect 32 26 22 17) (clear-rect 31 25 24 19) (label " ") (icon tab-decimal))
  (button (rect 73 27 16 16) (clear-rect 72 26 18 18) (active-rect 76 30 10 10) (label " "))
  (button (rect 188 26 39 18) (clear-rect 190 25 35 20) (active-rect 193 29 28 12) (label "1"))
  (button (rect 229 26 40 18) (clear-rect 230 25 35 20) (active-rect 233 29 28 12) (label "1½"))
  (button (rect 271 26 39 18) (clear-rect 270 25 35 20) (active-rect 273 29 28 12) (label "2"))
  (button (rect 327 26 33 18) (clear-rect 326 25 35 20) (active-rect 329 29 28 12) (label " ") (icon align-left))
  (button (rect 367 26 33 18) (clear-rect 366 25 35 20) (active-rect 369 29 28 12) (label " ") (icon align-center))
  (button (rect 407 26 33 18) (clear-rect 406 25 35 20) (active-rect 409 29 28 12) (label " ") (icon align-right))
  (button (rect 447 26 33 18) (clear-rect 446 25 35 20) (active-rect 449 29 28 12) (label " ") (icon align-justify))
  (suppress-bottom-right 22 22))
