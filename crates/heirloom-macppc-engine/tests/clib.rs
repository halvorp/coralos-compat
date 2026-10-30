//! StdCLib and MathLib natives against the C library's semantics (ISO C), called as the CPU calls them: r3.. / f1.. hold the arguments, the
//! host's `native` hook answers in r3 / f1. No interpreted code runs: each test plays the CPU's part exactly as `host.rs` describes.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::cfm::Fragment;
use heirloom_macppc_engine::fixture::{self, TOC_FIRST};
use heirloom_macppc_engine::MacPpc;
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_macppc_engine::cpu::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const STD: &[&str] = &["malloc", "calloc", "realloc", "free", "memcpy", "memmove", "memset", "memcmp", "memchr", "strlen", "strcpy", "strncpy", "strcat", "strncat",
    "strcmp", "strncmp", "strchr", "strrchr", "strstr", "strspn", "strcspn", "strpbrk", "atoi", "atol", "strtol", "strtoul", "abs", "rand", "srand", "sprintf",
    "snprintf", "printf", "puts", "putchar", "isalpha", "isdigit", "isspace", "toupper", "tolower", "getenv", "time", "clock", "signal", "sscanf"];
const MATH: &[&str] = &["sin", "cos", "tan", "atan", "atan2", "exp", "log", "log10", "pow", "sqrt", "floor", "ceil", "fabs", "fmod", "ldexp", "frexp", "modf", "hypot", "round", "rint", "pi"];

struct Rig { m: M, frag: Fragment, cpu: Cpu }

fn rig() -> Rig {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let std: Vec<(&str, SymbolClass, bool)> = STD.iter().map(|n| (*n, F, false)).collect();
    let math: Vec<(&str, SymbolClass, bool)> = MATH.iter().map(|n| (*n, F, false)).collect();
    let bytes = fixture::app(&[("StdCLib", false, &std), ("MathLib", false, &math)]);
    let frag = m.instantiate("test", &Pef::parse(&bytes).unwrap()).unwrap();
    let cpu = Cpu::new(0, heirloom_mac68k_engine::machine::STACK_TOP - 512);
    Rig { m, frag, cpu }
}

impl Rig {
    fn index(&self, lib: &str, name: &str) -> usize { self.frag.imports.iter().position(|i| i.library == lib && i.symbol == name).unwrap_or_else(|| panic!("{lib}/{name}")) }
    /// Call `lib/name`: the CPU's registers are as the test set them; returns whether the host handled it (a fault is `self.m.fault()`).
    fn call(&mut self, lib: &str, name: &str) -> Result<(), String> {
        let i = self.index(lib, name);
        let data = self.frag.section_bases[1].unwrap();
        let tv = self.m.mac.mem.read_u32(data + TOC_FIRST + 4 * i as u32).unwrap();
        let stub = self.m.mac.mem.read_u32(tv).unwrap();
        assert!(self.m.native(&mut self.cpu, stub), "{name} is not a glue stub");
        match self.m.fault() { Some(f) => Err(f.to_string()), None => Ok(()) }
    }
    fn c(&mut self, name: &str, args: &[u32]) -> u32 { self.cpu.gpr[3..3 + args.len()].copy_from_slice(args); self.call("StdCLib", name).unwrap_or_else(|e| panic!("{name}: {e}")); self.cpu.gpr[3] }
    fn math(&mut self, name: &str, x: f64, y: f64) -> f64 { self.cpu.fpr[1] = x.to_bits(); self.cpu.fpr[2] = y.to_bits(); self.call("MathLib", name).unwrap(); f64::from_bits(self.cpu.fpr[1]) }
    fn s(&mut self, at: u32, text: &str) -> u32 { let mut b = text.as_bytes().to_vec(); b.push(0); self.m.mac.mem.write_bytes(at, &b).unwrap(); at }
    fn read(&self, at: u32) -> String { String::from_utf8(self.m.cstr(at, "test").unwrap()).unwrap() }
}
const A: u32 = 0x4000;
const B: u32 = 0x4200;
const OUT: u32 = 0x4400;

