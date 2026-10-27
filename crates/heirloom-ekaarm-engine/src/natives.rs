use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::loader::Process;

/// The host-side state of the natives (no globals: it lives in the `Process`).
#[derive(Default)]
pub struct NativeState {
    /// `UserSvr::DllSetTls` / `DllTls`: (handle, DLL UID) -> pointer.
    pub tls: BTreeMap<(u32, u32), u32>,
    /// The `TCppRTExceptionsGlobals` objects the guest constructed, in order.
    pub exception_globals: Vec<u32>,
    /// The guest-resident `CApaProcess` placeholder (zeroed, vtable pointer 0), allocated on first use by the application start.
    pub apa_process: Option<u32>,
    /// `CleanupStack`: the pointers pushed and not yet popped (e32base.h:2944-2948 `PushL` x3, `Pop`, `Pop(TInt)`).
    pub cleanup: Vec<crate::cleanup::CleanupItem>,
    /// Cleanup-stack depths at each open TRAP bracket, and the guest block standing for the thread's `TTrapHandler` (`trap.rs`).
    pub trap_marks: Vec<u32>,
    pub trap_handler: Option<u32>,
    /// The guest sentinel of the active scheduler's queue (`sched.rs`).
    pub sched_head: Option<u32>,
    /// ROM resource files by id offset (`resource.rs`), built on first need.
    pub rom_resources: Option<alloc::collections::BTreeMap<u32, String>>,
    /// The Avkon shell objects (`shell.rs`).
    pub shell: crate::shell::ShellState,
    /// Host-only state of guest objects, by address and allocation generation (`objects.rs`).
    pub objs: crate::objects::Objects,
    /// The CDL engine's data and customisation stacks (`cdl.rs`).
    pub cdl: crate::cdl::CdlState,
}

/// A host extension to the natives: asked first with the import's mangled name; `Some` = handled. Used by tests to stand in
/// for an OS call that calls guest code; the engine itself leaves it `None`.
pub type NativeHook = Box<dyn FnMut(&mut Process, &str) -> Option<NativeOutcome>>;

#[derive(Debug)]
pub enum NativeOutcome {
    Return,
    Exit(i32),
    Refused(String),
}

/// The natives of each module, asked in turn: the first that knows the mangled `name` answers.
fn module_native(proc: &mut Process, name: &str) -> Option<NativeOutcome> {
    if let Some(o) = crate::appstart::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cone::native(proc, name) { return Some(o); }
    if let Some(o) = crate::appui::native(proc, name) { return Some(o); }
    if let Some(o) = crate::view::native(proc, name) { return Some(o); }
    if let Some(o) = crate::remcon::native(proc, name) { return Some(o); }
    if let Some(o) = crate::viewappui::native(proc, name) { return Some(o); }
    if let Some(o) = crate::trap::native(proc, name) { return Some(o); }
    if let Some(o) = crate::screen::native(proc, name) { return Some(o); }
    if let Some(o) = crate::desc::native(proc, name) { return Some(o); }
    if let Some(o) = crate::avkon::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fs::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cleanup::native(proc, name) { return Some(o); }
    if let Some(o) = crate::dictstore::native(proc, name) { return Some(o); }
    if let Some(o) = crate::sched::native(proc, name) { return Some(o); }
    if let Some(o) = crate::mixins::native(proc, name) { return Some(o); }
    if let Some(o) = crate::tz::native(proc, name) { return Some(o); }
    if let Some(o) = crate::timer::native(proc, name) { return Some(o); }
    if let Some(o) = crate::control::native(proc, name) { return Some(o); }
    if let Some(o) = crate::etel::native(proc, name) { return Some(o); }
    if let Some(o) = crate::geom::native(proc, name) { return Some(o); }
    if let Some(o) = crate::shell::native(proc, name) { return Some(o); }
    if let Some(o) = crate::window::native(proc, name) { return Some(o); }
    if let Some(o) = crate::toolbar::native(proc, name) { return Some(o); }
    if let Some(o) = crate::time::native(proc, name) { return Some(o); }
    if let Some(o) = crate::array::native(proc, name) { return Some(o); }
    if let Some(o) = crate::fileman::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cenrep::native(proc, name) { return Some(o); }
    if let Some(o) = crate::cdl::native(proc, name) { return Some(o); }
    if let Some(o) = crate::navi::native(proc, name) { return Some(o); }
    if let Some(o) = crate::avkonlayout::native(proc, name) { return Some(o); }
    None
}

