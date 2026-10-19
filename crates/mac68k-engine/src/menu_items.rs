//! The Menu Manager's per-item attribute accessors: the command-key character (`GetItemCmd` / `SetItemCmd`), the mark
//! (`GetItmMark` / `SetItmMark`), the character style (`GetItmStyle` / `SetItmStyle`) and the icon number (`GetItmIcon` /
//! `SetItmIcon`). Each reads or writes ONE byte of the item's four attribute bytes in the menu's own block (icon, key, mark,
//! style: Inside Macintosh I-"Menu Manager", the menu record), through the same store `CheckItem` uses, so the menu strip the
//! host shows follows. An item past the end is a named refusal (the book says the result is undefined).
use crate::machine::{Call, Done, Mac};
use crate::menu::{edit_item, h_of, Item};
use crate::pascal::Arg;
use crate::Refusal;
use alloc::format;
use guestui_abi::GuestUi;
use retro_m68k::Cpu;

type Field = fn(&mut Item) -> &mut u8;
const FIELDS: [(&str, &str, u16, u16, Field); 4] = [
    ("GetItemCmd", "SetItemCmd", 0xA84E, 0xA84F, |i| &mut i.key),
    ("GetItmMark", "SetItmMark", 0xA943, 0xA944, |i| &mut i.mark),
    ("GetItmStyle", "SetItmStyle", 0xA941, 0xA942, |i| &mut i.style),
    ("GetItmIcon", "SetItmIcon", 0xA93F, 0xA940, |i| &mut i.icon),
];

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    macro_rules! pair { ($($idx:expr),*) => {$(
        m.register(FIELDS[$idx].2, &[Arg::L, Arg::W, Arg::L], None, {
            fn get<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { read(m, c, $idx) } get::<U> })?;
        m.register(FIELDS[$idx].3, &[Arg::L, Arg::W, Arg::B], None, {
            fn set<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> { write(m, c, $idx) } set::<U> })?;
    )*} }
    pair!(0, 1, 2, 3);
    Ok(())
}

fn item_of<U: GuestUi>(m: &Mac<U>, name: &str, h: u32, item: u16) -> Result<Item, Refusal> {
    h_of(m, name, h)?;
    let p = m.menu_view(h)?;
    p.items.get((item as usize).wrapping_sub(1)).cloned().ok_or_else(|| Refusal(format!("{name}: menu {} has {} items, not {item}", p.id, p.items.len())))
}

fn read<U: GuestUi>(m: &mut Mac<U>, c: &Call, idx: usize) -> Result<Done, Refusal> {
    let (h, item, out) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2));
    let mut it = item_of(m, c.name, h, item)?;
    let v = *(FIELDS[idx].4)(&mut it);
    m.mem.write_u8(out, v).map_err(Refusal::mem("menu attribute"))?;
    Ok(Done { result: 0, log: format!("[menu] {} {h:#x} #{item} -> {v:#04x}", c.name) })
}

fn write<U: GuestUi>(m: &mut Mac<U>, c: &Call, idx: usize) -> Result<Done, Refusal> {
    let (h, item, v) = (c.args.get(0), c.args.get(1) as u16, c.args.get(2) as u8);
    if item == 0 { return Err(Refusal(format!("{}: item 0 is not an item", c.name))); }
    edit_item(m, c.name, h, item, |p, i| *(FIELDS[idx].4)(&mut p.items[i - 1]) = v)?;
    Ok(Done { result: 0, log: format!("[menu] {} {h:#x} #{item} = {v:#04x}", c.name) })
}
