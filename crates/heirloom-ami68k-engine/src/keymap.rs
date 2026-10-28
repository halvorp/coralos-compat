//! keymap.library: AskKeyMapDefault, SetKeyMapDefault, MapRawKey and MapANSI over a REAL `struct KeyMap` in guest memory.
//!
//! The default keymap is the US PC layout (AROS workbench/devs/keymaps/pc104_usa.akmd, APL, facts only: keymap_us.rs is generated from
//! it by tools/gen-keymap-us.py) laid out at the first use as the eight table pointers of `struct KeyMap` (devices/keymap.h; 32 bytes:
//! km_LoKeyMapTypes 0, km_LoKeyMap 4, km_LoCapsable 8, km_LoRepeatable 12, km_HiKeyMapTypes 16, km_HiKeyMap 20, km_HiCapsable 24,
//! km_HiRepeatable 28) plus the tables themselves, so a program can read, copy or replace it (SetKeyMapDefault takes any KeyMap, and the
//! two mapping functions take an explicit one). The algorithms are keymap.library's (AROS rom/keymap maprawkey.c, mapansi.c, support.c;
//! read for facts): the key's type picks one of four characters / string descriptors / dead-key descriptors by the qualifiers pressed,
//! caps lock shifts the "capsable" keys, a repeating event is dropped for keys that do not repeat, dead keys modify the next key through the
//! two previous keypresses the event carries, and MapANSI inverts all of it into (rawkey, qualifier) byte pairs. `MapANSI`'s `length`
//! counts those pairs, as in AROS' code (it returns the number of pairs).
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::format;
use crate::keymap_us::{Hi, HI_CAPS, HI_MAP, HI_REPEAT, HI_TYPES, LO_CAPS, LO_MAP, LO_REPEAT, LO_TYPES, STRINGS};
use crate::libtable::LibraryBuilder;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use crate::cpu::Cpu;

pub const KCF_SHIFT: u8 = 1;
pub const KCF_ALT: u8 = 2;
pub const KCF_CONTROL: u8 = 4;
pub const KCF_DEAD: u8 = 32;
pub const KCF_STRING: u8 = 64;
pub const KCF_NOP: u8 = 128;
pub const KC_VANILLA: u8 = 7;
pub const DPF_MOD: u8 = 1;
pub const DPF_DEAD: u8 = 8;
const IECLASS_RAWKEY: u8 = 1;
const IECODE_UP_PREFIX: u16 = 0x80;
pub const IEQUALIFIER_LSHIFT: u16 = 1;
pub const IEQUALIFIER_RSHIFT: u16 = 2;
pub const IEQUALIFIER_CAPSLOCK: u16 = 4;
pub const IEQUALIFIER_CONTROL: u16 = 8;
pub const IEQUALIFIER_LALT: u16 = 16;
pub const IEQUALIFIER_RALT: u16 = 32;
pub const IEQUALIFIER_REPEAT: u16 = 512;
/// struct InputEvent (22 bytes): ie_Class 4, ie_Code 6, ie_Qualifier 8, ie_position 10 (the dead-key fields ie_prev1DownCode 10, Qual 11, 2: 12, 13).
const IE_CLASS: u32 = 4;
const IE_CODE: u32 = 6;
const IE_QUALIFIER: u32 = 8;
const IE_PREV: u32 = 10;
pub const KEYMAP_SIZE: u32 = 32;

/// keymaptype_table / keymapstr_table of AROS' defaultkeymap.c (the variant that ignores qualifiers a key type does not list): the
/// index of the character (3 = no qualifier ... 0 = shift+alt) / of the descriptor, by the key type's qualifier bits and the pressed ones.
/// -1 = no mapping, -2 = the Ctrl-C-like combination (character 3 with bits 5 and 6 cleared).
const TYPE_TABLE: [[i8; 8]; 8] = [[3, 3, 3, 3, 3, 3, 3, 3], [3, 2, 3, 2, 3, 2, 3, 2], [3, 3, 2, 2, 3, 3, 2, 2], [3, 2, 1, 0, 3, 2, 1, 0],
    [3, 3, 3, 3, 2, 2, 2, 2], [3, 2, 3, 2, 1, 0, 1, 0], [3, 3, 2, 2, 1, 1, 0, 0], [3, 2, 1, 0, -2, -1, -1, -1]];
