//! utility.library TagItem list management: AllocateTagItems, CloneTagItems, FreeTagItems, RefreshTagItemClones, TagInArray,
//! FilterTagItems and MapTags. Facts: AROS rom/utility/{allocatetagitems,clonetagitems,refreshtagitemclones,taginarray,
//! filtertagitems,maptags}.c (APL; read for behaviour only, not copied). A clone is one flat AllocVec block (control tags
//! resolved away, zero-filled so it ends in TAG_DONE), which FreeTagItems frees.
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::memory::{alloc_vec_block, free_vec_block};
use crate::tags::{TAG_DONE, TAG_IGNORE};
use crate::utility::next_item;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const TAGFILTER_AND: u32 = 0;
const TAGFILTER_NOT: u32 = 1;
const MAP_REMOVE_NOT_FOUND: u32 = 0;
const FILTER_ARRAY_MAX: u32 = 4096;

fn alloc_tags<U: GuestUi>(a: &mut Amiga<U>, n: u32) -> Result<u32, Refusal> {
    if n == 0 { return Ok(0); }
    let size = n.checked_mul(8).ok_or_else(|| Refusal("AllocateTagItems: size overflows".into()))?;
    alloc_vec_block(a, size, true)
}

/// AllocateTagItems(d0 = numTags) -> a zeroed array, or NULL for 0 / no memory.
fn allocate_tag_items<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let n = cpu.dreg(0);
    let r = alloc_tags(a, n)?;
    cpu.set_d0(r);
    Ok(format!("[utility] AllocateTagItems {n} -> {r:#x}"))
}

/// Copy the real items of `original` (NULL = none) into the flat array `clone`; a NULL original ends the clone at once.
fn refresh<U: GuestUi>(a: &mut Amiga<U>, clone: u32, original: u32) -> Result<(), Refusal> {
    if clone == 0 { return Ok(()); }
    if original == 0 { return a.mem.write_u32(clone, TAG_DONE).map_err(Refusal::mem("clone ti_Tag")); }
    let (mut st, mut at) = (original, clone);
    while let Some((_, t, d)) = next_item(a, &mut st)? {
        a.mem.write_u32(at, t).map_err(Refusal::mem("clone ti_Tag"))?;
        a.mem.write_u32(at + 4, d).map_err(Refusal::mem("clone ti_Data"))?;
        at += 8;
    }
    Ok(())
}

/// CloneTagItems(a0 = tags) -> a flat copy (CloneTagItems(NULL) still returns a list: an empty one) or NULL.
fn clone_tag_items<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    let mut n = 1; // the TAG_DONE slot
    let mut st = list;
    while next_item(a, &mut st)?.is_some() { n += 1; }
    let new = alloc_tags(a, n)?;
    refresh(a, new, list)?;
    cpu.set_d0(new);
    Ok(format!("[utility] CloneTagItems {list:#x} ({} items) -> {new:#x}", n - 1))
}

/// FreeTagItems(a0 = a clone or AllocateTagItems array, or NULL).
fn free_tag_items<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let list = cpu.areg(0);
    free_vec_block(a, list)?;
    Ok(format!("[utility] FreeTagItems {list:#x}"))
}

/// RefreshTagItemClones(a0 = clone, a1 = original).
fn refresh_tag_item_clones<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (clone, original) = (cpu.areg(0), cpu.areg(1));
    refresh(a, clone, original)?;
    Ok(format!("[utility] RefreshTagItemClones {clone:#x} <- {original:#x}"))
}

/// Whether `tag` is in the TAG_DONE-ended ULONG array `array`.
fn in_array<U: GuestUi>(a: &Amiga<U>, tag: u32, array: u32) -> Result<bool, Refusal> {
    for i in 0..FILTER_ARRAY_MAX {
        let t = a.mem.read_u32(array + 4 * i).map_err(Refusal::mem("tag filter array"))?;
        if t == TAG_DONE { return Ok(false); }
        if t == tag { return Ok(true); }
    }
    Err(Refusal("TagInArray: the filter array has no TAG_DONE within 4096 entries".into()))
}

/// TagInArray(d0 = tag, a0 = array) -> BOOL.
fn tag_in_array<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (tag, array) = (cpu.dreg(0), cpu.areg(0));
    let r = in_array(a, tag, array)? as u32;
    cpu.set_d0(r);
    Ok(format!("[utility] TagInArray {tag:#x} -> {r}"))
}

/// FilterTagItems(a0 = tags, a1 = array, d0 = logic) -> how many items stay valid; the rest become TAG_IGNORE IN PLACE.
fn filter_tag_items<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, array, logic) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    let mut valid = 0;
    if list != 0 && array != 0 {
        if logic > TAGFILTER_NOT { return Err(Refusal(format!("FilterTagItems: logic {logic} is neither TAGFILTER_AND nor TAGFILTER_NOT"))); }
        let mut st = list;
        while let Some((at, t, _)) = next_item(a, &mut st)? {
            let hit = in_array(a, t, array)?;
            if hit == (logic == TAGFILTER_AND) { valid += 1; } else { a.mem.write_u32(at, TAG_IGNORE).map_err(Refusal::mem("ti_Tag"))?; }
        }
    }
    cpu.set_d0(valid);
    Ok(format!("[utility] FilterTagItems {list:#x} logic={logic} -> {valid}"))
}

/// MapTags(a0 = tags, a1 = mapList, d0 = mapType): rewrite each item's tag through the map IN PLACE (a map value of TAG_DONE removes it).
fn map_tags<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (list, map, kind) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    let mut st = list;
    while let Some((at, t, _)) = next_item(a, &mut st)? {
        let new = match crate::utility::find(a, t, map)? {
            Some((_, TAG_DONE)) => Some(TAG_IGNORE),
            Some((_, to)) => Some(to),
            None if kind == MAP_REMOVE_NOT_FOUND => Some(TAG_IGNORE),
            None => None,
        };
        if let Some(n) = new { a.mem.write_u32(at, n).map_err(Refusal::mem("ti_Tag"))?; }
    }
    Ok(format!("[utility] MapTags {list:#x} via {map:#x} type={kind}"))
}

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AllocateTagItems", allocate_tag_items::<U>).named("CloneTagItems", clone_tag_items::<U>).named("FreeTagItems", free_tag_items::<U>)
        .named("RefreshTagItemClones", refresh_tag_item_clones::<U>).named("TagInArray", tag_in_array::<U>)
        .named("FilterTagItems", filter_tag_items::<U>).named("MapTags", map_tags::<U>)
}
