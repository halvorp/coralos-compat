# Licences

- Our code: MIT (`LICENSE`), including `crates/heirloom-cpu-68k/vendor/shim/` and `vendor/m68k_nofpu.h`.
- `crates/heirloom-cpu-68k/vendor/`: third-party code, listed file by file in `NOTICE`:
  - Musashi (Karl Stenerud): MIT-style, notice kept in each file.
  - SoftFloat 2b (John R. Hauser): free redistribution with its notice retained (kept in each file).
- `crates/heirloom-raster/src/topaz8.rs`: the AROS built-in topaz 8 font data (`rom/graphics/default_font.c`), under the
  AROS Public License (APL) with its header kept; ported with Peter's 2026-10 ruling that AROS code may be ported.
  It is data only; the surrounding code is ours (MIT). No Amiga ROM font is used.
- `crates/heirloom-theme/src/uifont_data.rs`: Nunito Sans Regular (the CoralOS kit's `--font-ui`; Copyright 2016 The Nunito Sans Project Authors, https://github.com/Fonthausen/NunitoSans) rasterised to a 1-bit bitmap strip by `tools/gen-gute-form-font.py`, under the SIL Open Font License 1.1 (a derivative under the same licence; it is not sold by itself). Data only. The palette values in `heirloom-theme` are facts from the CoralOS UI kit.
- `crates/heirloom-cpu-ppc` is original MIT code written from the PowerPC Programming Environments Manual; no emulator source (MAME, dingusppc, SheepShaver, Executor) was copied. It depends on the `libm` crate (MIT or Apache-2.0, from crates.io, not vendored) for correctly rounded `fma` and `sqrt` in no_std. Its test encodings are produced by LLVM's assembler (`tools/ppc-regen.py`); no LLVM code is included.
- `crates/heirloom-cpu-ppc-oracle` (host-only, not part of the no_std workspace) depends on `ppc` 0.7.2 (benletchford/ppc-rs, MIT) from crates.io as a test oracle, and through it on `rustc_apfloat` (Apache-2.0 WITH LLVM-exception), `bitflags` and `smallvec` (MIT or Apache-2.0); none is vendored or linked into any engine. Its reference models are written from the PowerPC Programming Environments Manual.
- `crates/heirloom-cpu-arm`: adapted in part from HexRaysSA/rax (MIT, Copyright (c) 2026 RAX contributors, commit
  526acfa); each adapted file names its origin in a header, the full notice is in the crate's README.md (THIRD_PARTY),
  and its tests port rax's generated and hand-written AArch32 tests. Its test vectors' expectations come from Unicorn
  (host-side tooling in `tools/`, not a dependency or part of any binary).
- No GPL code is in the repository today: Musashi's MAME-derived FPU/PMMU files were replaced (2026-10-17) so the
  engines can link CoralOS's own user-mode libraries. GPL code may be added later, but any GPL code makes every
  binary that links it GPL as a whole, and a CoralOS engine statically links CoralOS libraries: GPL parts must
  then sit behind a process boundary (IPC), not in the same binary.
- No Amiga ROM, AROS (except the APL font data above), Workbench, Kickstart or Apple code is included.
