//! Host key -> Symbian key code. A host key event carries the key's `TStdScanCode` (e32keys.h:140+, the same layer as an Amiga raw key or a
//! Mac virtual key code) and the text it types; the window server's `EEventKey` carries a `TKeyCode` (e32keys.h:340+). A key that types text
//! is that text's first UTF-16 unit (its own character code); the other keys below are the ones whose code is not a character. A scan
//! code with neither gets only its key-down and key-up events; a text of more than one UTF-16 unit is refused, not cut.
use alloc::format;
use alloc::string::String;

/// (TStdScanCode, TKeyCode): e32keys.h:145-165 and :345-364 (the TKeyCode values of the non-character keys are
/// `ENonCharacterKeyBase` 0xf800 + their order, e32keys.h:17 and :354-364; checked by compiling the header).
const NON_TEXT_KEYS: &[(u32, u32)] = &[
    (0x01, 0x0008), // EStdKeyBackspace -> EKeyBackspace
    (0x02, 0x0009), // EStdKeyTab -> EKeyTab
    (0x03, 0x000d), // EStdKeyEnter -> EKeyEnter
    (0x04, 0x001b), // EStdKeyEscape -> EKeyEscape
    (0x08, 0xf802), // EStdKeyHome -> EKeyHome
    (0x09, 0xf803), // EStdKeyEnd -> EKeyEnd
    (0x0a, 0xf804), // EStdKeyPageUp -> EKeyPageUp
    (0x0b, 0xf805), // EStdKeyPageDown -> EKeyPageDown
    (0x0c, 0xf806), // EStdKeyInsert -> EKeyInsert
    (0x0d, 0x007f), // EStdKeyDelete -> EKeyDelete
    (0x0e, 0xf807), // EStdKeyLeftArrow -> EKeyLeftArrow
    (0x0f, 0xf808), // EStdKeyRightArrow -> EKeyRightArrow
    (0x10, 0xf809), // EStdKeyUpArrow -> EKeyUpArrow
    (0x11, 0xf80a), // EStdKeyDownArrow -> EKeyDownArrow
    (0xa7, 0xf845), // EStdKeyDevice3 -> EKeyDevice3 (the selection key)
];

/// The `TKeyCode` of the key press, or None when the key produces no `EEventKey` here.
pub fn key_code(scan: u32, text: &str) -> Result<Option<u32>, String> {
    let mut units = text.encode_utf16();
    match (units.next(), units.next()) {
        (Some(u), None) => return Ok(Some(u32::from(u))),
        (Some(_), Some(_)) => return Err(format!("a key event typing {text:?}: more than one UTF-16 unit is not one Symbian key code")),
        _ => {}
    }
    Ok(NON_TEXT_KEYS.iter().find(|(s, _)| *s == scan).map(|(_, c)| *c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_text_map_and_an_unmapped_key_has_no_code() {
        assert_eq!(key_code(0x11, ""), Ok(Some(0xf80a)));
        assert_eq!(key_code(0x10, ""), Ok(Some(0xf809)));
        assert_eq!(key_code(0x30, "a"), Ok(Some(0x61)));
        assert_eq!(key_code(0x12, ""), Ok(None), "Shift alone types nothing");
        assert!(key_code(0x30, "ab").is_err());
    }
}
