//! `CCoeControl` (coecntrl.h): the base of every on-screen control. Layout (ARM EABI, bases `CBase`, `MObjectProvider`): vptr 0,
//! `MObjectProvider` at 4 (vptr + reserved word, coemop.h:167; the import `_ZThn4_N24CEikFormattedCellListBox15MopSupplyObject...`
//! confirms 4), then the members declared at coecntrl.h:370-378.
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;

pub const CTRL_MOBJECT_PROVIDER: u32 = 4;
pub const CTRL_I_COE_ENV: u32 = 12; // coecntrl.h:370 CCoeEnv* iCoeEnv
pub const CTRL_I_CONTEXT: u32 = 16; // :371 MCoeControlContext* iContext
pub const CTRL_I_POSITION: u32 = 20; // :372 TPoint iPosition (iX, iY)
pub const CTRL_I_SIZE: u32 = 28; // :373 TSize iSize (iWidth, iHeight)
pub const CTRL_I_FLAGS: u32 = 36; // :375 TInt iFlags
pub const CTRL_I_WIN: u32 = 40; // :376 RDrawableWindow* iWin
pub const CTRL_I_DATA: u32 = 44; // :377 RCoeDynamicDataStorage* iData
pub const CTRL_I_MOP_PARENT: u32 = 48; // :378 MObjectProvider* iMopParent
pub const CCOE_CONTROL_SIZE: u32 = 52;

impl Process {
    /// `CCoeControl::CCoeControl()` / `CCoeControl(CCoeEnv*)`: everything after the vptrs cleared, `iCoeEnv` set (a null argument is
    /// `CCoeEnv::Static()`), so a control that reads `iCoeEnv`, `iPosition`, `iSize`, `iWin` inline finds the constructed state.
    fn control_construct(&mut self, this: u32, env: Option<u32>) -> Result<(), String> {
        let env = match env { Some(e) if e != 0 => e, _ => self.coe_env()? };
        for off in (CTRL_MOBJECT_PROVIDER + 4..CCOE_CONTROL_SIZE).step_by(4) {
            self.mem.vm.write_u32(this.wrapping_add(off), 0).map_err(|_| format!("the control at {this:#x} is unwritable"))?;
        }
        self.mem.vm.write_u32(this.wrapping_add(CTRL_I_COE_ENV), env).map_err(|_| format!("the control at {this:#x} is unwritable"))
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let done = |p: &mut Process, r: Result<(), String>| match r {
        Ok(()) => { p.cpu.regs[0] = this; NativeOutcome::Return }
        Err(m) => NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}")),
    };
    Some(match name {
        "_ZN11CCoeControlC1Ev" | "_ZN11CCoeControlC2Ev" => { let r = p.control_construct(this, None); done(p, r) }
        "_ZN11CCoeControlC1EP7CCoeEnv" | "_ZN11CCoeControlC2EP7CCoeEnv" => { let r = p.control_construct(this, Some(r1)); done(p, r) }
        _ => return None,
    })
}
