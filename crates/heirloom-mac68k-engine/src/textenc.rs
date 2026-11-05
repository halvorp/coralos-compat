//! The Text Encoding Converter's model, shared by both Mac families: the `TextEncoding` word (TextCommon), the encodings this
//! machine converts, and conversion between them through [`crate::macroman`] (the one Mac Roman table). Pure: no guest memory.
//!
//! Facts (Apple's TextCommon.h / UnicodeConverter.h / TextEncodingConverter.h / MacErrors.h, public interface headers):
//! * a `TextEncoding` is base (bits 0-15) | variant (bits 16-25) | format (bits 26-31). Bases: kTextEncodingMacRoman 0 .. the Mac
//!   script codes (0-32 = the Script Manager's script codes), kTextEncodingMacSymbol 33, MacDingbats 34, MacTurkish 35, MacCroatian 36,
//!   MacIcelandic 37, MacRomanian 38; kTextEncodingUnicodeDefault 0x100 (a meta-value), UnicodeV2_1 0x103, UnicodeV3_0 0x104;
//!   kTextEncodingISOLatin1 0x201; kTextEncodingUS_ASCII 0x600. Unicode formats: kUnicode16BitFormat 0 (UTF-16), kUnicodeUTF8Format 2.
//!   Mac Roman variants: kMacRomanDefaultVariant 0 ("maps to 1 or 2 depending on System"), CurrencySign 1, EuroSign 2.
//! * Mac OS 9.2.1's own TextCommon `ResolveDefaultTextEncoding` (read from the 9.2.1 System's TextCommon fragment, facts only):
//!   base 0x100 becomes 0x104 (Unicode 3.0); a default (0) variant of MacRoman / MacCroatian / MacRomanian / VT100 becomes 1, doubled to
//!   2 on a System 8.5 or later; MacIcelandic's becomes 2, doubled to 4; MacCyrillic / MacUkrainian's becomes MacCyrillic variant 3 on
//!   8.7 and later; the format's low four bits are kept. [`resolve_default`].
//!
//! What converts here ([`Charset`]): Mac Roman (both variants), UTF-16 (big-endian: the guest's native order) and UTF-8 of every
//! Unicode version base, ISO Latin-1, US-ASCII. Every other encoding is NAMED as not provided by the callers (kTECNoConversionPathErr),
//! never converted through a guessed table.
use alloc::vec::Vec;

pub type TextEncoding = u32;

pub const MAC_ROMAN: u32 = 0;
pub const MAC_SYMBOL: u32 = 33;
pub const MAC_DINGBATS: u32 = 34;
pub const MAC_TURKISH: u32 = 35;
pub const MAC_CROATIAN: u32 = 36;
pub const MAC_ICELANDIC: u32 = 37;
pub const MAC_ROMANIAN: u32 = 38;
pub const MAC_CYRILLIC: u32 = 7;
pub const MAC_UKRAINIAN: u32 = 0x98;
pub const MAC_VT100: u32 = 0xFC;
pub const UNICODE_DEFAULT: u32 = 0x100;
pub const UNICODE_V2_1: u32 = 0x103;
pub const UNICODE_V3_0: u32 = 0x104;
pub const ISO_LATIN1: u32 = 0x201;
pub const US_ASCII: u32 = 0x600;
pub const FORMAT_UTF16: u32 = 0;
pub const FORMAT_UTF8: u32 = 2;
pub const MAC_ROMAN_CURRENCY_SIGN: u32 = 1;
pub const MAC_ROMAN_EURO_SIGN: u32 = 2;

// MacErrors.h
pub const UNSUPPORTED_ENCODING: i16 = -8738; // kTextUnsupportedEncodingErr
pub const MALFORMED_INPUT: i16 = -8739; // kTextMalformedInputErr
pub const CORRUPT_CONVERTER: i16 = -8748; // kTECCorruptConverterErr
pub const NO_CONVERSION_PATH: i16 = -8749; // kTECNoConversionPathErr
pub const ARRAY_FULL: i16 = -8751; // kTECArrayFullErr
pub const PARTIAL_CHAR: i16 = -8753; // kTECPartialCharErr
pub const UNMAPPABLE: i16 = -8754; // kTECUnmappableElementErr
pub const USED_FALLBACKS: i16 = -8783; // kTECUsedFallbacksStatus
pub const OUTPUT_BUFFER_FULL: i16 = -8785; // kTECOutputBufferFullStatus

pub fn base(e: TextEncoding) -> u32 { e & 0xFFFF }
pub fn variant(e: TextEncoding) -> u32 { (e >> 16) & 0x3FF }
pub fn format(e: TextEncoding) -> u32 { (e >> 26) & 0x3F }
/// `CreateTextEncoding(base, variant, format)`: each part masked to its field.
pub fn create(base: u32, variant: u32, format: u32) -> TextEncoding { (base & 0xFFFF) | ((variant & 0x3FF) << 16) | ((format & 0x3F) << 26) }

