//! The PowerPC Gestalt machine profile. The shared Gestalt (`mac68k_engine::gestalt`) describes a 68k Macintosh II; a PowerPC program asks the
//! same selectors and must hear the PowerPC machine's answers, and ONLY true ones: a selector whose feature this engine does not provide is
//! `gestaltUndefSelectorErr` (-5551) with the selector and the missing piece NAMED in the log, never an invented value. 'sysv' (0x0921) and 'cbon' (0x0140) name the Mac OS 9.2.1 / CarbonLib 1.4 personality; what the engine lacks of it is refused per call, by name.
//! An entry moves from [`UNANSWERED`] to [`answer`] in the same change that lands its feature (the Thread Manager's `'thds'` with the Thread
//! Manager, ...); `docs/consults/2026-10-24-macppc-full-app-gap-map.md` lists what each one waits for.
//!
//! `'ram '`/`'lram'` (physical/logical RAM) are the machine's actual guest RAM size, answered in osroutines.rs (the shared profile says 16 MB).
//! Values: `gestaltSysArchitecture` ('sysa') 2 = PowerPC and `gestaltNativeCPUtype` ('cput') `gestaltCPU750` = 0x108 (executor src/gestalt.cpp:139-147
//! answers both for a PowerPC build; the interpreter runs the 601/603/G3 user set, a 750). `gestaltCFMAttr` ('cfrg') bit 0 = `gestaltCFMPresent`: the
//! Code Fragment Manager calls (`GetSharedLibrary`, `FindSymbol`, `CloseConnection`) are native here. `'altv'` 0 = no AltiVec (retro-ppc has none).

/// The answer for `selector`, if the PowerPC profile gives one.
pub fn answer(selector: [u8; 4]) -> Option<u32> {
    Some(match &selector {
        b"sysa" => 2,
        b"cput" => 0x108,
        b"cfrg" => 1,
        b"altv" => 0,
        // RULING (coordinator, 2026-10-24): the engine IS a Mac OS 9.2.1 personality with CarbonLib 1.4 (Peter's own system, confirmed by the oracle). A version selector names the
        // personality; every call the engine lacks is a per-call NAMED refusal. `gestaltSystemVersion` 0x0921 and `gestaltCarbonVersion` 0x0140 (CarbonLib 1.4, as on the 9.2.1 CD).
        b"sysv" => 0x0921,
        b"cbon" => 0x0140,
        // gestaltThreadMgrAttr: bit 0 gestaltThreadMgrPresent, bit 2 gestaltThreadsLibraryPresent (ThreadsLib is provided); bit 1 (exact-match support) is not claimed
        b"thds" => 0b101,
        _ => return None,
    })
}

/// Selectors the shared (68k) profile would answer WRONGLY for a PowerPC program, or that wait for a feature, with the reason each has no answer.
pub const UNANSWERED: &[(&[u8; 4], &str)] = &[
    (b"appr", "the Appearance Manager is not provided"),
    (b"nav ", "Navigation Services is not provided"),
    (b"mpsv", "Multiprocessing Services runs cooperatively (no preemption, one processor, see mp.rs); the selector's value carries the MP Services version and attribute bits, which are not claimed"),
    (b"mach", "no PowerPC machine model is claimed (the shared value names a 68k Macintosh II)"),
    (b"proc", "'proc' is the 68k processor family; the PowerPC question is 'cput'"),
    (b"fpu ", "'fpu ' describes a 68k coprocessor; the PowerPC FPU is part of the processor"),
];

/// The reason `selector` is unanswered for a PowerPC program, if it is one of [`UNANSWERED`].
pub fn unanswered(selector: [u8; 4]) -> Option<&'static str> { UNANSWERED.iter().find(|(s, _)| **s == selector).map(|&(_, why)| why) }
