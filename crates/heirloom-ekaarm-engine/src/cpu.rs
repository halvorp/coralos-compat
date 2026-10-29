//! The ARM CPU this engine runs on: ONE name, two backends with one API (doctrine ONE-NAME-ONE-CONCERN-LOADED).
//! `cpu-library` = cpu-arm.library, the dynamically loaded CPU, driven through its CpuVTable by heirloom-cpu-arm-client;
//! the CoralOS engine process builds this way and binds the table it opened before the first guest instruction.
//! `cpu-static` (the default) = heirloom-cpu-arm linked in, for the host tools and for consumers not yet split
//! (Heirloom.app; delete-when: the last such consumer is an engine process, docs/heirloom/plan.tsv row cpu-arm).
#[cfg(feature = "cpu-library")]
pub use heirloom_cpu_arm_client::*;
#[cfg(all(feature = "cpu-static", not(feature = "cpu-library")))]
pub use heirloom_cpu_arm::*;
#[cfg(not(any(feature = "cpu-static", feature = "cpu-library")))]
compile_error!("enable exactly the CPU backend this build runs on: `cpu-library` (cpu-arm.library) or `cpu-static`");
