//! AmigaDOS wildcard patterns, as pure data: `tokenize` (what ParsePattern does), `Pattern::from_tokens` and
//! `Pattern::matches` (MatchPattern). No guest memory here.
//!
//! Syntax (dos.doc ParsePattern): `?` any one character; `#p` zero or more repetitions of the item p, so `#?` is any
//! string; `(a|b)` alternatives; `~p` anything that does not match the item p; `[a-z]` / `[~a-z]` a (negated) class;
//! `%` the empty string (so `#?(%|.info)` makes sense); `'` makes the next character literal.
//!
//! The tokenised form is the pattern with every metacharacter replaced by one of dos/dosasl.h's P_* tokens
//! (P_ANY 0x80 for `#?`, P_SINGLE 0x81, P_ORSTART 0x82, P_ORNEXT 0x83, P_OREND 0x84, P_NOT 0x85, P_NOTCLASS 0x87,
//! P_CLASS 0x88, P_REPBEG 0x89, P_REPEND 0x8A, P_STOP 0x8B). It is FLAT: `~` and `#` are prefix operators over the next
//! item and have no closing token, a class is `P_CLASS body P_REPEND`, and `%` is the engine-private token 0x8C. A literal
//! byte that collides with a token (0x80-0x8C) or with `'` is written `'` + byte. The tokenised bytes are an opaque
//! handle to guests, who only pass them back to MatchPattern.
//!
//! Matching is backtracking with a step budget, so a pathological pattern is an error, not a hang.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

pub const P_ANY: u8 = 0x80;
pub const P_SINGLE: u8 = 0x81;
pub const P_ORSTART: u8 = 0x82;
pub const P_ORNEXT: u8 = 0x83;
pub const P_OREND: u8 = 0x84;
pub const P_NOT: u8 = 0x85;
pub const P_NOTCLASS: u8 = 0x87;
pub const P_CLASS: u8 = 0x88;
pub const P_REPBEG: u8 = 0x89;
pub const P_REPEND: u8 = 0x8A;
pub const P_STOP: u8 = 0x8B;
pub const P_NULL: u8 = 0x8C;
/// Steps one match may take before it is given up as pathological.
pub const STEP_BUDGET: u32 = 2_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError { Syntax(&'static str), TooExpensive }

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok { Lit(u8), Single, Any, OrStart, OrNext, OrEnd, Not, Rep, Null, Class { neg: bool, ranges: Vec<(u8, u8)> } }

fn is_token(b: u8) -> bool { (P_ANY..=P_NULL).contains(&b) }
fn needs_escape(b: u8) -> bool { is_token(b) || b == b'\'' }

/// Source text -> tokens. `'` escapes; `#?` is one Any.
fn lex_source(src: &[u8]) -> Result<Vec<Tok>, PatternError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        out.push(match c {
            b'\'' => { let n = *src.get(i).ok_or(PatternError::Syntax("pattern ends after '"))?; i += 1; Tok::Lit(n) }
            b'?' => Tok::Single,
            b'#' if src.get(i) == Some(&b'?') => { i += 1; Tok::Any }
            b'#' => Tok::Rep,
            b'(' => Tok::OrStart,
            b'|' => Tok::OrNext,
            b')' => Tok::OrEnd,
            b'~' => Tok::Not,
            b'%' => Tok::Null,
            b'[' => {
                let neg = src.get(i) == Some(&b'~');
                if neg { i += 1; }
                let mut ranges = Vec::new();
                loop {
                    let lo = *src.get(i).ok_or(PatternError::Syntax("unterminated ["))?;
                    i += 1;
                    if lo == b']' { break; }
                    let lo = if lo == b'\'' { let n = *src.get(i).ok_or(PatternError::Syntax("unterminated ["))?; i += 1; n } else { lo };
                    if src.get(i) == Some(&b'-') && src.get(i + 1).map_or(false, |&n| n != b']') {
                        let mut hi = src[i + 1];
                        i += 2;
                        if hi == b'\'' { hi = *src.get(i).ok_or(PatternError::Syntax("unterminated ["))?; i += 1; }
                        ranges.push((lo.min(hi), lo.max(hi)));
                    } else { ranges.push((lo, lo)); }
                }
                Tok::Class { neg, ranges }
            }
            c => Tok::Lit(c),
        });
    }
    Ok(out)
}

