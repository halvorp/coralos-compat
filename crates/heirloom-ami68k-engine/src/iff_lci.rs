//! iffparse.library, part 3: LocalContextItems and everything built on them: AllocLocalItem / FindLocalItem / StoreLocalItem and
//! friends, EntryHandler / ExitHandler, StopChunk / StopOnExit, PropChunk / FindProp, CollectionChunk / FindCollection,
//! FindPropContext, FreeIFF. (AROS alloclocalitem.c, storelocalitem.c, storeitemincontext.c, findlocalitem.c, prophooks.c,
//! collectionhooks.c, stophook.c ...; APL, read for facts only.)
//!
//! A LocalContextItem is a real guest struct (IntLocalContextItem, 32 bytes; m68k gcc -mno-align-int): lci_ID 8, lci_Type 12,
//! lci_Ident 16, the purge hook 20, the user data 24 and its size 28. The handlers PropChunk / StopChunk / CollectionChunk /
//! StopOnExit install are the ENGINE's (AROS has them as hooks inside the library): they run natively when `ParseIFF` reaches their
//! chunk, so no guest code is involved; a handler the PROGRAM installs (EntryHandler / ExitHandler) is a guest hook called through
//! `guest_call`, and so is a purge hook the program set with SetLocalItemPurge. Items stored by PropFunc and CollectionFunc are
//! StoredProperty / CollectionItem structs in guest memory, which is what FindProp / FindCollection hand back.
//! Deliberate difference from AROS: a PropChunk / CollectionChunk whose chunk cannot be read in full fails with that error
//! (IFFERR_MANGLED for a short chunk); AROS frees the buffer and then carries on to store it.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::guest_call::{Cont, Next};
use crate::iff::*;
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

pub const IFFLCI_PROP: u32 = u32::from_be_bytes(*b"prop");
pub const IFFLCI_COLLECTION: u32 = u32::from_be_bytes(*b"coll");
pub const IFFLCI_ENTRYHANDLER: u32 = u32::from_be_bytes(*b"enhd");
pub const IFFLCI_EXITHANDLER: u32 = u32::from_be_bytes(*b"exhd");
pub const IFFCMD_PURGELCI: u32 = 7;
const IFFSLI_ROOT: i32 = 1;
const IFFSLI_TOP: i32 = 2;
const IFFSLI_PROP: i32 = 3;

pub const LCI_SIZE: u32 = 32;
const LCI_ID: u32 = 8;
const LCI_TYPE: u32 = 12;
const LCI_IDENT: u32 = 16;
const LCI_PURGE: u32 = 20;
const LCI_DATA: u32 = 24;
const LCI_DATA_SIZE: u32 = 28;
/// struct HandlerInfo { hook, object }; StoredProperty { sp_Size, sp_Data }; CollectionItem { ci_Next, ci_Size, ci_Data }.
const HI_HOOK: u32 = 0;
const HI_OBJECT: u32 = 4;
const SP_SIZE: u32 = 0;
const SP_DATA: u32 = 4;
const CI_SIZE_BYTES: u32 = 12;
const CI_NEXT: u32 = 0;
const CI_SIZE: u32 = 4;
const CI_DATA: u32 = 8;

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocLocalItem", alloc_local_item::<U>).named("LocalItemData", local_item_data::<U>).named("SetLocalItemPurge", set_purge::<U>)
        .named("FreeLocalItem", free_local_item::<U>).named("FindLocalItem", find_local_item_lvo::<U>).named("StoreLocalItem", store_local_item::<U>)
        .named("StoreItemInContext", store_item_in_context::<U>).named("EntryHandler", entry_handler::<U>).named("ExitHandler", exit_handler::<U>)
        .named("StopChunk", stop_chunk::<U>).named("StopChunks", stop_chunks::<U>).named("StopOnExit", stop_on_exit::<U>)
        .named("PropChunk", prop_chunk::<U>).named("PropChunks", prop_chunks::<U>).named("CollectionChunk", collection_chunk::<U>).named("CollectionChunks", collection_chunks::<U>)
        .named("FindProp", find_prop::<U>).named("FindCollection", find_collection::<U>).named("FindPropContext", find_prop_context_lvo::<U>)
}

