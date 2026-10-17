//! utility.library TagItem lists (utility/tagitem.h): `{ ULONG ti_Tag; IPTR ti_Data; }` pairs, ended by TAG_DONE.
//! TAG_IGNORE skips one item, TAG_MORE continues at ti_Data, TAG_SKIP skips ti_Data more items. The walk is bounded,
//! so a cyclic TAG_MORE chain is a named refusal, not a hang.
use alloc::vec::Vec;
use crate::Refusal;
use retro_guest_mem::GuestMem;

pub const TAG_DONE: u32 = 0;
pub const TAG_IGNORE: u32 = 1;
pub const TAG_MORE: u32 = 2;
pub const TAG_SKIP: u32 = 3;
pub const TAG_USER: u32 = 1 << 31;
/// More items than any real program passes; beyond it the list is taken to be corrupt or cyclic.
const MAX_ITEMS: usize = 4096;

/// Every (tag, data) pair of the list at `at` (NULL = empty), control tags resolved.
pub fn walk(mem: &GuestMem, mut at: u32) -> Result<Vec<(u32, u32)>, Refusal> {
    let mut out = Vec::new();
    let mut steps = 0;
    while at != 0 {
        steps += 1;
        if steps > MAX_ITEMS { return Err(Refusal("TagItem list longer than 4096 items (cyclic TAG_MORE?)".into())); }
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
}