const STR_TABLE: [[i8; 8]; 8] = [[0; 8], [0, 1, 0, 1, 0, 1, 0, 1], [0, 0, 1, 1, 0, 0, 1, 1], [0, 1, 2, 3, 0, 1, 2, 3], [0, 0, 0, 0, 1, 1, 1, 1],
    [0, 1, 0, 1, 2, 3, 2, 3], [0, 0, 1, 1, 2, 2, 3, 3], [0, 1, 2, 3, 4, 5, 6, 7]];
const NUM_KEYS: [usize; 8] = [1, 2, 2, 4, 2, 4, 4, 4];
const NUM_KEYS2: [usize; 8] = [1, 2, 2, 4, 2, 4, 4, 8];
const L: u16 = IEQUALIFIER_LSHIFT;
const A: u16 = IEQUALIFIER_LALT;
const C: u16 = IEQUALIFIER_CONTROL;
const QUALIFIERS: [[u16; 4]; 8] = [[0, 0, 0, 0], [0, L, 0, 0], [0, A, 0, 0], [0, L, A, L | A], [0, C, 0, 0], [0, L, C, L | C], [0, A, C, A | C], [0, L, A, L | A]];
const QUALIFIERS2: [[u16; 8]; 8] = [[0; 8], [0, L, 0, 0, 0, 0, 0, 0], [0, A, 0, 0, 0, 0, 0, 0], [0, L, A, L | A, 0, 0, 0, 0], [0, C, 0, 0, 0, 0, 0, 0],
    [0, L, C, L | C, 0, 0, 0, 0], [0, A, C, A | C, 0, 0, 0, 0], [0, L, A, L | A, C, C | L, C | A, C | A | L]];

#[derive(Default)]
pub struct State { pub default: u32 }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AskKeyMapDefault", ask_default::<U>).named("SetKeyMapDefault", set_default::<U>).named("MapRawKey", map_raw_key::<U>).named("MapANSI", map_ansi::<U>)
}

/// Lay the built-in US keymap out in guest memory with `alloc` (one block of `total` bytes); returns the `struct KeyMap` address.
pub fn build_default(mem: &mut GuestMem, at: u32) -> Result<u32, Refusal> {
    let m = Refusal::mem("default KeyMap");
    // Layout: KeyMap, lo types 64, lo map 256, lo caps 8, lo repeat 8, hi types 56, hi map 224, hi caps 8, hi repeat 8, strings.
    let (lt, lm, lc, lr) = (at + KEYMAP_SIZE, at + KEYMAP_SIZE + 64, at + KEYMAP_SIZE + 64 + 256, at + KEYMAP_SIZE + 64 + 256 + 8);
    let (ht, hm, hc, hr) = (lr + 8, lr + 8 + 56, lr + 8 + 56 + 224, lr + 8 + 56 + 224 + 8);
    let mut s = hr + 8;
    for (i, p) in [lt, lm, lc, lr, ht, hm, hc, hr].into_iter().enumerate() { mem.write_u32(at + 4 * i as u32, p).map_err(&m)?; }
    mem.write_bytes(lt, &LO_TYPES).map_err(&m)?;
    for (i, v) in LO_MAP.iter().enumerate() { mem.write_u32(lm + 4 * i as u32, *v).map_err(&m)?; }
    mem.write_bytes(lc, &LO_CAPS).map_err(&m)?;
    mem.write_bytes(lr, &LO_REPEAT).map_err(&m)?;
    mem.write_bytes(ht, &HI_TYPES).map_err(&m)?;
    let mut str_at = Vec::new();
    for (_, d) in STRINGS.iter() { mem.write_bytes(s, d).map_err(&m)?; str_at.push(s); s += d.len() as u32; }
    for (i, e) in HI_MAP.iter().enumerate() { mem.write_u32(hm + 4 * i as u32, match e { Hi::Bytes(v) => *v, Hi::Str(n) => str_at[*n] }).map_err(&m)?; }
    mem.write_bytes(hc, &HI_CAPS).map_err(&m)?;
    mem.write_bytes(hr, &HI_REPEAT).map_err(&m)?;
    Ok(at)
}

