//! The 68k CPU this engine runs on: ONE name, two backends with one API (doctrine ONE-NAME-ONE-CONCERN-LOADED).
//! `cpu-library` = cpu-68k.library, the dynamically loaded CPU, driven through its CpuVTable by heirloom-cpu-68k-client;
//! the CoralOS engine process builds this way and binds the table it opened before the first guest instruction.
//! `cpu-static` (the default) = heirloom-cpu-68k linked in, for the host tools and for consumers not yet split
//! (Heirloom.app; delete-when: the last such consumer is an engine process, docs/heirloom/plan.tsv row cpu-68k).
#[cfg(feature = "cpu-library")]
pub use heirloom_cpu_68k_client::*;
#[cfg(all(feature = "cpu-static", not(feature = "cpu-library")))]
pub use heirloom_cpu_68k::*;
#[cfg(not(any(feature = "cpu-static", feature = "cpu-library")))]
compile_error!("enable exactly the CPU backend this build runs on: `cpu-library` (cpu-68k.library) or `cpu-static`");

/// Whether the CPU this build runs on is there: cpu-68k.library's table is bound (`cpu-library`), or the static core is linked in.
/// A caller that may run 68k code on a machine whose engine opened another CPU first (macppc.engine's Mixed Mode calls) asks this
/// and refuses by name instead of reaching an unbound table.
pub fn available() -> bool {
    #[cfg(feature = "cpu-library")]
    { heirloom_cpu_68k_client::is_bound() }
    #[cfg(not(feature = "cpu-library"))]
    { true }
}
