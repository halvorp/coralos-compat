//! The Script Manager's line-layout routines by C name (QuickdrawText.h / Script.h: `VisibleLength`, `PortionLine`, `DrawJustified`,
//! `PixelToChar`, `CharToPixel`, `StyledLineBreak`, `HiliteText`, `FindWordBreaks`, `CharacterType`, `CharacterByteType`), for a Mac with
//! the Roman script only, measured with the font QuickDraw really draws (`heirloom-mac68k-engine` qd/text.rs: every font is the topaz-8
//! stand-in, so a program's own line breaking agrees with what it then draws). A program that lays out its own text (Installer VISE's
//! licence view, Photoshop 7) calls these instead of TextEdit.
//!
//! Behaviour (Inside Macintosh: Text, 1993, chapters 3 and 5; nothing here is taken from another implementation's code):
//! - `PortionLine`: the share of a justified line's slop this run takes, `Fixed`: Roman text gives it to its spaces (the count of 0x20),
//!   1.0 when there are none, 0 for an empty run. `DrawJustified` spreads `slop` over the spaces (over the characters when there are
//!   none) exactly as the share says, so `CharToPixel` / `PixelToChar` with the same slop agree with what was drawn.
//! - `StyledLineBreak`: the run's text that fits `*textWidth` (Fixed pixels): smBreakOverflow (2) when all of it fits (`*textWidth` is
//!   then what is left), else smBreakWord (0) after the last space run that fits, else (first run on the line only) smBreakChar (1) after
//!   the last character that fits - at least one; `*textOffset` (input: non-zero = first run on the line) is the break's offset.
//! - `CharacterType`: type smCharPunct 0 / smCharAscii 1 / smCharEuro 7 (bits 0-3), a punctuation class (bits 8-11: normal 0, number
//!   0x100, symbol 0x200, blank 0x300) and smCharUpper 0x4000 for a capital; `CharacterByteType` is smSingleByte (0): Roman is one byte.
//! - `FindWordBreaks`: the word around `offset`: a run of letters and digits (and Roman letters above 0x7F), a run of white space, or one
//!   other character; `HiliteText` is the one span `[first, second)` in the offset table's first pair (Roman text is left to right).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const SM_BREAK_WORD: u32 = 0;
const SM_BREAK_CHAR: u32 = 1;
const SM_BREAK_OVERFLOW: u32 = 2;
const FIXED_ONE: i64 = 0x1_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlFn { VisibleLength, PortionLine, DrawJustified, PixelToChar, CharToPixel, StyledLineBreak, HiliteText, FindWordBreaks, CharacterType, CharacterByteType }

impl TlFn {
    pub fn by_name(name: &str) -> Option<TlFn> {
        use TlFn::*;
        Some(match name {
            "VisibleLength" => VisibleLength, "PortionLine" => PortionLine, "DrawJustified" => DrawJustified, "PixelToChar" => PixelToChar, "CharToPixel" => CharToPixel,
            "StyledLineBreak" => StyledLineBreak, "HiliteText" => HiliteText, "FindWordBreaks" => FindWordBreaks, "CharacterType" => CharacterType,
            "CharacterByteType" => CharacterByteType,
            _ => return None,
        })
    }
}

fn is_space(b: u8) -> bool { matches!(b, b' ' | b'\t') }
/// A Roman word character: a letter or digit, or a Mac Roman letter (0x80..=0xD8 are the accented letters and ligatures, 0xDE / 0xDF fi fl).
fn is_word(b: u8) -> bool { b.is_ascii_alphanumeric() || (0x80..=0x9F).contains(&b) || matches!(b, 0xAE | 0xAF | 0xBE | 0xBF | 0xCB | 0xCC | 0xCD | 0xCE | 0xCF | 0xD8 | 0xDE | 0xDF | 0xE5..=0xEF | 0xF1..=0xF4) }

/// Trailing blanks and tabs are not visible (the 68k `_ScriptUtil` VisibleLength answers the same).
pub(crate) fn visible_length(t: &[u8]) -> usize { t.iter().rposition(|&b| !is_space(b)).map_or(0, |i| i + 1) }

/// The share of the slop a run takes, Fixed (see the module docs).
pub(crate) fn portion(t: &[u8]) -> i64 {
    if t.is_empty() { return 0 }
    let spaces = t.iter().filter(|&&b| b == b' ').count() as i64;
    if spaces > 0 { spaces * FIXED_ONE } else { FIXED_ONE }
}

