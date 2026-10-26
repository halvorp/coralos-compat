//! The C FILE layer (`heirloom_macppc_engine::stdio`) over the File Manager volume: files written through `fwrite`/`fprintf` are the files the Toolbox sees.
//! Called as the CPU calls natives: r3.. = the arguments, the host's `native` hook answers in r3.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::cfm::Fragment;
use heirloom_macppc_engine::fixture::{self, TOC_FIRST};
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_fmt_pef::loader::SymbolClass;
use heirloom_fmt_pef::Pef;
use heirloom_cpu_ppc::{Cpu, Host};

type M = MacPpc<RecordingGuestUi>;
const F: SymbolClass = SymbolClass::TVector;
const NAMES: &[&str] = &["fopen", "fclose", "fread", "fwrite", "fputs", "fputc", "putc", "_flsbuf", "fgetc", "getc", "_filbuf", "fgets", "ungetc", "fprintf",
    "fseek", "ftell", "rewind", "feof", "ferror", "clearerr", "fflush", "setvbuf", "remove", "fscanf"];
const EOF: u32 = u32::MAX;
const A: u32 = 0x4000;
const B: u32 = 0x4200;
const OUT: u32 = 0x4400;

struct Rig { m: M, frag: Fragment, cpu: Cpu, iob: u32 }

fn rig() -> Rig {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    let mut imports: Vec<(&str, SymbolClass, bool)> = NAMES.iter().map(|n| (*n, F, false)).collect();
    imports.push(("_iob", SymbolClass::Data, false));
    let bytes = fixture::app(&[("StdCLib", false, &imports)]);
    let frag = m.instantiate("test", &Pef::parse(&bytes).unwrap()).unwrap();
    let iob = frag.imports.iter().find(|i| i.symbol == "_iob").unwrap().value;
    let cpu = Cpu::new(0, heirloom_mac68k_engine::machine::STACK_TOP - 512);
    Rig { m, frag, cpu, iob }
}
impl Rig {
    fn call(&mut self, name: &str) -> Result<(), String> {
        let i = self.frag.imports.iter().position(|i| i.symbol == name).unwrap_or_else(|| panic!("{name}"));
        let data = self.frag.section_bases[1].unwrap();
        let tv = self.m.mac.mem.read_u32(data + TOC_FIRST + 4 * i as u32).unwrap();
        let stub = self.m.mac.mem.read_u32(tv).unwrap();
        assert!(self.m.native(&mut self.cpu, stub), "{name} is not a glue stub");
        match self.m.fault() { Some(f) => Err(f.to_string()), None => Ok(()) }
    }
    fn c(&mut self, name: &str, args: &[u32]) -> u32 { self.cpu.gpr[3..3 + args.len()].copy_from_slice(args); self.call(name).unwrap_or_else(|e| panic!("{name}: {e}")); self.cpu.gpr[3] }
    fn s(&mut self, at: u32, text: &str) -> u32 { let mut b = text.as_bytes().to_vec(); b.push(0); self.m.mac.mem.write_bytes(at, &b).unwrap(); at }
    fn open(&mut self, path: &str, mode: &str) -> u32 { self.s(A, path); self.s(B, mode); self.c("fopen", &[A, B]) }
    fn file_bytes(&self, path: &str) -> Option<Vec<u8>> {
        let l = self.m.mac.fm.locate(0, 0, Some(path.as_bytes())).ok()?;
        l.node.and_then(|n| self.m.mac.fm.tree.file_data(n).map(<[u8]>::to_vec))
    }
    fn read_str(&self, at: u32) -> String { String::from_utf8(self.m.cstr(at, "t").unwrap()).unwrap() }
    fn stdout(&self) -> u32 { self.iob + 16 }
}

