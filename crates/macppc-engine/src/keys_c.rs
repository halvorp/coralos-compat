//! `KeyTranslate(transData, keycode, &state)`: virtual key code + modifiers -> character, through a `'KCHR'` keyboard-layout resource. The algorithm is Executor's `C_KeyTranslate`
//! (src/toolevent.cpp, MIT; the KCHR layout from src/include/rsys/keyboard.h): `keycode` = modifiers >> 8 in the high byte (index into the 256-byte modifier table at +2), virtual key in
//! the low 7 bits (bit 7 = key UP); tables of 128 characters follow the table count at +258; dead-key records follow the tables. The result is the character; a dead key sets `*state`
//! to the record's offset, and the key that completes it returns the replacement, or the character with the record's no-match character in bits 16..23.
//! There is NO installed layout on this machine (the host delivers characters, not key codes: translate/input.rs), so `transData == NULL` -- "the current KCHR" -- is refused BY NAME
//! rather than answered from a US table (a British/European default layout is a separate decision); a program that passes its own `'KCHR'` handle's data gets the real answer.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn key_translate(&mut self, cpu: &mut Cpu) -> Result<(), String> {
        let (kchr, code, state_p) = (arg(self, cpu, 0)?, arg(self, cpu, 1)? as u16, arg(self, cpu, 2)?);
        if kchr == 0 { return Err(String::from("KeyTranslate(NULL, ..): no keyboard layout ('KCHR') is installed on this machine; pass a KCHR resource's data")); }
        let e = |_| format!("KeyTranslate: the KCHR at {kchr:#x} leaves guest RAM");
        let m = &self.mac.mem;
        let virt = u32::from(code & 0x7F);
        let table_num = u32::from(m.read_u8(kchr + 2 + u32::from(code >> 8)).map_err(e)?);
        let n_tables = u32::from(m.read_u16(kchr + 258).map_err(e)?);
        if table_num >= n_tables { return Err(format!("KeyTranslate: modifier table maps to table {table_num} but the KCHR has {n_tables}")); }
        let mut ascii = u32::from(m.read_u8(kchr + 260 + 128 * table_num + virt).map_err(e)?);
        let up = code & 0x80 != 0;
        let state = if up || state_p == 0 { None } else { Some(m.read_u32(state_p).map_err(e)?) };
        let dead_base = kchr + 260 + 128 * n_tables; // n_dead_key_recs, then the records
        let rec_len = |m: &retro_guest_mem::GuestMem, at: u32| -> Result<u32, String> { Ok(6 + 2 * u32::from(m.read_u16(at + 2).map_err(e)?)) };
        // offsets stored in *state are relative to &n_tables (as Executor stores them)
        match state {
            None | Some(0) => if ascii == 0 {
                let n_dead = u32::from(m.read_u16(dead_base).map_err(e)?);
                let mut at = dead_base + 2;
                for _ in 0..n_dead {
                    if u32::from(m.read_u8(at).map_err(e)?) == table_num && u32::from(m.read_u8(at + 1).map_err(e)?) == virt {
                        let n = u32::from(m.read_u16(at + 2).map_err(e)?);
                        if up { ascii = u32::from(m.read_u8(at + 4 + 2 * n + 1).map_err(e)?); }
                        else if state_p != 0 { let off = at - (kchr + 258); self.mac.mem.write_u32(state_p, off).map_err(e)?; }
                        break;
                    }
                    at += rec_len(m, at)?;
                }
            },
            Some(off) => {
                let at = kchr + 258 + off;
                let n = u32::from(m.read_u16(at + 2).map_err(e)?);
                let mut replacement = None;
                for i in 0..n { if u32::from(m.read_u8(at + 4 + 2 * i).map_err(e)?) == ascii { replacement = Some(u32::from(m.read_u8(at + 5 + 2 * i).map_err(e)?)); break; } }
                let no_match = u32::from(m.read_u8(at + 4 + 2 * n + 1).map_err(e)?);
                self.mac.mem.write_u32(state_p, 0).map_err(e)?;
                ascii = match replacement { Some(r) => r, None => ascii | no_match << 16 };
            }
        }
        cpu.gpr[3] = ascii;
        Ok(())
    }
}
