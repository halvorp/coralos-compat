//! ExAll / ExAllEnd: whole-directory examination into a caller buffer, with `struct ExAllControl` continuation,
//! `eac_MatchString` filtering and an optional `eac_MatchFunc` hook that runs as GUEST code (guest_call.rs).
//!
//! Layout (m68k gcc -m68020 -mno-align-int over AROS dos/exall.h, 2026-10-18): ExAllControl 16 bytes (eac_Entries 0,
//! eac_LastKey 4, eac_MatchString 8, eac_MatchFunc 12); ExAllData ed_Next 0, ed_Name 4, ed_Type 8, ed_Size 12, ed_Prot
//! 16, ed_Days 20, ed_Mins 24, ed_Ticks 28, ed_Comment 32, ed_OwnerUID 36, ed_OwnerGID 38 (40 bytes). The data level
//! ED_NAME..ED_OWNER (1..7) picks how much of that a record holds: 8, 12, 16, 20, 32, 36, 40 bytes. Each record is
//! followed by its name (and comment) strings; ed_Next chains the records. ExAll returns DOSTRUE when the buffer filled
//! and more remain (call again), FALSE with ERROR_NO_MORE_ENTRIES at the end (eac_Entries still counts that last
//! batch), and FALSE with ERROR_BUFFER_OVERFLOW if not even one record fits. The continuation point is HOST state
//! keyed by the control block (eac_LastKey just holds a nonzero token; setting it to 0 restarts, as on AmigaOS).
//! The match string is a ParsePattern result and is matched case-insensitively, as file names are. The match hook is
//! called as A0 = hook, A1 = the record just built, A2 = a pointer to the `data` level, and keeps the record iff D0 is
//! nonzero.
use crate::volume::AmigaDate;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::dos_err::*;
use crate::dos_files::{ST_FILE, ST_USERDIR};
use crate::fs::*;
use crate::guest_call::{start, Next};
use crate::libtable::LibraryBuilder;
use crate::pattern::{tokenised_len, Pattern};
use crate::volume::{Kind, NodeId};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

const ERROR_BAD_NUMBER: DosErr = DosErr(115);
const ERROR_BUFFER_OVERFLOW: DosErr = DosErr(303);
const ED_OWNER: u32 = 7;
const RECORD_SIZE: [u32; 8] = [0, 8, 12, 16, 20, 32, 36, 40];
const H_ENTRY: u32 = 8;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("ExAll", ex_all::<U>).named("ExAllEnd", ex_all_end::<U>)
}

struct Run {
    ctl: u32, kids: Vec<NodeId>, idx: usize, end: u32, cur: u32, last: u32, count: u32, data: u32, pat: Option<Pattern>, hook: u32, scratch: u32,
}

fn align4(x: u32) -> u32 { (x + 3) & !3 }

/// Build the record for `node` at `at`; returns where the next record would start, or None if it does not fit.
fn build<U: GuestUi>(a: &mut Amiga<U>, r: &Run, node: NodeId) -> Result<Option<u32>, Refusal> {
    let m = Refusal::mem("ExAllData");
    let n = a.fs.vol.get(node);
    let (name, comment, prot, date) = (String::from(&n.name), n.comment.clone(), n.attrs, n.date());
    let (ty, size) = match &n.kind { Kind::Dir(_) => (ST_USERDIR, 0), Kind::File(d) => (ST_FILE, d.len() as u32) };
    let rec = RECORD_SIZE[r.data as usize];
    let name_at = r.cur + rec;
    let comment_at = name_at + name.len() as u32 + 1;
    let end = if r.data >= 6 { comment_at + comment.len() as u32 + 1 } else { comment_at };
    if end > r.end { return Ok(None); }
    a.mem.zero(r.cur, end - r.cur).map_err(&m)?;
    a.mem.write_u32(r.cur + 4, name_at).map_err(&m)?;
    crate::guest_str::write_z(&mut a.mem, name_at, name.as_bytes()).map_err(&m)?;
    for (level, off, v) in [(2, 8, ty as u32), (3, 12, size), (4, 16, prot), (5, 20, date[0] as u32), (5, 24, date[1] as u32), (5, 28, date[2] as u32)] {
        if r.data >= level { a.mem.write_u32(r.cur + off, v).map_err(&m)?; }
    }
    if r.data >= 6 {
        crate::guest_str::write_z(&mut a.mem, comment_at, comment.as_bytes()).map_err(&m)?;
        a.mem.write_u32(r.cur + 32, comment_at).map_err(&m)?;
    }
    Ok(Some(align4(end)))
}

fn finish<U: GuestUi>(a: &mut Amiga<U>, r: Run, done: bool) -> Result<Next<U>, Refusal> {
    let m = Refusal::mem("ExAllControl");
    if r.scratch != 0 { a.heap.free(r.scratch, 4).map_err(|e| Refusal(format!("ExAll: {e}")))?; }
    a.mem.write_u32(r.ctl, r.count).map_err(&m)?;
    if done || (r.count == 0) {
        a.fs.exall.remove(&r.ctl);
        a.mem.write_u32(r.ctl + 4, 0).map_err(&m)?;
        let e = if done { ERROR_NO_MORE_ENTRIES } else { ERROR_BUFFER_OVERFLOW };
        set_ioerr(&mut a.mem, e);
        return Ok(Next::Return(0));
    }
    a.fs.exall.insert(r.ctl, (a.fs.exall[&r.ctl].0, r.idx));
    a.mem.write_u32(r.ctl + 4, r.idx as u32 + 1).map_err(&m)?;
    set_ioerr(&mut a.mem, DosErr(0));
    Ok(Next::Return(DOSTRUE))
}