#[test]
fn a_file_written_through_the_c_stream_functions_is_the_file_the_toolbox_sees() {
    let mut r = rig();
    let fp = r.open("Log", "w");
    assert_ne!(fp, 0);
    r.s(OUT, "hello");
    assert_eq!(r.c("fputs", &[OUT, fp]), 1);
    assert_eq!(r.c("fputc", &[u32::from(b',') , fp]), u32::from(b','));
    assert_eq!(r.c("putc", &[u32::from(b' '), fp]), u32::from(b' '));
    r.m.mac.mem.write_bytes(OUT, b"world!").unwrap();
    assert_eq!(r.c("fwrite", &[OUT, 1, 6, fp]), 6, "fwrite returns the item count");
    r.s(B + 0x40, "n=%d,%s");
    r.s(B + 0x80, "ok");
    assert_eq!(r.c("fprintf", &[fp, B + 0x40, 42, B + 0x80]), 7);
    assert_eq!(r.c("fclose", &[fp]), 0);
    assert_eq!(r.file_bytes("Log").unwrap(), b"hello, world!n=42,ok");
    assert_eq!(r.m.mac.fm.open_forks().count(), 0, "fclose closed the fork");
    // the Toolbox's own C routines read the same bytes: FSOpen then FSRead (InterfaceLib glue, r3.. = arguments)
    let tb = |m: &mut M, name: &str, args: &[u32]| { m.bind(name); let mut regs = [0u32; 32]; regs[3..3 + args.len()].copy_from_slice(args); assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}"); regs[3] as i32 };
    r.m.mac.mem.write_bytes(A, b"\x03Log").unwrap(); // the Toolbox takes a Pascal string
    assert_eq!(tb(&mut r.m, "FSOpen", &[A, 0, OUT]), 0);
    let refnum = r.m.mac.mem.read_u16(OUT).unwrap() as u32;
    r.m.mac.mem.write_u32(OUT + 4, 5).unwrap();
    assert_eq!(tb(&mut r.m, "FSRead", &[refnum, OUT + 4, B]), 0);
    assert_eq!(r.m.get_bytes(B, 5, "t").unwrap(), b"hello", "FSRead sees what fputs wrote");
    tb(&mut r.m, "FSClose", &[refnum]);
    // NEGATIVE CONTROL: "w" TRUNCATES an existing file, "a" appends to it
    let fp = r.open("Log", "w");
    r.c("fclose", &[fp]);
    assert_eq!(r.file_bytes("Log").unwrap(), b"");
    let fp = r.open("Log", "a");
    r.s(OUT, "one");
    r.c("fputs", &[OUT, fp]);
    r.c("fclose", &[fp]);
    let fp = r.open("Log", "a");
    r.s(OUT, "two");
    r.c("fputs", &[OUT, fp]);
    r.c("fclose", &[fp]);
    assert_eq!(r.file_bytes("Log").unwrap(), b"onetwo");
}

#[test]
fn reading_gets_lines_at_cr_chars_ungetc_eof_and_seeks() {
    let mut r = rig();
    let root = r.m.mac.fm.def_vol;
    r.m.mac.fm.tree.put_file(root, "Text", b"ab\rcdef\rgh".to_vec()).unwrap();
    assert_eq!(r.open("Missing", "r"), 0, "a file that does not exist: NULL");
    let fp = r.open("Text", "r");
    assert_ne!(fp, 0);
    // fgets: a line ends AFTER the CR ('\n' is CR on the classic Mac); n-1 bytes at most
    assert_eq!(r.c("fgets", &[OUT, 100, fp]), OUT);
    assert_eq!(r.m.get_bytes(OUT, 4, "t").unwrap(), b"ab\r\0");
    assert_eq!(r.c("fgets", &[OUT, 4, fp]), OUT);
    assert_eq!(r.read_str(OUT), "cde", "n-1 = 3 bytes, no CR yet");
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'f'));
    assert_eq!(r.c("ungetc", &[u32::from(b'Z'), fp]), u32::from(b'Z'));
    assert_eq!(r.c("getc", &[fp]), u32::from(b'Z'), "the pushed-back byte first");
    assert_eq!(r.c("_filbuf", &[fp]), 0x0D);
    assert_eq!(r.c("ftell", &[fp]), 8);
    // fread of more than is left: the items read, then feof
    assert_eq!(r.c("feof", &[fp]), 0);
    assert_eq!(r.c("fread", &[OUT, 1, 10, fp]), 2);
    assert_eq!(r.m.get_bytes(OUT, 2, "t").unwrap(), b"gh");
    assert_eq!(r.c("feof", &[fp]), 1);
    assert_eq!(r.c("fgetc", &[fp]), EOF);
    assert_eq!(r.c("fgets", &[OUT, 10, fp]), 0, "NULL at end of file");
    // clearerr / rewind / fseek SET, CUR, END
    r.c("clearerr", &[fp]);
    assert_eq!(r.c("feof", &[fp]), 0);
    r.c("rewind", &[fp]);
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'a'));
    assert_eq!(r.c("fseek", &[fp, 3, 0]), 0);
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'c'));
    assert_eq!(r.c("fseek", &[fp, 1, 1]), 0);
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'e'));
    assert_eq!(r.c("fseek", &[fp, (-2i32) as u32, 2]), 0);
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'g'));
    // NEGATIVE CONTROLS: a read-only stream cannot be written (0 items, ferror set) and cannot be sought past its end
    r.s(OUT, "x");
    assert_eq!(r.c("fwrite", &[OUT, 1, 1, fp]), 0);
    assert_eq!(r.c("ferror", &[fp]), 1);
    assert_eq!(r.c("fseek", &[fp, 100, 0]), EOF);
    assert_eq!(r.c("fseek", &[fp, 0, 9]), EOF, "an unknown whence");
    assert_eq!(r.c("fclose", &[fp]), 0);
    // a GCC/Retro68 text file ('\n' = LF) is read the same way, and no byte is translated either way
    r.m.mac.fm.tree.put_file(root, "Unix", b"one\ntwo\r\nthree".to_vec()).unwrap();
    let fp = r.open("Unix", "r");
    assert_eq!(r.c("fgets", &[OUT, 100, fp]), OUT);
    assert_eq!(r.m.get_bytes(OUT, 5, "t").unwrap(), b"one\n\0");
    r.c("fgets", &[OUT, 100, fp]);
    assert_eq!(r.m.get_bytes(OUT, 5, "t").unwrap(), b"two\r\0", "a CRLF file: the CR line, then a one-byte LF line");
    r.c("fgets", &[OUT, 100, fp]);
    assert_eq!(r.m.get_bytes(OUT, 2, "t").unwrap(), b"\n\0");
    r.c("fclose", &[fp]);
    let w = r.open("Raw", "w");
    r.m.mac.mem.write_bytes(OUT, b"a\nb\rc").unwrap();
    r.c("fwrite", &[OUT, 1, 5, w]);
    r.c("fclose", &[w]);
    assert_eq!(r.file_bytes("Raw").unwrap(), b"a\nb\rc", "NEGATIVE CONTROL: nothing is translated on the way out");
}