fn known<U: GuestUi>(a: &Amiga<U>, func: &str, lci: u32) -> Result<(), Refusal> {
    if a.iff.lcis.contains_key(&lci) { Ok(()) } else { Err(Refusal(format!("{func}: {lci:#x} is not a LocalContextItem AllocLocalItem returned"))) }
}

/// AllocLocalItem's work: a cleared item with `data_size` bytes of user data, or None (no memory).
pub fn alloc_local<U: GuestUi>(a: &mut Amiga<U>, ty: u32, id: u32, ident: u32, data_size: u32, native: Option<Native>) -> Result<Option<u32>, Refusal> {
    let Some(l) = a.heap.alloc(LCI_SIZE) else { return Ok(None) };
    a.mem.zero(l, LCI_SIZE).map_err(m("LocalContextItem"))?;
    for (off, v) in [(LCI_ID, id), (LCI_TYPE, ty), (LCI_IDENT, ident)] { wr(a, l + off, v)?; }
    if (data_size as i32) > 0 {
        let Some(d) = a.heap.alloc(data_size) else { a.heap.free(l, LCI_SIZE).map_err(|e| Refusal(format!("AllocLocalItem: {e}")))?; return Ok(None) };
        a.mem.zero(d, data_size).map_err(m("LocalContextItem data"))?;
        wr(a, l + LCI_DATA, d)?;
        wr(a, l + LCI_DATA_SIZE, data_size)?;
    }
    a.iff.lcis.insert(l, native);
    Ok(Some(l))
}

/// FreeLocalItem's work.
pub fn free_local<U: GuestUi>(a: &mut Amiga<U>, l: u32) -> Result<(), Refusal> {
    let (d, n) = (rd(a, l + LCI_DATA)?, rd(a, l + LCI_DATA_SIZE)?);
    if d != 0 { a.heap.free(d, n).map_err(|e| Refusal(format!("FreeLocalItem: {e}")))?; }
    a.heap.free(l, LCI_SIZE).map_err(|e| Refusal(format!("FreeLocalItem: {e}")))?;
    a.iff.lcis.remove(&l);
    Ok(())
}

fn data<U: GuestUi>(a: &Amiga<U>, l: u32) -> Result<u32, Refusal> { rd(a, l + LCI_DATA) }

/// FindLocalItem's work: from the top context down to the default one, the first item of this type, id and ident.
pub fn find_local<U: GuestUi>(a: &Amiga<U>, iff: u32, ty: u32, id: u32, ident: u32) -> Result<Option<u32>, Refusal> {
    for &cn in a.iff.handles[&iff].stack.iter().rev() {
        for &l in &a.iff.cns[&cn].lcis {
            if rd(a, l + LCI_TYPE)? == ty && rd(a, l + LCI_ID)? == id && rd(a, l + LCI_IDENT)? == ident { return Ok(Some(l)); }
        }
    }
    Ok(None)
}

/// StoreItemInContext's work: the item goes to the head of the context's list; the equal items it replaces are returned to be purged.
pub fn store_in<U: GuestUi>(a: &mut Amiga<U>, l: u32, cn: u32) -> Result<Vec<u32>, Refusal> {
    let key = (rd(a, l + LCI_ID)?, rd(a, l + LCI_TYPE)?, rd(a, l + LCI_IDENT)?);
    let old = core::mem::take(&mut a.iff.cns.get_mut(&cn).expect("context node").lcis);
    let (mut keep, mut gone) = (Vec::new(), Vec::new());
    for o in old {
        if (rd(a, o + LCI_ID)?, rd(a, o + LCI_TYPE)?, rd(a, o + LCI_IDENT)?) == key { gone.push(o) } else { keep.push(o) }
    }
    keep.insert(0, l);
    a.iff.cns.get_mut(&cn).expect("context node").lcis = keep;
    Ok(gone)
}

