; Application-chrome binding (coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING; heirloom-bind `spec`): DATA, read by the host
; (System/Heirloom/Bindings/ami68k/) and handed to the engine; no code.
;
; Directory Opus 5.82 (AmigaOS 68k): button banks and environment placements stored as IFF FORM "EPUS" files.
; - BTNW (132 bytes): button bank window header — 32-byte NUL-terminated title at 0, IBox (left, top, width, height as big-endian
;   i16) at 32, flags (u32) at 128 (bit 4 = 16 = BTNWF_BORDERLESS: no 1 px cell inset).
; - BUTN (>= 14 bytes): button item — flags (u32) at 2 (bit 2 = 4 = graphic button skipped; bit 4 = 16 = new format with count at
;   offset 6, else 3 function records), packed NUL-terminated strings starting at 14 (2 strings per record: label then function name).
; - BANK (>= 29 bytes, in Environment/default): open bank placement — IBox at 20, path (NUL-terminated, max 256) at 28.
; - Anchoring: label-in-fill with prefix-cut, strip-lines-inside, and accent-from-swatch.
(binding
  (id "dopus5-5.82")
  (file (iff-form "EPUS"))
  (group (chunk "BTNW") (size 132) (title (cstr 0 32)) (window-box (box 32)) (inset 1) (no-inset-if (flag u32 128 16)))
  (item (chunk "BUTN") (min-size 14) (skip-if (flag u32 2 4))
        (label (strings 14) (count (field i16 6) (unless (flag u32 2 16) 3)) (per-record 2) (pick 0)))
  (placement (chunk "BANK") (min-size 29) (window-box (box 20)) (path (cstr 28 256)))
  (anchor (label-in-fill (prefix-cut true) (strip-lines-inside true) (accent-from-swatch true))))
