//! utility.library: tag helpers (FindTagItem returns the item's ADDRESS, GetTagData's default, PackBoolTags), CallHookPkt
//! into a guest hook, string and multiply helpers. Controls: missing tags, TAG_IGNORE/TAG_MORE handling, a non-matching compare.
mod gfxcommon;
use amiga68k_engine::machine::EXEC_BASE;
use gfxcommon::*;

const OPEN_LIBRARY: i32 = -552;
const FIND_TAG_ITEM: i32 = -30;
const GET_TAG_DATA: i32 = -36;
const PACK_BOOL_TAGS: i32 = -42;
const NEXT_TAG_ITEM: i32 = -48;
const CALL_HOOK_PKT: i32 = -102;
const SMULT32: i32 = -138;
const STRICMP: i32 = -162;
const STRNICMP: i32 = -168;
const TO_UPPER: i32 = -174;

fn util(g: &mut G) -> u32 {
    let n = g.s("utility.library");
    g.run(&[(EXEC_BASE, OPEN_LIBRARY, vec![a(1, n), d(0, 0)])]);
    g.result(0)
}
fn call(base: u32, lvo: i32, regs: &[(R, V)]) -> Call { (base, lvo, regs.to_vec()) }

#[test]
fn tag_helpers() {
    let mut g = G::new();
    let u = util(&mut g);
    assert_ne!(u, 0);
    let more = g.tags(&[(TAG_USER + 7, 70)]);
    let t = g.tags(&[(1, 0), (TAG_USER + 1, 10), (2, more), (TAG_USER + 9, 99)]); // TAG_IGNORE, item, TAG_MORE -> more
    let map = g.tags(&[(TAG_USER + 1, 0x4), (TAG_USER + 7, 0x10), (TAG_USER + 5, 0x20)]);
    let state = g.zeroed(4);
    g.a.mem.write_u32(state, t).unwrap();
    g.run(&[call(u, FIND_TAG_ITEM, &[d(0, TAG_USER + 7), a(0, t)]), call(u, FIND_TAG_ITEM, &[d(0, TAG_USER + 9), a(0, t)]),
        call(u, GET_TAG_DATA, &[d(0, TAG_USER + 1), d(1, 5), a(0, t)]), call(u, GET_TAG_DATA, &[d(0, TAG_USER + 3), d(1, 5), a(0, t)]),
        call(u, PACK_BOOL_TAGS, &[d(0, 0x1), a(0, t), a(1, map)]),
        call(u, NEXT_TAG_ITEM, &[a(0, state)]), call(u, NEXT_TAG_ITEM, &[a(0, state)]), call(u, NEXT_TAG_ITEM, &[a(0, state)])]);
    assert_eq!(g.r32(g.result(0) + 4), 70, "FindTagItem found the item reached through TAG_MORE and returned its address");
    assert_eq!(g.result(1), 0, "control: the item after the TAG_MORE jump is never reached");
    assert_eq!(g.result(2), 10);
    assert_eq!(g.result(3), 5, "GetTagData default");
    assert_eq!(g.result(4), 0x1 | 0x4 | 0x10, "PackBoolTags sets the bits of the tags present (data nonzero), leaves the absent one");
    let (n1, n2) = (g.result(5), g.result(6));
    assert_eq!((g.r32(n1), g.r32(n2)), (TAG_USER + 1, TAG_USER + 7), "NextTagItem walks past TAG_IGNORE and through TAG_MORE");
    assert_eq!(g.result(7), 0, "end of list");
}

#[test]
fn call_hook_pkt_runs_a_guest_hook_and_strings_compare() {
    let mut g = G::new();
    let u = util(&mut g);
    // hook entry: move.l (a1),d0 ; add.l a2,d0 ; rts  (message word + object)
    let code = g.bytes(&[0x20, 0x11, 0xD0, 0x8A, 0x4E, 0x75]);
    let hook = g.zeroed(20);
    g.a.mem.write_u32(hook + 8, code).unwrap();
    let msg = g.zeroed(4);
    g.a.mem.write_u32(msg, 1000).unwrap();
    let (s1, s2, s3) = (g.s("Hello"), g.s("hELLO"), g.s("Help"));
    g.run(&[call(u, CALL_HOOK_PKT, &[a(0, hook), a(2, 234), a(1, msg)]), call(u, STRICMP, &[a(0, s1), a(1, s2)]), call(u, STRICMP, &[a(0, s1), a(1, s3)]),
        call(u, STRNICMP, &[a(0, s1), a(1, s3), d(0, 3)]), call(u, SMULT32, &[d(0, (-6i32) as u32), d(1, 7)]), call(u, TO_UPPER, &[d(0, b'q' as u32)])]);
    assert_eq!(g.result(0), 1234, "the guest hook ran with A1 = message, A2 = object");
    assert_eq!(g.result(1), 0, "Stricmp ignores case");
    assert_ne!(g.result(2), 0, "control: Hello vs Help differ");
    assert_eq!(g.result(3), 0, "Strnicmp over the first 3 characters");
    assert_eq!(g.result(4) as i32, -42);
    assert_eq!(g.result(5), b'Q' as u32);
}