#[test]
fn malloc_calloc_realloc_free_follow_the_c_contract_and_a_wild_free_is_refused_by_name() {
    let mut r = rig();
    let p = r.c("malloc", &[16]);
    assert!(p != 0 && r.m.mac.mm.ptr_size(p) == Ok(16));
    r.m.mac.mem.write_bytes(p, b"0123456789abcdef").unwrap();
    let q = r.c("realloc", &[p, 64]);
    assert!(q != 0);
    assert_eq!(r.m.get_bytes(q, 16, "t").unwrap(), b"0123456789abcdef", "realloc keeps the old contents");
    assert_eq!(r.m.mac.mm.ptr_size(q), Ok(64));
    r.m.mac.mem.write_bytes(q, &[0xFF; 64]).unwrap();
    let z = r.c("calloc", &[4, 8]);
    assert_eq!(r.m.get_bytes(z, 32, "t").unwrap(), [0u8; 32], "calloc zeroes");
    assert_eq!(r.c("calloc", &[0x8000_0000, 4]), 0, "the size product overflows: NULL");
    assert_eq!(r.c("realloc", &[0, 8]) != 0, true, "realloc(NULL, n) is malloc");
    assert_eq!(r.c("realloc", &[q, 0]), 0, "realloc(p, 0) frees");
    assert!(r.m.mac.mm.ptr_size(q).is_err());
    assert_eq!(r.c("free", &[0]), 0, "free(NULL) is a no-op");
    assert_eq!(r.c("free", &[z]), 0);
    assert!(r.m.mac.mm.ptr_size(z).is_err());
    // NEGATIVE CONTROL: freeing something malloc never returned stops the run NAMING free
    r.cpu.gpr[3] = 0x1234;
    let e = r.call("StdCLib", "free").unwrap_err();
    assert!(e.starts_with("free: 0x1234 is not a block malloc returned"), "{e}");
}

#[test]
fn the_mem_functions_copy_fill_compare_and_search() {
    let mut r = rig();
    r.s(A, "abcdefgh");
    assert_eq!(r.c("memcpy", &[B, A, 8]), B);
    assert_eq!(r.m.get_bytes(B, 8, "t").unwrap(), b"abcdefgh");
    // memmove with overlap goes through a copy: A+2 <- A[0..6]
    r.c("memmove", &[A + 2, A, 6]);
    assert_eq!(r.m.get_bytes(A, 8, "t").unwrap(), b"ababcdef");
    assert_eq!(r.c("memset", &[A, 0x7A, 3]), A);
    assert_eq!(r.m.get_bytes(A, 4, "t").unwrap(), b"zzzb");
    r.s(A, "abc");
    r.s(B, "abd");
    assert_eq!(r.c("memcmp", &[A, B, 3]) as i32, -1);
    assert_eq!(r.c("memcmp", &[B, A, 3]) as i32, 1);
    assert_eq!(r.c("memcmp", &[A, B, 2]), 0, "only the first two bytes are compared");
    assert_eq!(r.c("memchr", &[A, u32::from(b'c'), 3]), A + 2);
    assert_eq!(r.c("memchr", &[A, u32::from(b'c'), 2]), 0, "not within n bytes");
    // an operand outside guest RAM stops the run, naming the function
    r.cpu.gpr[3..6].copy_from_slice(&[0xF000_0000, A, 4]);
    assert!(r.call("StdCLib", "memcpy").unwrap_err().starts_with("memcpy: 4 bytes at 0xf0000000 leave guest RAM"));
}

#[test]
fn the_string_functions_match_the_c_library() {
    let mut r = rig();
    r.s(A, "hello");
    assert_eq!(r.c("strlen", &[A]), 5);
    r.s(B, "xxxxxxxxxx");
    assert_eq!(r.c("strcpy", &[B, A]), B);
    assert_eq!(r.read(B), "hello", "strcpy copies the NUL too");
    // strncpy pads with NULs to n and does not terminate when the source is longer
    r.s(B, "xxxxxxxxxx");
    r.c("strncpy", &[B, A, 8]);
    assert_eq!(r.m.get_bytes(B, 10, "t").unwrap(), b"hello\0\0\0xx");
    r.s(B, "xxxxxxxxxx");
    r.c("strncpy", &[B, A, 3]);
    assert_eq!(r.m.get_bytes(B, 6, "t").unwrap(), b"helxxx");
    r.s(B, "ab");
    r.s(A, "cdefg");
    r.c("strcat", &[B, A]);
    assert_eq!(r.read(B), "abcdefg");
    r.s(B, "ab");
    r.c("strncat", &[B, A, 2]);
    assert_eq!(r.read(B), "abcd", "strncat appends at most n and always terminates");
    r.s(A, "apple");
    r.s(B, "apply");
    assert_eq!(r.c("strcmp", &[A, B]) as i32, -1);
    assert_eq!(r.c("strcmp", &[B, A]) as i32, 1);
    assert_eq!(r.c("strcmp", &[A, A]), 0);
    assert_eq!(r.c("strncmp", &[A, B, 4]), 0);
    r.s(A, "a/b/c");
    assert_eq!(r.c("strchr", &[A, u32::from(b'/')]), A + 1);
    assert_eq!(r.c("strrchr", &[A, u32::from(b'/')]), A + 3);
    assert_eq!(r.c("strchr", &[A, 0]), A + 5, "the terminator is part of the string");
    assert_eq!(r.c("strchr", &[A, u32::from(b'z')]), 0);
    r.s(B, "b/c");
    assert_eq!(r.c("strstr", &[A, B]), A + 2);
    r.s(B, "");
    assert_eq!(r.c("strstr", &[A, B]), A, "the empty needle is found at the start");
    r.s(A, "abc123");
    r.s(B, "abc");
    assert_eq!(r.c("strspn", &[A, B]), 3);
    assert_eq!(r.c("strcspn", &[A, B]), 0);
    r.s(B, "13");
    assert_eq!(r.c("strcspn", &[A, B]), 3);
    assert_eq!(r.c("strpbrk", &[A, B]), A + 3);
}