/// Tokenised bytes -> tokens (the inverse of `emit`).
fn lex_tokens(buf: &[u8]) -> Result<Vec<Tok>, PatternError> {
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let c = *buf.get(i).ok_or(PatternError::Syntax("tokenised pattern has no P_STOP"))?;
        i += 1;
        out.push(match c {
            P_STOP => return Ok(out),
            b'\'' => { let n = *buf.get(i).ok_or(PatternError::Syntax("tokenised pattern ends after '"))?; i += 1; Tok::Lit(n) }
            P_ANY => Tok::Any, P_SINGLE => Tok::Single, P_ORSTART => Tok::OrStart, P_ORNEXT => Tok::OrNext, P_OREND => Tok::OrEnd,
            P_NOT => Tok::Not, P_REPBEG => Tok::Rep, P_NULL => Tok::Null,
            P_CLASS | P_NOTCLASS => {
                let mut ranges = Vec::new();
                loop {
                    let mut lo = *buf.get(i).ok_or(PatternError::Syntax("unterminated class token"))?;
                    i += 1;
                    if lo == P_REPEND { break; }
                    if lo == b'\'' { lo = *buf.get(i).ok_or(PatternError::Syntax("unterminated class token"))?; i += 1; }
                    if buf.get(i) == Some(&b'-') {
                        let mut hi = *buf.get(i + 1).ok_or(PatternError::Syntax("unterminated class token"))?;
                        i += 2;
                        if hi == b'\'' { hi = *buf.get(i).ok_or(PatternError::Syntax("unterminated class token"))?; i += 1; }
                        ranges.push((lo, hi));
                    } else { ranges.push((lo, lo)); }
                }
                Tok::Class { neg: c == P_NOTCLASS, ranges }
            }
            c if is_token(c) => return Err(PatternError::Syntax("a stray token in a tokenised pattern")),
            c => Tok::Lit(c),
        });
    }
}