#[test]
fn update_mode_seeks_past_the_end_extends_and_remove_deletes() {
    let mut r = rig();
    let fp = r.open("Db", "w+");
    r.s(OUT, "abc");
    r.c("fputs", &[OUT, fp]);
    r.c("rewind", &[fp]);
    assert_eq!(r.c("fgetc", &[fp]), u32::from(b'a'), "w+ reads what it wrote");
    assert_eq!(r.c("fseek", &[fp, 6, 0]), 0, "a writable stream may be positioned past the end");
    r.s(OUT, "Z");
    r.c("fputs", &[OUT, fp]);
    r.c("fclose", &[fp]);
    assert_eq!(r.file_bytes("Db").unwrap(), b"abc\0\0\0Z");
    let fp = r.open("Db", "r+");
    r.s(OUT, "X");
    assert_eq!(r.c("fputs", &[OUT, fp]), 1);
    r.c("fclose", &[fp]);
    assert_eq!(r.file_bytes("Db").unwrap(), b"Xbc\0\0\0Z", "r+ writes in place at the start");
    r.s(A, "Db");
    assert_eq!(r.c("remove", &[A]), 0);
    assert!(r.file_bytes("Db").is_none());
    assert_eq!(r.c("remove", &[A]), EOF, "removing it twice fails");
    assert_eq!(r.open("Db", "r"), 0);
}

#[test]
fn the_standard_streams_are_the_console_and_a_bad_file_pointer_is_refused_by_name() {
    let mut r = rig();
    let (stdin, stdout, stderr) = (r.iob, r.iob + 16, r.iob + 32);
    r.s(OUT, "to out");
    r.c("fputs", &[OUT, stdout]);
    r.s(B, "err %d\n");
    r.c("fprintf", &[stderr, B, 5]);
    r.c("fputc", &[u32::from(b'!'), stdout]);
    assert_eq!(r.m.console, b"to outerr 5\n!");
    assert_eq!(r.c("feof", &[stdin]), 1, "there is no keyboard stream: stdin is at end of file");
    assert_eq!(r.c("fgetc", &[stdin]), EOF);
    assert_eq!(r.c("fflush", &[stdout]), 0);
    assert_eq!(r.c("fflush", &[0]), 0, "fflush(NULL)");
    assert_eq!(r.c("fclose", &[stdout]), 0, "closing a standard stream is harmless");
    r.c("fputc", &[u32::from(b'?'), stdout]);
    assert!(r.m.console.ends_with(b"!?"), "and stdout still works");
    // NEGATIVE CONTROL: a pointer that is no FILE stops the run naming the function; fscanf stays unprovided by name
    r.cpu.gpr[3] = 0x1234;
    assert!(r.call("fclose").unwrap_err().starts_with("fclose: 0x1234 is not a FILE this library opened"));
    let mut r = rig();
    let e = r.call("fscanf").unwrap_err();
    assert!(e.contains("NEXT-NEEDED") && e.contains("fscanf"), "{e}");
}