#[test]
fn the_number_conversions_and_ctype_follow_the_c_locale() {
    let mut r = rig();
    r.s(A, "  -123xyz");
    assert_eq!(r.c("atoi", &[A]) as i32, -123);
    r.s(A, "0x1Fz");
    assert_eq!(r.c("strtol", &[A, OUT, 0]), 31);
    assert_eq!(r.m.mac.mem.read_u32(OUT).unwrap(), A + 4, "endptr points at the first unconverted byte");
    r.s(A, "junk");
    r.c("strtol", &[A, OUT, 10]);
    assert_eq!(r.m.mac.mem.read_u32(OUT).unwrap(), A, "no digits: endptr is the start");
    r.s(A, "99999999999");
    assert_eq!(r.c("strtol", &[A, 0, 10]), 0x7FFF_FFFF, "clamped to LONG_MAX");
    r.s(A, "4294967295");
    assert_eq!(r.c("strtoul", &[A, 0, 10]), u32::MAX);
    assert_eq!(r.c("abs", &[(-5i32) as u32]), 5);
    assert_eq!((r.c("isalpha", &[u32::from(b'q')]) != 0, r.c("isalpha", &[u32::from(b'1')]) != 0, r.c("isalpha", &[300]) != 0), (true, false, false));
    assert_eq!((r.c("isdigit", &[u32::from(b'7')]) != 0, r.c("isspace", &[u32::from(b'\t')]) != 0, r.c("isspace", &[u32::from(b'x')]) != 0), (true, true, false));
    assert_eq!((r.c("toupper", &[u32::from(b'a')]), r.c("toupper", &[u32::from(b'A')]), r.c("tolower", &[u32::from(b'Q')]), r.c("tolower", &[u32::from(b'1')])), (65, 65, 113, 49));
}

#[test]
fn rand_is_deterministic_and_srand_restarts_it_and_getenv_time_clock_answer() {
    let mut r = rig();
    let first: Vec<u32> = (0..3).map(|_| r.c("rand", &[])).collect();
    assert!(first.iter().all(|&v| v <= 0x7FFF));
    r.c("srand", &[1]);
    let again: Vec<u32> = (0..3).map(|_| r.c("rand", &[])).collect();
    assert_eq!(first, again, "srand(1) is the initial state");
    r.c("srand", &[2]);
    assert_ne!(r.c("rand", &[]), first[0], "a different seed gives a different sequence");
    r.s(A, "HOME");
    assert_eq!(r.c("getenv", &[A]), 0, "there is no environment");
    let t = r.c("time", &[OUT]);
    assert_eq!(r.m.mac.mem.read_u32(OUT).unwrap(), t);
    assert!(t > 3_000_000_000, "seconds since 1904: {t}");
    r.m.mac.advance_ticks(120);
    assert_eq!(r.c("clock", &[]), r.m.mac.ev.ticks);
    assert_eq!(r.c("time", &[0]), t + 2, "two seconds later");
}

