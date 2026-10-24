//! CoreFoundation breadth by C name: strings from UniChars, mutation, substring, search, split/join, bytes out; arrays, dictionaries, numbers, file-system URLs, retain counts.
use guestui_abi::RecordingGuestUi;
use macppc_engine::symbols::Binding;
use macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const BUF: u32 = 0x3200;
const OUT: u32 = 0x3300;
const CHARS: u32 = 0x3400;

thread_local! { static R4: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind_from("CarbonLib", name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind_from("CarbonLib", name));
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    for (i, a) in args.iter().enumerate() { if i < 8 { regs[3 + i] = *a; } else { m.mac.mem.write_u32(0x6000 + 24 + 4 * i as u32, *a).unwrap(); } }
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    R4.with(|c| c.set(regs[4]));
    regs[3]
}
fn refused(m: &mut M, name: &str, args: &[u32]) -> String {
    m.bind_from("CarbonLib", name);
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    match m.glue_call(name, &mut regs) { GlueOutcome::Refused(w) => w, o => panic!("{name}: expected refusal, got {o:?}") }
}
fn cfstr(m: &mut M, s: &str) -> u32 { m.mac.mem.write_bytes(BUF, s.as_bytes()).unwrap(); m.mac.mem.write_u8(BUF + s.len() as u32, 0).unwrap(); call(m, "CFStringCreateWithCString", &[0, BUF, 0]) }
fn text(m: &M, r: u32) -> String { String::from_utf8(m.cf_bytes(r).unwrap().to_vec()).unwrap() }
fn unichars(m: &mut M, s: &str) { for (i, c) in s.chars().enumerate() { m.mac.mem.write_u16(CHARS + 2 * i as u32, c as u16).unwrap(); } }

#[test]
fn strings_from_unichars_mutation_substring_search_split_join_and_bytes() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    unichars(&mut m, "Hello");
    let s = call(&mut m, "CFStringCreateWithCharacters", &[0, CHARS, 5]);
    assert_eq!(text(&m, s), "Hello");
    let mu = call(&mut m, "CFStringCreateMutableCopy", &[0, 0, s]);
    unichars(&mut m, ", World");
    call(&mut m, "CFStringAppendCharacters", &[mu, CHARS, 7]);
    assert_eq!(text(&m, mu), "Hello, World");
    assert_eq!(text(&m, s), "Hello", "the copy is separate");
    let comma = cfstr(&mut m, ",");
    let rep = cfstr(&mut m, ";");
    // CFStringReplace(str, CFRange{5,1}, ";")
    call(&mut m, "CFStringReplace", &[mu, 5, 1, rep]);
    assert_eq!(text(&m, mu), "Hello; World");
    let sub = call(&mut m, "CFStringCreateWithSubstring", &[0, mu, 7, 5]);
    assert_eq!(text(&m, sub), "World");
    // CFStringFind: found -> (location, length); not found -> kCFNotFound (-1), 0
    let w = cfstr(&mut m, "wor");
    assert_eq!(call(&mut m, "CFStringFind", &[mu, w, 1]), 7);
    assert_eq!(R4.with(|c| c.get()), 3);
    assert_eq!(call(&mut m, "CFStringFind", &[mu, w, 0]) as i32, -1, "case-sensitive: NEGATIVE control");
    // FindWithOptions(str, find, CFRange{0,12}, options 1 = caseInsensitive, &range)
    assert_eq!(call(&mut m, "CFStringFindWithOptions", &[mu, w, 0, 12, 1, OUT]), 1);
    assert_eq!((m.mac.mem.read_u32(OUT).unwrap(), m.mac.mem.read_u32(OUT + 4).unwrap()), (7, 3));
    // GetBytes(str, CFRange{0,5}, MacRoman, lossByte 0, external 0, buf, max 3, &used) copies only what fits
    assert_eq!(call(&mut m, "CFStringGetBytes", &[mu, 0, 5, 0, 0, 0, BUF, 3, OUT]), 3);
    assert_eq!((m.get_bytes(BUF, 3, "t").unwrap(), m.mac.mem.read_u32(OUT).unwrap()), (b"Hel".to_vec(), 3));
    // spec vectors: range {1,3} with maxBufLen 2 -> 2; a NULL buffer reports the count that would convert, maxBufLen ignored; UTF-8 of ASCII text is the same bytes
    assert_eq!(call(&mut m, "CFStringGetBytes", &[mu, 1, 3, 0, 0, 0, BUF, 2, OUT]), 2);
    assert_eq!(call(&mut m, "CFStringGetBytes", &[mu, 0, 5, 0, 0, 0, 0, 0, OUT]), 5);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 5);
    assert_eq!(call(&mut m, "CFStringGetBytes", &[mu, 0, 5, 0x0800_0100, 0, 0, BUF, 16, OUT]), 5);
    // NEGATIVE control: UTF-16 (0x100) is a real conversion and is refused by name
    assert!(refused(&mut m, "CFStringGetBytes", &[mu, 0, 5, 0x0100, 0, 0, BUF, 16, OUT]).contains("not implemented"));
    // GetCharacters widens to UniChar
    call(&mut m, "CFStringGetCharacters", &[s, 0, 2, CHARS]);
    assert_eq!((m.mac.mem.read_u16(CHARS).unwrap(), m.mac.mem.read_u16(CHARS + 2).unwrap()), (u16::from(b'H'), u16::from(b'e')));
    // split and join
    let csv = cfstr(&mut m, "a,b,,c");
    let arr = call(&mut m, "CFStringCreateArrayBySeparatingStrings", &[0, csv, comma]);
    assert_eq!(call(&mut m, "CFArrayGetCount", &[arr]), 4);
    let third = call(&mut m, "CFArrayGetValueAtIndex", &[arr, 2]);
    assert_eq!(text(&m, third), "");
    let dash = cfstr(&mut m, "-");
    let joined = call(&mut m, "CFStringCreateByCombiningStrings", &[0, arr, dash]);
    assert_eq!(text(&m, joined), "a-b--c");
    assert_eq!(call(&mut m, "CFStringGetSystemEncoding", &[]), 0);
    // NEGATIVE CONTROLS, refused by name: a non-ASCII UniChar, a range off the end, an index past the array, a non-CFString
    m.mac.mem.write_u16(CHARS, 0x20AC).unwrap();
    assert!(refused(&mut m, "CFStringCreateWithCharacters", &[0, CHARS, 1]).contains("not ASCII"));
    assert!(refused(&mut m, "CFStringCreateWithSubstring", &[0, s, 3, 9]).contains("outside the 5-character string"));
    assert!(refused(&mut m, "CFArrayGetValueAtIndex", &[arr, 9]).contains("past the end"));
    assert!(refused(&mut m, "CFStringCreateWithSubstring", &[0, 0x1234, 0, 1]).contains("not a CFString"));
}