/// This machine's System version as `ResolveDefaultTextEncoding` reads it (Gestalt 'sysv'): Mac OS 9.2.1.
pub const SYSTEM_VERSION: u32 = 0x0921;

/// `ResolveDefaultTextEncoding` (see the module doc): the meta-values replaced by what they mean on `system` (a 'sysv' word).
pub fn resolve_default(e: TextEncoding, system: u32) -> TextEncoding {
    let mut b = base(e);
    if b == UNICODE_DEFAULT { b = UNICODE_V3_0; }
    let mut v = variant(e);
    let step = match b {
        MAC_ROMAN | MAC_VT100 | MAC_CROATIAN | MAC_ROMANIAN => 1,
        MAC_ICELANDIC => 2,
        MAC_CYRILLIC | MAC_UKRAINIAN if v == 0 => { v = if system >= 0x870 { 3 } else if b == MAC_CYRILLIC { 1 } else { 2 }; b = MAC_CYRILLIC; 0 }
        _ => 0,
    };
    if step != 0 && v < step { v += if system >= 0x850 { step * 2 } else { step }; }
    (b & 0xFFFF) | ((v & 0x3FF) << 16) | (((e >> 26) & 0xF) << 26)
}

/// The encodings `TECGetAvailableTextEncodings` lists: the ones [`charset`] converts, each once, in their resolved form.
pub const AVAILABLE: [TextEncoding; 6] = [
    MAC_ROMAN | (MAC_ROMAN_EURO_SIGN << 16),
    MAC_ROMAN | (MAC_ROMAN_CURRENCY_SIGN << 16),
    UNICODE_V3_0 | (FORMAT_UTF16 << 26),
    UNICODE_V3_0 | (FORMAT_UTF8 << 26),
    ISO_LATIN1,
    US_ASCII,
];

/// A byte form this machine converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// Mac OS Roman; `euro` = 0xDB is EURO SIGN (variant 2, and the default on 8.5+), else CURRENCY SIGN (variant 1).
    MacRoman { euro: bool },
    /// UTF-16, big-endian (the PowerPC's and 68k's native order; a leading BOM is read and dropped).
    Utf16,
    Utf8,
    Latin1,
    Ascii,
}

/// What `e` is as a [`Charset`], or `None` when this machine has no conversion for it.
pub fn charset(e: TextEncoding) -> Option<Charset> {
    let e = resolve_default(e, SYSTEM_VERSION);
    match base(e) {
        MAC_ROMAN => match variant(e) { MAC_ROMAN_CURRENCY_SIGN => Some(Charset::MacRoman { euro: false }), MAC_ROMAN_EURO_SIGN => Some(Charset::MacRoman { euro: true }), _ => None },
        0x101..=0x1FF => match format(e) { FORMAT_UTF16 => Some(Charset::Utf16), FORMAT_UTF8 => Some(Charset::Utf8), _ => None },
        ISO_LATIN1 if variant(e) == 0 => Some(Charset::Latin1),
        US_ASCII if variant(e) == 0 => Some(Charset::Ascii),
        _ => None,
    }
}

/// One character read from the input: the character and the bytes it took.
enum Read { Char(char, usize), Partial, Malformed }

fn read_one(cs: Charset, b: &[u8]) -> Read {
    match cs {
        Charset::MacRoman { euro } => Read::Char(crate::macroman::to_char_variant(b[0], euro), 1),
        Charset::Latin1 => Read::Char(char::from(b[0]), 1),
        Charset::Ascii => if b[0] < 0x80 { Read::Char(char::from(b[0]), 1) } else { Read::Malformed },
        Charset::Utf16 => {
            if b.len() < 2 { return Read::Partial; }
            let u = u16::from_be_bytes([b[0], b[1]]);
            match u {
                0xD800..=0xDBFF => {
                    if b.len() < 4 { return Read::Partial; }
                    let l = u16::from_be_bytes([b[2], b[3]]);
                    if !(0xDC00..=0xDFFF).contains(&l) { return Read::Malformed; }
                    let c = 0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(l) - 0xDC00);
                    char::from_u32(c).map_or(Read::Malformed, |c| Read::Char(c, 4))
                }
                0xDC00..=0xDFFF => Read::Malformed,
                _ => char::from_u32(u32::from(u)).map_or(Read::Malformed, |c| Read::Char(c, 2)),
            }
        }
        Charset::Utf8 => {
            let n = match b[0] { 0x00..=0x7F => 1, 0xC2..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF4 => 4, _ => return Read::Malformed };
            if b.len() < n {
                return if b[1..].iter().all(|&x| x & 0xC0 == 0x80) { Read::Partial } else { Read::Malformed };
            }
            match core::str::from_utf8(&b[..n]) { Ok(s) => Read::Char(s.chars().next().unwrap_or('\0'), n), Err(_) => Read::Malformed }
        }
    }
}

