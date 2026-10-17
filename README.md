# coralos-compat

Retro-compatibility engines for CoralOS, native-first: a guest program's UI becomes
native CoralOS windows through `guestui`, not a pixel emulator. One concern per crate, every source file under
1000 lines. Status: early skeleton; the Amiga 68k fixture runs inside CoralOS in QEMU and on a Z83 (2026-10-17).

| crate | concern |
|---|---|
| `retro-guest-mem` | checked big-endian guest RAM, spans, named refusals |
| `retro-m68k` | Musashi 68k core wrapper (ONE guest CPU per process) |
| `retro-hunk` | AmigaOS Hunk executable loader (+ test-only writer) |
| `guestui-abi` | FIRST DRAFT of the normalized UI interface engines call (`GuestUi`) |
| `amiga68k-engine` | fake ExecBase, LVO trap bridge, Intuition handlers, host runner |

The crates are `no_std` + `alloc`, so the same code runs on the host and in CoralOS user mode.
`cargo test --workspace` runs everything. `cargo run -p amiga68k-engine --bin amiga68k-run -- --fixture`
runs the built-in demo program and prints the recorded UI calls.

Licensing: see `LICENSES.md`. Our code is MIT; `retro-m68k` bundles MAME-derived files distributed under
GPL-2.0-or-later, so anything that links it (the engines) is GPL-2.0-or-later as a whole.
