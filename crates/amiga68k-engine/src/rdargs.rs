//! dos.library ReadArgs / FreeArgs: AmigaDOS command-template parsing (dos/rdargs.h).
//!
//! A template is comma-separated items `NAME[=ALIAS...][/A][/K][/S][/N][/T][/M][/F]`. [`parse`] is the pure half
//! (template + argument line -> one [`Value`] per item, or the DOS error), unit-tested on the host; [`read_args`]
//! stores the values into the caller's LONG array, with strings, numbers and /M vectors in one guest buffer that
//! [`free_args`] releases. Items the line does not mention keep the caller's defaults, as on AmigaDOS.
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::vec;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_READ_ARGS: i32 = -798;
pub const LVO_FREE_ARGS: i32 = -858;

/// dos/dos.h error codes ReadArgs reports through IoErr.
pub const ERROR_BAD_NUMBER: u32 = 115;
pub const ERROR_REQUIRED_ARG_MISSING: u32 = 116;
pub const ERROR_TOO_MANY_ARGS: u32 = 118;
pub const ERROR_LINE_TOO_LONG: u32 = 120;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item { pub names: Vec<String>, pub required: bool, pub keyword: bool, pub switch: bool, pub number: bool, pub toggle: bool, pub multi: bool, pub rest: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value { Absent, Str(String), Switch(bool), Num(i32), Multi(Vec<String>) }

pub fn parse_template(t: &str) -> Vec<Item> {
    t.split(',').map(|it| {
        let mut parts = it.split('/');
        let mut item = Item { names: parts.next().unwrap_or("").split('=').map(|n| n.to_ascii_uppercase()).collect(), ..Default::default() };
        for f in parts {
            match f.to_ascii_uppercase().as_str() {
                "A" => item.required = true, "K" => item.keyword = true, "S" => item.switch = true, "N" => item.number = true,
                "T" => item.toggle = true, "M" => item.multi = true, "F" => item.rest = true, _ => {}
            }
        }
        item
    }).collect()
}

/// Split an argument line into words: blanks separate, double quotes group (`*"` is a quote, `*N` a newline).
pub fn words(line: &str) -> Vec<String> {
    let mut out = vec![]; let mut cur = String::new(); let mut quoted = false; let mut any = false;
    let mut it = line.trim_end_matches(['\n', '\r']).chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => { quoted = !quoted; any = true; }
            '*' if quoted => match it.next() { Some('N') | Some('n') => cur.push('\n'), Some('E') | Some('e') => cur.push('\x1b'), Some(o) => cur.push(o), None => {} },
            ' ' | '\t' if !quoted => { if any || !cur.is_empty() { out.push(core::mem::take(&mut cur)); any = false; } }
            _ => { cur.push(c); any = true; }
        }
    }
    if any || !cur.is_empty() { out.push(cur); }
    out
}