/// `c` in `cs`, appended to `out`; false when `cs` has no form for it.
fn write_one(cs: Charset, c: char, out: &mut Vec<u8>) -> bool {
    match cs {
        Charset::MacRoman { euro } => crate::macroman::from_char_variant(c, euro).map(|b| out.push(b)).is_some(),
        Charset::Latin1 => u8::try_from(u32::from(c)).map(|b| out.push(b)).is_ok(),
        Charset::Ascii => (u32::from(c) < 0x80).then(|| out.push(c as u8)).is_some(),
        Charset::Utf16 => { let mut u = [0u16; 2]; for w in c.encode_utf16(&mut u) { out.extend_from_slice(&w.to_be_bytes()); } true }
        Charset::Utf8 => { let mut u = [0u8; 4]; out.extend_from_slice(c.encode_utf8(&mut u).as_bytes()); true }
    }
}

/// The result of [`convert`]: bytes read and written, the status (0, or one of the TEC status / error codes), and for each character
/// read the (input offset, output offset) of its start (what the Unicode Converter's offset arrays are built from).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted { pub read: usize, pub out: Vec<u8>, pub status: i16, pub offsets: Vec<(usize, usize)> }

/// Convert `input` from `from` to `to`, writing at most `cap` bytes. Stops at: an output that would pass `cap` (OUTPUT_BUFFER_FULL,
/// nothing of that character written); a character `to` cannot hold (UNMAPPABLE, `read` = its offset) unless `fallback` is given
/// (that byte sequence is written instead and the status is USED_FALLBACKS); malformed input (MALFORMED_INPUT); input that ends
/// inside a character (PARTIAL_CHAR, `read` excludes it). A leading UTF-16 byte-order mark is read and not written.
pub fn convert(from: Charset, to: Charset, input: &[u8], cap: usize, fallback: Option<&[u8]>) -> Converted {
    let mut c = Converted { read: 0, out: Vec::new(), status: 0, offsets: Vec::new() };
    if from == Charset::Utf16 && input.len() >= 2 && input[..2] == [0xFE, 0xFF] { c.read = 2; }
    let mut used_fallback = false;
    let mut buf = Vec::with_capacity(4);
    while c.read < input.len() {
        let (ch, n) = match read_one(from, &input[c.read..]) {
            Read::Char(ch, n) => (ch, n),
            Read::Partial => { c.status = PARTIAL_CHAR; return c; }
            Read::Malformed => { c.status = MALFORMED_INPUT; return c; }
        };
        buf.clear();
        if !write_one(to, ch, &mut buf) {
            match fallback { Some(f) => { buf.extend_from_slice(f); used_fallback = true; } None => { c.status = UNMAPPABLE; return c; } }
        }
        if c.out.len() + buf.len() > cap { c.status = OUTPUT_BUFFER_FULL; return c; }
        c.offsets.push((c.read, c.out.len()));
        c.out.extend_from_slice(&buf);
        c.read += n;
    }
    if used_fallback { c.status = USED_FALLBACKS; }
    c
}

