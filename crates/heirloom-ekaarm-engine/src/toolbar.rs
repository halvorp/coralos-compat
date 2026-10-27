//! The fixed toolbar of a touch AppUi (`CAknToolbar`, akntoolbar.h). The device profile is touch (`AknLayoutUtils::PenEnabled`), and an
//! application there addresses `CAknAppUi::CurrentFixedToolbar()` without testing it, so the AppUi owns one. It has no items (the
//! view's `AVKON_VIEW` toolbar resource is 0), so every control lookup finds none; what the guest sets is recorded. Nothing is drawn:
//! DELETE WHEN the host draws a toolbar (then give it the items its resource names).
use alloc::format;
use alloc::string::String;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::ToolbarState;

const TOOLBAR_BYTES: u32 = 0x40;
const VTABLE: &[(usize, &str)] = &[(0, "_ZN11CAknToolbarD1Ev"), (1, "_ZN11CAknToolbarD0Ev"), (2, "_ZN5CBase10Extension_EjRPvS0_")];

impl Process {
    fn fixed_toolbar(&mut self, appui: u32) -> Result<u32, String> {
        if let Some(t) = self.appui_state(appui)?.toolbar {
            return Ok(t);
        }
        let t = self.alloc_zeroed(TOOLBAR_BYTES).ok_or_else(|| String::from("no guest heap for the toolbar"))?;
        let vptr = self.build_vtable("CAknToolbar", &slots_with_gaps(3, VTABLE))?;
        self.mem.vm.write_u32(t, vptr).map_err(|_| String::from("toolbar unwritable"))?;
        self.control_state(t)?.toolbar = Some(ToolbarState { visible: true, ..Default::default() });
        self.appui_state(appui)?.toolbar = Some(t);
        Ok(t)
    }
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED eikcoctl {name}: {m}"));
    if name == "_ZNK9CAknAppUi19CurrentFixedToolbarEv" {
        return Some(match p.fixed_toolbar(this) { Ok(t) => { p.cpu.regs[0] = t; NativeOutcome::Return } Err(m) => refuse(m) });
    }
    if !name.contains("11CAknToolbar") && !name.contains("CAknToolbar") { return None; }
    let st = match p.control_state(this) { Ok(c) => match c.toolbar.as_mut() { Some(t) => t, None => return Some(refuse(format!("{this:#x} is not a toolbar"))) }, Err(m) => return Some(refuse(m)) };
    Some(match name {
        "_ZN11CAknToolbar18SetToolbarObserverEP19MAknToolbarObserver" => { st.observer = r1; NativeOutcome::Return }
        "_ZN11CAknToolbar20SetToolbarVisibilityEi" => { st.visible = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar15DisableToolbarLEi" => { st.disabled = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar14SetWithSlidingEi" => { st.sliding = r1 != 0; NativeOutcome::Return }
        "_ZN11CAknToolbar21SetCloseOnAllCommandsEi" => { st.close_on_all = r1 != 0; NativeOutcome::Return }
        // HideItem(TInt aCommandId, TBool aHide, TBool aDraw): the toolbar has no items, so there is nothing to hide.
        "_ZN11CAknToolbar8HideItemEiii" => NativeOutcome::Return,
        // CCoeControl* ControlOrNull(TInt aId) const: no items, so none.
        "_ZNK11CAknToolbar13ControlOrNullEi" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        // TInt EventModifiers() const: no pointer event is in progress.
        "_ZNK11CAknToolbar14EventModifiersEv" => { p.cpu.regs[0] = 0; NativeOutcome::Return }
        _ => return None,
    })
}