const MAP_TAGS: i32 = -60;
const ALLOCATE_TAG_ITEMS: i32 = -66;
const CLONE_TAG_ITEMS: i32 = -72;
const FREE_TAG_ITEMS: i32 = -78;
const REFRESH_TAG_ITEM_CLONES: i32 = -84;
const TAG_IN_ARRAY: i32 = -90;
const FILTER_TAG_ITEMS: i32 = -96;

fn items(g: &G, at: u32, n: u32) -> Vec<(u32, u32)> { (0..n).map(|i| (g.r32(at + 8 * i), g.r32(at + 8 * i + 4))).collect() }

#[test]
fn clone_refresh_and_free_tag_items() {
    let mut g = G::new();
    let u = util(&mut g);
    let more = g.tags(&[(TAG_USER + 7, 70)]);
    let t = g.tags(&[(1, 0), (TAG_USER + 1, 10), (2, more), (TAG_USER + 9, 99)]);
    g.run(&[call(u, CLONE_TAG_ITEMS, &[a(0, t)]), call(u, CLONE_TAG_ITEMS, &[a(0, 0)])]);
    let (c, empty) = (g.result(0), g.result(1));
    assert_ne!(c, 0);
    assert_eq!(items(&g, c, 3), vec![(TAG_USER + 1, 10), (TAG_USER + 7, 70), (0, 0)], "control tags resolved, a flat TAG_DONE-ended copy");
    assert_ne!(empty, 0, "CloneTagItems(NULL) still returns a list");
    assert_eq!(g.r32(empty), 0, "...an empty one");
    // the clone does not alias the original: changing the original and refreshing copies the new data
    g.a.mem.write_u32(t + 12, 11).unwrap();
    assert_eq!(g.r32(c + 4), 10, "control: the clone still holds the old value until refreshed");
    g.run(&[call(u, REFRESH_TAG_ITEM_CLONES, &[a(0, c), a(1, t)]), call(u, FREE_TAG_ITEMS, &[a(0, c)]), call(u, FREE_TAG_ITEMS, &[a(0, empty)]), call(u, FREE_TAG_ITEMS, &[a(0, 0)])]);
    assert_eq!(g.r32(c + 4), 11);
    g.run(&[call(u, ALLOCATE_TAG_ITEMS, &[d(0, 4)]), call(u, ALLOCATE_TAG_ITEMS, &[d(0, 0)])]);
    assert_ne!(g.result(0), 0);
    assert_eq!(g.result(1), 0, "AllocateTagItems(0) is NULL");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}

#[test]
fn filter_and_map_tag_items() {
    let mut g = G::new();
    let u = util(&mut g);
    let keep = g.bytes(&[TAG_USER + 1, TAG_USER + 3, 0].iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<_>>());
    let mk = |g: &mut G| g.tags(&[(TAG_USER + 1, 10), (TAG_USER + 2, 20), (TAG_USER + 3, 30)]);
    let (t1, t2) = (mk(&mut g), mk(&mut g));
    let (inn, ina) = (d(0, TAG_USER + 3), d(0, TAG_USER + 2));
    g.run(&[call(u, TAG_IN_ARRAY, &[inn, a(0, keep)]), call(u, TAG_IN_ARRAY, &[ina, a(0, keep)]),
        call(u, FILTER_TAG_ITEMS, &[a(0, t1), a(1, keep), d(0, 0)]), call(u, FILTER_TAG_ITEMS, &[a(0, t2), a(1, keep), d(0, 1)])]);
    assert_eq!((g.result(0), g.result(1)), (1, 0), "TagInArray: present / absent (negative control)");
    assert_eq!((g.result(2), g.result(3)), (2, 1), "TAGFILTER_AND keeps the hits, TAGFILTER_NOT the misses");
    assert_eq!(items(&g, t1, 3).iter().map(|x| x.0).collect::<Vec<_>>(), vec![TAG_USER + 1, 1, TAG_USER + 3], "AND: the miss became TAG_IGNORE in place");
    assert_eq!(items(&g, t2, 3).iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, TAG_USER + 2, 1], "NOT: the hits became TAG_IGNORE");
    // MapTags: map list {1 -> 0x11 ... }
    let m = g.tags(&[(TAG_USER + 1, TAG_USER + 0x11), (TAG_USER + 2, 0)]);
    let (k, r) = (mk(&mut g), mk(&mut g));
    g.run(&[call(u, MAP_TAGS, &[a(0, k), a(1, m), d(0, 1)]), call(u, MAP_TAGS, &[a(0, r), a(1, m), d(0, 0)])]);
    assert_eq!(items(&g, k, 3).iter().map(|x| x.0).collect::<Vec<_>>(), vec![TAG_USER + 0x11, 1, TAG_USER + 3], "MAP_KEEP_NOT_FOUND: renamed, TAG_DONE value removes, unmapped kept");
    assert_eq!(items(&g, r, 3).iter().map(|x| x.0).collect::<Vec<_>>(), vec![TAG_USER + 0x11, 1, 1], "MAP_REMOVE_NOT_FOUND: unmapped tags are removed too (negative control)");
    assert!(g.a.refused.is_empty(), "{:?}", g.a.refused);
}
