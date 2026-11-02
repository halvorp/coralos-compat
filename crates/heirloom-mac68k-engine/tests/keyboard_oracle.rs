//! The installed keyboard layout (src/keyboard.rs, British) against the Mac OS 9.2.1 System file's own `'KCHR'` 2 "British", read from the owner's
//! install CD export as an ORACLE (never committed; the test skips loudly without it): for every key the engine's layout types a character with,
//! the plain, Shift, Caps Lock and Control characters are the System's. The Option layer and dead keys are not tabled by the engine (0 there).
use heirloom_fmt_resfork::{container, ResFork};

const SYSTEM: &str = "/data/build/corpus-bin/mac/macos921/out/System Folder_System.bin";

#[test]
fn the_installed_layout_agrees_with_the_mac_os_9_british_kchr_for_every_key_it_tables() {
    let Ok(file) = std::fs::read(SYSTEM) else { eprintln!("SKIPPED: the Mac OS 9.2.1 System file export is absent ({SYSTEM})"); return };
    let opened = container::open(&file).expect("a MacBinary System file");
    let fork = ResFork::parse(file[opened.rsrc].to_vec()).expect("its resource fork");
    let apple = fork.data(fork.find(*b"KCHR", 2).expect("KCHR 2 British")).unwrap().to_vec();
    let ours = heirloom_mac68k_engine::keyboard::kchr();
    let table = |k: &[u8], mods: usize, key: usize| -> u8 { k[260 + 128 * usize::from(k[2 + mods]) + key] };
    let mut compared = 0;
    for mods in [0x00, 0x02, 0x04, 0x10] { // plain, shift, caps lock, control
        for key in 0..128 {
            if table(&ours, 0, key) == 0 { continue } // a key the engine does not table
            assert_eq!(table(&ours, mods, key), table(&apple, mods, key), "modifier byte {mods:#x} key {key:#x}");
            compared += 1;
        }
    }
    assert!(compared > 4 * 90, "compared {compared}");
    // and the one Option character the engine tables
    assert_eq!(table(&ours, 0x08, 0x14), table(&apple, 0x08, 0x14), "Option-3");
}
