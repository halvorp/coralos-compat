//! ParsePattern / MatchPattern / MatchFirst / MatchNext driven by a compiled 68k program (tests/guest/c/pattern.c).
mod common;
use common::*;

const PROG: &[u8] = include_bytes!("guest/pattern.bin");
const NEG1: u32 = u32::MAX;

#[test]
fn patterns_and_directory_walks_follow_amigados() {
    let mut a = machine();
    for (p, d) in [("SYS:Docs/a.txt", "a"), ("SYS:Docs/b.txt", "b"), ("SYS:Docs/c.info", "c"), ("SYS:Docs/Sub/d.txt", "d"), ("SYS:Docs/Sub/e.info", "e")] { a.fs.vol.put_file(p, d.as_bytes().to_vec()).unwrap(); }
    a.fs.vol.mkdirs("SYS:Docs/Sub2").unwrap();
    let r = run(&mut a, PROG);
    assert_eq!((r[0], r[1], r[2]), (1, NEG1, 0), "ParsePattern reports wildcards; MatchPattern says yes/no");
    assert_eq!((r[3], r[4]), (NEG1, 0), "NoCase folds, the plain form does not");
    assert_eq!(r[5], 0, "no wildcards: 0");
    assert_eq!((r[6], r[7]), (NEG1, 114), "bad syntax: -1, ERROR_BAD_TEMPLATE");
    assert_eq!((r[8], r[9]), (NEG1, 303), "buffer too small: -1, ERROR_BUFFER_OVERFLOW");
    assert_eq!((r[10], r[11], r[12], r[13], r[14]), (0, 1, 1, 1, 1), "MatchFirst: a.txt with its full path in ap_Buf, ITSWILD, ap_Base/ap_Last chain SYS: -> Docs, the last locking SYS:Docs");
    assert_eq!((r[15], r[16]), (0, 1), "MatchNext: b.txt");
    assert_eq!((r[17], r[18]), (232, 232), "then ERROR_NO_MORE_ENTRIES, also in IoErr");
    assert_eq!(r[19], 0, "MatchEnd clears ap_Base");
    assert_eq!((r[20], r[21], r[22]), (11, 3, 232), "a directory left with APF_DODIR set is walked recursively and each directory is reported again with APF_DIDDIR");
    assert_eq!(r[23], 1, "clearing APF_DODIR keeps MatchNext out of the directory");
    assert_eq!(r[24], 7, "entering only the first directory lists one level");
    assert_eq!((r[25], r[26], r[27], r[28]), (0, 1, 232, 1), "two wildcard components: SYS:#?/#?.info finds Docs/c.info only; two AChains while inside");
    assert_eq!((r[29], r[30]), (205, 232), "a missing directory is 205; a wildcard with no match is 232");
    assert_eq!((r[31], r[32], r[33]), (0, 1, 232), "a plain name matches itself, ITSWILD clear, a file is not entered");
    assert_eq!(r[34], 303, "ap_Buf too small for the path: ERROR_BUFFER_OVERFLOW");
    assert!(a.fs.anchors.live() == 0, "every walk was ended or failed cleanly");
    let cwd = a.mem.read_u32(0x6000 + 152).unwrap(); // pr_CurrentDir: the process owns this lock for its whole life
    assert!(a.fs.locks.keys().all(|&l| a.fs.dos.entries.iter().any(|e| e.lock_bptrs().contains(&(l >> 2))) || l == 0 || l >> 2 == cwd), "no AChain lock leaked: {:?}", a.fs.locks.keys().collect::<Vec<_>>());
}