/// Add records until the buffer is full, the directory is exhausted, or a guest hook must be asked.
fn step<U: GuestUi>(a: &mut Amiga<U>, mut r: Run) -> Result<Next<U>, Refusal> {
    loop {
        let Some(&node) = r.kids.get(r.idx) else { return finish(a, r, true) };
        if let Some(p) = &r.pat {
            if !p.matches(a.fs.vol.name(node).as_bytes()).map_err(|e| Refusal(format!("ExAll match string: {e:?}")))? { r.idx += 1; continue; }
        }
        let Some(next_at) = build(a, &r, node)? else { return finish(a, r, false) };
        if r.hook == 0 {
            commit(a, &mut r, next_at)?;
            r.idx += 1;
            continue;
        }
        let entry = a.mem.read_u32(r.hook + H_ENTRY).map_err(Refusal::mem("Hook.h_Entry"))?;
        a.mem.write_u32(r.scratch, r.data).map_err(Refusal::mem("ExAll scratch"))?;
        let (hook, rec, scratch) = (r.hook, r.cur, r.scratch);
        return Ok(Next::Call {
            target: entry, d0: 0, aregs: alloc::vec![(0, hook), (1, rec), (2, scratch)],
            then: Box::new(move |a, _cpu, d0| {
                // The hook answers in D0; the low word is the BOOL, as for any hook.
                if d0 & 0xFFFF != 0 { commit(a, &mut r, next_at)?; }
                r.idx += 1;
                step(a, r)
            }),
        });
    }
}

/// Keep the record just built: chain it and move on.
fn commit<U: GuestUi>(a: &mut Amiga<U>, r: &mut Run, next_at: u32) -> Result<(), Refusal> {
    if r.last != 0 { a.mem.write_u32(r.last, r.cur).map_err(Refusal::mem("ed_Next"))?; }
    r.last = r.cur;
    r.cur = next_at;
    r.count += 1;
    Ok(())
}

/// ExAll(d1 = lock, d2 = buffer, d3 = size, d4 = data level, d5 = ExAllControl) -> DOSTRUE to call again, else FALSE.
fn ex_all<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (lock, buf, size, data, ctl) = (cpu.dreg(1), cpu.dreg(2), cpu.dreg(3), cpu.dreg(4), cpu.dreg(5));
    let what = format!("[dos] ExAll {:#x} level {data} size {size}", lock << 2);
    let m = Refusal::mem("ExAllControl");
    if data == 0 || data > ED_OWNER { return fail(a, cpu, ERROR_BAD_NUMBER, 0, what); }
    let dir = match lock_node(a, lock) { Ok(n) => n, Err(e) => return fail(a, cpu, e, 0, what) };
    if !a.fs.vol.is_dir(dir) { return fail(a, cpu, ERROR_OBJECT_WRONG_TYPE, 0, what); }
    let key = a.mem.read_u32(ctl + 4).map_err(&m)?;
    let idx = if key == 0 { a.fs.exall.remove(&ctl); 0 } else {
        match a.fs.exall.get(&ctl) {
            Some(&(d, i)) if d == dir && key == i as u32 + 1 => i,
            _ => return Err(Refusal(format!("ExAll: eac_LastKey {key} of control {ctl:#x} is not one this call returned"))),
        }
    };
    let (ms, hook) = (a.mem.read_u32(ctl + 8).map_err(&m)?, a.mem.read_u32(ctl + 12).map_err(&m)?);
    let pat = if ms == 0 { None } else {
        let n = tokenised_len(|i| a.mem.read_u8(ms + i as u32).ok().filter(|_| i < 0x2_0000)).ok_or_else(|| Refusal("ExAll: eac_MatchString is not a ParsePattern result".into()))?;
        let t: Vec<u8> = (0..n as u32).map(|i| a.mem.read_u8(ms + i).map_err(&m)).collect::<Result<_, _>>()?;
        Some(Pattern::from_tokens(&t, true).map_err(|e| Refusal(format!("ExAll: eac_MatchString: {e:?}")))?)
    };
    let scratch = if hook != 0 { a.heap.alloc(4).ok_or_else(|| Refusal("ExAll: out of guest memory (NOMEM)".into()))? } else { 0 };
    let kids = a.fs.vol.children(dir).to_vec();
    a.fs.exall.insert(ctl, (dir, idx));
    let run = Run { ctl, kids, idx, end: buf.checked_add(size).ok_or_else(|| Refusal("ExAll: buffer wraps the address space".into()))?, cur: buf, last: 0, count: 0, data, pat, hook, scratch };
    let next = step(a, run)?;
    start(a, cpu, next)?;
    Ok(what)
}

/// ExAllEnd(d1 = lock, d2 = buffer, d3 = size, d4 = data level, d5 = ExAllControl): abandon a scan early.
fn ex_all_end<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ctl = cpu.dreg(5);
    a.fs.exall.remove(&ctl);
    a.mem.write_u32(ctl + 4, 0).map_err(Refusal::mem("ExAllControl"))?;
    Ok(format!("[dos] ExAllEnd {ctl:#x}"))
}
