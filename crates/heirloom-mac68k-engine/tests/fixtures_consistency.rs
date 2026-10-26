//! Every `.equ _Name, 0xWORD` in the assembly fixtures names the trap the generated table says it does.
use heirloom_mac68k_engine::traps::trap_name;

#[test]
fn trap_words_in_traps_inc_match_the_generated_table() {
    let inc = include_str!("../fixtures/testapp/traps.inc");
    let mut n = 0;
    for line in inc.lines().filter(|l| l.trim_start().starts_with(".equ")) {
        let rest = line.trim_start().trim_start_matches(".equ").trim();
        let (name, word) = rest.split_once(',').unwrap();
        let word = u16::from_str_radix(word.trim().trim_start_matches("0x"), 16).unwrap();
        assert_eq!(trap_name(word), Some(name.trim().trim_start_matches('_')), "{line}");
        n += 1;
    }
    assert!(n > 30, "parsed {n} lines");
}

#[test]
fn negative_control_the_check_can_fail() {
    assert_ne!(trap_name(0xA122), Some("NewPtr"), "a wrong name is detectable");
}
