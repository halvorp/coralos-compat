//! A window's gadget list (`Window.FirstGadget`) and drawing it: AddGList / AddGadget / RemoveGList / RemoveGadget /
//! RefreshGList / RefreshGadgets. List surgery is plain pointer work on the guest's `struct Gadget` chain. BOOPSI gadgets
//! (those the class core made: `Amiga::classes.objs`) get GM_LAYOUT when added and GM_RENDER (gpr_Redraw = GREDRAW_REDRAW,
//! the window's RastPort, a built GadgetInfo) on refresh, each a guest dispatcher call run in sequence; classic and
//! GadTools gadgets draw natively, so the refresh skips them. Facts: AROS rom/intuition addgadget.c, addglist.c,
//! removeglist.c, refreshglist.c; position -1 (0xFFFF as a UWORD) = the end of the list.
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::boopsi_ids::*;
use crate::native_gadgets;
use crate::boopsi_send::make_msg;
use crate::guest_call::{self, Next};
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;

pub const WD_FIRST_GADGET: u32 = 62;

fn rd<U: GuestUi>(a: &Amiga<U>, at: u32, what: &'static str) -> Result<u32, Refusal> { a.mem.read_u32(at).map_err(Refusal::mem(what)) }
fn open<U: GuestUi>(a: &Amiga<U>, win: u32, what: &str) -> Result<(), Refusal> {
    if a.windows.contains_key(&win) { Ok(()) } else { Err(Refusal(format!("{what}: {win:#x} is not an open window"))) }
}

/// The chain starting at `g`, at most `num` long (-1 = to the end). A gadget met twice is a cyclic NextGadget chain, refused by
/// name; the list itself has no length limit.
pub fn chain<U: GuestUi>(a: &Amiga<U>, mut g: u32, num: i32) -> Result<Vec<u32>, Refusal> {
    let (mut v, mut seen) = (Vec::new(), BTreeSet::new());
    while g != 0 && (num < 0 || (v.len() as i32) < num) {
        if !seen.insert(g) { return Err(Refusal(format!("cyclic NextGadget chain: gadget {g:#x} comes back after {} gadgets", v.len()))); }
        v.push(g);
        g = rd(a, g + GAD_NEXT, "Gadget.NextGadget")?;
    }
    Ok(v)
}

/// Every gadget of `win`'s list.
pub fn window_gadgets<U: GuestUi>(a: &Amiga<U>, win: u32) -> Result<Vec<u32>, Refusal> { chain(a, rd(a, win + WD_FIRST_GADGET, "Window.FirstGadget")?, -1) }

/// Link the `num` gadgets from `first` into `win`'s list at `pos` (past the end = the end); returns the position used.
pub(crate) fn link_in<U: GuestUi>(a: &mut Amiga<U>, win: u32, first: u32, pos: u32, num: i32) -> Result<u32, Refusal> {
    let new = chain(a, first, num)?;
    if new.is_empty() { return Ok(pos); }
    let all = window_gadgets(a, win)?;
    if new.iter().any(|g| all.contains(g)) { return Err(Refusal(format!("AddGList: a gadget of the chain is already in window {win:#x}"))); }
    let at = (pos as usize).min(all.len());
    let after = all.get(at).copied().unwrap_or(0);
    let e = Refusal::mem("Gadget.NextGadget");
    a.mem.write_u32(*new.last().unwrap() + GAD_NEXT, after).map_err(&e)?;
    if at == 0 { a.mem.write_u32(win + WD_FIRST_GADGET, new[0]).map_err(Refusal::mem("Window.FirstGadget"))?; }
    else { a.mem.write_u32(all[at - 1] + GAD_NEXT, new[0]).map_err(&e)?; }
    Ok(at as u32)
}

/// AddGList(a0 = window, a1 = gadget, d0 = position, d1 = numGad, a2 = requester) -> the position used.
pub fn add_glist<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, g, pos, num) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as u16 as u32, cpu.dreg(1) as i16 as i32); // numGad is a WORD: -1 = all (the upper half of D1 is the caller's)
    open(a, win, "AddGList")?;
    let used = link_in(a, win, g, pos, num)?;
    // boopsigadgets.c DoGMLayout: only custom gadgets with some relativity are laid out.
    let mut boopsi = Vec::new();
    for x in chain(a, g, num)? {
        let (ty, fl) = (a.mem.read_u16(x + GAD_TYPE).map_err(Refusal::mem("Gadget.GadgetType"))? as u32, a.mem.read_u16(x + GAD_FLAGS).map_err(Refusal::mem("Gadget.Flags"))? as u32);
        if ty & GTYP_GTYPEMASK == GTYP_CUSTOMGADGET && fl & (GFLG_RELSPECIAL | GFLG_RELRIGHT | GFLG_RELBOTTOM | GFLG_RELWIDTH | GFLG_RELHEIGHT) != 0
            && a.classes.class_of(x).is_some() { boopsi.push(x); }
    }
    crate::native_gadgets::sync_chain(a, win, &chain(a, g, num)?)?;
    cpu.set_d0(used);
    let log = format!("[intuition] AddGList {g:#x} into {win:#x} at {used} ({} laid out)", boopsi.len());
    run_methods(a, cpu, Job { win, gadgets: boopsi, i: 0, method: GM_LAYOUT, result: used })?;
    Ok(log)
}

