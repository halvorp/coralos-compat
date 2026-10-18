# coralos-compat

Retro-compatibility engines for CoralOS, native-first: a guest program's UI becomes
native CoralOS windows through `guestui`, not a pixel emulator. One concern per crate, every source file under
1000 lines. Status: early skeleton; the Amiga 68k fixture runs inside CoralOS in QEMU and on a Z83 (2026-10-17).

| crate | concern |
|---|---|
| `retro-guest-mem` | checked big-endian guest RAM, spans, named refusals |
| `retro-m68k` | Musashi 68k core wrapper (ONE guest CPU per process) |
| `retro-ppc` | PowerPC 32-bit user-mode interpreter (601/603/G3 user set, FPU, no AltiVec); no global state, native-glue hook for Toolbox imports |
| `retro-hunk` | AmigaOS Hunk executable loader (+ test-only writer) |
| `guestui-abi` | FIRST DRAFT of the normalized UI interface engines call (`GuestUi`) |
| `retro-common` | `Refusal` and `Step`, shared by every engine |
| `mac68k-engine` | classic Mac 68k skeleton: A-line trap bridge, Pascal-convention Toolbox calls (docs/MAC68K-SKELETON.md) |
| `retro-pef` | PEF (Code Fragment Manager) container: loader section, pattern data, every relocation opcode, export hash, writer |
| `macppc-engine` | classic Mac PowerPC plumbing: CFM instantiation, imports bound by name onto the mac68k Toolbox handlers (docs/MACPPC-PLUMBING.md); no CPU yet |
| `amiga68k-engine` | fake ExecBase, LVO trap bridge, Intuition handlers, host runner |

The crates are `no_std` + `alloc`, so the same code runs on the host and in CoralOS user mode.
`cargo test --workspace` runs everything. `cargo run -p amiga68k-engine --bin amiga68k-run -- --fixture`
runs the built-in demo program and prints the recorded UI calls.

Real binaries (never committed): `docs/CORPUS.md` lists the 64 Amiga and 19 Mac programs the engines are censused on, where each
came from and under which licence, the toolkit usage matrix, and the table rows to cover first; `tools/corpus-*.py` reproduce it
and the `census_corpus.rs` tests of each engine run the binaries (they skip loudly when `/data/build/corpus-bin` is absent).

Licensing: MIT (our code, Musashi) plus SoftFloat's own permissive terms; see `LICENSES.md` and `NOTICE`.
