# coralos-compat (local skeleton)

This is the LOCAL skeleton of `halvorp/coralos-compat`. It is NOT published: no remote, no push.

Retro-compatibility engines for CoralOS, native-first (guest UI becomes native windows through
`guestui`, not a pixel emulator). One concern per crate, every source file under 1000 lines.

| crate | concern |
|---|---|
| `retro-guest-mem` | checked big-endian guest RAM, spans, named refusals |
| `retro-m68k` | Musashi 68k core wrapper (ONE guest CPU per process) |
| `retro-hunk` | AmigaOS Hunk executable loader (+ test-only writer) |
| `guestui-abi` | FIRST DRAFT of the normalized UI interface engines call (`GuestUi`) |
| `amiga68k-engine` | fake ExecBase, LVO trap bridge, Intuition handlers, host runner |

`cargo test --workspace` runs everything. `cargo run -p amiga68k-engine --bin amiga68k-run -- --fixture`
runs the built-in demo program and prints the recorded UI calls.
