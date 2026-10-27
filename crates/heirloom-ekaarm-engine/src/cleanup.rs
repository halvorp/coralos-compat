//! The cleanup stack (`CleanupStack`, e32base.h:2944-2948): objects a function holds while it can leave. `Pop` forgets items,
//! `PopAndDestroy` runs their destruction: `delete` for a `CBase*` (its virtual destructor, slot 1 = the deleting one: slots 0 and 1
//! are `~CBase`, e32base.h:48), `User::Free` for a `TAny*`, and the function for a `TCleanupItem` (e32base.h `TCleanupItem`: iOperation, iThis).
use alloc::format;
use alloc::string::String;
use crate::callback::Continuation;
use crate::loader::Process;
use crate::natives::NativeOutcome;
use crate::run::Outcome;

/// How an item on the cleanup stack is destroyed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupItem {
    /// `PushL(CBase*)`.
    Base(u32),
    /// `PushL(TAny*)`.
    Any(u32),
    /// `PushL(TCleanupItem)`: call `func(arg)` (bit 0 = Thumb).
    Func { func: u32, arg: u32 },
}

const SLOT_DELETING_DTOR: u32 = 1; // e32base.h:48 `virtual ~CBase()`: complete 0, deleting 1

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let (r0, r1) = (p.cpu.regs[0], p.cpu.regs[1]);
    let refuse = |m: String| NativeOutcome::Refused(format!("NEXT-NEEDED euser {name}: {m}"));
    Some(match name {
        "_ZN12CleanupStack5PushLEPv" => { p.nat.cleanup.push(CleanupItem::Any(r0)); NativeOutcome::Return }
        "_ZN12CleanupStack5PushLEP5CBase" => { p.nat.cleanup.push(CleanupItem::Base(r0)); NativeOutcome::Return }
        // PushL(TCleanupItem): the two-word item is passed in r0 (iOperation) and r1 (iThis).
        "_ZN12CleanupStack5PushLE12TCleanupItem" => { p.nat.cleanup.push(CleanupItem::Func { func: r0, arg: r1 }); NativeOutcome::Return }
        "_ZN12CleanupStack3PopEv" | "_ZN12CleanupStack3PopEi" => {
            let n = if name.ends_with("Ev") { 1 } else { r0 as i32 as usize };
            let have = p.nat.cleanup.len();
            if (r0 as i32) < 0 && !name.ends_with("Ev") || n > have {
                return Some(refuse(format!("pop of {} with {have} on the cleanup stack (a CBASE-STRAY panic on a device)", r0 as i32)));
            }
            p.nat.cleanup.truncate(have - n);
            NativeOutcome::Return
        }
        "_ZN12CleanupStack13PopAndDestroyEv" | "_ZN12CleanupStack13PopAndDestroyEi" => {
            let n = if name.ends_with("Ev") { 1 } else { r0 as i32 };
            let have = p.nat.cleanup.len();
            if n < 0 || n as usize > have {
                return Some(refuse(format!("PopAndDestroy of {n} with {have} on the cleanup stack (a CBASE-STRAY panic on a device)")));
            }
            match destroy_next(p, n as u32) {
                Ok(()) => NativeOutcome::Return,
                Err(Outcome::Refused(m)) => NativeOutcome::Refused(m),
                Err(Outcome::Fault(m)) => refuse(m),
                Err(_) => refuse(String::from("PopAndDestroy ended the run")),
            }
        }
        _ => return None,
    })
}

/// Destroy up to `left` items from the top: items with no guest code are finished here; one with guest code starts that call and
/// continues when it returns (`Continuation::CleanupDestroy`).
pub(crate) fn destroy_next(p: &mut Process, mut left: u32) -> Result<(), Outcome> {
    while left > 0 {
        let Some(item) = p.nat.cleanup.pop() else {
            return Err(Outcome::Fault(String::from("PopAndDestroy: the cleanup stack ran out")));
        };
        left -= 1;
        let (target, args): (u32, [u32; 1]) = match item {
            CleanupItem::Any(a) => {
                if let Some(h) = p.heap.as_mut() {
                    if a != 0 && h.free(a).is_err() {
                        return Err(Outcome::Refused(format!("NEXT-NEEDED euser PopAndDestroy: the TAny* {a:#x} pushed on the cleanup stack is not a live heap cell")));
                    }
                }
                continue;
            }
            CleanupItem::Func { func, arg } => (func, [arg]),
            CleanupItem::Base(obj) => {
                if obj == 0 {
                    continue;
                }
                let vptr = p.mem.vm.read_u32(obj).map_err(|_| Outcome::Fault(format!("PopAndDestroy: the CBase at {obj:#x} is unreadable")))?;
                if vptr == 0 {
                    return Err(Outcome::Refused(format!("NEXT-NEEDED euser PopAndDestroy: the CBase at {obj:#x} has vtable pointer 0, so there is no destructor to run")));
                }
                let dtor = p.mem.vm.read_u32(vptr.wrapping_add(SLOT_DELETING_DTOR * 4)).map_err(|_| Outcome::Fault(format!("PopAndDestroy: the vtable of {obj:#x} is unreadable")))?;
                (dtor, [obj])
            }
        };
        return p.call_guest(target, &args, Continuation::CleanupDestroy { left }).map_err(Outcome::Fault);
    }
    Ok(())
}