pub fn find_prop_context<U: GuestUi>(a: &Amiga<U>, iff: u32) -> Result<Option<u32>, Refusal> {
    let mut node = Some(top(a, iff));
    while let Some(n) = node {
        let id = rd(a, n + CN_ID)?;
        if id == ID_FORM || id == ID_LIST { return Ok(Some(n)); }
        node = crate::iff_chunk::parent_of(a, n);
    }
    Ok(None)
}

/// StoreLocalItem's work: (error, the displaced items to purge).
pub fn store_local<U: GuestUi>(a: &mut Amiga<U>, iff: u32, l: u32, pos: i32) -> Result<(i32, Vec<u32>), Refusal> {
    let cn = match pos {
        IFFSLI_ROOT => root(a, iff),
        IFFSLI_TOP => top(a, iff),
        IFFSLI_PROP => match find_prop_context(a, iff)? { Some(c) => c, None => return Ok((IFFERR_NOSCOPE, Vec::new())) },
        _ => return Ok((0, Vec::new())), // AROS' switch has no default: nothing is stored, no error
    };
    Ok((0, store_in(a, l, cn)?))
}

/// PurgeLCI of one item: Some(hook) when a GUEST purge hook must run (it frees the item itself), else freed natively.
fn purge_one<U: GuestUi>(a: &mut Amiga<U>, l: u32) -> Result<Option<u32>, Refusal> {
    let Some(&native) = a.iff.lcis.get(&l) else { return Ok(None) };
    match native {
        Some(Native::Prop) => {
            let sp = data(a, l)?;
            let (size, buf) = (rd(a, sp + SP_SIZE)?, rd(a, sp + SP_DATA)?);
            if buf != 0 { a.heap.free(buf, size).map_err(|e| Refusal(format!("PropPurge: {e}")))?; }
        }
        Some(Native::Collection) => {
            let mut ci = rd(a, data(a, l)?)?;
            while ci != 0 {
                let (next, size, buf) = (rd(a, ci + CI_NEXT)?, rd(a, ci + CI_SIZE)?, rd(a, ci + CI_DATA)?);
                if buf != 0 { a.heap.free(buf, size).map_err(|e| Refusal(format!("CollectionPurge: {e}")))?; }
                a.heap.free(ci, CI_SIZE_BYTES).map_err(|e| Refusal(format!("CollectionPurge: {e}")))?;
                ci = next;
            }
        }
        Some(Native::Handler) => {}
        None => {
            let hook = rd(a, l + LCI_PURGE)?;
            if hook != 0 { return Ok(Some(hook)); }
        }
    }
    free_local(a, l)?;
    Ok(None)
}

/// Purge `lcis` in order (PurgeLCI: a program's purge hook is called with A0 = hook, A2 = the item, A1 = IFFCMD_PURGELCI and frees the
/// item itself), then continue with `then`.
pub fn purge_chain<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, mut lcis: Vec<u32>, then: Cont<U>) -> Result<Next<U>, Refusal> {
    while !lcis.is_empty() {
        let l = lcis.remove(0);
        if let Some(hook) = purge_one(a, l)? {
            let entry = rd(a, hook + 8)?;
            if entry == 0 { return Err(Refusal(format!("PurgeLCI: purge hook {hook:#x} has a NULL h_Entry"))); }
            return Ok(Next::Call { target: entry, d0: 0, aregs: alloc::vec![(0, hook), (1, IFFCMD_PURGELCI), (2, l)], then: Box::new(move |a, cpu, _| purge_chain(a, cpu, lcis, then)) });
        }
    }
    then(a, cpu, 0)
}

/// Finish the trapped call with `ret` after purging `lcis` (guest purge hooks may run first).
pub fn finish_with<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, lcis: Vec<u32>, ret: i32) -> Result<(), Refusal> {
    let next = purge_chain(a, cpu, lcis, Box::new(move |_, _, _| Ok(Next::Return(ret as u32))))?;
    crate::guest_call::start(a, cpu, next)
}

