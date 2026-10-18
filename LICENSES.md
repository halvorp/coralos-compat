# Licences

- Our code: MIT (`LICENSE`), including `crates/retro-m68k/vendor/shim/` and `vendor/m68k_nofpu.h`.
- `crates/retro-m68k/vendor/`: third-party code, listed file by file in `NOTICE`:
  - Musashi (Karl Stenerud): MIT-style, notice kept in each file.
  - SoftFloat 2b (John R. Hauser): free redistribution with its notice retained (kept in each file).
- `crates/retro-raster/src/topaz8.rs`: the AROS built-in topaz 8 font data (`rom/graphics/default_font.c`), under the
  AROS Public License (APL) with its header kept; ported with Peter's 2026-10 ruling that AROS code may be ported.
  It is data only; the surrounding code is ours (MIT). No Amiga ROM font is used.
- No GPL code is in the repository today: Musashi's MAME-derived FPU/PMMU files were replaced (2026-10-17) so the
  engines can link CoralOS's own user-mode libraries. GPL code may be added later, but any GPL code makes every
  binary that links it GPL as a whole, and a CoralOS engine statically links CoralOS libraries: GPL parts must
  then sit behind a process boundary (IPC), not in the same binary.
- No Amiga ROM, AROS (except the APL font data above), Workbench, Kickstart or Apple code is included.