#[test]
fn printf_family_formats_integers_strings_chars_and_flags() {
    let mut r = rig();
    r.s(B, "%d|%5d|%-5d|%05d|%+d|%x|%X|%#x|%o|%u|%c|%s|%5.2s|%%");
    r.s(A + 0x100, "str");
    r.s(A + 0x120, "abcdef");
    // sprintf(buf, fmt, ...): r3 buf, r4 fmt, r5.. the arguments (14 of them: past r10 they come from the parameter area)
    let args = [OUT, B, (-42i32) as u32, 42, 42, 42, 42, 255, 255, 255, 8, 4_000_000_000, u32::from(b'Z'), A + 0x100, A + 0x120];
    let sp = r.cpu.gpr[1];
    for (i, &v) in args.iter().enumerate() { if i < 8 { r.cpu.gpr[3 + i] = v; } else { r.m.mac.mem.write_u32(sp + 24 + 4 * i as u32, v).unwrap(); } }
    let n = r.call("StdCLib", "sprintf").map(|_| r.cpu.gpr[3]).unwrap();
    let want = "-42|   42|42   |00042|+42|ff|FF|0xff|10|4000000000|Z|str|   ab|%";
    assert_eq!(r.read(OUT), want);
    assert_eq!(n as usize, want.len(), "sprintf returns the length");
    // long long (two words) and precision on integers
    r.s(B, "%lld|%.5d|%08.3d");
    r.cpu.gpr[3..8].copy_from_slice(&[OUT, B, 0xFFFF_FFFF, 0xFFFF_FFFE, 7]);
    r.cpu.gpr[8] = 9;
    r.call("StdCLib", "sprintf").unwrap();
    assert_eq!(r.read(OUT), "-2|00007|     009");
}

#[test]
fn doubles_come_from_the_fprs_and_reserve_two_word_slots_so_later_integers_are_found() {
    let mut r = rig();
    r.s(B, "%d %.2f %d|%e|%g|%g|%10.3f|%-8.1f|%.0f");
    // r3 buf, r4 fmt, r5 = 1, f1 = 2.5 (r6,r7 shadow), r8 = 3, f2 f3 f4 f5 f6 f7
    r.cpu.gpr[3..9].copy_from_slice(&[OUT, B, 1, 0xDEAD, 0xBEEF, 3]);
    for (k, v) in [2.5f64, 12345.678, 0.0001, 1e10, -3.14159, 0.5, 2.5].iter().enumerate() { r.cpu.fpr[1 + k] = v.to_bits(); }
    r.call("StdCLib", "sprintf").unwrap();
    assert_eq!(r.read(OUT), "1 2.50 3|1.234568e+04|0.0001|1e+10|    -3.142|0.5     |2");
    // NEGATIVE CONTROL for the shadow slots: had the double taken ONE word slot, the second %d would read r6 = 0xDEAD, not 3
    assert!(!r.read(OUT).contains("57005"));
    // snprintf: truncates but returns the full length, always terminating
    r.s(B, "value=%d");
    r.cpu.gpr[3..7].copy_from_slice(&[OUT, 6, B, 12345]);
    r.m.mac.mem.write_bytes(OUT, &[b'#'; 16]).unwrap();
    r.call("StdCLib", "snprintf").unwrap();
    assert_eq!(r.cpu.gpr[3], 11);
    assert_eq!(r.m.get_bytes(OUT, 8, "t").unwrap(), b"value\0##");
}

#[test]
fn a_string_after_a_double_is_read_from_the_slot_past_the_doubles_shadow_not_from_the_doubles_words() {
    // The convention (Mac OS Runtime Architectures): a double argument is in an FPR AND occupies two word slots, so the pointer for a `%s` that FOLLOWS it is the word after the
    // shadow. A reader that gave the double one slot would take the double's low word as the pointer. Differential check of the three readings against the one convention:
    let mut r = rig();
    r.s(B, "v=%f s=%s n=%d");
    r.s(0x3600, "1.2");
    // r3 buf, r4 fmt, f1 = 1.2 (r5, r6 are its shadow words), r7 = the string, r8 = 5
    r.cpu.gpr[3..9].copy_from_slice(&[OUT, B, (1.2f64.to_bits() >> 32) as u32, 1.2f64.to_bits() as u32, 0x3600, 5]);
    r.cpu.fpr[1] = 1.2f64.to_bits();
    r.call("StdCLib", "sprintf").unwrap();
    assert_eq!(r.read(OUT), "v=1.200000 s=1.2 n=5");
    // the failure the Warcraft III TFT error path shows when the CALLER passes a double where the format says %s: the pointer read is the double's high word, 0x3ff33333, by
    // construction of the caller's arguments -- this engine reads the slot it is told to, so the bad read is refused by name instead of faulting somewhere else
    r.s(B, "OpenGL %s");
    r.cpu.gpr[3..6].copy_from_slice(&[OUT, B, 0x3ff3_3333]);
    r.call("StdCLib", "sprintf").unwrap_err();
}