/// Where the line breaks in `t[start..end]` for `avail` pixels: (code, offset, pixels left), `width(i)` the advance of byte `i`.
pub(crate) fn line_break(t: &[u8], start: usize, end: usize, avail: i64, first_on_line: bool, width: impl Fn(u8) -> i64) -> (u32, usize, i64) {
    let (start, end) = (start.min(t.len()), end.min(t.len()).max(start.min(t.len())));
    let (mut used, mut fit) = (0i64, start);
    while fit < end && used + width(t[fit]) <= avail { used += width(t[fit]); fit += 1; }
    if fit == end { return (SM_BREAK_OVERFLOW, end, avail - used) }
    // the last space run that starts inside what fits: the break is after it (trailing blanks hang past the margin)
    let mut word = None;
    let mut i = start;
    while i < fit {
        if is_space(t[i]) { let mut j = i + 1; while j < end && is_space(t[j]) { j += 1; } word = Some(j); i = j; } else { i += 1; }
    }
    let span = |to: usize| t[start..to].iter().map(|&b| width(b)).sum::<i64>();
    match word {
        Some(w) => (SM_BREAK_WORD, w, (avail - span(w)).max(0)),
        None if first_on_line => { let c = fit.max(start + 1).min(end); (SM_BREAK_CHAR, c, (avail - span(c)).max(0)) }
        None => (SM_BREAK_WORD, start, avail),
    }
}

/// The word around `offset` (leading edge: the character AT offset, else the one before it): `[start, end)`.
pub(crate) fn word_at(t: &[u8], offset: usize, leading: bool) -> (usize, usize) {
    if t.is_empty() { return (0, 0) }
    let i = if leading { offset.min(t.len() - 1) } else { offset.saturating_sub(1).min(t.len() - 1) };
    let class = |b: u8| if is_word(b) { 1 } else if is_space(b) || b == b'\r' { 2 } else { 3 };
    let k = class(t[i]);
    if k == 3 { return (i, i + 1) }
    let (mut s, mut e) = (i, i + 1);
    while s > 0 && class(t[s - 1]) == k { s -= 1; }
    while e < t.len() && class(t[e]) == k { e += 1; }
    (s, e)
}