/// Purge items that a native handler displaced: only the engine's own and hook-less ones can be freed here; one with a program's purge
/// hook is left (and named), because a handler running inside ParseIFF cannot call guest code.
pub fn purge_native<U: GuestUi>(a: &mut Amiga<U>, lcis: Vec<u32>) -> Result<(), Refusal> {
    for l in lcis {
        if let Some(hook) = purge_one(a, l)? { a.refused.push(format!("iffparse.library/ParseIFF: a stored item {l:#x} replaced by PropChunk/CollectionChunk has the program's purge hook {hook:#x}, which cannot run inside ParseIFF; it is left unpurged")); }
    }
    Ok(())
}

// ---- the native handlers ----------------------------------------------------------------------------------------------

/// PropFunc: store the whole chunk as a StoredProperty in the enclosing FORM / LIST. 0 or an IFFERR code.
pub fn prop_func<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<i32, Refusal> {
    let cn = top(a, iff);
    let (ty, id, size) = (rd(a, cn + CN_TYPE)?, rd(a, cn + CN_ID)?, rd(a, cn + CN_SIZE_F)?);
    let Some(l) = alloc_local(a, ty, id, IFFLCI_PROP, 8, Some(Native::Prop))? else { return Ok(IFFERR_NOMEM) };
    let sp = data(a, l)?;
    let buf = if size != 0 {
        match a.heap.alloc(size) { Some(b) => b, None => { free_local(a, l)?; return Ok(IFFERR_NOMEM); } }
    } else { 0 };
    wr(a, sp + SP_DATA, buf)?;
    wr(a, sp + SP_SIZE, size)?;
    if buf != 0 {
        let got = crate::iff_chunk::read_chunk_core(a, iff, buf, size as i32)?;
        if got != size as i32 {
            a.heap.free(buf, size).map_err(|e| Refusal(format!("PropFunc: {e}")))?;
            free_local(a, l)?;
            return Ok(if got < 0 { got } else { IFFERR_MANGLED });
        }
    }
    let (err, displaced) = store_local(a, iff, l, IFFSLI_PROP)?;
    if err != 0 {
        if buf != 0 { a.heap.free(buf, size).map_err(|e| Refusal(format!("PropFunc: {e}")))?; }
        free_local(a, l)?;
        return Ok(err);
    }
    purge_native(a, displaced)?;
    Ok(0)
}

/// CollectionFunc: add the chunk to the CollectionItem list of its type and id (the list lives in the enclosing FORM / LIST).
pub fn collection_func<U: GuestUi>(a: &mut Amiga<U>, iff: u32) -> Result<i32, Refusal> {
    let cn = top(a, iff);
    let (ty, id, size) = (rd(a, cn + CN_TYPE)?, rd(a, cn + CN_ID)?, rd(a, cn + CN_SIZE_F)?);
    let (l, fresh) = match find_local(a, iff, ty, id, IFFLCI_COLLECTION)? {
        Some(l) => (l, false),
        None => {
            let Some(l) = alloc_local(a, ty, id, IFFLCI_COLLECTION, 4, Some(Native::Collection))? else { return Ok(IFFERR_NOMEM) };
            let (err, displaced) = store_local(a, iff, l, IFFSLI_PROP)?;
            if err != 0 { free_local(a, l)?; return Ok(err); }
            purge_native(a, displaced)?;
            (l, true)
        }
    };
    let undo = |a: &mut Amiga<U>| -> Result<(), Refusal> {
        if fresh {
            for c in a.iff.cns.values_mut() { c.lcis.retain(|&x| x != l); }
            free_local(a, l)?;
        }
        Ok(())
    };
    let Some(item) = a.heap.alloc(CI_SIZE_BYTES) else { undo(a)?; return Ok(IFFERR_NOMEM) };
    a.mem.zero(item, CI_SIZE_BYTES).map_err(m("CollectionItem"))?;
    let buf = if size != 0 { match a.heap.alloc(size) { Some(b) => b, None => { a.heap.free(item, CI_SIZE_BYTES).map_err(|e| Refusal(format!("CollectionFunc: {e}")))?; undo(a)?; return Ok(IFFERR_NOMEM); } } } else { 0 };
    if buf != 0 {
        let got = crate::iff_chunk::read_chunk_core(a, iff, buf, size as i32)?;
        if got != size as i32 {
            a.heap.free(buf, size).map_err(|e| Refusal(format!("CollectionFunc: {e}")))?;
            a.heap.free(item, CI_SIZE_BYTES).map_err(|e| Refusal(format!("CollectionFunc: {e}")))?;
            undo(a)?;
            return Ok(if got < 0 { got } else { IFFERR_MANGLED });
        }
    }
    let ciptr = data(a, l)?;
    let first = rd(a, ciptr)?;
    wr(a, item + CI_NEXT, first)?;
    wr(a, item + CI_DATA, buf)?;
    wr(a, item + CI_SIZE, size)?;
    wr(a, ciptr, item)?;
    Ok(0)
}

