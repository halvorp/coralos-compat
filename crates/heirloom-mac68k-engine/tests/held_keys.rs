//! The keys a game plays with (h86, Warcraft III): a key that types nothing reaches the program as its Mac key (the arrows are the camera
//! keys), the modifiers the host holds ride on the event, and `GetKeys` reports the keys held from their downs and ups.
mod common;
use common::{Prog, Rig};
use heirloom_mac68k_engine::Delivered;
use heirloom_ui_abi::{host_key, GuestUiEvent, Modifiers, UiObjectRef};

const KEYS: u32 = 0x3100;

fn native(g: &Rig) -> UiObjectRef { g.m.windows[&g.win].native.unwrap() }
fn key(g: &mut Rig, code: u32, text: &str, m: u8, down: bool) -> Result<Delivered, heirloom_mac68k_engine::Refusal> {
    let win = native(g);
    g.m.deliver(GuestUiEvent::Key { win, code, text: text.into(), modifiers: Modifiers(m), down })
}
/// (what, message, modifiers) of the last queued event.
fn last(g: &Rig) -> (u16, u32, u16) { g.m.ev.queue.back().map(|e| (e.what, e.message, e.modifiers)).unwrap() }
/// `GetKeys(KEYS)` run by a program, the 16 bytes it wrote.
fn get_keys(g: &mut Rig) -> Vec<u8> {
    for i in 0..16 { g.m.mem.write_u8(KEYS + i, 0xEE).unwrap(); }
    g.run(Prog::new().l(KEYS).trap(0xA976).exit());
    (0..16).map(|i| g.m.mem.read_u8(KEYS + i).unwrap()).collect()
}
/// The key codes a KeyMap holds (bit k % 8 of byte k / 8: Inside Macintosh, Toolbox Event Manager, GetKeys).
fn held(map: &[u8]) -> Vec<u8> { (0..128u8).filter(|k| map[usize::from(k >> 3)] >> (k & 7) & 1 != 0).collect() }

/// A key with no text is the Mac key in the key-code byte and the character that key types on a Mac in the low byte (Inside Macintosh:
/// Macintosh Toolbox Essentials, the ADB key codes; the arrows type 0x1C..0x1F, the function keys 0x10). Before h86 the host's C0 wire
/// byte arrived AS the text and was taken for a character: Up (0x10) became the function-key character, so no camera key ever reached
/// the game. The modifiers ride along; Ctrl with a letter's control character is that letter's key; British Shift-3 types the pound sign.
#[test]
fn a_key_that_types_nothing_is_its_mac_key_and_the_modifiers_ride_on_the_event() {
    let mut g = Rig::new();
    for (code, message) in [
        (host_key::UP, 0x7E1E), (host_key::DOWN, 0x7D1F), (host_key::LEFT, 0x7B1C), (host_key::RIGHT, 0x7C1D),
        (host_key::HOME, 0x7301), (host_key::END, 0x7704), (host_key::PAGE_UP, 0x740B), (host_key::PAGE_DOWN, 0x790C),
        (host_key::DELETE, 0x757F), (host_key::INSERT, 0x7205), (host_key::F1, 0x7A10), (host_key::F1 + 4, 0x6010), (host_key::F1 + 11, 0x6F10),
    ] {
        assert_eq!(key(&mut g, code, "", 0, true), Ok(Delivered::Queued), "code {code:#x}");
        assert_eq!(last(&g), (3, message, 0), "code {code:#x}");
        key(&mut g, code, "", 0, false).unwrap();
    }
    key(&mut g, host_key::UP, "", Modifiers::SHIFT, true).unwrap();
    assert_eq!(last(&g), (3, 0x7E1E, 0x0200), "Shift+Up: shiftKey");
    key(&mut g, 'P' as u32, "P", Modifiers::SHIFT, true).unwrap();
    assert_eq!(last(&g), (3, 0x2350, 0x0200), "a typed capital carries shiftKey");
    key(&mut g, 1, "\u{1}", Modifiers::CTRL, true).unwrap();
    assert_eq!(last(&g), (3, 0x0001, 0x1000), "Ctrl-A is the A key (0x00) typing 0x01, not Home");
    key(&mut g, '1' as u32, "1", Modifiers::ALT | Modifiers::CTRL, true).unwrap();
    assert_eq!(last(&g), (3, 0x1231, 0x1800), "Alt is optionKey, Ctrl controlKey");
    key(&mut g, 0xA3, "£", Modifiers::SHIFT, true).unwrap();
    assert_eq!(last(&g), (3, 0x14A3, 0x0200), "the pound sign is Mac Roman 0xA3 on British Shift-3");
    key(&mut g, 0x1B, "\u{1b}", 0, true).unwrap();
    assert_eq!(last(&g), (3, 0x351B, 0), "Escape types itself on the Escape key");
    // NEGATIVE CONTROL: a code that types nothing and is no key the Mac has is a named refusal, and queues nothing.
    let n = g.m.ev.queue.len();
    assert!(key(&mut g, 0x7777, "", 0, true).unwrap_err().0.contains("no key"));
    assert_eq!(g.m.ev.queue.len(), n);
}

/// `GetKeys` is the keys held: each down sets its key's bit and its release clears it, and a held key's modifiers set the modifier keys'
/// bits (Shift 0x38, Option 0x3A, Control 0x3B) while it is held. Warcraft III reads GetKeys at startup (Option held = byte 7 bit 2) and
/// Storm keeps its own count of keys held; before h86 the map was always zero.
#[test]
fn get_keys_reports_the_keys_held_and_the_modifiers_held_with_them() {
    let mut g = Rig::new();
    assert_eq!(held(&get_keys(&mut g)), Vec::<u8>::new(), "NEGATIVE CONTROL: nothing held, and every byte written");
    key(&mut g, host_key::UP, "", Modifiers::SHIFT, true).unwrap();
    assert_eq!(held(&get_keys(&mut g)), [0x38, 0x7E], "Up and Shift");
    key(&mut g, 'p' as u32, "p", 0, true).unwrap();
    key(&mut g, '1' as u32, "1", Modifiers::ALT, true).unwrap();
    let map = get_keys(&mut g);
    assert_eq!(held(&map), [0x12, 0x23, 0x38, 0x3A, 0x7E], "+ P, 1 and Option");
    assert_eq!(map[7] >> 2 & 1, 1, "Option is byte 7 bit 2, the bit Warcraft III tests");
    key(&mut g, host_key::UP, "", Modifiers::SHIFT, false).unwrap();
    key(&mut g, '1' as u32, "1", Modifiers::ALT, false).unwrap();
    assert_eq!(held(&get_keys(&mut g)), [0x23], "released: Up, 1, and the modifiers held only with them");
    key(&mut g, 'p' as u32, "p", 0, false).unwrap();
    assert_eq!(held(&get_keys(&mut g)), Vec::<u8>::new());
}
