//! The CPU MODES each guest engine offers - THE declaration (one table, one home) the host's install choice and its per-app
//! `CPU Mode` menu are driven by. A mode is the user's word for how the guest's code runs; the engine's own tier names stay the
//! engine's (heirloom-cpu-abi `Tier::parse` for PowerPC), and a mode carries the one it sends, if any, as the engine's named
//! input [`CPU_TIER_INPUT`].
//!
//! | family                     | Automatic | JIT        | AOT                 | Compatible      |
//! |----------------------------|-----------|------------|---------------------|-----------------|
//! | macppc, amippc (cpu-ppc)   | engine's  | `jit`      | not yet (JIT runs)  | `interp`        |
//! | mac68k, ami68k (cpu-68k)   | the interpreter (Musashi-derived, one tier): JIT / AOT not yet | the interpreter |
//! | ekaarm (cpu-arm)           | the interpreter (one tier): JIT / AOT not yet                 | the interpreter |
//!
//! A family with ONE execution tier sends nothing: Automatic and Compatible are the same interpreter, the others are shown
//! "not available yet". When cpu-68k or cpu-arm gain a tier, its row here changes and every host follows. Each engine crate's
//! tests hold its row against its CPU's real parser (heirloom-macppc-engine `cpu_modes_are_the_tiers_tier_parse_accepts`).

/// The engine's named input (heirloom-ui-abi `Control::Input`) that forces its CPU tier.
pub const CPU_TIER_INPUT: &str = "cpu-tier";

/// How a guest's code runs, in the user's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuMode {
    /// The engine decides: AOT where a translation of the binary exists, else the fastest tier it has.
    #[default]
    Auto,
    /// Compiled to host code as it runs.
    Jit,
    /// Translated on this machine at install, cached; the JIT where it cannot.
    Aot,
    /// The reference interpreter.
    Compatible,
}

impl CpuMode {
    /// Menu order.
    pub const ALL: [CpuMode; 4] = [CpuMode::Auto, CpuMode::Jit, CpuMode::Aot, CpuMode::Compatible];

    /// The token a bundle's `(cpu-tier ...)` clause and a saved choice carry.
    pub fn token(self) -> &'static str {
        match self {
            CpuMode::Auto => "auto",
            CpuMode::Jit => "jit",
            CpuMode::Aot => "aot",
            CpuMode::Compatible => "interp",
        }
    }

    pub fn from_token(t: &str) -> Option<CpuMode> {
        CpuMode::ALL.into_iter().find(|m| m.token() == t)
    }
}

/// One mode as an engine offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    pub mode: CpuMode,
    /// Whether it can be chosen today.
    pub available: bool,
    /// The engine tier this mode sends as [`CPU_TIER_INPUT`]; None = nothing (the engine's own choice). An unavailable mode
    /// names the tier that runs in its place (AOT's JIT fallback).
    pub sends: Option<&'static str>,
}

const fn offer(mode: CpuMode, available: bool, sends: Option<&'static str>) -> Offer {
    Offer { mode, available, sends }
}

/// The PowerPC engines (cpu-ppc.library: interpreter, blocks, the JIT).
pub const PPC: [Offer; 4] = [
    offer(CpuMode::Auto, true, None),
    offer(CpuMode::Jit, true, Some("jit")),
    offer(CpuMode::Aot, false, Some("jit")),
    offer(CpuMode::Compatible, true, Some("interp")),
];

/// An engine whose CPU has one execution tier, an interpreter (cpu-68k.library, cpu-arm.library today).
pub const ONE_TIER: [Offer; 4] = [
    offer(CpuMode::Auto, true, None),
    offer(CpuMode::Jit, false, None),
    offer(CpuMode::Aot, false, None),
    offer(CpuMode::Compatible, true, None),
];

/// The modes the engine of `family` (a platform token: `macppc`, `mac68k`, `ami68k`, `ekaarm`, `amippc`) offers; None for a
/// family this table does not know.
pub fn offers(family: &str) -> Option<&'static [Offer]> {
    match family {
        "macppc" | "amippc" => Some(&PPC),
        "mac68k" | "ami68k" | "ekaarm" => Some(&ONE_TIER),
        _ => None,
    }
}

/// Whether the engine of `family` takes a [`CPU_TIER_INPUT`] at all (a family with one tier refuses it).
pub fn takes_tier_input(family: &str) -> bool {
    offers(family).is_some_and(|o| o.iter().any(|m| m.sends.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_offers_every_mode_once_in_menu_order_and_automatic_always() {
        for f in ["macppc", "amippc", "mac68k", "ami68k", "ekaarm"] {
            let o = offers(f).unwrap();
            assert_eq!(o.iter().map(|m| m.mode).collect::<Vec<_>>(), CpuMode::ALL, "{f}");
            assert!(o[0].available && o[0].sends.is_none(), "{f}: Automatic is the engine's own choice");
            assert!(!o[2].available, "{f}: AOT does not exist yet");
        }
        assert_eq!(offers("x86"), None);
        assert!(takes_tier_input("macppc") && takes_tier_input("amippc"));
        assert!(!takes_tier_input("ami68k") && !takes_tier_input("ekaarm") && !takes_tier_input("mac68k"));
        for m in CpuMode::ALL {
            assert_eq!(CpuMode::from_token(m.token()), Some(m));
        }
        assert_eq!(CpuMode::from_token("blocks"), None, "an engine tier is not a mode");
    }
}
