//! The resources a classic Mac's System file always had and programs take for granted: the four standard cursors (`CURS` 1-4: iBeam,
//! crosshair, plus, watch), searched AFTER every other open resource file, so a program's own `CURS` of the same id wins. Programs
//! ask for them with `GetCursor(iBeamCursor)` and treat NIL as a fatal error (Glider PRO stopped on it with "Couldn't Load a Resource").
//!
//! The 68-byte resources (16x16 image, 16x16 mask, hot spot as v then h in the last four bytes: iBeam (4,7), crosshair (5,6), plus (6,8),
//! watch (9,7)) are the ones in Executor's MIT-licensed `res/System` (read through its AppleDouble `System.ad`); the shapes are the ones
//! Inside Macintosh I documents for `iBeamCursor`, `crossCursor`, `plusCursor` and `watchCursor`.
use alloc::vec::Vec;
use heirloom_fmt_resfork::{writer::ForkBuilder, ResFork};

const CURSORS: [(i16, &str); 4] = [
    (1, "0c60028001000100010001000100010001000100010001000100010002800c60000000000000000000000000000000000000000000000000000000000000000000040007"),
    (2, "020002000200020002007ff00200020002000200020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000050006"),
    (3, "000003e0026002201e3c18041084100c1e3c0220032003e0000000000000000000001ffc1ffc1ffc1ffc1ffc1ffc1ffc1ffc1ffc1ffc1ffc000000000000000000060008"),
    (4, "0000000019983cbc18980ff008101108210421e420041008081007e00c30000000003ffc7ffe7ffe7ffe3ffc3ffc7ffe7ffe7ffe7ffe7ffe3ffc1ff81ff81c3800090007"),
];

fn unhex(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0)).collect() }

/// The built-in System resource fork.
pub fn system_fork() -> ResFork {
    let mut b = ForkBuilder::new();
    for (id, hex) in CURSORS { b = b.add(b"CURS", id, None, &unhex(hex)); }
    ResFork::parse(b.build()).expect("the built-in System fork is well formed")
}
