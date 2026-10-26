//! A-line trap words: what number a `$Axxx` opcode is, and what it is called.
//!
//! Layout (Inside Macintosh): bit 11 set = Toolbox trap, a 10-bit number in bits 0-9, bit 10 = "auto-pop" flag.
//! Bit 11 clear = OS trap, an 8-bit number in bits 0-7, bits 8-10 are register/clear flags. The table key drops the
//! flags so `_NewWindow` is found whichever flag bits a program sets. The NAMES are generated ([`crate::trap_names`]).
use crate::trap_names::TRAPS;

/// The table key of an A-line opcode: the trap with its flag bits cleared.
pub fn trap_key(opcode: u16) -> u16 {
    if opcode & 0x0800 != 0 { 0xA800 | (opcode & 0x03FF) } else { 0xA000 | (opcode & 0x00FF) }
}

/// True for a Toolbox trap ($A800-$A81E), false for an OS trap ($A000-$A7FF).
pub fn is_toolbox(opcode: u16) -> bool { opcode & 0x0800 != 0 }

/// The trap's name without the underscore: the exact word first (flag-bearing aliases are listed), else the key.
pub fn trap_name(opcode: u16) -> Option<&'static str> {
    let find = |w: u16| TRAPS.binary_search_by_key(&w, |&(k, _)| k).ok().map(|i| TRAPS[i].1);
    find(opcode).or_else(|| find(trap_key(opcode)))
}

/// "_NewWindow ($A913)", or "$A123 (unnamed)" for a word no table names.
pub fn describe(opcode: u16) -> alloc::string::String {
    use alloc::format;
    match trap_name(opcode) { Some(n) => format!("_{n} (${opcode:04X})"), None => format!("${opcode:04X} (unnamed)") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_trap_numbers() {
        for (w, n) in [(0xA913, "NewWindow"), (0xA914, "DisposeWindow"), (0xA970, "GetNextEvent"), (0xA860, "WaitNextEvent"),
                       (0xA9F4, "ExitToShell"), (0xA86E, "InitGraf")] {
            assert_eq!(trap_name(w), Some(n), "{w:#06x}");
        }
        assert_eq!(describe(0xA913), "_NewWindow ($A913)");
        assert_eq!(describe(0xA81E), "$A81E (unnamed)");
    }

    #[test]
    fn table_is_sorted_and_flags_are_dropped() {
        assert!(TRAPS.windows(2).all(|w| w[0].0 < w[1].0), "binary search needs ascending, unique words");
        assert_eq!(trap_key(0xAD13), 0xA913, "auto-pop bit ignored");
        assert_eq!(trap_key(0xA122), 0xA022, "OS flag bits ignored");
        assert!(is_toolbox(0xA913) && !is_toolbox(0xA022));
        assert_eq!(trap_name(0xA913 | 0x0400), Some("NewWindow"), "name found through the key");
    }
}