#[test]
fn arrays_dictionaries_numbers_urls_and_retain_counts() {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let (k1, k2) = (cfstr(&mut m, "name"), cfstr(&mut m, "size"));
    let (v1, v2) = (cfstr(&mut m, "Frost"), cfstr(&mut m, "big"));
    for (i, x) in [k1, k2].into_iter().enumerate() { m.mac.mem.write_u32(CHARS + 4 * i as u32, x).unwrap(); }
    for (i, x) in [v1, v2].into_iter().enumerate() { m.mac.mem.write_u32(CHARS + 0x10 + 4 * i as u32, x).unwrap(); }
    let d = call(&mut m, "CFDictionaryCreate", &[0, CHARS, CHARS + 0x10, 2, 0, 0]);
    assert_eq!(call(&mut m, "CFDictionaryGetCount", &[d]), 2);
    // lookups by another CFString with the same text find the value (string keys compare by content)
    let probe = cfstr(&mut m, "size");
    assert_eq!(call(&mut m, "CFDictionaryGetValue", &[d, probe]), v2);
    assert_eq!(call(&mut m, "CFDictionaryContainsKey", &[d, probe]), 1);
    let absent = cfstr(&mut m, "none");
    assert_eq!((call(&mut m, "CFDictionaryGetValue", &[d, absent]), call(&mut m, "CFDictionaryContainsKey", &[d, absent])), (0, 0), "NEGATIVE control: a missing key is NULL / FALSE");
    // numbers: SInt32 round trip, narrowing is reported as lossy, float out of an int
    m.mac.mem.write_u32(OUT, 300).unwrap();
    let n = call(&mut m, "CFNumberCreate", &[0, 3, OUT]);
    assert_eq!(call(&mut m, "CFNumberGetValue", &[n, 3, OUT + 8]), 1);
    assert_eq!(m.mac.mem.read_u32(OUT + 8).unwrap(), 300);
    assert_eq!(call(&mut m, "CFNumberGetValue", &[n, 1, OUT + 8]), 0, "300 does not fit an SInt8: lossy -> FALSE");
    // URLs: POSIX and HFS appends
    let p = cfstr(&mut m, "/Apps");
    let u = call(&mut m, "CFURLCreateWithFileSystemPath", &[0, p, 0, 1]);
    let c = cfstr(&mut m, "War3");
    let u2 = call(&mut m, "CFURLCreateCopyAppendingPathComponent", &[0, u, c, 0]);
    assert_ne!(u2, u);
    assert_eq!(call(&mut m, "CFBundleCreate", &[0, u2]), 0, "no bundles");
    // retain counts across kinds
    assert_eq!(call(&mut m, "CFGetRetainCount", &[d]), 1);
    call(&mut m, "CFRetain", &[d]);
    assert_eq!(call(&mut m, "CFGetRetainCount", &[d]), 2);
    call(&mut m, "CFRelease", &[d]);
    call(&mut m, "CFRelease", &[d]);
    assert!(refused(&mut m, "CFDictionaryGetCount", &[d]).contains("not a CFDictionary"), "released to zero: gone");
    // allocator constants are data imports
    assert!(matches!(m.bind_from("CarbonLib", "kCFAllocatorDefault"), Binding::CData(4)));
    // spec vectors for CFStringCreateWithFormat (macos9-specs/CoreFoundationLib/CFStringCreateWithFormat)
    let f = cfstr(&mut m, "%d-%d");
    let r = call(&mut m, "CFStringCreateWithFormat", &[0, 0, f, 3, 4]);
    assert_eq!(text(&m, r), "3-4");
    let (f, a) = (cfstr(&mut m, "%@ x"), cfstr(&mut m, "a"));
    let r = call(&mut m, "CFStringCreateWithFormat", &[0, 0, f, a]);
    assert_eq!(text(&m, r), "a x");
    let f = cfstr(&mut m, "100%%");
    let r = call(&mut m, "CFStringCreateWithFormat", &[0, 0, f]);
    assert_eq!(text(&m, r), "100%");
    // NEGATIVE controls: a NULL format gives NULL; %@ of something that is no CFString is refused by name
    assert_eq!(call(&mut m, "CFStringCreateWithFormat", &[0, 0, 0]), 0);
    let f = cfstr(&mut m, "%@");
    assert!(refused(&mut m, "CFStringCreateWithFormat", &[0, 0, f, 0x4321]).contains("not a CFString"));
    // still unbound on purpose: property-list parsing stops by name
    assert!(matches!(m.bind_from("CarbonLib", "CFPropertyListCreateFromXMLData"), Binding::Unbound(_)));
}
