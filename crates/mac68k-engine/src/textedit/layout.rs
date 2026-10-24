//! TextEdit line layout and hit-testing as pure functions over text and a width function, so every rule is testable without a
//! machine. Rules, from Inside Macintosh I "TextEdit" and followed in Executor's MIT `teMisc.cpp` `nextbreak` / `teInsert.cpp`:
//!
//! * `crOnly < 0`: a line ends only after a carriage return (no wrapping). Otherwise a line takes characters while the running
//!   width stays within the destination width and stops after a CR.
//! * When the character that overflows is a SPACE, the line takes the run of spaces (they hang past the edge); otherwise it
//!   breaks just after the last word break before the overflow, or exactly at the overflow when the word has none.
//! * A line always advances at least one character. `lineStarts` holds `nLines + 1` entries, the last one `teLength`; an empty
//!   text has `nLines = 0`. A CR has no width.
//! * `just` is `teFlushDefault` 0 (left for the Roman script), `teCenter` 1, `teFlushRight` -1, `teFlushLeft` -2.
use alloc::vec::Vec;

pub const CR: u8 = b'\r';
pub const JUST_LEFT_DEFAULT: i16 = 0;
pub const JUST_CENTER: i16 = 1;
pub const JUST_RIGHT: i16 = -1;
pub const JUST_LEFT: i16 = -2;

/// The default word-break test: a space, tab or CR separates words.
pub fn is_break(c: u8) -> bool { matches!(c, b' ' | b'\t' | CR) }

/// The width of one character; a CR has none.
pub fn cw(c: u8, w: &dyn Fn(u8) -> i32) -> i32 { if c == CR { 0 } else { w(c) } }

/// Where the line starting at `off` ends (the next line's start).
pub fn next_break(text: &[u8], off: usize, max_width: i32, cr_only: i16, w: &dyn Fn(u8) -> i32) -> usize {
    let len = text.len();
    let mut end;
    if cr_only < 0 {
        end = off;
        while end < len { let c = text[end]; end += 1; if c == CR { break; } }
    } else {
        let (mut sp, mut width) = (off, 0);
        while width <= max_width && sp < len && text[sp] != CR { width += cw(text[sp], w); sp += 1; }
        if width > max_width {
            let over = sp - 1; // the character that did not fit
            if text[over] == b' ' {
                let mut s = over;
                while s < len && text[s] == b' ' { s += 1; }
                end = s;
            } else {
                // Executor's `while (sp > minsp && !wordb(--sp))`: the character at `off` itself never counts as a break
                end = (off + 1..over).rev().find(|&j| is_break(text[j])).map_or(over, |j| j + 1);
            }
        } else if sp >= len { end = len; } else { end = sp + 1; } // past the CR
    }
    if end == off && off < len { end += 1; }
    end
}

/// Every line start plus the end: `[0, b1, .., len]` for non-empty text, `[0]` for none (so `nLines = starts.len() - 1`).
pub fn line_starts(text: &[u8], width: i32, cr_only: i16, w: &dyn Fn(u8) -> i32) -> Vec<u16> {
    let mut v = alloc::vec![0u16];
    let mut at = 0;
    while at < text.len() {
        at = next_break(text, at, width, cr_only, w);
        v.push(at as u16);
    }
    v
}

fn n_lines(starts: &[u16]) -> usize { starts.len().saturating_sub(1) }

/// The width of `text[a..b]`.
pub fn span_width(text: &[u8], a: usize, b: usize, w: &dyn Fn(u8) -> i32) -> i32 { text[a.min(b)..b].iter().map(|&c| cw(c, w)).sum() }

/// The line `sel` falls on; a position after a trailing CR is on the (virtual) line below the last. Lines are numbered from 0.
pub fn line_of(text: &[u8], starts: &[u16], sel: usize) -> usize {
    let n = n_lines(starts);
    if n == 0 { return 0; }
    if sel >= text.len() { return if text.last() == Some(&CR) { n } else { n - 1 }; }
    (0..n).rev().find(|&i| usize::from(starts[i]) <= sel).unwrap_or(0)
}

/// The horizontal offset of a line's text inside the destination width.
pub fn line_offset(text: &[u8], starts: &[u16], line: usize, width: i32, just: i16, w: &dyn Fn(u8) -> i32) -> i32 {
    if line >= n_lines(starts) || matches!(just, JUST_LEFT | JUST_LEFT_DEFAULT) { return 0; }
    let (a, b) = (usize::from(starts[line]), usize::from(starts[line + 1]));
    let lw = span_width(text, a, b, w);
    let free = width - lw - 1;
    if just == JUST_CENTER { free / 2 } else { free }
}

/// The top-left of the insertion point for `sel`: (v, h), relative to the destination rectangle's top-left.
pub fn char_to_point(text: &[u8], starts: &[u16], sel: usize, line_height: i32, width: i32, just: i16, w: &dyn Fn(u8) -> i32) -> (i32, i32) {
    let line = line_of(text, starts, sel);
    if line >= n_lines(starts) { return (line as i32 * line_height, 0); }
    let ls = usize::from(starts[line]);
    (line as i32 * line_height, line_offset(text, starts, line, width, just, w) + span_width(text, ls, sel, w))
}

