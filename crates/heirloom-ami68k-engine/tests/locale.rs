//! locale.library's built-in English locale, driven by a compiled 68k program (tests/guest/c/locale.c): the Locale struct,
//! strings, classes, StrnCmp, no catalogs, and FormatDate / FormatString / ParseDate through guest hooks.
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/locale.bin");

#[test]
fn the_default_locale_formats_and_parses_through_guest_hooks() {
    let mut a = machine();
    let r = run_refusing(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2], r[3], r[4]), (1, 1, 1, 1, 1), "OpenLocale(NULL): names, date formats, separators, currency of the US locale");
    assert_eq!((r[5], r[6], r[7]), (100, 0x5553_4100, 3), "MS_AMERICAN/CT_7SUN, country code 'USA', grouping of 3");
    assert_eq!((r[8], r[9]), (1, 0), "GetLocaleStr: the english strings; id 51 has none");
    assert_eq!((r[10], r[11]), (0, 1), "OpenCatalogA -> NULL, GetCatalogStr -> the caller's default string");
    assert_eq!((r[12], r[13], r[14], r[15], r[16]), (1, 0, 1, 0, 1), "IsAlpha/IsDigit/IsUpper, with their negative controls; locale NULL = default");
    assert_eq!((r[17], r[18], r[19]), ('A' as u32, 0xE9, 1), "ConvToUpper/ConvToLower (ISO 8859-1)");
    assert_eq!((r[20], r[21], r[22], r[23]), (1, 0, 0, 1), "StrnCmp: SC_ASCII is case-insensitive, the length limits it, 'abc' < 'abd'");
    assert_eq!((r[24], r[25]), (3, 1), "StrConvert");
    assert_eq!((r[26], r[27]), (1, 1), "FormatDate -> the hook got the text and the final NUL (\"1:05 pm\": %Q has no leading zero)");
    assert_eq!((r[28], r[29], r[30]), (1, 1, 1), "%c %j %w %U %y %m %%; midnight is 12 in the 12-hour formats; NULL template = one NUL");
    assert_eq!((r[31], r[32]), (1, 24), "FormatString: %s %ld %d %lx(upper case) widths, left, zero fill, %lD grouping; returns the end of the arguments");
    assert_eq!((r[33], r[34], r[35], r[36]), (1, 16860, 13 * 60 + 5, 0), "ParseDate \"02/29/24 01:05 pm\" -> 2024-02-29 13:05");
    assert_eq!((r[37], r[38]), (0, 0), "negative controls: February 30th, and a wrong separator, are FALSE");
    assert_eq!((r[39], r[40], r[41]), (1, 16860, 0), "ParseDate with day and month names");
    let refused: Vec<&str> = a.refused.iter().map(String::as_str).collect();
    assert_eq!(refused.len(), 4, "{refused:?}");
    assert!(refused[0].contains("CloseLocale") && refused[0].contains("not open"), "{refused:?}");
    assert!(refused[1].contains("GetLocaleStr") && refused[1].contains("0x1234 is not a struct Locale"), "{refused:?}");
    assert!(refused[2].contains("GetCatalogStr") && refused[2].contains("not a catalog"), "{refused:?}");
    assert!(refused[3].contains("OpenLocale") && refused[3].contains("preference files are not read"), "{refused:?}");
}
