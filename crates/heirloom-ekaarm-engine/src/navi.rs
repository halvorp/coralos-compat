//! The navigation pane's contents (`CAknNavigationControlContainer`, aknnavi.h; `CAknNavigationDecorator`, aknnavide.h). The pane
//! itself is a control the shell built (`shell.rs`); what an application puts into it is a decorator around a control. Nothing of it
//! reaches the host yet (the GuestUi seam has no navigation pane), so the label text and the stack are only kept.
//! DELETE WHEN the GuestUi seam grows a navigation pane (then show the top decorator's text).
use alloc::format;
use alloc::string::String;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::objects::HostObj;

/// `CAknNavigationDecorator::TControlType` (aknnavide.h): ENaviLabel and EHintText (a message label).
const NAVI_LABEL: u32 = 2;
const HINT_TEXT: u32 = 4;

/// The host state of one decorator.
#[derive(Debug, Clone)]
pub struct NaviDecorator {
    pub text: String,
    pub control_type: u32,
}

pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (this, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED avkon {name}: {m}"));
    Some(match name {
        // CAknNavigationDecorator* CAknNavigationControlContainer::CreateNavigationLabelL(const TDesC& aNaviText) (aknnavi.h:156): a
        // decorator owning a label of that text, not yet on the pane's stack.
        "_ZN30CAknNavigationControlContainer22CreateNavigationLabelLERK7TDesC16" | "_ZN30CAknNavigationControlContainer19CreateMessageLabelLERK7TDesC16" => {
            let control_type = if name.contains("CreateMessageLabelL") { HINT_TEXT } else { NAVI_LABEL };
            let made = crate::desc::read16(p, r1).map(|u| crate::desc::to_utf8(&u)).and_then(|text| {
                let d = p.make_control("CAknNavigationDecorator", 0x80, &[])?;
                p.obj_set(d, HostObj::NaviDecorator(NaviDecorator { text, control_type }))?;
                Ok(d)
            });
            match made { Ok(d) => { p.cpu.regs[0] = d; NativeOutcome::Return } Err(m) => refuse(m) }
        }
        // void CAknNavigationControlContainer::PushL(CAknNavigationDecorator&) (aknnavi.h:264): the pane shows its top decorator.
        "_ZN30CAknNavigationControlContainer5PushLER23CAknNavigationDecorator" => {
            let _ = this;
            if matches!(p.obj_get(r1), Some(HostObj::NaviDecorator(_))) { p.nat.shell.navi_stack.push(r1); NativeOutcome::Return } else { refuse(format!("{r1:#x} is not a decorator this engine made")) }
        }
        // void CAknNavigationControlContainer::Pop(): the top decorator leaves the stack (it is not deleted: the application owns it).
        "_ZN30CAknNavigationControlContainer3PopEv" => { p.nat.shell.navi_stack.pop(); NativeOutcome::Return }
        // CAknNavigationDecorator::~CAknNavigationDecorator(): leaves the pane's stack, forgets its state; the deleting form frees it.
        "_ZN23CAknNavigationDecoratorD1Ev" | "_ZN23CAknNavigationDecoratorD0Ev" => {
            p.nat.shell.navi_stack.retain(|d| *d != this);
            p.obj_remove(this);
            if name.ends_with("D0Ev") {
                if let Some(h) = p.heap.as_mut() { let _ = h.free(this); }
            }
            NativeOutcome::Return
        }
        _ => return None,
    })
}