/// Match `line` against `template`. Err = the IoErr code.
pub fn parse(template: &str, line: &str) -> Result<Vec<Value>, u32> {
    let items = parse_template(template);
    let mut vals = vec![Value::Absent; items.len()];
    let find = |w: &str| -> Option<(usize, Option<String>)> {
        let (k, v) = match w.split_once('=') { Some((k, v)) => (k, Some(v.to_string())), None => (w, None) };
        items.iter().position(|it| it.names.iter().any(|n| n.eq_ignore_ascii_case(k))).map(|i| (i, v))
    };
    let ws = words(line);
    let mut i = 0;
    let mut positional: Vec<String> = vec![];
    while i < ws.len() {
        let w = &ws[i];
        if let Some((ix, inline)) = find(w) {
            let it = &items[ix];
            if it.switch { vals[ix] = Value::Switch(true); i += 1; continue; }
            if it.toggle { vals[ix] = Value::Switch(true); i += 1; continue; }
            let v = match inline { Some(v) => v, None => { i += 1; ws.get(i).cloned().ok_or(ERROR_REQUIRED_ARG_MISSING)? } };
            if it.rest { let rest: Vec<String> = core::iter::once(v).chain(ws[i + 1..].iter().cloned()).collect(); vals[ix] = Value::Str(rest.join(" ")); i = ws.len(); continue; }
            vals[ix] = if it.multi { let mut m = match core::mem::replace(&mut vals[ix], Value::Absent) { Value::Multi(m) => m, _ => vec![] }; m.push(v); Value::Multi(m) } else { Value::Str(v) };
            i += 1;
        } else { positional.push(w.clone()); i += 1; }
    }
    // Positional words fill, in template order, the items that are not /K /S /T and are still unset; a /M item takes
    // all the remaining ones; a /F item takes the rest of the line.
    let mut p = positional.into_iter().peekable();
    for (ix, it) in items.iter().enumerate() {
        if it.keyword || it.switch || it.toggle || p.peek().is_none() { continue; }
        match &vals[ix] {
            Value::Absent => {}
            Value::Multi(_) if it.multi => {}
            _ => continue,
        }
        vals[ix] = if it.multi { let mut m = match core::mem::replace(&mut vals[ix], Value::Absent) { Value::Multi(m) => m, _ => vec![] }; m.extend(p.by_ref()); Value::Multi(m) }
            else if it.rest { Value::Str(p.by_ref().collect::<Vec<_>>().join(" ")) } else { Value::Str(p.next().unwrap()) };
    }
    if p.peek().is_some() { return Err(ERROR_TOO_MANY_ARGS); }
    for (ix, it) in items.iter().enumerate() {
        if it.number { if let Value::Str(s) = &vals[ix] { vals[ix] = Value::Num(s.parse::<i32>().map_err(|_| ERROR_BAD_NUMBER)?); } }
        if it.required && vals[ix] == Value::Absent { return Err(ERROR_REQUIRED_ARG_MISSING); }
    }
    Ok(vals)
}

/// struct RDArgs (dos/rdargs.h): RDA_Source (CSource, 12 bytes), RDA_DAList 12, RDA_Buffer 16, RDA_BufSiz 20,
/// RDA_ExtHelp 24, RDA_Flags 28; 32 bytes. This engine keeps its value buffer's address in RDA_DAList.
const RDARGS_SIZE: u32 = 32;
const RDA_DALIST: u32 = 12;
const DOSTRUE: u32 = 0xFFFF_FFFF;
const PR_RESULT2: u32 = 148;