// ---- the API ----------------------------------------------------------------------------------------------------------

/// AllocLocalItem(d0 = type, d1 = id, d2 = ident, d3 = dataSize) -> the item or NULL.
fn alloc_local_item<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ty, id, ident, n) = (cpu.dreg(0), cpu.dreg(1), cpu.dreg(2), cpu.dreg(3));
    let l = alloc_local(a, ty, id, ident, n, None)?.unwrap_or(0);
    cpu.set_d0(l);
    Ok(format!("[iffparse] AllocLocalItem {} {} {} size={n} -> {l:#x}", crate::iff_chunk::fourcc(ty), crate::iff_chunk::fourcc(id), crate::iff_chunk::fourcc(ident)))
}

/// LocalItemData(a0 = item) -> its user data (NULL for NULL).
fn local_item_data<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let l = cpu.areg(0);
    let d = if l == 0 { 0 } else { known(a, "LocalItemData", l)?; data(a, l)? };
    cpu.set_d0(d);
    Ok(format!("[iffparse] LocalItemData {l:#x} -> {d:#x}"))
}

/// SetLocalItemPurge(a0 = item, a1 = hook).
fn set_purge<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (l, hook) = (cpu.areg(0), cpu.areg(1));
    known(a, "SetLocalItemPurge", l)?;
    wr(a, l + LCI_PURGE, hook)?;
    Ok(format!("[iffparse] SetLocalItemPurge {l:#x} hook={hook:#x}"))
}

/// FreeLocalItem(a0 = item): NULL is ignored.
fn free_local_item<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let l = cpu.areg(0);
    if l == 0 { return Ok("[iffparse] FreeLocalItem NULL".into()); }
    known(a, "FreeLocalItem", l)?;
    free_local(a, l)?;
    Ok(format!("[iffparse] FreeLocalItem {l:#x}"))
}

/// FindLocalItem(a0 = iff, d0 = type, d1 = id, d2 = ident).
fn find_local_item_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id, ident) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1), cpu.dreg(2));
    handle(a, "FindLocalItem", iff)?;
    let l = find_local(a, iff, ty, id, ident)?.unwrap_or(0);
    cpu.set_d0(l);
    Ok(format!("[iffparse] FindLocalItem {iff:#x} -> {l:#x}"))
}

/// StoreLocalItem(a0 = iff, a1 = item, d0 = position) -> 0 or IFFERR_NOSCOPE.
fn store_local_item<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, l, pos) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32);
    handle(a, "StoreLocalItem", iff)?;
    known(a, "StoreLocalItem", l)?;
    let (err, displaced) = store_local(a, iff, l, pos)?;
    finish_with(a, cpu, displaced, err)?;
    Ok(format!("[iffparse] StoreLocalItem {iff:#x} {l:#x} position={pos} -> {err}"))
}