/// Bytes the built-in keymap needs in guest memory.
pub fn default_size() -> u32 { KEYMAP_SIZE + 64 + 256 + 8 + 8 + 56 + 224 + 8 + 8 + STRINGS.iter().map(|(_, d)| d.len() as u32).sum::<u32>() }

/// The keymap tables of one `struct KeyMap`, read from guest memory.
pub struct Km<'a> { mem: &'a GuestMem, base: u32 }

struct Key { ty: u8, mapping: u32, qual: u8 }

impl<'a> Km<'a> {
    pub fn new(mem: &'a GuestMem, base: u32) -> Self { Km { mem, base } }
    fn e(&self) -> impl Fn(heirloom_mem_flat32::MemError) -> Refusal { Refusal::mem("KeyMap") }
    fn p(&self, field: u32) -> Result<u32, Refusal> { self.mem.read_u32(self.base + field).map_err(self.e()) }
    fn b(&self, addr: u32) -> Result<u8, Refusal> { self.mem.read_u8(addr).map_err(self.e()) }
    fn lo_type(&self, i: u32) -> Result<u8, Refusal> { self.b(self.p(0)? + i) }
    fn lo_map(&self, i: u32) -> Result<u32, Refusal> { self.mem.read_u32(self.p(4)? + 4 * i).map_err(self.e()) }
    fn hi_type(&self, i: u32) -> Result<u8, Refusal> { self.b(self.p(16)? + i) }
    fn hi_map(&self, i: u32) -> Result<u32, Refusal> { self.mem.read_u32(self.p(20)? + 4 * i).map_err(self.e()) }
    fn bit(&self, table: u32, i: u32) -> Result<bool, Refusal> { Ok(self.b(self.p(table)? + i / 8)? & (1 << (i & 7)) != 0) }

    /// GetKeyInfo: the key's type and mapping for raw key `code` under qualifiers `qual`, or None (key up, past the table, a repeat of a key that does not repeat).
    fn key_info(&self, code: u16, qual: u16) -> Result<Option<Key>, Refusal> {
        if code & IECODE_UP_PREFIX != 0 || code >= 128 { return Ok(None); }
        let mut q = 0u8;
        if qual & (IEQUALIFIER_LSHIFT | IEQUALIFIER_RSHIFT) != 0 { q |= KCF_SHIFT; }
        if qual & (IEQUALIFIER_LALT | IEQUALIFIER_RALT) != 0 { q |= KCF_ALT; }
        if qual & IEQUALIFIER_CONTROL != 0 { q |= KCF_CONTROL; }
        let (ty, mapping, capsable, repeatable) = if code <= 0x3F {
            let i = code as u32;
            (self.lo_type(i)?, self.lo_map(i)?, self.bit(8, i)?, self.bit(12, i)?)
        } else {
            let i = code as u32 - 0x40;
            if i >= 0x38 { return Ok(None); }
            (self.hi_type(i)?, self.hi_map(i)?, self.bit(24, i)?, self.bit(28, i)?)
        };
        if qual & IEQUALIFIER_REPEAT != 0 && !repeatable { return Ok(None); }
        if qual & IEQUALIFIER_CAPSLOCK != 0 && capsable { q |= KCF_SHIFT; }
        Ok(Some(Key { ty, mapping, qual: q }))
    }

    /// GetDeadKeyIndex: the dead-key index a previous keypress stands for, or -1.
    fn dead_index(&self, code: u16, qual: u16) -> Result<i32, Refusal> {
        if code >= 0x78 { return Ok(-1); }
        let Some(k) = self.key_info(code, qual)? else { return Ok(-1) };
        if k.ty & KCF_DEAD == 0 { return Ok(-1); }
        let idx = STR_TABLE[(k.ty & KC_VANILLA) as usize][k.qual as usize];
        if idx == -1 { return Ok(-1); }
        if self.b(k.mapping + 2 * idx as u32)? == DPF_DEAD { Ok(self.b(k.mapping + 2 * idx as u32 + 1)? as i32) } else { Ok(-1) }
    }

    /// MapRawKey: the bytes a raw key event types (None = the buffer is too small).
    pub fn map_raw_key(&self, class: u8, code: u16, qual: u16, prev: [u8; 4], room: i32) -> Result<Option<Vec<u8>>, Refusal> {
        if class != IECLASS_RAWKEY || code >= 0x78 { return Ok(Some(Vec::new())); }
        let Some(k) = self.key_info(code, qual)? else { return Ok(Some(Vec::new())) };
        let out: Vec<u8> = match k.ty & (KCF_STRING | KCF_DEAD | KCF_NOP) {
            0 => match TYPE_TABLE[(k.ty & KC_VANILLA) as usize][k.qual as usize] {
                -1 => vec![],
                -2 => vec![((k.mapping & 0xFF) as u8) & !0x60].into_iter().filter(|&c| c != 0).collect(),
                i => { let c = ((k.mapping >> ((3 - i as u32) * 8)) & 0xFF) as u8; if c != 0 { vec![c] } else { vec![] } }
            },
            KCF_STRING => match STR_TABLE[(k.ty & KC_VANILLA) as usize][k.qual as usize] {
                -1 => vec![],
                idx => {
                    let (len, off) = (self.b(k.mapping + 2 * idx as u32)? as u32, self.b(k.mapping + 2 * idx as u32 + 1)? as u32);
                    (0..len).map(|i| self.b(k.mapping + off + i)).collect::<Result<_, _>>()?
                }
            },
            KCF_DEAD => match STR_TABLE[(k.ty & KC_VANILLA) as usize][k.qual as usize] {
                -1 => vec![],
                idx => {
                    let (dt, dv) = (self.b(k.mapping + 2 * idx as u32)?, self.b(k.mapping + 2 * idx as u32 + 1)?);
                    if dt == 0 { vec![dv] }
                    else if dt == DPF_DEAD { vec![] } // a dead key types nothing by itself
                    else if dt == DPF_MOD {
                        let mut dk = 0i32;
                        let d1 = self.dead_index(prev[0] as u16, prev[1] as u16)?;
                        if d1 != -1 {
                            dk = d1;
                            if d1 >> 4 != 0 {
                                dk = (d1 & 0x0f) * (d1 >> 4);
                                let d2 = self.dead_index(prev[2] as u16, prev[3] as u16)?;
                                if d2 != -1 { dk += d2 & 0x0f; }
                            }
                        }
                        vec![self.b(k.mapping + dv as u32 + dk as u32)?]
                    } else { vec![] }
                }
            },
            _ => vec![], // KCF_NOP, or a type with several of the flags
        };
        Ok(if out.len() as i32 > room { None } else { Some(out) })
    }

    /// Every key of the map in the order MapANSI looks at them: raw codes 0x67 down to 0x40, then 0x3F down to 0x00.
    fn keys_descending(&self) -> Result<Vec<(u8, u8, u32)>, Refusal> {
        let mut v = Vec::new();
        for code in (0x40u32..=0x67).rev() { v.push((code as u8, self.hi_type(code - 0x40)?, self.hi_map(code - 0x40)?)); }
        for code in (0u32..=0x3F).rev() { v.push((code as u8, self.lo_type(code)?, self.lo_map(code)?)); }
        Ok(v)
    }

    /// MapANSI: (code, qualifier) byte pairs for `string`; Ok(Err(())) = the buffer (`length` pairs) is too small; Ok(Ok(None)) = a character no key makes.
    pub fn map_ansi(&self, string: &[u8], length: i32) -> Result<Result<Option<Vec<u8>>, ()>, Refusal> {
        let keys = self.keys_descending()?;
        let (mut max_dead, mut max_double) = (0usize, 0usize);
        let (mut dead_code, mut dead_qual, mut dd_code, mut dd_qual) = ([0u8; 16], [0xFFu8; 16], [0u8; 16], [0xFFu8; 16]);
        for &(code, ty, descr) in &keys {
            if ty & KCF_NOP == 0 && ty & KCF_DEAD != 0 {
                for k in 0..NUM_KEYS2[(ty & 7) as usize] {
                    if self.b(descr + 2 * k as u32)? == DPF_DEAD {
                        let v = self.b(descr + 2 * k as u32 + 1)?;
                        let (index, double) = ((v & 0x0f) as usize, v >> 4);
                        let my = QUALIFIERS2[(ty & 7) as usize][k] as u8;
                        if index > max_dead || dead_qual[index] & my == my { dead_code[index] = code; dead_qual[index] = my; if index > max_dead { max_dead = index; } }
                        if double != 0 && (index > max_double || dd_qual[index] & my == my) { dd_code[index] = code; dd_qual[index] = my; if index > max_double { max_double = index; } }
                    }
                }
            }
        }
        let num_deads = (max_double + 1) * (max_dead + 1);
        let (mut out, mut left, mut at) = (Vec::new(), length, 0usize);
        while at < string.len() {
            let (count, ch) = (string.len() - at, string[at]);
            let (mut found_len, mut found_code, mut found_qual, mut dead_idx) = (0usize, 0u8, !0u32, 0usize);
            for &(code, ty, descr) in &keys {
                let tq = (ty & 7) as usize;
                if ty & KCF_NOP != 0 { continue; }
                if ty & KCF_DEAD != 0 {
                    if found_len <= 1 {
                        for k in 0..NUM_KEYS2[tq] {
                            let (kind, val) = (self.b(descr + 2 * k as u32)?, self.b(descr + 2 * k as u32 + 1)?);
                            if kind == 0 {
                                if val == ch {
                                    let my = QUALIFIERS2[tq][k] as u32;
                                    if found_qual & my == my { found_len = 1; found_code = code; found_qual = my; dead_idx = 0; }
                                }
                            } else if kind == DPF_MOD {
                                for l in 0..num_deads {
                                    if self.b(descr + val as u32 + l as u32)? == ch {
                                        let my = QUALIFIERS2[tq][k] as u32;
                                        if found_len == 0 || (l <= dead_idx && found_qual & my == my) { found_len = 1; found_code = code; found_qual = my; dead_idx = l; }
                                    }
                                }
                            }
                        }
                    }
                } else if ty & KCF_STRING != 0 {
                    for k in 0..NUM_KEYS2[tq] {
                        let (len, off) = (self.b(descr + 2 * k as u32)? as usize, self.b(descr + 2 * k as u32 + 1)? as u32);
                        if len <= count && len >= found_len {
                            let mut same = true;
                            for i in 0..len { if string[at + i] != self.b(descr + off + i as u32)? { same = false; break; } }
                            if same {
                                let my = QUALIFIERS2[tq][k] as u32;
                                if len > found_len || dead_idx != 0 || found_qual & my == my { found_len = len; found_code = code; found_qual = my; dead_idx = 0; }
                            }
                        }
                    }
                } else if found_len <= 1 {
                    let (mut d, descr2) = (descr, descr);
                    for k in 0..NUM_KEYS[tq] {
                        if d as u8 == ch {
                            let my = QUALIFIERS[tq][k] as u32;
                            if dead_idx != 0 || found_qual & my == my { found_len = 1; found_code = code; found_qual = my; dead_idx = 0; }
                            break;
                        }
                        d >>= 8;
                    }
                    if ty == KC_VANILLA && ch & 0x60 == 0 && (found_len == 0 || dead_idx != 0) {
                        let (mut my, mut d2) = (IEQUALIFIER_CONTROL as u8, descr2);
                        if ch & 0x80 != 0 { my |= IEQUALIFIER_LALT as u8; d2 = d2.rotate_left(16); }
                        if d2 & 0xC0 == 0x40 && d2 & 0x1F != (ch & 0x1F) as u32 { my |= IEQUALIFIER_LSHIFT as u8; d2 >>= 8; }
                        let mut k = 0;
                        while k < 4 && d2 & 0xC0 != 0x40 { k += 1; d2 >>= 8; }
                        if k < 4 && d2 & 0x1F == (ch & 0x1F) as u32 { found_len = 1; found_code = code; found_qual = my as u32; dead_idx = 0; }
                    }
                }
            }
            if found_len == 0 { return Ok(Ok(None)); }
            left -= 1 + (dead_idx != 0) as i32 + (dead_idx > max_dead) as i32;
            if left < 0 { return Ok(Err(())); }
            if dead_idx != 0 {
                if dead_idx > max_dead {
                    out.extend([dd_code[dead_idx % (max_dead + 1)], dd_qual[dead_idx % (max_dead + 1)]]);
                    dead_idx /= max_dead + 1;
                    out.extend([dd_code[dead_idx], dd_qual[dead_idx]]);
                } else { out.extend([dead_code[dead_idx], dead_qual[dead_idx]]); }
            }
            out.extend([found_code, found_qual as u8]);
            at += found_len;
        }
        Ok(Ok(Some(out)))
    }
}

/// The default keymap's address, building it on first use.
pub fn default_keymap<U: GuestUi>(a: &mut Amiga<U>) -> Result<u32, Refusal> {
    if a.keymap.default == 0 {
        let at = crate::fs::alloc_zeroed_sys(a, default_size())?.ok_or_else(|| Refusal("keymap.library: no memory for the default KeyMap (NOMEM)".into()))?;
        a.keymap.default = build_default(&mut a.mem, at)?;
    }
    Ok(a.keymap.default)
}

fn ask_default<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let km = default_keymap(a)?;
    cpu.set_d0(km);
    Ok(format!("[keymap] AskKeyMapDefault -> {km:#x}"))
}