/// `CharacterType` for one Roman byte (see the module docs).
pub(crate) fn char_type(b: u8) -> u32 {
    match b {
        b'A'..=b'Z' => 0x4001,
        b'a'..=b'z' => 0x0001,
        b'0'..=b'9' => 0x0100,
        b' ' | b'\t' | 0xCA => 0x0300,
        0x80..=0x86 | 0xAE | 0xAF | 0xCB..=0xCF | 0xE5..=0xEF | 0xF1..=0xF4 => 0x4007,
        _ if is_word(b) => 0x0007,
        _ => 0x0200,
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn tl_bytes(&self, p: u32, n: u32, who: &str) -> Result<Vec<u8>, String> {
        if n == 0 { return Ok(Vec::new()) }
        self.mac.mem.span(p, n).and_then(|s| self.mac.mem.bytes(s).map(<[u8]>::to_vec)).map_err(|_| format!("{who}: {n} text bytes at {p:#x} leave guest RAM"))
    }
    fn advance(&self, b: u8) -> i64 { i64::from(self.mac.qd.font.text_length(&[b])) }
    /// The pixel offsets of each character edge with `slop` (Fixed) spread by [`portion`]'s rule: `edges[i]` = left edge of byte i (Fixed).
    fn edges(&self, t: &[u8], slop: i64) -> Vec<i64> {
        let spaces = t.iter().filter(|&&b| b == b' ').count() as i64;
        let mut x = 0i64;
        let mut out = Vec::with_capacity(t.len() + 1);
        for &b in t {
            out.push(x);
            x += self.advance(b) * FIXED_ONE;
            if slop != 0 { if spaces > 0 { if b == b' ' { x += slop / spaces; } } else { x += slop / t.len() as i64; } }
        }
        out.push(x);
        out
    }
    fn tl_trap(&mut self, name: &str, values: &[u32]) -> Result<(), String> {
        let op = heirloom_mac68k_engine::mixed::trap_opcode(name).ok_or_else(|| format!("{name} has no trap word"))?;
        self.mac.pascal_call(op, values).map(|_| ()).map_err(|e| format!("{name}: {e:?}"))
    }

    pub(crate) fn tl_call(&mut self, f: TlFn, cpu: &mut Cpu) -> Result<(), String> {
        use TlFn::*;
        let name = format!("{f:?}");
        // the arguments, read once (r3..r10, then the stack parameter area; past the call's own count they are unused)
        let args: Vec<u32> = (0..9).map(|i| arg(self, cpu, i).unwrap_or(0)).collect();
        let a = |i: usize| -> Result<u32, String> { Ok(args[i]) };
        let (p, n) = (a(0)?, a(1)?);
        let r: u32 = match f {
            VisibleLength => visible_length(&self.tl_bytes(p, n, &name)?) as u32,
            PortionLine => portion(&self.tl_bytes(p, n, &name)?) as u32,
            // DrawJustified(textPtr, textLength, slop, styleRunPosition, numer, denom): the run from the pen, its spaces (else characters) widened
            DrawJustified => {
                let (t, slop) = (self.tl_bytes(p, n, &name)?, i64::from(a(2)? as i32));
                if slop == 0 || t.is_empty() { self.tl_trap("DrawText", &[p, 0, n])?; } else {
                    let e = self.edges(&t, slop);
                    let (mut drawn, mut i) = (0i64, 0usize);
                    while i < t.len() {
                        let mut j = i + 1;
                        while j < t.len() && t[j] != b' ' && t[j - 1] != b' ' { j += 1; }
                        // move the pen to where this piece starts with the slop so far, then draw it plainly
                        let want = e[i] >> 16;
                        if want != drawn { self.tl_trap("Move", &[(want - drawn) as i16 as u16 as u32, 0])?; }
                        self.tl_trap("DrawText", &[p, i as u32, (j - i) as u32])?;
                        drawn = want + t[i..j].iter().map(|&b| self.advance(b)).sum::<i64>();
                        i = j;
                    }
                    let end = e[t.len()] >> 16;
                    if end != drawn { self.tl_trap("Move", &[(end - drawn) as i16 as u16 as u32, 0])?; }
                }
                0
            }
            // PixelToChar(textBuf, textLength, slop, pixelWidth, &leadingEdge, &widthRemaining, styleRunPosition, numer, denom)
            PixelToChar => {
                let (t, slop, px, lead_p, rem_p) = (self.tl_bytes(p, n, &name)?, i64::from(a(2)? as i32), i64::from(a(3)? as i32), a(4)?, a(5)?);
                let e = self.edges(&t, slop);
                let total = *e.last().unwrap_or(&0);
                let (off, lead, rem) = if px >= total { (t.len(), false, px - total) } else if px < 0 { (0, true, -1) } else {
                    let i = e.windows(2).position(|w| px < w[1]).unwrap_or(0);
                    // the nearer edge of character i: its leading half is offset i, its trailing half offset i + 1
                    if px - e[i] < (e[i + 1] - e[i]) / 2 { (i, true, -1) } else { (i + 1, false, -1) }
                };
                let mut w = |at: u32, v: u32, sz: u8| if at == 0 { Ok(()) } else if sz == 1 { self.mac.mem.write_u8(at, v as u8) } else { self.mac.mem.write_u32(at, v) };
                w(lead_p, u32::from(lead), 1).and_then(|_| w(rem_p, if rem < 0 { (-FIXED_ONE) as u32 } else { rem as u32 }, 4)).map_err(|_| format!("{name}: result pointers leave guest RAM"))?;
                off as u32
            }
            // CharToPixel(textBuf, textLength, slop, offset, direction, styleRunPosition, numer, denom) -> pixels (short)
            CharToPixel => {
                let (t, slop, off) = (self.tl_bytes(p, n, &name)?, i64::from(a(2)? as i32), a(3)? as usize);
                let e = self.edges(&t, slop);
                ((e[off.min(t.len())] >> 16) as i16) as u16 as u32
            }
            // StyledLineBreak(textPtr, textLen, textStart, textEnd, flags, Fixed *textWidth, long *textOffset)
            StyledLineBreak => {
                let (t, start, end, w_p, o_p) = (self.tl_bytes(p, n, &name)?, a(2)? as usize, a(3)? as usize, a(5)?, a(6)?);
                let m = &self.mac.mem;
                let (avail, first) = (i64::from(m.read_u32(w_p).map_err(|_| format!("{name}: textWidth leaves guest RAM"))? as i32), m.read_u32(o_p).map_err(|_| format!("{name}: textOffset leaves guest RAM"))? != 0);
                let (code, off, left) = line_break(&t, start, end, avail >> 16, first, |b| self.advance(b));
                self.mac.mem.write_u32(w_p, (left << 16) as u32).and_then(|_| self.mac.mem.write_u32(o_p, off as u32)).map_err(|_| format!("{name}: results leave guest RAM"))?;
                code
            }
            // HiliteText(textPtr, short textLength, short firstOffset, short secondOffset, OffsetTable offsets)
            HiliteText => {
                let (first, second, tab) = (a(2)? as i16, a(3)? as i16, a(4)?);
                let (lo, hi) = (first.min(second).max(0), first.max(second).min(n as i16));
                let mut v = Vec::with_capacity(12);
                for (s, e) in [(lo, hi), (0, 0), (0, 0)] { v.extend_from_slice(&s.to_be_bytes()); v.extend_from_slice(&e.to_be_bytes()); }
                self.mac.mem.write_bytes(tab, &v).map_err(|_| format!("{name}: the OffsetTable leaves guest RAM"))?;
                0
            }
            // FindWordBreaks(textPtr, textLength, offset, leadingEdge, breaks, offsets, script): the Roman word rule, no break table needed
            FindWordBreaks => {
                let (t, off, lead, tab) = (self.tl_bytes(p, n, &name)?, a(2)? as usize, a(3)? & 0xFF != 0, a(5)?);
                let (s, e) = word_at(&t, off, lead);
                let mut v = Vec::with_capacity(12);
                for (x, y) in [(s as i16, e as i16), (0, 0), (0, 0)] { v.extend_from_slice(&x.to_be_bytes()); v.extend_from_slice(&y.to_be_bytes()); }
                self.mac.mem.write_bytes(tab, &v).map_err(|_| format!("{name}: the OffsetTable leaves guest RAM"))?;
                0
            }
            CharacterType => char_type(self.mac.mem.read_u8(p.wrapping_add(n as u16 as i16 as i32 as u32)).map_err(|_| format!("{name}: the character leaves guest RAM"))?),
            CharacterByteType => 0,
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