/// The character offset nearest the point (`v`, `h` relative to the destination rectangle's top-left). A click below the text
/// lands at the end; the offset never passes the CR (or wrap) that ends a line except on the last line.
pub fn find(text: &[u8], starts: &[u16], v: i32, h: i32, line_height: i32, width: i32, just: i16, w: &dyn Fn(u8) -> i32) -> usize {
    let n = n_lines(starts);
    if n == 0 { return 0; }
    let line = if v < 0 || line_height <= 0 { 0 } else { ((v / line_height) as usize).min(n - 1) };
    let (ls, le) = (usize::from(starts[line]), usize::from(starts[line + 1]));
    let last = line == n - 1;
    let max = if last { le } else { le.saturating_sub(1).max(ls) };
    let mut x = line_offset(text, starts, line, width, just, w);
    for i in ls..max {
        let cwid = cw(text[i], w);
        if h < x + (cwid + 1) / 2 { return i; }
        x += cwid;
    }
    max
}

/// The word around `pos`: a run of non-separators, or the run of separators when `pos` is on one.
pub fn word_at(text: &[u8], pos: usize) -> (usize, usize) {
    let pos = pos.min(text.len());
    let on_break = text.get(pos).is_some_and(|&c| is_break(c));
    let same = |c: u8| is_break(c) == on_break;
    let mut a = pos;
    while a > 0 && same(text[a - 1]) { a -= 1; }
    let mut b = pos;
    while b < text.len() && same(text[b]) { b += 1; }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w8(_: u8) -> i32 { 8 }

    #[test]
    fn lines_wrap_at_the_last_word_break_and_a_cr_ends_a_line() {
        // 40 wide = 5 chars: "hello world" -> "hello " / "world"
        assert_eq!(line_starts(b"hello world", 40, 1, &w8), [0, 6, 11]);
        assert_eq!(line_starts(b"ab\rcd", 40, 1, &w8), [0, 3, 5]);
        // no word break inside a long word: break at the overflow
        assert_eq!(line_starts(b"abcdefghij", 40, 1, &w8), [0, 5, 10]);
        assert_eq!(line_starts(b"", 40, 1, &w8), [0], "empty text has no lines");
    }

    #[test]
    fn crlf_only_mode_never_wraps_and_spaces_hang_at_the_edge() {
        assert_eq!(line_starts(b"hello world\rx", 16, -1, &w8), [0, 12, 13]);
        // the overflowing char is a space: the line takes the run of spaces
        assert_eq!(line_starts(b"abcde   fgh", 40, 1, &w8), [0, 8, 11]);
    }

    #[test]
    fn a_line_advances_at_least_one_character() {
        assert_eq!(line_starts(b"abc", 3, 1, &w8), [0, 1, 2, 3]);
    }

    #[test]
    fn points_and_hits_round_trip_and_trailing_cr_puts_the_caret_below() {
        let t = b"ab\rcd";
        let s = line_starts(t, 100, 1, &w8);
        assert_eq!(char_to_point(t, &s, 1, 10, 100, 0, &w8), (0, 8));
        assert_eq!(char_to_point(t, &s, 4, 10, 100, 0, &w8), (10, 8));
        for sel in 0..=t.len() {
            let (v, h) = char_to_point(t, &s, sel, 10, 100, 0, &w8);
            if sel != 2 { assert_eq!(find(t, &s, v + 1, h, 10, 100, 0, &w8), sel, "sel {sel}"); }
        }
        let t2 = b"ab\r";
        let s2 = line_starts(t2, 100, 1, &w8);
        assert_eq!(char_to_point(t2, &s2, 3, 10, 100, 0, &w8), (10, 0), "after a trailing CR the caret is on the next line");
        assert_eq!(find(t, &s, 5, 999, 10, 100, 0, &w8), 2, "right of a CR-ended line is before the CR");
        assert_eq!(find(t, &s, 500, 999, 10, 100, 0, &w8), 5, "below the text is the end");
    }

    #[test]
    fn centre_and_right_justification_offset_the_line() {
        let t = b"ab";
        let s = line_starts(t, 100, 1, &w8);
        assert_eq!(char_to_point(t, &s, 0, 10, 100, JUST_CENTER, &w8).1, (100 - 16 - 1) / 2);
        assert_eq!(char_to_point(t, &s, 0, 10, 100, JUST_RIGHT, &w8).1, 100 - 16 - 1);
        assert_eq!(char_to_point(t, &s, 0, 10, 100, JUST_LEFT, &w8).1, 0);
    }

    #[test]
    fn words() {
        assert_eq!(word_at(b"foo bar", 1), (0, 3));
        assert_eq!(word_at(b"foo bar", 3), (3, 4));
        assert_eq!(word_at(b"foo bar", 7), (4, 7));
    }
}