fn set_default<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let km = cpu.areg(0);
    default_keymap(a)?; // the built-in one stays alive for anyone holding it
    a.keymap.default = km;
    Ok(format!("[keymap] SetKeyMapDefault {km:#x}"))
}

/// MapRawKey(a0 = InputEvent, a1 = buffer, d1 = length, a2 = keymap or NULL for the default) -> bytes written, or -1 when the buffer is too small.
fn map_raw_key<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ev, buf, len, km) = (cpu.areg(0), cpu.areg(1), cpu.dreg(1) as i32, cpu.areg(2));
    let km = if km == 0 { default_keymap(a)? } else { km };
    let m = Refusal::mem("InputEvent");
    let (class, code, qual) = (a.mem.read_u8(ev + IE_CLASS).map_err(&m)?, a.mem.read_u16(ev + IE_CODE).map_err(&m)?, a.mem.read_u16(ev + IE_QUALIFIER).map_err(&m)?);
    let mut prev = [0u8; 4];
    for (i, p) in prev.iter_mut().enumerate() { *p = a.mem.read_u8(ev + IE_PREV + i as u32).map_err(&m)?; }
    let out = Km::new(&a.mem, km).map_raw_key(class, code, qual, prev, len)?;
    let n = match out {
        Some(bytes) => { a.mem.write_bytes(buf, &bytes).map_err(Refusal::mem("MapRawKey buffer"))?; bytes.len() as i32 }
        None => -1,
    };
    cpu.set_d0(n as i16 as i32 as u32);
    Ok(format!("[keymap] MapRawKey code={code:#x} qual={qual:#x} -> {n}"))
}