/// AddGList without the guest call: link `num` gadgets from `g` into `win`'s list at `pos` and register their native controls. Used by Request
/// (a requester's gadgets are added to its window, request.c:171).
pub(crate) fn attach<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32, pos: u32, num: i32) -> Result<u32, Refusal> {
    let used = link_in(a, win, g, pos, num)?;
    native_gadgets::sync_chain(a, win, &chain(a, g, num)?)?;
    Ok(used)
}

/// RemoveGList without the guest call (EndRequest takes a requester's gadgets back out).
pub(crate) fn detach<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32, num: i32) -> Result<u32, Refusal> { unlink(a, win, g, num) }

/// AddGadget(a0 = window, a1 = gadget, d0 = position): one gadget.
pub fn add_gadget<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, g, pos) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as u16 as u32);
    open(a, win, "AddGadget")?;
    let nxt = rd(a, g + GAD_NEXT, "Gadget.NextGadget")?;
    a.mem.write_u32(g + GAD_NEXT, 0).map_err(Refusal::mem("Gadget.NextGadget"))?;
    let used = link_in(a, win, g, pos, 1);
    let used = match used { Ok(u) => u, Err(e) => { let _ = a.mem.write_u32(g + GAD_NEXT, nxt); return Err(e) } };
    crate::native_gadgets::sync_chain(a, win, &[g])?;
    cpu.set_d0(used);
    Ok(format!("[intuition] AddGadget {g:#x} into {win:#x} at {used}"))
}

/// Unlink `num` gadgets from `g` on; returns the old position, or 0xFFFF when `g` is not in the window.
fn unlink<U: GuestUi>(a: &mut Amiga<U>, win: u32, g: u32, num: i32) -> Result<u32, Refusal> {
    let all = window_gadgets(a, win)?;
    let Some(at) = all.iter().position(|x| *x == g) else { return Ok(0xFFFF) };
    let take = chain(a, g, num)?;
    crate::native_gadgets::forget(a, &take)?;
    let after = rd(a, take.last().unwrap() + GAD_NEXT, "Gadget.NextGadget")?;
    if at == 0 { a.mem.write_u32(win + WD_FIRST_GADGET, after).map_err(Refusal::mem("Window.FirstGadget"))?; }
    else { a.mem.write_u32(all[at - 1] + GAD_NEXT, after).map_err(Refusal::mem("Gadget.NextGadget"))?; }
    a.mem.write_u32(take.last().unwrap() + GAD_NEXT, 0).map_err(Refusal::mem("Gadget.NextGadget"))?;
    Ok(at as u32)
}

/// RemoveGList(a0 = window, a1 = gadget, d0 = numGad) -> the old position (0xFFFF = not found).
pub fn remove_glist<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, g, num) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i16 as i32);
    open(a, win, "RemoveGList")?;
    let p = unlink(a, win, g, num)?;
    cpu.set_d0(p);
    Ok(format!("[intuition] RemoveGList {g:#x} from {win:#x} -> {p}"))
}

/// RemoveGadget(a0 = window, a1 = gadget) -> the old position.
pub fn remove_gadget<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (win, g) = (cpu.areg(0), cpu.areg(1));
    open(a, win, "RemoveGadget")?;
    let p = unlink(a, win, g, 1)?;
    cpu.set_d0(p);
    Ok(format!("[intuition] RemoveGadget {g:#x} from {win:#x} -> {p}"))
}