/// ReadArgs(d1 = template, d2 = LONG array, d3 = RDArgs or NULL) -> RDArgs or NULL (IoErr says why).
pub fn read_args<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let m = Refusal::mem;
    let (tp, array, given) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let template = a.mem.read_cstr(tp).map_err(m("ReadArgs.template"))?;
    let line = a.mem.read_cstr(crate::process::arg_ptr(a)).map_err(m("argument line"))?;
    let vals = match parse(&template, &line) {
        Ok(v) => v,
        Err(code) => {
            a.mem.write_u32(a.sched.cur + PR_RESULT2, code).map_err(m("pr_Result2"))?;
            cpu.set_d0(0);
            return Ok(format!("[dos] ReadArgs \"{template}\" line={line:?} -> NULL (IoErr {code})"));
        }
    };
    // One guest buffer for every string, number and /M vector this call stores.
    let mut need = 0u32;
    for v in &vals { need += match v { Value::Str(s) => s.len() as u32 + 1, Value::Num(_) => 4, Value::Multi(m) => 4 * (m.len() as u32 + 1) + m.iter().map(|s| s.len() as u32 + 1).sum::<u32>(), _ => 0 }; }
    let buf = if need == 0 { 0 } else { a.heap.alloc(need).ok_or_else(|| Refusal("ReadArgs: no memory for values (NOMEM)".into()))? };
    let rd = if given != 0 { given } else { a.heap.alloc(RDARGS_SIZE).ok_or_else(|| Refusal("ReadArgs: no memory for RDArgs (NOMEM)".into()))? };
    if given == 0 { for i in 0..RDARGS_SIZE { a.mem.write_u8(rd + i, 0).map_err(m("RDArgs"))?; } }
    a.mem.write_u32(rd + RDA_DALIST, buf).map_err(m("RDA_DAList"))?;
    let mut at = buf;
    let put_str = |a: &mut Amiga<U>, s: &str, at: &mut u32| -> Result<u32, Refusal> {
        let p = *at;
        for (i, b) in s.bytes().chain(core::iter::once(0)).enumerate() { a.mem.write_u8(p + i as u32, b).map_err(Refusal::mem("ReadArgs value"))?; }
        *at += s.len() as u32 + 1; Ok(p)
    };
    for (ix, v) in vals.iter().enumerate() {
        let slot = array + 4 * ix as u32;
        let word = match v {
            Value::Absent => continue,
            Value::Switch(b) => if *b { DOSTRUE } else { 0 },
            Value::Str(s) => put_str(a, s, &mut at)?,
            Value::Num(n) => { let p = at; a.mem.write_u32(p, *n as u32).map_err(m("ReadArgs number"))?; at += 4; p }
            Value::Multi(list) => {
                let vec_at = at; at += 4 * (list.len() as u32 + 1);
                for (k, s) in list.iter().enumerate() { let p = put_str(a, s, &mut at)?; a.mem.write_u32(vec_at + 4 * k as u32, p).map_err(m("ReadArgs /M"))?; }
                a.mem.write_u32(vec_at + 4 * list.len() as u32, 0).map_err(m("ReadArgs /M"))?;
                vec_at
            }
        };
        a.mem.write_u32(slot, word).map_err(m("ReadArgs array"))?;
    }
    a.rdargs.push((rd, buf, need, given == 0));
    cpu.set_d0(rd);
    Ok(format!("[dos] ReadArgs \"{template}\" line={line:?} -> {rd:#x} {vals:?}"))
}

/// FreeArgs(d1 = RDArgs from ReadArgs, or NULL).
pub fn free_args<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rd = cpu.dreg(1);
    if rd == 0 { return Ok("[dos] FreeArgs NULL".into()); }
    let k = a.rdargs.iter().position(|r| r.0 == rd).ok_or_else(|| Refusal(format!("FreeArgs {rd:#x}: not from ReadArgs")))?;
    let (_, buf, need, ours) = a.rdargs.remove(k);
    if buf != 0 { a.heap.free(buf, need).map_err(|e| Refusal(format!("FreeArgs: {e}")))?; }
    if ours { a.heap.free(rd, RDARGS_SIZE).map_err(|e| Refusal(format!("FreeArgs: {e}")))?; }
    Ok(format!("[dos] FreeArgs {rd:#x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(x: &str) -> Value { Value::Str(x.into()) }
    #[test]
    fn templates_match_like_amigados() {
        let t = "FROM/A/M,TO/K,ALL/S,BUF=BUFFER/K/N,QUIET/S";
        assert_eq!(parse(t, "a b TO c ALL BUFFER 20\n").unwrap(), vec![Value::Multi(vec!["a".into(), "b".into()]), s("c"), Value::Switch(true), Value::Num(20), Value::Absent]);
        assert_eq!(parse(t, "\n"), Err(ERROR_REQUIRED_ARG_MISSING));
        assert_eq!(parse("NAME/A,COUNT/N", "x y"), Err(ERROR_BAD_NUMBER));
        assert_eq!(parse("NAME", "x y"), Err(ERROR_TOO_MANY_ARGS));
        assert_eq!(parse("PORT/K,HOST", "PORT=8080 \"my host\""), Ok(vec![s("8080"), s("my host")]));
        assert_eq!(parse("CMD/F", "echo hi there"), Ok(vec![s("echo hi there")]));
        assert_eq!(parse("NAME,QUIET/S", "\n"), Ok(vec![Value::Absent, Value::Absent]), "absent keeps the caller's defaults");
    }
}
