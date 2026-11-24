; In-plane compact-palette bindings (coralos doctrine HEIRLOOM-APP-CHROME-NEEDS-A-BINDING; heirloom-bind `plane_chrome`):
; DATA, read by the host (System/Heirloom/Bindings/macppc/) and handed to the engine via `plane-chrome-binding`; no code.
;
; Adobe Photoshop 7.0 Tryout (Mac OS 9 PowerPC): translates the 5 floating ADM utility palette interiors (Tools, Navigator/Info,
; Color/Swatches/Styles, History/Actions/Presets, Layers/Channels/Paths) from offscreen Platinum pixel blits into semantic
; `DrawOp::Palette` (`pl-*`, CAP_KIT_PART_PALETTE_V1) ops while preserving live content viewports (Navigator thumbnail, Color
; spectrum ramp, History/Layers lists) via `palette-keep-pixels`.
;
; Keyed strictly by Photoshop 7's exact WDEF 128 resource (length 15012, FNV-1a 64 0xccfe72ef63c8a2ab), window pixel range,
; and active/inactive-invariant 1-bit dark-pixel mask guards over each palette's header/tab text.

; 1. Tools palette (57x400): 2-column 22-tool grid in 4 groups + foreground/background ColorWell + Edit/Screen mode MiniToggles.
(plane-chrome
  (id "photoshop-7.0-palette-tools")
  (resource (type "WDEF") (id 128) (length 15012) (fnv1a64 "0xccfe72ef63c8a2ab"))
  (window-range 52 64 380 420)
  (pixel-guard (rect 4 4 48 24) (dark-luma 128) (fnv1a64 "0x6ce2ca7f690ebf5f"))
  (palette-surface (rect 0 0 0 0) (clear-rect 0 0 0 0) (pen surface))
  (palette-keep-pixels (rect 0 0 57 24))
  (palette-tool-grid (rect 4 27 49 244) (cols 2)
    ; Group 1: Selection & Crop (probe each cell's 2px top inner bevel: sunken active ~110 vs raised ~238)
    (tool (icon box-select)   (flyout true)  (probe 6 28 18 2))
    (tool (icon move)         (flyout false) (probe 32 28 18 2))
    (tool (icon lasso)        (flyout true)  (probe 6 49 18 2))
    (tool (icon magic-wand)   (flyout false) (probe 32 49 18 2))
    (tool (icon crop)         (flyout false) (probe 6 70 18 2))
    (tool (icon shapes)       (flyout true)  (break-after true) (probe 32 70 18 2))
    ; Group 2: Retouch & Paint
    (tool (icon pipette)      (flyout true)  (probe 6 95 18 2))
    (tool (icon paintbrush)   (flyout true)  (probe 32 95 18 2))
    (tool (icon stamp)        (flyout true)  (probe 6 116 18 2))
    (tool (icon layers)       (flyout true)  (probe 32 116 18 2))
    (tool (icon eraser)       (flyout true)  (probe 6 137 18 2))
    (tool (icon paint-bucket) (flyout true)  (probe 32 137 18 2))
    (tool (icon droplet)      (flyout true)  (probe 6 158 18 2))
    (tool (icon sliders)      (flyout true)  (break-after true) (probe 32 158 18 2))
    ; Group 3: Vector & Type
    (tool (icon mouse-pointer) (flyout true) (probe 6 183 18 2))
    (tool (icon type)          (flyout true) (probe 32 183 18 2))
    (tool (icon pen-tool)      (flyout true) (probe 6 204 18 2))
    (tool (icon square)        (flyout true) (break-after true) (probe 32 204 18 2))
    ; Group 4: Utility & Navigation
    (tool (icon file-plus)    (flyout true)  (probe 6 229 18 2))
    (tool (icon pipette)      (flyout true)  (probe 32 229 18 2))
    (tool (icon hand)         (flyout false) (probe 6 250 18 2))
    (tool (icon zoom-in)      (flyout false) (probe 32 250 18 2)))
  (palette-color-well (rect 6 276 44 46) (fore 16 294) (back 44 300))
  (palette-mini-toggles (rect 6 328 44 20)
    (toggle (icon square-dashed) (probe 8 331 18 14) (threshold-luma 170))
    (toggle (icon square)        (probe 30 331 18 14) (threshold-luma 170)))
  (palette-mini-toggles (rect 4 354 48 20)
    (toggle (icon square)           (probe 6 356 12 12) (threshold-luma 170))
    (toggle (icon squares-unite)    (probe 22 356 12 12) (threshold-luma 170))
    (toggle (icon squares-subtract) (probe 38 356 12 12) (threshold-luma 170))))

; 2. Navigator / Info palette (206x112): compact tab bar + live document thumbnail viewport + zoom footer.
(plane-chrome
  (id "photoshop-7.0-palette-navigator")
  (resource (type "WDEF") (id 128) (length 15012) (fnv1a64 "0xccfe72ef63c8a2ab"))
  (window-range 180 280 90 220)
  (pixel-guard (rect 4 3 56 12) (dark-luma 128) (fnv1a64 "0xfd3b0bf594ce8af5"))
  (palette-surface (rect 0 0 0 0) (clear-rect 0 0 0 0) (pen surface))
  (palette-tabs (rect 0 0 0 22) (flyout true)
    (tab (label "Navigator") (probe 8 3 64 2))
    (tab (label "Info")      (probe 96 3 32 2)))
  (palette-keep-pixels (rect 2 22 -2 -26))
  (palette-footer (rect 0 -24 -16 24)
    (key (icon zoom-out))
    (key (icon zoom-in)))
  (suppress-bottom-right 16 16))

; 3. Color / Swatches / Styles palette (206x112): compact tab bar + ColorWell + R/G/B GradientSliders + live RGB spectrum bar.
(plane-chrome
  (id "photoshop-7.0-palette-color")
  (resource (type "WDEF") (id 128) (length 15012) (fnv1a64 "0xccfe72ef63c8a2ab"))
  (window-range 180 280 90 220)
  (pixel-guard (rect 4 3 56 12) (dark-luma 128) (fnv1a64 "0x730f42326c11b067"))
  (palette-surface (rect 0 0 0 0) (clear-rect 0 0 0 0) (pen surface))
  (palette-tabs (rect 0 0 0 22) (flyout true)
    (tab (label "Color")    (probe 8 3 40 2))
    (tab (label "Swatches") (probe 64 3 56 2))
    (tab (label "Styles")   (probe 132 3 40 2)))
  (palette-color-well (rect 4 24 40 36) (fore 16 32) (back 32 48))
  (palette-gradient-slider (rect 48 24 -8 18) (label "R") (stops "0x000000" "0xff0000") (track 58 31 82 6) (default 0))
  (palette-gradient-slider (rect 48 44 -8 18) (label "G") (stops "0x000000" "0x00ff00") (track 58 51 82 6) (default 0))
  (palette-gradient-slider (rect 48 64 -8 18) (label "B") (stops "0x000000" "0x0000ff") (track 58 71 82 6) (default 0))
  (palette-keep-pixels (rect 4 91 -4 19)))

; 4. History / Actions / Presets palette (206x112): compact tab bar + live list viewport + 3-key action footer.
(plane-chrome
  (id "photoshop-7.0-palette-history")
  (resource (type "WDEF") (id 128) (length 15012) (fnv1a64 "0xccfe72ef63c8a2ab"))
  (window-range 180 280 90 260)
  (pixel-guard (rect 4 3 56 12) (dark-luma 128) (fnv1a64 "0xd4cf362068dabc07"))
  (palette-surface (rect 0 0 0 0) (clear-rect 0 0 0 0) (pen surface))
  (palette-tabs (rect 0 0 0 22) (flyout true)
    (tab (label "History") (probe 8 3 52 2))
    (tab (label "Actions") (probe 76 3 28 2))
    (tab (label "Presets") (probe 120 3 52 2)))
  (palette-keep-pixels (rect 2 22 -18 -26))
  (palette-footer (rect 0 -24 -16 24)
    (key (icon file-plus) (probe 112 -20 18 16))
    (key (icon eye)       (probe 140 -20 18 16))
    (key (icon trash)     (probe 168 -20 18 16)))
  (suppress-bottom-right 16 16))

; 5. Layers / Channels / Paths palette (220x400): compact tab bar + lock MiniToggles + live layer list + 6-key action footer.
(plane-chrome
  (id "photoshop-7.0-palette-layers")
  (resource (type "WDEF") (id 128) (length 15012) (fnv1a64 "0xccfe72ef63c8a2ab"))
  (window-range 180 300 180 640)
  (pixel-guard (rect 4 3 56 12) (dark-luma 128) (fnv1a64 "0xdc641afc47ab6d49"))
  (palette-surface (rect 0 0 0 0) (clear-rect 0 0 0 0) (pen surface))
  (palette-tabs (rect 0 0 0 22) (flyout true)
    (tab (label "Layers")   (probe 8 3 48 2))
    (tab (label "Channels") (probe 72 3 64 2))
    (tab (label "Paths")    (probe 152 3 40 2)))
  (palette-mini-toggles (rect 46 58 80 20)
    (toggle (icon square-dashed) (probe 50 61 14 14) (threshold-luma 150))
    (toggle (icon paintbrush)    (probe 68 61 14 14) (threshold-luma 150))
    (toggle (icon move)          (probe 86 61 14 14) (threshold-luma 150))
    (toggle (icon lock)          (probe 104 61 14 14) (threshold-luma 150)))
  (palette-keep-pixels (rect 2 82 -18 -26))
  (palette-footer (rect 0 -24 -16 24)
    (key (icon sliders)     (probe 68 -20 18 16))
    (key (icon square-dashed) (probe 90 -20 18 16))
    (key (icon folder-plus) (probe 112 -20 18 16))
    (key (icon layers)      (probe 134 -20 18 16))
    (key (icon file-plus)   (probe 156 -20 18 16))
    (key (icon trash)       (probe 178 -20 18 16)))
  (suppress-bottom-right 16 16))