fn emit(toks: &[Tok]) -> Vec<u8> {
    let mut o = Vec::new();
    let lit = |o: &mut Vec<u8>, b: u8| { if needs_escape(b) || b == b'-' { o.push(b'\''); } o.push(b); };
    for t in toks {
        match t {
            Tok::Lit(b) => { if needs_escape(*b) { o.push(b'\''); } o.push(*b) }
            Tok::Single => o.push(P_SINGLE), Tok::Any => o.push(P_ANY), Tok::OrStart => o.push(P_ORSTART), Tok::OrNext => o.push(P_ORNEXT),
            Tok::OrEnd => o.push(P_OREND), Tok::Not => o.push(P_NOT), Tok::Rep => o.push(P_REPBEG), Tok::Null => o.push(P_NULL),
            Tok::Class { neg, ranges } => {
                o.push(if *neg { P_NOTCLASS } else { P_CLASS });
                for &(lo, hi) in ranges { lit(&mut o, lo); if hi != lo { o.push(b'-'); lit(&mut o, hi); } }
                o.push(P_REPEND);
            }
        }
    }
    o.push(P_STOP);
    o
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node { Lit(u8), Single, Any, Null, Class { neg: bool, ranges: Vec<(u8, u8)> }, Alt(Vec<Vec<Node>>), Rep(Box<Node>), Not(Box<Node>) }

fn parse_seq(t: &[Tok], i: &mut usize) -> Result<Vec<Node>, PatternError> {
    let mut seq = Vec::new();
    while *i < t.len() && !matches!(t[*i], Tok::OrNext | Tok::OrEnd) { seq.push(parse_item(t, i)?); }
    Ok(seq)
}

fn parse_item(t: &[Tok], i: &mut usize) -> Result<Node, PatternError> {
    let tok = t.get(*i).ok_or(PatternError::Syntax("# or ~ with nothing after it"))?.clone();
    *i += 1;
    Ok(match tok {
        Tok::Lit(b) => Node::Lit(b),
        Tok::Single => Node::Single,
        Tok::Any => Node::Any,
        Tok::Null => Node::Null,
        Tok::Class { neg, ranges } => Node::Class { neg, ranges },
        Tok::Rep => Node::Rep(Box::new(parse_item(t, i)?)),
        Tok::Not => Node::Not(Box::new(parse_item(t, i)?)),
        Tok::OrStart => {
            let mut alts = vec![parse_seq(t, i)?];
            loop {
                match t.get(*i) {
                    Some(Tok::OrNext) => { *i += 1; alts.push(parse_seq(t, i)?); }
                    Some(Tok::OrEnd) => { *i += 1; break; }
                    _ => return Err(PatternError::Syntax("( without a closing )")),
                }
            }
            Node::Alt(alts)
        }
        Tok::OrNext => return Err(PatternError::Syntax("| outside parentheses")),
        Tok::OrEnd => return Err(PatternError::Syntax(") without a matching (")),
    })
}

fn parse(toks: &[Tok]) -> Result<Vec<Node>, PatternError> {
    let mut i = 0;
    let seq = parse_seq(toks, &mut i)?;
    if i != toks.len() { return Err(PatternError::Syntax("unbalanced parentheses")); }
    Ok(seq)
}

/// ParsePattern: the tokenised form of `src` (ending in P_STOP) and whether it contains any wildcard.
/// Length of the tokenised pattern at the start of `buf`, including its P_STOP (escapes skipped), or None.
pub fn tokenised_len(buf: impl Fn(usize) -> Option<u8>) -> Option<usize> {
    let mut i = 0;
    loop {
        match buf(i)? { P_STOP => return Some(i + 1), b'\'' => i += 2, _ => i += 1 }
    }
}

pub fn tokenize(src: &[u8]) -> Result<(Vec<u8>, bool), PatternError> {
    let toks = lex_source(src)?;
    parse(&toks)?; // reject a pattern MatchPattern could not read
    let wild = toks.iter().any(|t| !matches!(t, Tok::Lit(_)));
    Ok((emit(&toks), wild))
}

/// The text `src` stands for if it has no wildcard at all (escapes removed), else None.
pub fn plain_text(src: &[u8]) -> Result<Option<Vec<u8>>, PatternError> {
    let toks = lex_source(src)?;
    Ok(toks.iter().map(|t| if let Tok::Lit(b) = t { Some(*b) } else { None }).collect())
}

#[derive(Clone)]
pub struct Pattern { seq: Vec<Node>, nocase: bool }

fn fold(b: u8) -> u8 { if b.is_ascii_uppercase() || (0xC0..=0xDE).contains(&b) && b != 0xD7 { b | 0x20 } else { b } }

struct Budget(u32);

type Cont<'a> = &'a mut dyn FnMut(usize, &mut Budget) -> bool;

impl Pattern {
    pub fn from_tokens(buf: &[u8], nocase: bool) -> Result<Pattern, PatternError> {
        Ok(Pattern { seq: parse(&lex_tokens(buf)?)?, nocase })
    }

    pub fn matches(&self, s: &[u8]) -> Result<bool, PatternError> {
        let mut b = Budget(STEP_BUDGET);
        let r = self.seq_m(&self.seq, s, 0, &mut |e, _| e == s.len(), &mut b);
        if b.0 == 0 { Err(PatternError::TooExpensive) } else { Ok(r) }
    }

    fn eq(&self, a: u8, b: u8) -> bool { if self.nocase { fold(a) == fold(b) } else { a == b } }

    fn class_hit(&self, ranges: &[(u8, u8)], c: u8) -> bool {
        ranges.iter().any(|&(lo, hi)| if self.nocase { let f = fold(c); (lo..=hi).contains(&c) || (lo..=hi).contains(&f) || (lo..=hi).any(|x| fold(x) == f) } else { (lo..=hi).contains(&c) })
    }

    fn seq_m(&self, nodes: &[Node], s: &[u8], pos: usize, k: Cont, b: &mut Budget) -> bool {
        if b.0 == 0 { return false; }
        b.0 -= 1;
        let Some((first, rest)) = nodes.split_first() else { return k(pos, b) };
        match first {
            Node::Lit(c) => pos < s.len() && self.eq(s[pos], *c) && self.seq_m(rest, s, pos + 1, k, b),
            Node::Single => pos < s.len() && self.seq_m(rest, s, pos + 1, k, b),
            Node::Null => self.seq_m(rest, s, pos, k, b),
            Node::Class { neg, ranges } => pos < s.len() && self.class_hit(ranges, s[pos]) != *neg && self.seq_m(rest, s, pos + 1, k, b),
            Node::Any => (pos..=s.len()).any(|e| self.seq_m(rest, s, e, k, b)),
            Node::Alt(alts) => alts.iter().any(|alt| self.seq_m(alt, s, pos, &mut |e, b: &mut Budget| self.seq_m(rest, s, e, k, b), b)),
            Node::Rep(item) => self.rep(item, rest, s, pos, k, b),
            Node::Not(item) => (pos..=s.len()).any(|e| {
                let whole = self.seq_m(core::slice::from_ref(item), &s[pos..e], 0, &mut |x, _| x == e - pos, b);
                !whole && self.seq_m(rest, s, e, k, b)
            }),
        }
    }

    fn rep(&self, item: &Node, rest: &[Node], s: &[u8], pos: usize, k: Cont, b: &mut Budget) -> bool {
        if self.seq_m(rest, s, pos, k, b) { return true; }
        self.seq_m(core::slice::from_ref(item), s, pos, &mut |e, b: &mut Budget| e > pos && self.rep(item, rest, s, e, k, b), b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, s: &str) -> bool { Pattern::from_tokens(&tokenize(p.as_bytes()).unwrap().0, false).unwrap().matches(s.as_bytes()).unwrap() }
    fn mi(p: &str, s: &str) -> bool { Pattern::from_tokens(&tokenize(p.as_bytes()).unwrap().0, true).unwrap().matches(s.as_bytes()).unwrap() }

    #[test]
    fn wildcards_behave_as_dos_doc_says() {
        for (p, s, want) in [
            ("#?", "", true), ("#?", "anything.at.all", true), ("?", "", false), ("?", "a", true), ("??", "a", false),
            ("#?.info", "Disk.info", true), ("#?.info", "Disk.inf", false), ("#?.info", ".info", true),
            ("a#b", "abbb", true), ("a#b", "a", true), ("a#b", "abc", false),
            ("(a|b)c", "bc", true), ("(a|b)c", "cc", false), ("(a|bb)#?", "bbzz", true),
            ("#?(%|.info)", "x", true), ("#?(%|.info)", "x.info", true), ("x(%|y)", "xz", false),
            ("[a-c]x", "bx", true), ("[a-c]x", "dx", false), ("[~a-c]x", "dx", true), ("[~a-c]x", "bx", false), ("[abc]", "b", true),
            ("~(a)", "b", true), ("~(a)", "a", false), ("~(a)", "", true), ("~(#?.info)", "x.dat", true), ("~(#?.info)", "x.info", false),
            ("~(a)#?", "bcd", true), ("a~(b)", "ab", false), ("a~(b)", "ac", true),
            ("'#?", "#?", true), ("'#?", "xy", false), ("a'?b", "a?b", true), ("a'?b", "axb", false), ("it''s", "it's", true),
            ("#(ab)", "ababab", true), ("#(ab)", "aba", false), ("#(a|b)", "abba", true),
        ] { assert_eq!(m(p, s), want, "pattern {p:?} against {s:?}"); }
    }

    #[test]
    fn nocase_folds_literals_and_classes_only_when_asked() {
        assert!(mi("#?.INFO", "disk.info"));
        assert!(!m("#?.INFO", "disk.info"), "negative control: the case-sensitive form does not fold");
        assert!(mi("[a-c]", "B"));
        assert!(!m("[a-c]", "B"));
        assert!(mi("(ABC|def)", "DEF"));
    }

    #[test]
    fn tokenize_reports_wildcards_and_rejects_bad_syntax() {
        assert_eq!(tokenize(b"plain.txt").unwrap().1, false);
        assert_eq!(tokenize(b"pl'?in").unwrap().1, false, "an escaped wildcard is plain");
        assert_eq!(tokenize(b"a#?").unwrap().1, true);
        assert_eq!(plain_text(b"pl'?in").unwrap().as_deref(), Some(&b"pl?in"[..]));
        assert_eq!(plain_text(b"pl?in").unwrap(), None);
        assert_eq!(tokenize(b"#?").unwrap().0, [P_ANY, P_STOP]);
        assert_eq!(tokenize(b"a?").unwrap().0, [b'a', P_SINGLE, P_STOP]);
        for bad in ["(a", "a)", "a|b", "#", "~", "[a", "x'"] { assert!(tokenize(bad.as_bytes()).is_err(), "{bad:?} must be a syntax error"); }
        assert!(tokenize(b"(a|b)").is_ok(), "negative control");
    }

    #[test]
    fn literal_bytes_that_collide_with_tokens_survive_the_round_trip() {
        let src = [b'a', 0x81, b'\'', b'\'', 0x8C, b'-'];
        let (t, wild) = tokenize(&src).unwrap();
        assert!(!wild);
        assert!(Pattern::from_tokens(&t, false).unwrap().matches(&[b'a', 0x81, b'\'', 0x8C, b'-']).unwrap());
        assert!(Pattern::from_tokens(b"abc", false).is_err(), "bytes without P_STOP are not a tokenised pattern");
    }

    #[test]
    fn a_pathological_pattern_is_an_error_not_a_hang() {
        let p = Pattern::from_tokens(&tokenize(b"#?#?#?#?#?#?#?#?#?#?x").unwrap().0, false).unwrap();
        assert_eq!(p.matches(&[b'a'; 60]), Err(PatternError::TooExpensive));
    }
}
