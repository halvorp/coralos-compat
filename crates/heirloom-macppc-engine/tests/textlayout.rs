//! The Script Manager's line layout by C name (`heirloom_macppc_engine::textlayout_c`), Roman, measured with the font QuickDraw draws: a program
//! that breaks and justifies its own lines (Installer VISE's licence view) gets breaks, widths and hit-testing that agree with each other.
use heirloom_ui_abi::RecordingGuestUi;
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const TEXT: u32 = 0x4000;
const OUT: u32 = 0x4400;

fn call(m: &mut M, name: &str, args: &[u32]) -> u32 {
    assert!(!matches!(m.bind(name), Binding::Unbound(_)), "{name} must be bound: {:?}", m.bind(name));
    let mut regs = [0u32; 32];
    regs[3..3 + args.len()].copy_from_slice(args);
    assert_eq!(m.glue_call(name, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}
fn machine(text: &[u8]) -> (M, i64) {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.mac.mem.write_bytes(TEXT, text).unwrap();
    let w = i64::from(m.mac.qd.font.text_length(b"a"));
    (m, w)
}

#[test]
fn visible_length_and_portion_line() {
    let (mut m, _) = machine(b"one two  \t");
    assert_eq!(call(&mut m, "VisibleLength", &[TEXT, 10]), 7, "trailing blanks and tabs are not visible");
    assert_eq!(call(&mut m, "PortionLine", &[TEXT, 7, 0, 0x0001_0001, 0x0001_0001]), 0x0001_0000, "one space: 1.0");
    assert_eq!(call(&mut m, "PortionLine", &[TEXT, 3, 0, 0x0001_0001, 0x0001_0001]), 0x0001_0000, "no space: 1.0 (the characters share it)");
    assert_eq!(call(&mut m, "PortionLine", &[TEXT, 0, 0, 0x0001_0001, 0x0001_0001]), 0, "an empty run takes none");
}

/// "hello world again" with room for 10 characters: "hello worl" fits, so the word break is after "hello "; with room for 13 it is after
/// "world " (the blank hangs at the margin); with room for all, overflow
/// with the rest of the width; a single long word on the first run is broken at a character.
#[test]
fn styled_line_break_word_char_and_overflow() {
    let (mut m, w) = machine(b"hello world again");
    let brk = |m: &mut M, chars: i64, first: bool, len: u32| {
        m.mac.mem.write_u32(OUT, ((chars * w) << 16) as u32).unwrap();
        m.mac.mem.write_u32(OUT + 4, u32::from(first)).unwrap();
        let code = call(m, "StyledLineBreak", &[TEXT, len, 0, len, 0, OUT, OUT + 4]);
        (code, m.mac.mem.read_u32(OUT + 4).unwrap(), (m.mac.mem.read_u32(OUT).unwrap() as i32 >> 16) as i64)
    };
    assert_eq!(brk(&mut m, 10, true, 17), (0, 6, 4 * w), "smBreakWord after \"hello \", 4 characters of width left");
    assert_eq!(brk(&mut m, 13, true, 17), (0, 12, w), "after \"world \"");
    assert_eq!(brk(&mut m, 20, true, 17), (2, 17, 3 * w), "smBreakOverflow: the whole run fits, 3 left");
    assert_eq!(brk(&mut m, 3, true, 5), (1, 3, 0), "smBreakChar inside one word on the line's first run");
    assert_eq!(brk(&mut m, 3, false, 5).0, 0, "NEGATIVE CONTROL: not the first run: no character break (the run moves to the next line)");
}

/// CharToPixel and PixelToChar agree, without and with slop spread over the spaces (DrawJustified's rule).
#[test]
fn char_to_pixel_and_pixel_to_char_round_trip_with_slop() {
    let (mut m, w) = machine(b"ab cd");
    assert_eq!(call(&mut m, "CharToPixel", &[TEXT, 5, 0, 3, 0, 0, 0x0001_0001, 0x0001_0001]) as i64, 3 * w);
    let slop = (10 << 16) as u32;
    assert_eq!(call(&mut m, "CharToPixel", &[TEXT, 5, slop, 3, 0, 0, 0x0001_0001, 0x0001_0001]) as i64, 3 * w + 10, "the space took the 10 pixels");
    assert_eq!(call(&mut m, "CharToPixel", &[TEXT, 5, slop, 2, 0, 0, 0x0001_0001, 0x0001_0001]) as i64, 2 * w, "but not before it");
    let px = |p: i64| (p << 16) as u32;
    assert_eq!(call(&mut m, "PixelToChar", &[TEXT, 5, 0, px(w + 1), OUT, OUT + 4, 0, 0x0001_0001, 0x0001_0001]), 1);
    assert_eq!(m.mac.mem.read_u8(OUT).unwrap(), 1, "the leading half of 'b'");
    assert_eq!(call(&mut m, "PixelToChar", &[TEXT, 5, 0, px(2 * w - 1), OUT, OUT + 4, 0, 0x0001_0001, 0x0001_0001]), 2, "the trailing half of 'b' is offset 2");
    assert_eq!(call(&mut m, "PixelToChar", &[TEXT, 5, 0, px(9 * w), OUT, OUT + 4, 0, 0x0001_0001, 0x0001_0001]), 5, "past the end");
    assert_eq!(m.mac.mem.read_u32(OUT + 4).unwrap() as i64, (4 * w) << 16, "widthRemaining: what lies beyond the text");
}

#[test]
fn word_breaks_hilite_text_and_character_types() {
    let (mut m, _) = machine(b"Read Me, now");
    call(&mut m, "FindWordBreaks", &[TEXT, 12, 6, 1, 0, OUT, 0]);
    let pair = |m: &M, i: u32| (m.mac.mem.read_u16(OUT + 4 * i).unwrap(), m.mac.mem.read_u16(OUT + 4 * i + 2).unwrap());
    assert_eq!(pair(&m, 0), (5, 7), "\"Me\"");
    call(&mut m, "FindWordBreaks", &[TEXT, 12, 7, 1, 0, OUT, 0]);
    assert_eq!(pair(&m, 0), (7, 8), "the comma alone");
    call(&mut m, "FindWordBreaks", &[TEXT, 12, 9, 0, 0, OUT, 0]);
    assert_eq!(pair(&m, 0), (8, 9), "trailing edge of offset 9: the blank before \"now\"");
    call(&mut m, "HiliteText", &[TEXT, 12, 9, 2, OUT]);
    assert_eq!((pair(&m, 0), pair(&m, 1)), ((2, 9), (0, 0)), "one span, ordered");
    for (at, want) in [(0, 0x4001), (1, 0x0001), (4, 0x0300), (7, 0x0200)] { assert_eq!(call(&mut m, "CharacterType", &[TEXT, at, 0]), want, "byte {at}"); }
    assert_eq!(call(&mut m, "CharacterByteType", &[TEXT, 0, 0]), 0, "smSingleByte");
}
