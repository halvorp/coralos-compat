//! workbench.library's application registry: AddAppWindowA / AddAppIconA / AddAppMenuItemA and their Remove calls.
//!
//! The registrations are kept (a handle is a small block in the engine's arena, unique and valid) and validated like
//! AROS workbench/libs/workbench does at the entry: a window must be an open guest window, the message port non-NULL.
//! NOT modelled: delivery. There is no Workbench file manager in the engine, so no AppMessage (a file dropped on the window, a
//! menu pick, an icon drop) is ever sent to the port; the log says "(registered; nothing sends AppMessages)".
//! Registers (NDK workbench_lib.fd): AddAppWindowA(d0 id, d1 userdata, a0 window, a1 msgport, a2 tags),
//! AddAppIconA(d0 id, d1 userdata, a0 text, a1 msgport, a2 lock, a3 diskobj, a4 tags), AddAppMenuItemA(d0 id, d1 userdata,
//! a0 text, a1 msgport, a2 tags); RemoveAppWindow/RemoveAppIcon/RemoveAppMenuItem(a0 handle) -> TRUE.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use crate::libtable::LibraryBuilder;
use crate::machine::Amiga;
use crate::Refusal;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind { Window, Icon, MenuItem }
const HANDLE_SIZE: u32 = 16;

#[derive(Default)]
pub struct State { apps: BTreeMap<u32, (Kind, u32)> }

pub fn register<U: GuestUi>(b: LibraryBuilder<U>) -> LibraryBuilder<U> {
    b.named("AddAppWindowA", |a, c| add(a, c, Kind::Window)).named("AddAppIconA", |a, c| add(a, c, Kind::Icon)).named("AddAppMenuItemA", |a, c| add(a, c, Kind::MenuItem))
        .named("RemoveAppWindow", |a, c| remove(a, c, Kind::Window)).named("RemoveAppIcon", |a, c| remove(a, c, Kind::Icon)).named("RemoveAppMenuItem", |a, c| remove(a, c, Kind::MenuItem))
}

fn add<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, kind: Kind) -> Result<String, Refusal> {
    let (id, port) = (cpu.dreg(0), cpu.areg(1));
    if kind == Kind::Window && !a.windows.contains_key(&cpu.areg(0)) {
        return Err(Refusal(format!("AddAppWindowA: {:#x} is not an open window", cpu.areg(0))));
    }
    if port == 0 { cpu.set_d0(0); return Ok(format!("[workbench] Add{kind:?}: NULL message port -> NULL")); }
    let Some(h) = crate::fs::alloc_zeroed_sys(a, HANDLE_SIZE)? else { cpu.set_d0(0); return Ok(format!("[workbench] Add{kind:?} -> NULL (no memory)")) };
    a.workbench.apps.insert(h, (kind, id));
    cpu.set_d0(h);
    Ok(format!("[workbench] Add{kind:?} id {id:#x} port {port:#x} -> {h:#x} (registered; nothing sends AppMessages)"))
}

fn remove<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, kind: Kind) -> Result<String, Refusal> {
    let h = cpu.areg(0);
    match a.workbench.apps.get(&h) {
        Some((k, _)) if *k == kind => {}
        _ => return Err(Refusal(format!("Remove{kind:?}: {h:#x} is not a handle Add{kind:?} returned"))),
    }
    a.workbench.apps.remove(&h);
    crate::fs::free_block(a, h, HANDLE_SIZE).map_err(|e| Refusal(format!("Remove{kind:?}: {e}")))?;
    cpu.set_d0(1);
    Ok(format!("[workbench] Remove{kind:?} {h:#x}"))
}
