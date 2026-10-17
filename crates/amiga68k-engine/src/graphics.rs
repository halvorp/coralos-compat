//! graphics.library, the text metrics GUI programs lay themselves out with. The only font a guest sees so far is the
//! screen's topaz.font 8 (screen.rs), a fixed 8-pixel-wide font, so a string's pixel width is 8 per character. This
//! is THAT font's metric, not a font engine: a program that SetFont()s another font is refused until fonts exist.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

pub const LVO_TEXT_LENGTH: i32 = -54;
/// topaz.font 8: every glyph is 8 pixels wide.
pub const TOPAZ8_WIDTH: u32 = 8;

/// TextLength(a1 = RastPort, a0 = string, d0 = count) -> pixel width in the RastPort's font.
pub fn text_length<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s, n) = (cpu.areg(0), cpu.dreg(0) & 0xFFFF);
    let mut text = String::new();
    for i in 0..n.min(64) { text.push(a.mem.read_u8(s + i).map_err(Refusal::mem("TextLength.string"))? as char); }
    cpu.set_d0(n * TOPAZ8_WIDTH);
    Ok(format!("[graphics] TextLength {text:?} ({n} chars) -> {}", n * TOPAZ8_WIDTH))
}
