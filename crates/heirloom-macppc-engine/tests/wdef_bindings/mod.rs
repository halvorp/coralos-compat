//! TEST-ONLY reader of window-definition binding files (`bindings/macppc/*.sexp`, the one s-expression grammar: lists, symbols,
//! "strings", decimal integers, `;` comments). The product's host reads the same files with CoralOS's s-expression reader and hands
//! the engine the same values (`MacPpc::add_window_def_binding`); this exists so the harness and tests load the DATA, not a copy of it.
use heirloom_bind::{WindowDefBinding, WindowDefIdentity, WindowMeaning};
use heirloom_ui_abi::WindowRole;

#[derive(Debug, Clone)]
pub enum S { List(Vec<S>), Atom(String), Str(String) }

fn parse(src: &str) -> Result<Vec<S>, String> {
    let mut stack: Vec<Vec<S>> = vec![vec![]];
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            ';' => { while let Some(&d) = it.peek() { if d == '\n' { break; } it.next(); } }
            '(' => stack.push(vec![]),
            ')' => { let l = stack.pop().ok_or("unbalanced )")?; stack.last_mut().ok_or("unbalanced )")?.push(S::List(l)); }
            '"' => { let mut t = String::new(); loop { match it.next() { Some('"') => break, Some(d) => t.push(d), None => return Err("unterminated string".into()) } } stack.last_mut().unwrap().push(S::Str(t)); }
            c if c.is_whitespace() => {}
            c => { let mut t = String::from(c); while let Some(&d) = it.peek() { if d.is_whitespace() || d == '(' || d == ')' { break; } t.push(d); it.next(); } stack.last_mut().unwrap().push(S::Atom(t)); }
        }
    }
    if stack.len() != 1 { return Err("unbalanced (".into()); }
    Ok(stack.pop().unwrap())
}

fn field<'a>(l: &'a [S], name: &str) -> Result<&'a [S], String> {
    l.iter().find_map(|s| match s { S::List(v) if matches!(v.first(), Some(S::Atom(a)) if a == name) => Some(&v[1..]), _ => None }).ok_or_else(|| format!("no ({name} ...)"))
}
fn int(l: &[S], name: &str) -> Result<u64, String> {
    match field(l, name)?.first() {
        Some(S::Atom(a)) => a.parse().map_err(|_| format!("({name} {a}): not a decimal integer")),
        Some(S::Str(t)) if t.starts_with("0x") => u64::from_str_radix(&t[2..], 16).map_err(|_| format!("({name} \"{t}\"): not hex")),
        x => Err(format!("({name} {x:?}): not an integer")),
    }
}
fn boolean(l: &[S], name: &str) -> Result<bool, String> {
    match field(l, name)?.first() { Some(S::Atom(a)) if a == "true" => Ok(true), Some(S::Atom(a)) if a == "false" => Ok(false), x => Err(format!("({name} {x:?}): not a boolean")) }
}

/// Every `(window-definition ...)` in `src`; a malformed one is an Err naming what is wrong.
pub fn read(src: &str) -> Result<Vec<WindowDefBinding>, String> {
    let mut out = vec![];
    for top in parse(src)? {
        let S::List(v) = top else { return Err("a top-level atom".into()) };
        if !matches!(v.first(), Some(S::Atom(a)) if a == "window-definition") { continue; }
        let id = match field(&v, "id")?.first() { Some(S::Str(t)) => t.clone(), x => return Err(format!("(id {x:?})")) };
        let fc = field(&v, "fragment-code")?;
        let identity = WindowDefIdentity::FragmentCode { code_length: int(fc, "length")? as u32, code_fnv1a64: int(fc, "fnv1a64")?, entry_offset: int(fc, "entry-offset")? as u32 };
        let m = field(&v, "meaning")?;
        let role = match field(m, "role")?.first() { Some(S::Atom(a)) if a == "dialog" => WindowRole::Dialog, Some(S::Atom(a)) if a == "document" => WindowRole::Document, Some(S::Atom(a)) if a == "utility" => WindowRole::Utility, x => return Err(format!("(role {x:?})")) };
        out.push(WindowDefBinding { id, identity, meaning: WindowMeaning { role, titled: boolean(m, "titled")?, movable: boolean(m, "movable")?, closable: boolean(m, "closable")?, resizable: boolean(m, "resizable")? } });
    }
    Ok(out)
}

/// The compat repo's own binding files for `platform`.
pub fn load(platform: &str) -> Vec<WindowDefBinding> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bindings").join(platform);
    let mut out = vec![];
    for e in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "sexp") { out.extend(read(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()))); }
    }
    out
}
