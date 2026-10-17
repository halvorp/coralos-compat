# Licences

- Our code: MIT (`LICENSE`). Permissive and GPL-compatible.
- `crates/retro-m68k/vendor/`: third-party code, listed file by file in `NOTICE`:
  - Musashi (Karl Stenerud): MIT-style, notice kept in each file.
  - SoftFloat 2b (John R. Hauser): free redistribution with its notice retained (kept in each file).
  - `m68kfpu.c`, `m68kmmu.h` (MAME team): distributed here under the MAME project licence,
    GPL-2.0-or-later (`LICENSES/GPL-2.0.txt`).
- Consequence: the `retro-m68k` crate, and every binary that links it (the `amiga68k` engines), is
  GPL-2.0-or-later as a whole. The other crates (`retro-guest-mem`, `retro-hunk`, `guestui-abi`) are MIT
  on their own.
- No Amiga ROM, AROS, Workbench, Kickstart or Apple code is included.
