//! The PowerPC Gestalt machine profile. The shared Gestalt (`mac68k_engine::gestalt`) describes a 68k Macintosh II; a PowerPC program asks the
//! same selectors and must hear the PowerPC machine's answers, and ONLY true ones: a selector whose feature this engine does not provide is
//! `gestaltUndefSelectorErr` (-5551) with the selector and the missing piece NAMED in the log, never an invented value. A Carbon application
//! that finds `'cbon'` unanswered takes its own "CarbonLib not present" path instead of running into calls that would refuse mid-session.
//! An entry moves from [`UNANSWERED`] to [`answer`] in the same change that lands its feature (the Thread Manager's `'thds'` with the Thread
//! Manager, ...); `docs/consults/2026-10-24-macppc-full-app-gap-map.md` lists what each one waits for.
//!
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
        // gestaltThreadMgrAttr: bit 0 gestaltThreadMgrPresent, bit 2 gestaltThreadsLibraryPresent (ThreadsLib is provided); bit 1 (exact-match support) is not claimed
        b"thds" => 0b101,
        _ => return None,
    })
}

/// Selectors the shared (68k) profile would answer WRONGLY for a PowerPC program, or that wait for a feature, with the reason each has no answer.
pub const UNANSWERED: &[(&[u8; 4], &str)] = &[
    (b"sysv", "reports the Mac OS version the engine implements; a Carbon game requires 9.1 and this engine does not yet provide CarbonLib's event model, Appearance, Navigation Services, Thread Manager or Multiprocessing Services, so no version above the shared profile's System 7.0 is claimed"),
    (b"cbon", "CarbonLib is bound to the Toolbox table by name only; there is no Carbon event loop or window model"),
    (b"appr", "the Appearance Manager is not provided"),
    (b"nav ", "Navigation Services is not provided"),
    (b"mpsv", "Multiprocessing Services runs cooperatively (no preemption, one processor, see mp.rs); the selector's value carries the MP Services version and attribute bits, which are not claimed"),
    (b"mach", "no PowerPC machine model is claimed (the shared value names a 68k Macintosh II)"),
    (b"proc", "'proc' is the 68k processor family; the PowerPC question is 'cput'"),
    (b"fpu ", "'fpu ' describes a 68k coprocessor; the PowerPC FPU is part of the processor"),
];

/// The reason `selector` is unanswered for a PowerPC program, if it is one of [`UNANSWERED`].
pub fn unanswered(selector: [u8; 4]) -> Option<&'static str> { UNANSWERED.iter().find(|(s, _)| **s == selector).map(|&(_, why)| why) }
