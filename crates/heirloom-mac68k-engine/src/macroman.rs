//! Mac OS Roman <-> Unicode: the ONE table every Mac text path of the engines uses (native controls, menus, dialog and alert
//! text, QuickDraw text runs recorded for the host, the Text Encoding Converter, CoreFoundation strings, Unicode file names).
//!
//! SOURCE: the Unicode Consortium's published mapping of Apple's Mac OS Roman, `MAPPINGS/VENDORS/APPLE/ROMAN.TXT`
//! (<https://www.unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT>, version c02 2005-04-05, "Matches internal xml <c1.1> and
//! Text Encoding Converter 2.0"; sha256 18e571645be895e9553ed5c842ea8f65f9c5d3c9ccb43e66e0c33a132ed0d721). [`HIGH`] is its 0x80..0xFF
//! column, generated from the file (0x20..0x7E map to themselves there, checked when the table was generated). The file leaves the
//! control characters out "following the conventions of the standard UTC mapping tables" and says Mac OS Roman "uses the standard
//! control characters at 0x00-0x1F and 0x7F": they map to themselves here.
//! * 0xDB is EURO SIGN (U+20AC): the Mac OS 8.5-and-later encoding, which is this machine's (Mac OS 9.2.1). The older CURRENCY SIGN
//!   reading (U+00A4) is the "currency sign" variant, [`to_char_variant`] with `euro = false`.
//! * 0xBD is GREEK CAPITAL LETTER OMEGA (U+03A9), not OHM SIGN (the file's n04 change); 0xF0 is the Apple logo, U+F8FF (corporate zone).
//! * Unicode -> Mac Roman is the exact inverse (the table is one-to-one); a character with no Mac Roman byte is `None`, never guessed.
//!   Apple's "loose" mappings (e.g. OHM SIGN -> 0xBD) are not applied.
use alloc::string::String;
use alloc::vec::Vec;

/// Mac OS Roman 0x80..=0xFF as UTF-16 code units (ROMAN.TXT, see the module doc).
pub const HIGH: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, // 0x80
    0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5, 0x00E7, 0x00E9, 0x00E8, // 0x88
    0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, // 0x90
    0x00F2, 0x00F4, 0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, // 0x98
    0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6, 0x00DF, // 0xA0
    0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, // 0xA8
    0x221E, 0x00B1, 0x2264, 0x2265, 0x00A5, 0x00B5, 0x2202, 0x2211, // 0xB0
    0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, // 0xB8
    0x00BF, 0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, // 0xC0
    0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5, 0x0152, 0x0153, // 0xC8
    0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, // 0xD0
    0x00FF, 0x0178, 0x2044, 0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, // 0xD8
    0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1, // 0xE0
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4, // 0xE8
    0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9, 0x0131, 0x02C6, 0x02DC, // 0xF0
    0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7, // 0xF8
];

/// 0xDB in the "currency sign" variant (Mac OS before 8.5): CURRENCY SIGN.
const CURRENCY_SIGN: char = '\u{A4}';

/// The character of Mac Roman byte `b` (this machine's euro-sign encoding).
pub fn to_char(b: u8) -> char { to_char_variant(b, true) }

/// The character of Mac Roman byte `b`; `euro = false` is the pre-8.5 currency-sign variant (0xDB = U+00A4).
pub fn to_char_variant(b: u8, euro: bool) -> char {
    if b < 0x80 { return char::from(b); }
    if b == 0xDB && !euro { return CURRENCY_SIGN; }
    // every entry of HIGH is a BMP scalar value (no surrogate), checked by the tests
    char::from_u32(u32::from(HIGH[usize::from(b - 0x80)])).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// The Mac Roman byte of `c`, if Mac Roman has one (euro-sign encoding).
pub fn from_char(c: char) -> Option<u8> { from_char_variant(c, true) }

/// The Mac Roman byte of `c` in the euro-sign (`true`) or currency-sign (`false`) variant.
pub fn from_char_variant(c: char, euro: bool) -> Option<u8> {
    let u = u32::from(c);
    if u < 0x80 { return Some(u as u8); }
    if !euro {
        if c == CURRENCY_SIGN { return Some(0xDB); }
        if u == 0x20AC { return None; }
    }
    let u = u16::try_from(u).ok()?;
    HIGH.iter().position(|&h| h == u).map(|i| 0x80 + i as u8)
}

/// Mac Roman text as a `String` (every byte has a character).
pub fn decode(bytes: &[u8]) -> String { bytes.iter().map(|&b| to_char(b)).collect() }

/// `s` as Mac Roman bytes; a character Mac Roman lacks becomes `?` (for host text going INTO a guest field, where the guest cannot be
/// told about a failure; the TEC paths use [`from_char`] and report unmappable characters).
pub fn encode_lossy(s: &str) -> Vec<u8> { s.chars().map(|c| from_char(c).unwrap_or(b'?')).collect() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_round_trips_and_the_table_is_one_to_one() {
        for b in 0..=255u8 {
            let c = to_char(b);
            assert_ne!(c, char::REPLACEMENT_CHARACTER, "{b:#04x}");
            assert_eq!(from_char(c), Some(b), "{b:#04x} -> {c:?}");
        }
    }

    #[test]
    fn the_rows_programs_show_map_by_roman_txt() {
        // "Read Me..." (0xC9 HORIZONTAL ELLIPSIS), "Español" (0x96), curly quotes, the euro, the Apple logo
        assert_eq!(decode(b"Read Me\xC9"), "Read Me\u{2026}");
        assert_eq!(decode(b"Espa\x96ol"), "Espa\u{F1}ol");
        assert_eq!(decode(b"\xD2q\xD3 \xD4s\xD5"), "\u{201C}q\u{201D} \u{2018}s\u{2019}");
        assert_eq!(to_char(0xDB), '\u{20AC}');
        assert_eq!(to_char(0xBD), '\u{3A9}');
        assert_eq!(to_char(0xF0), '\u{F8FF}');
        assert_eq!(to_char(0x0D), '\r');
        assert_eq!(encode_lossy("Read Me\u{2026}"), b"Read Me\xC9");
    }

    #[test]
    fn unmappable_characters_are_none_not_guessed() {
        // NEGATIVE: Latin-1 reading would give 0xE9 for e-acute (Mac Roman 0x8E); OHM SIGN is a loose mapping only; CJK has no byte
        assert_eq!(from_char('\u{E9}'), Some(0x8E));
        assert_eq!(from_char('\u{2126}'), None);
        assert_eq!(from_char('\u{65E5}'), None);
        assert_eq!(encode_lossy("\u{65E5}"), b"?");
        // the currency-sign variant: 0xDB is U+00A4 and the euro has no byte there
        assert_eq!(to_char_variant(0xDB, false), '\u{A4}');
        assert_eq!(from_char_variant('\u{A4}', false), Some(0xDB));
        assert_eq!(from_char_variant('\u{20AC}', false), None);
        assert_eq!(from_char('\u{A4}'), None);
    }
}