/// RefreshGList(a0 = gadgets, a1 = window, a2 = requester, d0 = numGad) / RefreshGadgets(.., all).
pub fn refresh_glist<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { refresh(a, cpu, cpu.dreg(0) as i16 as i32, "RefreshGList") }
pub fn refresh_gadgets<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { refresh(a, cpu, -1, "RefreshGadgets") }

pub(crate) fn refresh<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, num: i32, what: &'static str) -> Result<String, Refusal> {
    let (g, win) = (cpu.areg(0), cpu.areg(1));
    open(a, win, what)?;
    let all = chain(a, g, num)?;
    let boopsi: Vec<u32> = all.iter().copied().filter(|x| a.classes.class_of(*x).is_some()).collect();
    let log = format!("[intuition] {what} {g:#x} in {win:#x}: {} gadgets, {} BOOPSI rendered (the rest draw natively)", all.len(), boopsi.len());
    run_methods(a, cpu, Job { win, gadgets: boopsi, i: 0, method: GM_RENDER, result: 0 })?;
    Ok(log)
}

/// "Send `method` to each of these gadgets in turn", a chain of guest calls.
struct Job { win: u32, gadgets: Vec<u32>, i: usize, method: u32, result: u32 }

fn run_methods<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, job: Job) -> Result<(), Refusal> {
    let next = step(a, job)?;
    guest_call::start(a, cpu, next)
}

fn step<U: GuestUi>(a: &mut Amiga<U>, job: Job) -> Result<Next<U>, Refusal> {
    let Some(&g) = job.gadgets.get(job.i) else { return Ok(Next::Return(job.result)) };
    let cl = a.classes.class_of(g).ok_or_else(|| Refusal(format!("gadget {g:#x} is not a BOOPSI object")))?;
    let entry = rd(a, cl + CL_H_ENTRY, "cl_Dispatcher.h_Entry")?;
    let gi = crate::boopsi_gadget::make_ginfo(a, job.win, 0, g)?;
    let rp = rd(a, gi + GI_RASTPORT, "GadgetInfo.gi_RastPort")?;
    let msg = match job.method {
        GM_RENDER => make_msg(a, &[GM_RENDER, gi, rp, GREDRAW_REDRAW])?,
        _ => make_msg(a, &[GM_LAYOUT, gi, 1])?,
    };
    let then = Box::new(move |a: &mut Amiga<U>, _cpu: &mut Cpu, _d0: u32| {
        a.heap.free(msg, crate::boopsi_send::MSG_BLOCK).map_err(|e| Refusal(format!("BOOPSI message: {e}")))?;
        crate::boopsi_gadget::free_ginfo(a, gi)?;
        step(a, Job { i: job.i + 1, gadgets: job.gadgets, ..job })
    });
    Ok(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, cl), (1, msg), (2, g)], then })
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("AddGList", add_glist::<U>).named("AddGadget", add_gadget::<U>).named("RemoveGList", remove_glist::<U>)
        .named("RemoveGadget", remove_gadget::<U>).named("RefreshGList", refresh_glist::<U>).named("RefreshGadgets", refresh_gadgets::<U>)
        .named("NewModifyProp", crate::modify_prop::new_modify_prop::<U>).named("ModifyProp", crate::modify_prop::modify_prop::<U>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_ui_abi::RecordingGuestUi;

    #[test]
    fn a_long_chain_is_walked_whole_and_a_cyclic_one_is_refused() {
        let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
        let n = 5000u32;
        let base = a.heap.alloc(n * 4).unwrap();
        for i in 0..n { a.mem.write_u32(base + 4 * i + GAD_NEXT, if i + 1 < n { base + 4 * (i + 1) } else { 0 }).unwrap(); }
        assert_eq!(chain(&a, base, -1).unwrap().len(), n as usize, "no length limit");
        assert_eq!(chain(&a, base, 3).unwrap(), alloc::vec![base, base + 4, base + 8]);
        a.mem.write_u32(base + 4 * (n - 1) + GAD_NEXT, base + 4 * 10).unwrap();
        let e = chain(&a, base, -1).unwrap_err();
        assert!(e.0.contains("cyclic NextGadget"), "{e:?}");
    }
}