#[test]
fn printf_puts_putchar_write_the_console_and_percent_n_is_refused() {
    let mut r = rig();
    r.s(B, "n=%d\n");
    r.cpu.gpr[3..5].copy_from_slice(&[B, 7]);
    r.call("StdCLib", "printf").unwrap();
    assert_eq!(r.cpu.gpr[3], 4);
    r.s(A, "line");
    r.c("puts", &[A]);
    r.c("putchar", &[u32::from(b'!')]);
    assert_eq!(r.m.console, b"n=7\nline\n!");
    assert!(r.m.mac.log.iter().any(|l| l == "[console] n=7"), "{:?}", r.m.mac.log);
    // NEGATIVE CONTROL: %n writes through a pointer the format picks: refused, naming printf
    r.s(B, "abc%n");
    r.cpu.gpr[3..5].copy_from_slice(&[OUT, B]);
    assert!(r.call("StdCLib", "sprintf").unwrap_err().contains("%n is refused"));
}

#[test]
fn mathlib_functions_take_doubles_in_f1_f2_and_answer_in_f1() {
    let mut r = rig();
    let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
    assert!(close(r.math("sin", core::f64::consts::FRAC_PI_2, 0.0), 1.0));
    assert!(close(r.math("cos", 0.0, 0.0), 1.0));
    assert!(close(r.math("tan", core::f64::consts::FRAC_PI_4, 0.0), 1.0));
    assert!(close(r.math("atan2", 1.0, 1.0), core::f64::consts::FRAC_PI_4));
    assert!(close(r.math("exp", 1.0, 0.0), core::f64::consts::E));
    assert!(close(r.math("log", core::f64::consts::E, 0.0), 1.0));
    assert!(close(r.math("log10", 1000.0, 0.0), 3.0));
    assert!(close(r.math("pow", 2.0, 10.0), 1024.0));
    assert_eq!(r.math("sqrt", 144.0, 0.0), 12.0);
    assert_eq!((r.math("floor", -1.5, 0.0), r.math("ceil", -1.5, 0.0), r.math("fabs", -2.5, 0.0), r.math("round", 2.5, 0.0), r.math("rint", 2.5, 0.0)), (-2.0, -1.0, 2.5, 3.0, 2.0));
    assert_eq!(r.math("fmod", 7.5, 2.0), 1.5);
    assert!(close(r.math("hypot", 3.0, 4.0), 5.0));
    assert!(r.math("sqrt", -1.0, 0.0).is_nan(), "sqrt of a negative is NaN");
    // ldexp(x, int); frexp(x, int*); modf(x, double*): the double fills parameter words 0-1 (r3/r4), so the second argument is
    // word 2, r5 (Mac OS Runtime Architectures, the PowerPC parameter area). r3 holds a decoy that must NOT be read.
    r.cpu.gpr[3] = 99;
    r.cpu.gpr[5] = 3;
    assert_eq!(r.math("ldexp", 1.5, 0.0), 12.0);
    r.cpu.gpr[5] = OUT;
    assert_eq!(r.math("frexp", 48.0, 0.0), 0.75);
    assert_eq!(r.m.mac.mem.read_u32(OUT).unwrap(), 6, "48 = 0.75 * 2^6");
    r.cpu.gpr[5] = OUT;
    assert_eq!(r.math("modf", -3.25, 0.0), -0.25);
    let bits = (u64::from(r.m.mac.mem.read_u32(OUT).unwrap()) << 32) | u64::from(r.m.mac.mem.read_u32(OUT + 4).unwrap());
    assert_eq!(f64::from_bits(bits), -3.0, "the integral part is stored big-endian through the pointer");
}

#[test]
fn what_is_not_provided_stays_refused_by_name_in_both_libraries() {
    let mut r = rig();
    let q = r.call("StdCLib", "signal").unwrap_err();
    assert!(q.contains("NEXT-NEEDED") && q.contains("signal"), "{q}");
    let f = r.call("StdCLib", "sscanf").unwrap_err();
    assert!(f.contains("sscanf"), "{f}");
    let p = r.call("MathLib", "pi");
    assert!(p.is_err(), "pi is a data symbol MathLib exports; this engine does not provide it");
}