/// StoreItemInContext(a0 = iff, a1 = item, a2 = contextNode).
fn store_item_in_context<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, l, cn) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    handle(a, "StoreItemInContext", iff)?;
    known(a, "StoreItemInContext", l)?;
    if !a.iff.handles[&iff].stack.contains(&cn) { return Err(Refusal(format!("StoreItemInContext: {cn:#x} is not a ContextNode of {iff:#x}"))); }
    let displaced = store_in(a, l, cn)?;
    finish_with(a, cpu, displaced, 0)?;
    Ok(format!("[iffparse] StoreItemInContext {iff:#x} {l:#x} in {cn:#x}"))
}

/// Install a handler item (type, id) holding (hook, object) at `pos`: the shared body of EntryHandler / ExitHandler / StopChunk ...
/// (error, the items it displaced).
fn install_core<U: GuestUi>(a: &mut Amiga<U>, iff: u32, ty: u32, id: u32, ident: u32, pos: i32, hook: u32, object: u32) -> Result<(i32, Vec<u32>), Refusal> {
    let Some(l) = alloc_local(a, ty, id, ident, 8, Some(Native::Handler))? else { return Ok((IFFERR_NOMEM, Vec::new())) };
    let d = data(a, l)?;
    wr(a, d + HI_HOOK, hook)?;
    wr(a, d + HI_OBJECT, object)?;
    let (err, displaced) = store_local(a, iff, l, pos)?;
    // Nothing was stored for an unknown position (AROS leaks the item there); an error stored nothing either.
    if err != 0 || !(IFFSLI_ROOT..=IFFSLI_PROP).contains(&pos) { free_local(a, l)?; }
    Ok((err, displaced))
}

fn install<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, iff: u32, ty: u32, id: u32, ident: u32, pos: i32, hook: u32, object: u32, what: &str) -> Result<String, Refusal> {
    handle(a, what, iff)?;
    let (err, displaced) = install_core(a, iff, ty, id, ident, pos, hook, object)?;
    finish_with(a, cpu, displaced, err)?;
    Ok(format!("[iffparse] {what} {iff:#x} {} {} -> {err}", crate::iff_chunk::fourcc(ty), crate::iff_chunk::fourcc(id)))
}

/// EntryHandler(a0 = iff, d0 = type, d1 = id, d2 = position, a1 = hook, a2 = object): `hook` is called when ParseIFF enters the chunk.
fn entry_handler<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id, pos, hook, obj) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1), cpu.dreg(2) as i32, cpu.areg(1), cpu.areg(2));
    install(a, cpu, iff, ty, id, IFFLCI_ENTRYHANDLER, pos, hook, obj, "EntryHandler")
}
/// ExitHandler: the same, called when ParseIFF leaves the chunk.
fn exit_handler<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id, pos, hook, obj) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1), cpu.dreg(2) as i32, cpu.areg(1), cpu.areg(2));
    install(a, cpu, iff, ty, id, IFFLCI_EXITHANDLER, pos, hook, obj, "ExitHandler")
}

fn native_install<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, which: NativeHook, ident: u32, what: &str) -> Result<String, Refusal> {
    let (iff, ty, id) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1));
    let hook = native_hook(a, which);
    install(a, cpu, iff, ty, id, ident, IFFSLI_TOP, hook, iff, what)
}
fn stop_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { native_install(a, cpu, NativeHook::Stop, IFFLCI_ENTRYHANDLER, "StopChunk") }
fn prop_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { native_install(a, cpu, NativeHook::Prop, IFFLCI_ENTRYHANDLER, "PropChunk") }
fn collection_chunk<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { native_install(a, cpu, NativeHook::Collection, IFFLCI_ENTRYHANDLER, "CollectionChunk") }
fn stop_on_exit<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { native_install(a, cpu, NativeHook::ExitContext, IFFLCI_EXITHANDLER, "StopOnExit") }

