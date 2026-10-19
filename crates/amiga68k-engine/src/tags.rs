//! utility.library TagItem lists (utility/tagitem.h): `{ ULONG ti_Tag; IPTR ti_Data; }` pairs, ended by TAG_DONE.
//! TAG_IGNORE skips one item, TAG_MORE continues at ti_Data, TAG_SKIP skips ti_Data more items. A list has no length
//! limit; an item reached twice (a TAG_MORE / TAG_SKIP loop) is a named refusal, not a hang.
use alloc::format;
use alloc::vec::Vec;
use crate::Refusal;
use retro_guest_mem::GuestMem;

pub const TAG_DONE: u32 = 0;
pub const TAG_IGNORE: u32 = 1;
pub const TAG_MORE: u32 = 2;
pub const TAG_SKIP: u32 = 3;
pub const TAG_USER: u32 = 1 << 31;

/// Every (tag, data) pair of the list at `at` (NULL = empty), control tags resolved.
pub fn walk(mem: &GuestMem, mut at: u32) -> Result<Vec<(u32, u32)>, Refusal> {
    let (mut out, mut seen) = (Vec::new(), alloc::collections::BTreeSet::new());
    while at != 0 {
        if !seen.insert(at) { return Err(Refusal(format!("cyclic TagItem list: item {at:#x} comes back (TAG_MORE / TAG_SKIP loop)"))); }
        let tag = mem.read_u32(at).map_err(Refusal::mem("ti_Tag"))?;
        let data = mem.read_u32(at + 4).map_err(Refusal::mem("ti_Data"))?;
        match tag {
            TAG_DONE => break,
            TAG_IGNORE => at += 8,
            TAG_MORE => at = data,
            TAG_SKIP => at += 8 * (data + 1),
            _ => { out.push((tag, data)); at += 8; }
        }
    }
    Ok(out)
}

/// The value of `tag`: the FIRST matching item, as utility.library's FindTagItem / GetTagData return it.
pub fn get(tags: &[(u32, u32)], tag: u32) -> Option<u32> { tags.iter().find(|(t, _)| *t == tag).map(|(_, d)| *d) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn more_skip_ignore_and_a_cycle() {
        let mut m = GuestMem::new(0x1000);
        let put = |m: &mut GuestMem, at: u32, items: &[(u32, u32)]| for (i, (t, d)) in items.iter().enumerate() { m.write_u32(at + 8 * i as u32, *t).unwrap(); m.write_u32(at + 8 * i as u32 + 4, *d).unwrap(); };
        put(&mut m, 0x100, &[(TAG_USER + 1, 10), (TAG_IGNORE, 0), (TAG_SKIP, 1), (TAG_USER + 9, 99), (TAG_MORE, 0x200)]);
        put(&mut m, 0x200, &[(TAG_USER + 2, 20), (TAG_DONE, 0)]);
        assert_eq!(walk(&m, 0x100).unwrap(), vec![(TAG_USER + 1, 10), (TAG_USER + 2, 20)]);
        put(&mut m, 0x300, &[(TAG_MORE, 0x300)]);
        assert!(walk(&m, 0x300).is_err(), "a cyclic TAG_MORE is refused");
    }

    #[test]
    fn a_list_longer_than_the_old_4096_item_bound_is_walked_whole() {
        let n = 5000u32;
        let mut m = GuestMem::new((0x10000 + 8 * (n + 1)) as usize);
        for i in 0..n { m.write_u32(0x10000 + 8 * i, TAG_USER + i).unwrap(); m.write_u32(0x10000 + 8 * i + 4, i).unwrap(); }
        m.write_u32(0x10000 + 8 * n, TAG_DONE).unwrap();
        assert_eq!(walk(&m, 0x10000).unwrap().len(), n as usize);
    }
}
