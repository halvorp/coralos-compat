//! The AppUi's control stack (`CCoeAppUi::iStack`, coeaui.h:229): the controls an AppUi offers key events to. `AddToStackL` puts a
//! control in priority order (coeaui.h:47-61: a higher priority is offered first; among equal priorities the newest is above), and
//! key events are offered topmost first (`ctrlstack` is the engine's own, the Foundation sources do not contain CCoeControlStack).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::StackItem;

/// `ECoeStackFlagRefusesAllKeys` (coeaui.h:77): the control is never offered a key event.
pub const STACK_FLAG_REFUSES_ALL_KEYS: u32 = 0x01;

impl Process {
    /// Stack `control` for `view` (0 = none) at `priority` with `flags`: above every entry of lower or equal priority.
    fn stack_add(&mut self, appui: u32, view: u32, control: u32, priority: i32, flags: u32) -> Result<(), String> {
        if control == 0 {
            return Err(String::from("AddToStackL(NULL): CONE panic on a device"));
        }
        let st = self.appui_state(appui)?;
        if st.stack.iter().any(|i| i.control == control) {
            return Err(format!("control {control:#x} is already on the stack (a second AddToStackL of it)"));
        }
        let at = st.stack.iter().position(|i| i.priority <= priority).unwrap_or(st.stack.len());
        st.stack.insert(at, StackItem { control, priority, flags, view });
        self.log.push(format!("[stack] {control:#x} added at priority {priority}, flags {flags:#x}"));
        Ok(())
    }

    /// The controls to offer a key event to, topmost first: the ones that do not refuse all keys.
    pub(crate) fn stack_key_targets(&mut self, appui: u32) -> Result<alloc::vec::Vec<u32>, String> {
        Ok(self.appui_state(appui)?.stack.iter().filter(|i| i.flags & STACK_FLAG_REFUSES_ALL_KEYS == 0).map(|i| i.control).collect())
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1, r2, r3) = (p.cpu.regs[0], p.cpu.regs[1], p.cpu.regs[2], p.cpu.regs[3]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED cone {name}: {m}"));
    let done = |r: Result<(), String>| match r { Ok(()) => NativeOutcome::Return, Err(m) => refuse(m) };
    Some(match name {
        // void AddToStackL(const MCoeView& aView, CCoeControl* aControl, TInt aPriority, TInt aStackingFlags): flags is the fifth word, on the stack.
        // The view is recorded; DELETE WHEN a view's deactivation removes its controls (then offer keys only to the active view's).
        "_ZN9CCoeAppUi11AddToStackLERK8MCoeViewP11CCoeControlii" => {
            let sp = p.cpu.regs[13];
            let r = p.rd(sp).and_then(|flags| p.stack_add(this, r1, r2, r3 as i32, flags));
            done(r)
        }
        // void AddToStackL(CCoeControl* aControl, TInt aPriority, TInt aStackingFlags)
        "_ZN9CCoeAppUi11AddToStackLEP11CCoeControlii" => { let r = p.stack_add(this, 0, r1, r2 as i32, r3); done(r) }
        // void RemoveFromStack(CCoeControl* aControl): not on the stack is a no-op (coeaui.h:114).
        "_ZN9CCoeAppUi15RemoveFromStackEP11CCoeControl" => done(p.appui_state(this).map(|s| s.stack.retain(|i| i.control != r1))),
        // void UpdateStackedControlFlags(CCoeControl*, TInt aFlags, TInt aMask): the flags under the mask become aFlags'.
        "_ZN9CCoeAppUi25UpdateStackedControlFlagsEP11CCoeControlii" => done(p.appui_state(this).map(|s| {
            if let Some(i) = s.stack.iter_mut().find(|i| i.control == r1) { i.flags = (i.flags & !r3) | (r2 & r3); }
        })),
        _ => return None,
    })
}
