//! The Text Encoding Converter natives (TextCommon, TextEncodingConverter, UnicodeConverter) from PowerPC, each against its contract with negative
//! controls, and the ONE Mac Roman table reaching every other text path: menus, CoreFoundation strings, Unicode file names.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const TC: &str = "TextCommon";
const TEC: &str = "TextEncodingConverter";
const UC: &str = "UnicodeConverter";
const IN: u32 = 0x3000;
const OUT: u32 = 0x3400;
const R: u32 = 0x3800; // result words
const OFFS_IN: u32 = 0x3900;
const OFFS_OUT: u32 = 0x3A00;
const MAP: u32 = 0x3B00;
const SP: u32 = 0x7000;

const NO_PATH: i32 = -8749;
const CORRUPT: i32 = -8748;
const ARRAY_FULL: i32 = -8751;
const UNMAPPABLE: i32 = -8754;
const USED_FALLBACKS: i32 = -8783;
const OUTPUT_FULL: i32 = -8785;
const UTF8: u32 = 0x0800_0100; // CreateTextEncoding(kTextEncodingUnicodeDefault, 0, kUnicodeUTF8Format)

fn machine() -> M { MacPpc::new(RecordingGuestUi::new()).unwrap() }

/// Call `lib`'s `name` with `args` (the ninth and later in the stack parameter area, as the PowerPC ABI puts them).
fn call(m: &mut M, lib: &str, name: &str, args: &[u32]) -> i32 {
    let b = m.bind_from(lib, name);
    assert!(!matches!(b, Binding::Unbound(_)), "{lib}/{name} must be bound: {b:?}");
    let mut regs = [0u32; 32];
    regs[1] = SP;
    for (i, &a) in args.iter().enumerate() {
        if i < 8 { regs[3 + i] = a } else { m.mac.mem.write_u32(SP + 24 + 4 * i as u32, a).unwrap() }
    }
    let key = heirloom_macppc_engine::glue::binding_key(lib, name);
    assert_eq!(m.glue_call(&key, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn word(m: &M, at: u32) -> u32 { m.mac.mem.read_u32(at).unwrap() }
fn bytes(m: &M, at: u32, n: u32) -> Vec<u8> { (0..n).map(|i| m.mac.mem.read_u8(at + i).unwrap()).collect() }

#[test]
fn text_common_words_and_script_info() {
    let mut m = machine();
    assert_eq!(call(&mut m, TC, "CreateTextEncoding", &[0x100, 0, 2]) as u32, UTF8);
    assert_eq!(call(&mut m, TC, "GetTextEncodingBase", &[UTF8]), 0x100);
    // the meta-values resolve as 9.2.1's TextCommon does: Unicode default -> 3.0, Mac Roman's default variant -> the euro sign (2)
    assert_eq!(call(&mut m, TC, "ResolveDefaultTextEncoding", &[UTF8]) as u32, 0x0800_0104);
    assert_eq!(call(&mut m, TC, "ResolveDefaultTextEncoding", &[0]) as u32, 0x0002_0000);
    // UpgradeScriptInfoToTextEncoding(smSystemScript, kTextLanguageDontCare, kTextRegionDontCare, NULL, &enc) -> Mac Roman
    m.mac.mem.write_u32(R, 0xDEAD).unwrap();
    assert_eq!(call(&mut m, TC, "UpgradeScriptInfoToTextEncoding", &[0xFFFF, 0xFF80, 0xFF80, 0, R]), 0);
    assert_eq!(word(&m, R), 0);
    // NEGATIVE: a script code outside the Script Manager's range is paramErr and writes nothing
    m.mac.mem.write_u32(R, 0xDEAD).unwrap();
    assert_eq!(call(&mut m, TC, "UpgradeScriptInfoToTextEncoding", &[40, 0xFF80, 0xFF80, 0, R]), -50);
    assert_eq!(word(&m, R), 0xDEAD);
}

#[test]
fn tec_lists_its_encodings_and_converts_mac_roman() {
    let mut m = machine();
    assert_eq!(call(&mut m, TEC, "TECCountAvailableTextEncodings", &[R]), 0);
    let n = word(&m, R);
    assert!(n >= 4, "{n}");
    assert_eq!(call(&mut m, TEC, "TECGetAvailableTextEncodings", &[OUT, n, R]), 0);
    assert_eq!(word(&m, R), n);
    assert!((0..n).any(|i| word(&m, OUT + 4 * i) & 0xFFFF == 0), "Mac Roman is listed");
    // NEGATIVE: an array too small is kTECArrayFullErr with what fitted
    assert_eq!(call(&mut m, TEC, "TECGetAvailableTextEncodings", &[OUT, 1, R]), ARRAY_FULL);
    assert_eq!(word(&m, R), 1);

    // Mac Roman "Read Me<C9>" -> UTF-8 "Read Me\u{2026}"
    assert_eq!(call(&mut m, TEC, "TECCreateConverter", &[R, 0, UTF8]), 0);
    let conv = word(&m, R);
    m.mac.mem.write_bytes(IN, b"Read Me\xC9").unwrap();
    assert_eq!(call(&mut m, TEC, "TECConvertText", &[conv, IN, 8, R, OUT, 64, R + 4]), 0);
    assert_eq!((word(&m, R), word(&m, R + 4)), (8, 10));
    assert_eq!(bytes(&m, OUT, 10), "Read Me\u{2026}".as_bytes());
    // output buffer full: whole characters only, the read count says where to go on
    assert_eq!(call(&mut m, TEC, "TECConvertText", &[conv, IN, 8, R, OUT, 9, R + 4]), OUTPUT_FULL);
    assert_eq!((word(&m, R), word(&m, R + 4)), (7, 7));
    assert_eq!(call(&mut m, TEC, "TECFlushText", &[conv, OUT, 64, R + 4]), 0);
    assert_eq!(word(&m, R + 4), 0);
    assert_eq!(call(&mut m, TEC, "TECDisposeConverter", &[conv]), 0);
    // NEGATIVES: a disposed converter is kTECCorruptConverterErr; an encoding with no table (MacJapanese) is kTECNoConversionPathErr, named
    assert_eq!(call(&mut m, TEC, "TECConvertText", &[conv, IN, 8, R, OUT, 64, R + 4]), CORRUPT);
    assert_eq!(call(&mut m, TEC, "TECCreateConverter", &[R, 1, UTF8]), NO_PATH);
    assert_eq!(word(&m, R), 0);
    assert!(m.mac.log.iter().any(|l| l.contains("[tec] TECCreateConverter(0x1") && l.contains("kTECNoConversionPathErr")));
}

#[test]
fn unicode_converter_both_directions_with_offsets_and_fallbacks() {
    let mut m = machine();
    // text -> Unicode: "Caf<8E>" (Mac Roman e-acute) as UTF-16, the offsets of characters 1 and 3 mapped
    assert_eq!(call(&mut m, UC, "CreateTextToUnicodeInfoByEncoding", &[0, R]), 0);
    let t2u = word(&m, R);
    m.mac.mem.write_bytes(IN, b"Caf\x8E").unwrap();
    for (i, o) in [1u32, 3].iter().enumerate() { m.mac.mem.write_u32(OFFS_IN + 4 * i as u32, *o).unwrap(); }
    // (info, srcLen, src, flags, iOffsetCount, iOffsets, *oOffsetCount, oOffsets, outBufLen, *srcRead, *uniLen, out)
    assert_eq!(call(&mut m, UC, "ConvertFromTextToUnicode", &[t2u, 4, IN, 0, 2, OFFS_IN, R, OFFS_OUT, 64, R + 4, R + 8, OUT]), 0);
    assert_eq!((word(&m, R), word(&m, R + 4), word(&m, R + 8)), (2, 4, 8));
    assert_eq!((word(&m, OFFS_OUT), word(&m, OFFS_OUT + 4)), (2, 6));
    assert_eq!(bytes(&m, OUT, 8), [0, b'C', 0, b'a', 0, b'f', 0, 0xE9], "U+00E9, not the Latin-1 reading of 0x8E");

    // Unicode -> text through a UnicodeMapping {unicodeEncoding, otherEncoding, mappingVersion}
    for (o, v) in [(0, 0x100), (4, 0), (8, u32::MAX)] { m.mac.mem.write_u32(MAP + o, v).unwrap(); }
    assert_eq!(call(&mut m, UC, "CreateUnicodeToTextInfo", &[MAP, R]), 0);
    let u2t = word(&m, R);
    m.mac.mem.write_bytes(IN, &[0x20, 0x26, 0x65, 0xE5, 0, b'!']).unwrap(); // U+2026, U+65E5 (no Mac Roman byte), '!'
    // NEGATIVE: without kUnicodeUseFallbacksMask the conversion stops AT the unmappable character
    assert_eq!(call(&mut m, UC, "ConvertFromUnicodeToText", &[u2t, 6, IN, 0, 0, 0, R, 0, 64, R + 4, R + 8, OUT]), UNMAPPABLE);
    assert_eq!((word(&m, R + 4), word(&m, R + 8), bytes(&m, OUT, 1)), (2, 1, vec![0xC9]));
    // with it: the character becomes '?' and the status says fallbacks were used
    assert_eq!(call(&mut m, UC, "ConvertFromUnicodeToText", &[u2t, 6, IN, 1, 0, 0, R, 0, 64, R + 4, R + 8, OUT]), USED_FALLBACKS);
    assert_eq!((word(&m, R + 4), bytes(&m, OUT, word(&m, R + 8))), (6, b"\xC9?!".to_vec()));
    // ConvertFromUnicodeToPString: a Str255
    m.mac.mem.write_bytes(IN, &[0, b'O', 0x20, 0xAC]).unwrap();
    assert_eq!(call(&mut m, UC, "ConvertFromUnicodeToPString", &[u2t, 4, IN, OUT]), 0);
    assert_eq!(bytes(&m, OUT, 3), [2, b'O', 0xDB], "the euro is Mac Roman 0xDB on Mac OS 8.5 and later");
    // NEGATIVES: an info used in the wrong direction is paramErr; Dispose clears the caller's reference and a second use is paramErr
    assert_eq!(call(&mut m, UC, "ConvertFromTextToUnicode", &[u2t, 4, IN, 0, 0, 0, R, 0, 64, R + 4, R + 8, OUT]), -50);
    m.mac.mem.write_u32(R + 12, t2u).unwrap();
    assert_eq!(call(&mut m, UC, "DisposeTextToUnicodeInfo", &[R + 12]), 0);
    assert_eq!(word(&m, R + 12), 0);
    assert_eq!(call(&mut m, UC, "ConvertFromTextToUnicode", &[t2u, 4, IN, 0, 0, 0, R, 0, 64, R + 4, R + 8, OUT]), -50);
    // NEGATIVE: a mapping whose "Unicode" side is not Unicode is kTextUnsupportedEncodingErr
    m.mac.mem.write_u32(MAP, 0x201).unwrap();
    assert_eq!(call(&mut m, UC, "CreateUnicodeToTextInfo", &[MAP, R]), -8738);
}

#[test]
fn the_sniffer_orders_encodings_by_errors_then_features() {
    let mut m = machine();
    let encs = [0x600u32, 0, UTF8]; // ASCII, Mac Roman, UTF-8
    for (i, e) in encs.iter().enumerate() { m.mac.mem.write_u32(IN + 4 * i as u32, *e).unwrap(); }
    assert_eq!(call(&mut m, TEC, "TECCreateSniffer", &[R, IN, 3]), 0);
    let sn = word(&m, R);
    m.mac.mem.write_bytes(OUT, "na\u{EF}ve".as_bytes()).unwrap();
    // (sniffer, in, inLen, encs[], n, errs[], maxErrs, features[], maxFeatures)
    assert_eq!(call(&mut m, TEC, "TECSniffTextEncoding", &[sn, OUT, 6, IN, 3, OFFS_IN, 100, OFFS_OUT, 100]), 0);
    let order: Vec<u32> = (0..3).map(|i| word(&m, IN + 4 * i)).collect();
    assert_eq!(order, [UTF8, 0, 0x600], "UTF-8 (no errors, one multi-byte character) first; ASCII (two bad bytes) last");
    assert_eq!((word(&m, OFFS_IN), word(&m, OFFS_IN + 8)), (0, 2));
    assert_eq!(call(&mut m, TEC, "TECDisposeSniffer", &[sn]), 0);
    // NEGATIVE: a sniffer for an encoding with no table is refused with kTECNoConversionPathErr
    m.mac.mem.write_u32(IN, 1).unwrap();
    assert_eq!(call(&mut m, TEC, "TECCreateSniffer", &[R, IN, 1]), NO_PATH);
}

#[test]
fn every_text_path_reads_mac_roman_through_the_one_table() {
    // a menu or control title: "Read Me<C9>" is an ellipsis, not Latin-1 'É'; "Espa<96>ol" is n-tilde
    assert_eq!(heirloom_mac68k_engine::menu::mac_text(b"Read Me\xC9"), "Read Me\u{2026}");
    assert_eq!(heirloom_mac68k_engine::menu::mac_text(b"Espa\x96ol"), "Espa\u{F1}ol");
    let mut m = machine();
    // a Pascal string read by any Toolbox call
    m.mac.mem.write_bytes(IN, b"\x04\xD2hi\xD3").unwrap();
    assert_eq!(heirloom_mac68k_engine::pascal::read_pstring(&m.mac.mem, IN).unwrap(), "\u{201C}hi\u{201D}");
    // CoreFoundation: CFStringCreateWithCharacters takes U+00E9 to Mac Roman 0x8E, CFStringGetCharacters gives it back, a UTF-8 C string comes in converted
    m.mac.mem.write_bytes(IN, &[0, b'C', 0x00, 0xE9]).unwrap();
    let s = call(&mut m, "CarbonLib", "CFStringCreateWithCharacters", &[0, IN, 2]) as u32;
    assert_eq!(call(&mut m, "CarbonLib", "CFStringGetCharacters", &[s, 0, 2, OUT]), 0);
    assert_eq!(bytes(&m, OUT, 4), [0, b'C', 0, 0xE9]);
    m.mac.mem.write_bytes(IN, "\u{2026}\0".as_bytes()).unwrap();
    let u = call(&mut m, "CarbonLib", "CFStringCreateWithCString", &[0, IN, UTF8]) as u32;
    assert_eq!(call(&mut m, "CarbonLib", "CFStringGetLength", &[u]), 1, "one Mac Roman character");
    assert_eq!(call(&mut m, "CarbonLib", "CFStringGetCString", &[u, OUT, 16, 0]), 1);
    assert_eq!(bytes(&m, OUT, 2), [0xC9, 0]);
    // NEGATIVE: Mac Roman text asked for as ASCII cannot be held: false, not mangled
    assert_eq!(call(&mut m, "CarbonLib", "CFStringGetCString", &[u, OUT, 16, 0x600]), 0);
    // NEGATIVE: a UniChar Mac Roman lacks is refused by name (the store is Mac Roman)
    m.mac.mem.write_bytes(IN, &[0x65, 0xE5]).unwrap();
    let mut regs = [0u32; 32];
    regs[3..6].copy_from_slice(&[0, IN, 1]);
    assert!(matches!(m.glue_call("CFStringCreateWithCharacters", &mut regs), GlueOutcome::Refused(r) if r.contains("U+65E5") || r.contains("0x65e5")));
}
