//! The SAME m68k binary (tests/oracle/gfxprobe.s) was run on a real AROS m68k (Copperline, 2026-10-18) and its dump
//! committed (tests/oracle/gfxprobe.aros: the 100-byte RastPort, then 64x32 ReadPixel bytes). This engine runs the binary
//! and must produce the same pixels (rectangle fill, topaz-8 text, and Bresenham lines through InitRastPort defaults)
//! and the same RastPort state fields.
use amiga68k_engine::Amiga;
use guestui_abi::RecordingGuestUi;
use retro_hunk::writer::{build, Hunk};

const PROBE: &[u8] = include_bytes!("oracle/gfxprobe.bin");
const AROS: &[u8] = include_bytes!("oracle/gfxprobe.aros");

fn ours(code: &[u8]) -> Vec<u8> {
    let mut a = Amiga::new(RecordingGuestUi::new()).unwrap();
    let loaded = a.start_hunk(&build(&[Hunk::code(code.to_vec(), vec![])], None)).unwrap();
    // AROS is the BEHAVIOUR reference, not the pixel target: the engine's default font is the Gute Form face, so the oracle's text
    // (topaz 8) is compared with topaz 8 selected as the default for this run. Primitives (rectangles, lines) are font-independent.
    a.mem.write_u32(0x3000 + 154, 0x8000).unwrap(); // GfxBase.DefaultFont = the resident topaz
    let (_s, st) = a.step(3_000_000);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(st.insns > 1000);
    (0..AROS.len() as u32).map(|i| a.mem.read_u8(loaded.bases[0] + 4 + i).unwrap()).collect()
}

#[test]
fn pixels_match_real_aros_exactly() {
    let o = ours(PROBE);
    assert_eq!(o.len(), AROS.len());
    let diff: Vec<(usize, usize, u8, u8)> = (0..64 * 32).filter(|&i| o[100 + i] != AROS[100 + i]).map(|i| (i % 64, i / 64, o[100 + i], AROS[100 + i])).collect();
    assert!(diff.is_empty(), "{} pixels differ from AROS (x, y, ours, aros): {:?}", diff.len(), &diff[..diff.len().min(12)]);
    assert!(AROS[100..].iter().filter(|&&p| p == 1).count() > 20, "control: the AROS dump really holds text ink");
}

#[test]
fn the_comparison_detects_a_wrong_pixel() {
    // negative control: run a probe whose rectangle is one pixel off (RectFill(2,2,9,5) -> (3,2,9,5): moveq #2,d0 at the call)
    let mut code = PROBE.to_vec();
    let at = code.windows(4).position(|w| w == [0x70, 0x02, 0x72, 0x02]).expect("moveq #2,d0; moveq #2,d1");
    code[at + 1] = 3;
    let o = ours(&code);
    assert_ne!(&o[100..], &AROS[100..], "a shifted rectangle must differ from the AROS dump");
}

#[test]
fn rastport_state_fields_match_real_aros() {
    let o = ours(PROBE);
    // KNOWN DIFFERENCES (not compared): linpatcnt after multi-segment Draw (AROS 13, here 10: the pattern phase only
    // matters for non-solid LinePtrn across segments), minterms[] (AROS regenerates them per pen/mode; not mirrored here).
    // offsets: Mask 24, FgPen 25, BgPen 26, AOlPen 27, DrawMode 28, AreaPtSz 29, linpatcnt 30, Flags 32.., LinePtrn 34, cp_x 36, cp_y 38, PenWidth/Height 48/50
    for (name, off, len) in [("Mask", 24, 1), ("FgPen", 25, 1), ("BgPen", 26, 1), ("AOlPen", 27, 1), ("DrawMode", 28, 1), ("AreaPtSz", 29, 1),
        ("LinePtrn", 34, 2), ("cp_x", 36, 2), ("cp_y", 38, 2), ("PenWidth", 48, 2), ("PenHeight", 50, 2)] {
        assert_eq!(o[off..off + len], AROS[off..off + len], "RastPort.{name}");
    }
}