/// The *Chunks forms: (a0 = iff, a1 = array of (type, id) longs, d0 = pairs); the first error stops, as in AROS.
fn pairs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, which: NativeHook, ident: u32, what: &str) -> Result<String, Refusal> {
    let (iff, arr, n) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as i32);
    handle(a, what, iff)?;
    let hook = native_hook(a, which);
    let (mut err, mut displaced) = (0, Vec::new());
    for i in 0..n.max(0) as u32 {
        let (ty, id) = (rd(a, arr + 8 * i)?, rd(a, arr + 8 * i + 4)?);
        let (e, d) = install_core(a, iff, ty, id, ident, IFFSLI_TOP, hook, iff)?;
        displaced.extend(d);
        if e != 0 { err = e; break; }
    }
    finish_with(a, cpu, displaced, err)?;
    Ok(format!("[iffparse] {what} {iff:#x} {n} pairs -> {err}"))
}
fn stop_chunks<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { pairs(a, cpu, NativeHook::Stop, IFFLCI_ENTRYHANDLER, "StopChunks") }
fn prop_chunks<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { pairs(a, cpu, NativeHook::Prop, IFFLCI_ENTRYHANDLER, "PropChunks") }
fn collection_chunks<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> { pairs(a, cpu, NativeHook::Collection, IFFLCI_ENTRYHANDLER, "CollectionChunks") }

/// FindProp(a0 = iff, d0 = type, d1 = id) -> the StoredProperty or NULL.
fn find_prop<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1));
    handle(a, "FindProp", iff)?;
    let p = match find_local(a, iff, ty, id, IFFLCI_PROP)? { Some(l) => data(a, l)?, None => 0 };
    cpu.set_d0(p);
    Ok(format!("[iffparse] FindProp {} {} -> {p:#x}", crate::iff_chunk::fourcc(ty), crate::iff_chunk::fourcc(id)))
}

/// FindCollection(a0 = iff, d0 = type, d1 = id) -> the first (most recently collected) CollectionItem or NULL.
fn find_collection<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (iff, ty, id) = (cpu.areg(0), cpu.dreg(0), cpu.dreg(1));
    handle(a, "FindCollection", iff)?;
    let p = match find_local(a, iff, ty, id, IFFLCI_COLLECTION)? { Some(l) => rd(a, data(a, l)?)?, None => 0 };
    cpu.set_d0(p);
    Ok(format!("[iffparse] FindCollection {} {} -> {p:#x}", crate::iff_chunk::fourcc(ty), crate::iff_chunk::fourcc(id)))
}

/// FindPropContext(a0 = iff) -> the innermost FORM or LIST ContextNode, or NULL.
fn find_prop_context_lvo<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let iff = cpu.areg(0);
    handle(a, "FindPropContext", iff)?;
    let c = find_prop_context(a, iff)?.unwrap_or(0);
    cpu.set_d0(c);
    Ok(format!("[iffparse] FindPropContext {iff:#x} -> {c:#x}"))
}

/// FreeIFF(a0 = iff): the default context's items are purged, chunks still open are dropped, and the handle goes back.
pub fn free_iff<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let iff = cpu.areg(0);
    if iff == 0 { return Ok("[iffparse] FreeIFF NULL".into()); }
    handle(a, "FreeIFF", iff)?;
    let mut purge = Vec::new();
    while a.iff.handles[&iff].stack.len() > 1 { purge.extend(pop_context(a, iff)?); }
    let r = root(a, iff);
    purge.extend(a.iff.cns.remove(&r).map(|c| c.lcis).unwrap_or_default());
    a.iff.handles.remove(&iff);
    let next = purge_chain(a, cpu, purge, Box::new(move |a, _, _| {
        a.heap.free(iff, HANDLE_SIZE).map_err(|e| Refusal(format!("FreeIFF: {e}")))?;
        Ok(Next::Return(0))
    }))?;
    crate::guest_call::start(a, cpu, next)?;
    Ok(format!("[iffparse] FreeIFF {iff:#x}"))
}
