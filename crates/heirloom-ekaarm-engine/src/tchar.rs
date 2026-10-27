//! `TChar` (e32cmn.h:427): one member, `TUint iChar` (:1098). Only what a corpus app has called.
use alloc::format;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let this = p.cpu.regs[0];
    Some(match name {
        // TBool TChar::IsAlphaDigit() const (e32cmn.h:1072): a letter or a decimal digit. ASCII only: the other characters need the Unicode
        // category tables the device keeps, which this engine does not have, so they are refused rather than guessed.
        "_ZNK5TChar12IsAlphaDigitEv" => match p.rd(this) {
            Ok(c) if c < 0x80 => { p.cpu.regs[0] = u32::from((c as u8).is_ascii_alphanumeric()); NativeOutcome::Return }
            Ok(c) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: character {c:#x} is not ASCII (no Unicode category tables)")),
            Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}")),
        },
        _ => return None,
    })
}