/// `TECSniffTextEncoding`'s measure of `input` read as `cs`: (errors, features). Errors are bytes that cannot be read as `cs`
/// (each malformed position counts once and the scan moves on one byte); features are characters that only `cs` explains:
/// a multi-byte UTF-8 sequence. Nothing else counts (every UTF-16 unit is two bytes, and a byte above 0x7F of a single-byte
/// encoding is equally plausible in all of them).
pub fn sniff(cs: Charset, input: &[u8]) -> (u32, u32) {
    let (mut errs, mut feats, mut at) = (0u32, 0u32, 0usize);
    while at < input.len() {
        match read_one(cs, &input[at..]) {
            Read::Char(_, n) => { if n > 1 && cs == Charset::Utf8 { feats += 1; } at += n; }
            Read::Partial => { errs += 1; break; }
            Read::Malformed => { errs += 1; at += 1; }
        }
    }
    if cs == Charset::Utf16 && input.len() % 2 == 1 { errs += 1; }
    (errs, feats)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ROMAN: Charset = Charset::MacRoman { euro: true };

    #[test]
    fn encoding_words_and_resolution_follow_textcommon() {
        let e = create(UNICODE_DEFAULT, 0, FORMAT_UTF8);
        assert_eq!((base(e), variant(e), format(e)), (0x100, 0, 2));
        assert_eq!(create(0x1_0000, 0x400, 0x40), 0); // each part masked to its field
        assert_eq!(resolve_default(e, SYSTEM_VERSION), create(UNICODE_V3_0, 0, FORMAT_UTF8));
        assert_eq!(resolve_default(MAC_ROMAN, SYSTEM_VERSION), create(MAC_ROMAN, 2, 0));
        assert_eq!(resolve_default(MAC_ROMAN, 0x0810), create(MAC_ROMAN, 1, 0)); // before 8.5: currency sign
        assert_eq!(resolve_default(MAC_ICELANDIC, SYSTEM_VERSION), create(MAC_ICELANDIC, 4, 0));
        assert_eq!(resolve_default(MAC_UKRAINIAN, SYSTEM_VERSION), create(MAC_CYRILLIC, 3, 0));
        assert_eq!(resolve_default(create(MAC_ROMAN, 1, 0), SYSTEM_VERSION), create(MAC_ROMAN, 1, 0)); // an explicit variant stays
        assert_eq!(charset(MAC_ROMAN), Some(ROMAN));
        assert_eq!(charset(create(MAC_ROMAN, 1, 0)), Some(Charset::MacRoman { euro: false }));
        assert_eq!(charset(UNICODE_DEFAULT), Some(Charset::Utf16));
        // NEGATIVE: encodings with no table here are None, never guessed
        assert_eq!(charset(1), None); // MacJapanese
        assert_eq!(charset(0x0500), None); // Windows Latin-1
        assert_eq!(charset(create(UNICODE_V3_0, 0, 1)), None); // UTF-7
    }

    #[test]
    fn mac_roman_and_unicode_convert_both_ways_with_offsets() {
        let c = convert(ROMAN, Charset::Utf16, b"Caf\x8E \xDB", 100, None);
        assert_eq!(c.status, 0);
        assert_eq!(c.out, [0, b'C', 0, b'a', 0, b'f', 0, 0xE9, 0, b' ', 0x20, 0xAC]);
        assert_eq!(c.offsets[3], (3, 6));
        let back = convert(Charset::Utf16, ROMAN, &c.out, 100, None);
        assert_eq!((back.status, back.out.as_slice(), back.read), (0, &b"Caf\x8E \xDB"[..], 12));
        let u8s = convert(ROMAN, Charset::Utf8, b"\xC9", 10, None);
        assert_eq!(u8s.out, "\u{2026}".as_bytes());
        // a supplementary character as a surrogate pair, and the BOM dropped
        let s = convert(Charset::Utf8, Charset::Utf16, "\u{1F600}".as_bytes(), 10, None);
        assert_eq!(s.out, [0xD8, 0x3D, 0xDE, 0x00]);
        assert_eq!(convert(Charset::Utf16, Charset::Utf8, &[0xFE, 0xFF, 0, b'A'], 10, None).out, b"A");
    }

    #[test]
    fn stops_are_named_statuses() {
        // NEGATIVE: unmappable without a fallback stops AT the character; with one it is replaced and reported
        let u = convert(Charset::Utf16, ROMAN, &[0, b'A', 0x65, 0xE5, 0, b'B'], 10, None);
        assert_eq!((u.status, u.read, u.out.as_slice()), (UNMAPPABLE, 2, &b"A"[..]));
        let f = convert(Charset::Utf16, ROMAN, &[0, b'A', 0x65, 0xE5, 0, b'B'], 10, Some(b"?"));
        assert_eq!((f.status, f.read, f.out.as_slice()), (USED_FALLBACKS, 6, &b"A?B"[..]));
        // output full: whole characters only
        let o = convert(ROMAN, Charset::Utf16, b"abc", 5, None);
        assert_eq!((o.status, o.read, o.out.len()), (OUTPUT_BUFFER_FULL, 2, 4));
        // partial and malformed input
        assert_eq!(convert(Charset::Utf8, ROMAN, b"a\xE2\x80", 10, None).status, PARTIAL_CHAR);
        assert_eq!(convert(Charset::Utf8, ROMAN, b"a\xFF", 10, None).status, MALFORMED_INPUT);
        assert_eq!(convert(Charset::Utf16, ROMAN, &[0xDC, 0x00], 10, None).status, MALFORMED_INPUT);
        assert_eq!(convert(Charset::Ascii, ROMAN, b"\x80", 10, None).status, MALFORMED_INPUT);
    }

    #[test]
    fn sniffing_counts_errors_and_features() {
        assert_eq!(sniff(Charset::Utf8, "na\u{EF}ve".as_bytes()), (0, 1));
        assert_eq!(sniff(Charset::Ascii, "na\u{EF}ve".as_bytes()), (2, 0));
        assert_eq!(sniff(ROMAN, b"na\x95ve"), (0, 0));
        assert_eq!(sniff(Charset::Utf8, b"na\x95ve"), (1, 0));
    }
}
