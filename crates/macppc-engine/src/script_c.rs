//! The Script Manager by C name (`GetScriptManagerVariable`, `SetScriptManagerVariable`, `KeyScript`, `FontScript`, `IntlScript`, `FontToScript`, `GetScriptVariable`) for a Mac with the
//! Roman script only. The answers are the 68k `_ScriptUtil` ones (`mac68k_engine::script::manager_variable`): one table for both CPUs. Anything without an honest Roman-only answer
//! is refused BY NAME (the selector is in the message), never answered with a made-up number. `SetScriptManagerVariable` accepts only the value the variable already has
//! (there is one script system, so there is nothing to change to); `GetScriptVariable` has no verified Roman-only table yet and refuses every selector.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_ppc::Cpu;

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
            GetScriptManagerVariable => {
                let sel = a0 as u16 as i16;
                mac68k_engine::script::manager_variable(sel).map(|(v, _)| v).ok_or_else(|| format!("GetScriptManagerVariable({sel}) has no Roman-only answer here"))?
            }
            SetScriptManagerVariable => {
                let sel = a0 as u16 as i16;
                match mac68k_engine::script::manager_variable(sel) {
                    Some((v, what)) if v == a1 => { let _ = what; 0 }
                    Some((v, what)) => return Err(format!("SetScriptManagerVariable({sel}, {a1}): {what} is {v} and this machine has one script system, so it cannot be changed")),
                    None => return Err(format!("SetScriptManagerVariable({sel}) has no Roman-only meaning here")),
                }
            }
            // one keyboard script: switching to it is a no-op the program may rely on
            KeyScript => 0,
            FontScript | IntlScript | FontToScript => 0, // smRoman
            GetScriptVariable => return Err(format!("GetScriptVariable(script {}, selector {}) has no verified Roman-only answer yet", a0 as u16 as i16, a1 as u16 as i16)),
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