/// MapANSI(a0 = string, d0 = count, a1 = buffer, d1 = length (in pairs), a2 = keymap or NULL) -> pairs written, 0 for a character no key makes, -1 when the buffer is too small.
fn map_ansi<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (s, count, buf, len, km) = (cpu.areg(0), cpu.dreg(0) as i32, cpu.areg(1), cpu.dreg(1) as i32, cpu.areg(2));
    let km = if km == 0 { default_keymap(a)? } else { km };
    let string: Vec<u8> = (0..count.max(0) as u32).map(|i| a.mem.read_u8(s + i).map_err(Refusal::mem("MapANSI string"))).collect::<Result<_, _>>()?;
    let r = Km::new(&a.mem, km).map_ansi(&string, len)?;
    let n = match r {
        Err(()) => -1,
        Ok(None) => 0,
        Ok(Some(bytes)) => { a.mem.write_bytes(buf, &bytes).map_err(Refusal::mem("MapANSI buffer"))?; (bytes.len() / 2) as i32 }
    };
    cpu.set_d0(n as u32);
    Ok(format!("[keymap] MapANSI {count} chars -> {n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: u32 = 0x1000;
    fn mem() -> GuestMem { let mut m = GuestMem::new(0x10000); build_default(&mut m, AT).unwrap(); m }
    fn raw(m: &GuestMem, code: u16, qual: u16) -> Vec<u8> { Km::new(m, AT).map_raw_key(IECLASS_RAWKEY, code, qual, [0; 4], 16).unwrap().unwrap() }
    const SHIFT: u16 = IEQUALIFIER_LSHIFT;

    #[test]
    fn the_us_keymap_types_what_a_us_keyboard_types() {
        let m = mem();
        assert_eq!(raw(&m, 0x20, 0), b"a");
        assert_eq!(raw(&m, 0x20, SHIFT), b"A");
        assert_eq!(raw(&m, 0x20, IEQUALIFIER_RSHIFT), b"A", "either shift");
        assert_eq!(raw(&m, 0x20, IEQUALIFIER_CAPSLOCK), b"A", "caps lock shifts a capsable key ...");
        assert_eq!(raw(&m, 0x01, IEQUALIFIER_CAPSLOCK), b"1", "... and not the digit row (negative control)");
        assert_eq!((raw(&m, 0x01, SHIFT), raw(&m, 0x00, 0), raw(&m, 0x00, SHIFT)), (b"!".to_vec(), b"`".to_vec(), b"~".to_vec()));
        assert_eq!(raw(&m, 0x33, IEQUALIFIER_CONTROL), [3], "Ctrl-C: the character with bits 5 and 6 cleared");
        assert_eq!(raw(&m, 0x10, IEQUALIFIER_LALT), b"@", "Alt-Q in this layout");
        assert_eq!((raw(&m, 0x44, 0), raw(&m, 0x44, IEQUALIFIER_CONTROL)), (vec![13], vec![10]), "Return; Ctrl-Return is a line feed");
        assert_eq!(raw(&m, 0x45, 0), [27]);
        assert_eq!((raw(&m, 0x50, 0), raw(&m, 0x50, SHIFT)), (b"\x9B0~".to_vec(), b"\x9B10~".to_vec()), "F1 and shifted F1 are string keys");
        assert_eq!((raw(&m, 0x4C, 0), raw(&m, 0x4C, SHIFT)), (b"\x9BA".to_vec(), b"\x9BT".to_vec()), "cursor up and shifted cursor up");
        assert_eq!(raw(&m, 0x50 | 0x80, 0), b"", "a key release types nothing");
        assert_eq!(raw(&m, 0x78, 0), b"", "codes from 0x78 are not keyboard keys");
        assert_eq!(raw(&m, 0x60, 0), b"", "a shift key is a KCF_NOP key");
        assert_eq!((raw(&m, 0x41, IEQUALIFIER_REPEAT), raw(&m, 0x43, IEQUALIFIER_REPEAT)), (vec![8], vec![]), "a repeating Enter is dropped, a repeating Backspace is not");
        assert_eq!(Km::new(&m, AT).map_raw_key(0, 0x20, 0, [0; 4], 16).unwrap().unwrap(), b"", "only IECLASS_RAWKEY events are mapped");
        assert_eq!(Km::new(&m, AT).map_raw_key(IECLASS_RAWKEY, 0x20, 0, [0; 4], 0).unwrap(), None, "a buffer too small is -1");
        assert_eq!(Km::new(&m, AT).map_raw_key(IECLASS_RAWKEY, 0x50, 0, [0; 4], 2).unwrap(), None, "... also for a string key");
    }

    #[test]
    fn map_ansi_inverts_map_raw_key_for_the_whole_ascii_range() {
        let m = mem();
        let km = Km::new(&m, AT);
        let mut unreachable = vec![];
        for ch in 0x20u8..0x7F {
            let Some(pairs) = km.map_ansi(&[ch], 8).unwrap().unwrap() else { unreachable.push(ch); continue };
            assert_eq!(pairs.len(), 2, "{ch:#x}: one (code, qualifier) pair");
            assert_eq!(raw(&m, pairs[0] as u16, pairs[1] as u16), [ch], "{ch:#x} through key {:#x} qual {:#x}", pairs[0], pairs[1]);
        }
        assert_eq!(unreachable, b"|", "the shipped US layout types '|' on no key: its backslash key is typed KC_NOQUAL, so only the unshifted '\\' is reachable");
        let hello = km.map_ansi(b"Hi!", 8).unwrap().unwrap().unwrap();
        assert_eq!(hello, [0x25, 1, 0x17, 0, 0x01, 1], "'H' = shift+h, 'i', '!' = shift+1");
        assert_eq!(km.map_ansi(b"ab", 1).unwrap(), Err(()), "two pairs do not fit in a length of 1");
        assert_eq!(km.map_ansi(b"|", 8).unwrap(), Ok(None), "a character no key makes: 0");
        assert_eq!(km.map_ansi(b"a|", 8).unwrap(), Ok(None), "... even after good ones");
        assert!(km.map_ansi(&[0x90], 8).unwrap().unwrap().is_some(), "a control character with the high bit is Ctrl-Alt-letter");
    }

    #[test]
    fn dead_keys_modify_the_next_key_from_the_two_previous_presses() {
        let mut m = mem();
        // Key 0x28 becomes a dead key (index 1) when unshifted, 'L' when shifted; key 0x12 takes the accent after it.
        let (d1, d2) = (0x8000u32, 0x8010u32);
        m.write_bytes(d1, &[DPF_DEAD, 1, 0, b'L']).unwrap();
        m.write_bytes(d2, &[DPF_MOD, 4, 0, b'E', b'e', 0xE9]).unwrap();
        let (lt, lm) = (m.read_u32(AT).unwrap(), m.read_u32(AT + 4).unwrap());
        for (code, d) in [(0x28u32, d1), (0x12, d2)] { m.write_u8(lt + code, KCF_DEAD | KCF_SHIFT).unwrap(); m.write_u32(lm + 4 * code, d).unwrap(); }
        let km = Km::new(&m, AT);
        let key = |code: u16, qual: u16, prev: [u8; 4]| km.map_raw_key(IECLASS_RAWKEY, code, qual, prev, 8).unwrap().unwrap();
        assert_eq!(key(0x28, 0, [0; 4]), b"", "a dead key types nothing by itself");
        assert_eq!(key(0x28, SHIFT, [0; 4]), b"L", "shifted it is an ordinary key");
        assert_eq!(key(0x12, 0, [0; 4]), b"e", "no dead key before: the plain letter");
        assert_eq!(key(0x12, 0, [0x28, 0, 0, 0]), [0xE9], "after the dead key: the accented letter");
        assert_eq!(key(0x12, 0, [0x20, 0, 0, 0]), b"e", "negative control: after an ordinary key it is plain");
        assert_eq!(key(0x12, SHIFT, [0x28, 0, 0, 0]), b"E", "shifted: its second descriptor, a plain character");
        // MapANSI finds the dead key + letter for the accented character, and the letter alone for the plain one.
        assert_eq!(km.map_ansi(&[0xE9], 8).unwrap(), Ok(Some(vec![0x28, 0, 0x12, 0])), "dead key then letter");
        assert_eq!(km.map_ansi(b"e", 8).unwrap(), Ok(Some(vec![0x12, 0])));
        assert_eq!(km.map_ansi(&[0xE9], 1).unwrap(), Err(()), "the pair of keys needs 2");
        // ... and the output feeds back: dead press, then the letter with that press as its previous key.
        let back = key(0x12, 0, [0x28, 0, 0, 0]);
        assert_eq!(back, [0xE9]);
    }

    #[test]
    fn the_built_in_keymap_is_laid_out_inside_its_block() {
        assert_eq!(default_size(), 32 + 64 + 256 + 8 + 8 + 56 + 224 + 8 + 8 + STRINGS.iter().map(|(_, d)| d.len() as u32).sum::<u32>());
        let mut m = GuestMem::new(0x10000);
        build_default(&mut m, AT).unwrap();
        for f in 0..8u32 { let p = m.read_u32(AT + 4 * f).unwrap(); assert!(p > AT && p < AT + default_size(), "table {f} at {p:#x} is inside the block"); }
    }
}