pub fn call(proc: &mut Process, stub_index: usize) -> NativeOutcome {
    let stub = match proc.stubs.get(stub_index) {
        Some(stub) => stub,
        None => return NativeOutcome::Refused(String::from("NEXT-NEEDED <bad stub index>")),
    };
    let dll_base = stub.dll_base.clone();
    let ordinal = stub.ordinal;
    let stub_name = stub.name.clone();
    let name = stub_name.as_deref().unwrap_or("");
    if let Some(t) = proc.trace.as_mut() {
        let r = &proc.cpu.regs;
        t.push(format!("{} {}/{} {name} r0-r3={:#x} {:#x} {:#x} {:#x} lr={:#x}", t.len(), dll_base, ordinal, r[0], r[1], r[2], r[3], r[14]));
    }

    // RAllocator / RHeap members need a guest-resident allocator object with a vtable, which this engine does not build
    // (User::Allocator is refused by name for the same reason), so they are refused before anything dereferences one.
    // DELETE WHEN the first corpus image's next refusal is an RAllocator member or User::Allocator (then build the object).
    if name.starts_with("_ZN9RAllocator") || name.starts_with("_ZN5RHeap") {
        return NativeOutcome::Refused(format!(
            "NEXT-NEEDED euser/{} {} (no guest RAllocator object yet)", ordinal, name
        ));
    }

    if let Some(mut hook) = proc.native_hook.take() {
        let handled = hook(proc, name);
        proc.native_hook = Some(hook);
        if let Some(outcome) = handled {
            return outcome;
        }
    }
    if let Some(outcome) = module_native(proc, name) {
        return outcome;
    }

    let r0 = proc.cpu.regs[0];
    let r1 = proc.cpu.regs[1];
    match name {
        // TBufBase16(TInt aMaxLength) / TBufBase8(TInt): `TDes(EBuf, 0, aMaxLength)` (e32des16.h:332 TDes16(aType, aLength, aMaxLength)). A
        // descriptor has no vptr: word 0 is iLength | type << KShiftDesType16 (e32des16.h:16 = 28), word 1 is iMaxLength (:341).
        // DES_TYPE_EBUF: the TType enum is internal to the SDK; EKA2L1 utils/des.h:50-55 (facts) lists buf_const 0, ptr_const 1, ptr 2, buf 3.
        "_ZN10TBufBase16C1Ei" | "_ZN10TBufBase16C2Ei" | "_ZN9TBufBase8C1Ei" | "_ZN9TBufBase8C2Ei" => {
            const DES_TYPE_EBUF: u32 = 3;
            const KSHIFT_DES_TYPE: u32 = 28; // e32des16.h:16
            let ok = proc.mem.vm.write_u32(r0, DES_TYPE_EBUF << KSHIFT_DES_TYPE).is_ok() && proc.mem.vm.write_u32(r0.wrapping_add(4), r1).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name}: descriptor at {r0:#x} is unwritable")) }
        }
        // RApaLsSession(): RSessionBase -> RHandleBase { iHandle } = 0 (e32std.h:3153, :4100), then apgcli.h:224 iExtension = 0: two zero words.
        "_ZN13RApaLsSessionC1Ev" => {
            let ok = proc.mem.vm.write_u32(r0, 0).is_ok() && proc.mem.vm.write_u32(r0.wrapping_add(4), 0).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED apgrfx/{name}: session object at {r0:#x} is unwritable")) }
        }
        // drtaeabi's exception-globals object: recorded, `this` returned. Unwinding itself is slice 1b (see the leaves below).
        "_ZN23TCppRTExceptionsGlobalsC1Ev" => {
            proc.nat.exception_globals.push(r0);
            NativeOutcome::Return
        }
        // Per-process initialisation of the EKA2 user side: nothing in this engine needs initialising, so it returns.
        "_ZN4User11InitProcessEv" => {
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        // UserSvr::DllSetTls(aHandle, aDllUid, aPtr) -> KErrNone; UserSvr::DllTls(aHandle, aDllUid) -> the pointer (0 = none).
        "_ZN7UserSvr9DllSetTlsEiiPv" => {
            let ptr = proc.cpu.regs[2];
            proc.nat.tls.insert((r0, r1), ptr);
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN7UserSvr6DllTlsEii" => {
            proc.cpu.regs[0] = proc.nat.tls.get(&(r0, r1)).copied().unwrap_or(0);
            NativeOutcome::Return
        }
        // Leaves: EHABI unwinding is slice 1b, so every way into one is refused by name, never ignored. DELETE WHEN the EHABI unwinder lands
        // (18-SYMBIAN-EKA2L1.md slice 1b; ruling coral_handoffs/consult-2026-10-26-symbian-runapplication.md step 2).
        "_ZN4User5LeaveEi" => NativeOutcome::Refused(format!("NEXT-NEEDED leave User::Leave({}): EHABI unwinding is slice 1b", r0 as i32)),
        "_ZN4User12LeaveIfErrorEi" if (r0 as i32) < 0 => NativeOutcome::Refused(format!("NEXT-NEEDED leave User::LeaveIfError({}): EHABI unwinding is slice 1b", r0 as i32)),
        "_ZN4User12LeaveIfErrorEi" => NativeOutcome::Return, // a non-negative value is returned as it is (r0 unchanged)
        "__cxa_throw" => NativeOutcome::Refused(String::from("NEXT-NEEDED leave __cxa_throw: EHABI unwinding is slice 1b")),
        "__cxa_allocate_exception" => NativeOutcome::Refused(String::from("NEXT-NEEDED leave __cxa_allocate_exception: EHABI unwinding is slice 1b")),
        // CBase::operator new (inline in e32base.inl) is User::AllocZ / AllocZL: zero-filled. The L forms leave on failure.
        "_ZN4User7AllocZLEi" | "_ZN4User6AllocLEi" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            if proc.heap.is_none() {
                return NativeOutcome::Refused(format!("NEXT-NEEDED euser/{name} before heap setup"));
            }
            match proc.alloc_zeroed(len) {
                Some(a) => { proc.cpu.regs[0] = a; NativeOutcome::Return }
                None => NativeOutcome::Refused(format!("NEXT-NEEDED leave {name}({len}): out of memory leaves KErrNoMemory (EHABI unwinding is slice 1b)")),
            }
        }
        "_ZN8UserHeap15SetupThreadHeapEiR24SStdEpocThreadCreateInfo" => {
            let info_ptr = proc.cpu.regs[1];
            let allocator = match proc.mem.vm.read_u32(info_ptr + 0x30) {
                Ok(v) => v,
                Err(_) => return NativeOutcome::Refused(format!(
                    "NEXT-NEEDED euser SetupThreadHeap: fault reading aInfo+0x30"
                )),
            };
            if proc.heap.is_none() {
                if allocator == 0 {
                    let heap_min = proc.mem.vm.read_u32(info_ptr + 0x34).unwrap_or(0);
                    let heap_max = proc.mem.vm.read_u32(info_ptr + 0x38).unwrap_or(0);
                    let already = crate::layout::pru(heap_min);
                    proc.heap = Some(crate::heap::Heap::new(
                        proc.heap_chunk_id,
                        crate::layout::HEAP_BASE,
                        heap_max,
                        already,
                    ));
                } else {
                    return NativeOutcome::Refused(format!(
                        "NEXT-NEEDED euser SetupThreadHeap with foreign allocator {:#x}", allocator
                    ));
                }
            }
            proc.cpu.regs[0] = 0;
            NativeOutcome::Return
        }
        "_ZN4User5AllocEi" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            match proc.heap.as_mut() {
                Some(h) => {
                    proc.cpu.regs[0] = h.alloc(len, &mut proc.mem).unwrap_or(0);
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::Alloc before heap setup".into()
                ),
            }
        }
        "_ZN4User6AllocZEi" => {
            let len = proc.cpu.regs[0] as i32 as u32;
            if proc.heap.is_none() {
                return NativeOutcome::Refused("NEXT-NEEDED euser/User::AllocZ before heap setup".into());
            }
            proc.cpu.regs[0] = proc.alloc_zeroed(len).unwrap_or(0);
            NativeOutcome::Return
        }
        "_ZN4User4FreeEPv" => {
            let addr = proc.cpu.regs[0];
            if addr == 0 {
                proc.cpu.regs[0] = 0;
                return NativeOutcome::Return;
            }
            match proc.heap.as_mut() {
                Some(h) => match h.free(addr) {
                    Ok(()) => {
                        proc.cpu.regs[0] = 0;
                        NativeOutcome::Return
                    }
                    Err(_) => NativeOutcome::Refused(format!(
                        "NEXT-NEEDED euser/User::Free unknown ptr {:#010x}", addr
                    )),
                },
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::Free before heap setup".into()
                ),
            }
        }
        "_ZN4User7ReAllocEPvii" => {
            let addr = proc.cpu.regs[0];
            let new_len = proc.cpu.regs[1] as i32 as u32;
            let _mode = proc.cpu.regs[2] as i32;
            match proc.heap.as_mut() {
                Some(h) => {
                    if addr == 0 {
                        proc.cpu.regs[0] = h.alloc(new_len, &mut proc.mem).unwrap_or(0);
                    } else {
                        proc.cpu.regs[0] = h.realloc(addr, new_len, &mut proc.mem).unwrap_or(0);
                    }
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::ReAlloc before heap setup".into()
                ),
            }
        }
        "_ZN4User8AllocLenEPKv" => {
            let addr = proc.cpu.regs[0];
            match proc.heap.as_ref() {
                Some(h) => {
                    proc.cpu.regs[0] = h.len_of(addr).unwrap_or(0);
                    NativeOutcome::Return
                }
                None => NativeOutcome::Refused(
                    "NEXT-NEEDED euser/User::AllocLen before heap setup".into()
                ),
            }
        }
        "_ZN4User4ExitEi" => NativeOutcome::Exit(r0 as i32),
        "_ZN4User5PanicERK7TDesC16i" => match crate::desc::read16(proc, r0) {
            Ok(units) => NativeOutcome::Refused(format!(
                "NEXT-NEEDED {}/{} _ZN4User5PanicE (panic=\"{}\" reason={})",
                dll_base, ordinal, crate::desc::to_utf8(&units[..units.len().min(64)]), r1 as i32
            )),
            Err(e) => NativeOutcome::Refused(format!("NEXT-NEEDED {}/{} _ZN4User5PanicE ({e})", dll_base, ordinal)),
        },
        "" => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} <unnamed-ordinal>", dll_base, ordinal
        )),
        _ => NativeOutcome::Refused(format!(
            "NEXT-NEEDED {}/{} {}", dll_base, ordinal, name
        )),
    }
}
impl Process {
    /// Allocate `len` bytes from the guest heap, all of them zero (the whole block, as `AllocLen` reports it).
    pub(crate) fn alloc_zeroed(&mut self, len: u32) -> Option<u32> {
        let h = self.heap.as_mut()?;
        let addr = h.alloc(len, &mut self.mem)?;
        let total = h.len_of(addr).unwrap_or(0);
        for i in 0..total {
            let _ = self.mem.vm.write_u8(addr + i, 0);
        }
        Some(addr)
    }
}
