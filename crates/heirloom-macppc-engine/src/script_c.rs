//! The Script Manager by C name (`GetScriptManagerVariable`, `SetScriptManagerVariable`, `KeyScript`, `FontScript`, `IntlScript`, `FontToScript`, `GetScriptVariable`) for a Mac with the
//! Roman script only. The answers are the 68k `_ScriptUtil` ones (`heirloom_mac68k_engine::script::manager_variable`): one table for both CPUs. `GetScriptManagerVariable`
//! follows the spec (macos9-specs/InterfaceLib/GetScriptManagerVariable): a stored value, else the table (smRegionCode 40 is 2 = verBritain, the CoralOS default region), else 0;
//! smKCHRCache (38) is the installed keyboard layout's data in guest memory (`keys_c.rs`; the layout is `heirloom_mac68k_engine::keyboard`, British).
//! `SetScriptManagerVariable` stores and answers noErr; paramErr for the read-only verbs (smVersion, smEnabled, smSysRef). `GetScriptVariable`: 0 for a script that is not installed, Roman's smScriptEnabled TRUE; Roman's other selectors have no verified table and are refused.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// `smKCHRCache` (Script.h): the current keyboard layout's 'KCHR' data.
pub const SM_KCHR_CACHE: i16 = 38;

#[derive(Default)]
pub struct ScriptState { vars: alloc::collections::BTreeMap<i16, u32>, pub(crate) kchr: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScFn { GetScriptManagerVariable, SetScriptManagerVariable, KeyScript, FontScript, IntlScript, FontToScript, GetScriptVariable }

impl ScFn {
    pub fn by_name(name: &str) -> Option<ScFn> {
        use ScFn::*;
        Some(match name {
            "GetScriptManagerVariable" => GetScriptManagerVariable, "SetScriptManagerVariable" => SetScriptManagerVariable, "KeyScript" => KeyScript, "FontScript" => FontScript,
            "IntlScript" => IntlScript, "FontToScript" => FontToScript, "GetScriptVariable" => GetScriptVariable,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn sc_call(&mut self, f: ScFn, cpu: &mut Cpu) -> Result<(), String> {
        use ScFn::*;
        let (a0, a1) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0));
        let r: u32 = match f {
            // spec (macos9-specs/InterfaceLib/GetScriptManagerVariable): a value the program stored wins; else the Roman-only table (smRegionCode is 2, verBritain: the CoralOS default
            // region); a verb outside the table answers 0 (the ROM sets ScriptMgrError and returns 0)
            GetScriptManagerVariable => {
                let sel = a0 as u16 as i16;
                match self.sc.vars.get(&sel).copied() {
                    Some(v) => v,
                    // smKCHRCache: the installed keyboard layout's 'KCHR' data, the transData a program passes to KeyTranslate
                    None if sel == SM_KCHR_CACHE => self.installed_kchr()?,
                    None => heirloom_mac68k_engine::script::manager_variable(sel).map(|(v, _)| v).unwrap_or(0),
                }
            }
            // SetScriptManagerVariable(verb, param) -> OSErr: stores the value; the read-only verbs smVersion (0), smEnabled (4) and smSysRef (24) are paramErr (-50)
            SetScriptManagerVariable => {
                let sel = a0 as u16 as i16;
                if matches!(sel, 0 | 4 | 24) { (-50i32) as u32 } else if (0..=42).contains(&sel) && sel % 2 == 0 { self.sc.vars.insert(sel, a1); 0 } else { (-50i32) as u32 }
            }
            // one keyboard script: switching to it is a no-op the program may rely on
            KeyScript => 0,
            FontScript | IntlScript | FontToScript => 0, // smRoman
            // GetScriptVariable(script, selector): this Mac has the Roman script only. A script that is not installed answers 0 for every selector
            // (Inside Macintosh: Text, "Script Manager": the Script Manager returns 0 for a script that is not enabled); Roman's smScriptEnabled (4) is
            // TRUE; Roman's other selectors still have no verified table and stay refused by name
            GetScriptVariable => match (a0 as u16 as i16, a1 as u16 as i16) {
                (s, sel) if s != 0 => { self.mac.log.push(format!("[script] GetScriptVariable(script {s}, selector {sel}) -> 0 (script not installed: Roman only)")); 0 }
                (0, 4) => 0xFF,
                // Roman's font selectors answer THIS system's fonts, the same ones LMGetSysFontFam / LMGetSysFontSize, GetAppFont and GetDefFontSize
                // answer (one source): smScriptSysFond 12, smScriptAppFond 14, smScriptSysFondSize 84 and smScriptAppFondSize 86 (family << 16 | size)
                (0, 12) => u32::from(self.mac.mem.read_u16(0x0BA6).unwrap_or(0)),
                (0, 14) => 1,
                // smScriptRight 6 (a right-to-left script) and smScriptJust 8 (its justification): Roman, from TESysJust as GetSysDirection answers it
                (0, 6) => if self.mac.mem.read_u16(0x0BAC).unwrap_or(0) != 0 { 0xFF } else { 0 },
                (0, 8) => u32::from(self.mac.mem.read_u16(0x0BAC).unwrap_or(0)),
                // smScriptFlags 22 (Script.h smsf* bits) for Roman as THIS system's text handling is: single-byte (smsfSingByte, bit 1) with
                // intelligent cut and paste (smsfIntellCP, bit 0); no contextual forms, ligatures or two-byte text (named: not checked against a
                // 9.2.1 Roman 'itlb')
                (0, 22) => 0x3,
                (0, 84) => (u32::from(self.mac.mem.read_u16(0x0BA6).unwrap_or(0)) << 16) | u32::from(self.mac.mem.read_u16(0x0BA8).ok().filter(|&s| s != 0).unwrap_or(12)),
                (0, 86) => (1 << 16) | 12,
                (s, sel) => return Err(format!("GetScriptVariable(script {s}, selector {sel}) has no verified Roman-only answer yet")),
            },
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
