use alloc::{format, vec, vec::Vec};
use core::ptr::null;

use crate::{
    enter::{osxx86_enter, SavedCtx},
    libsystem, load,
    CodeMem, Host, Prot, Refusal,
};

/// How a guest ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The guest returned from main or called exit with this code.
    Exited(i32),
    /// The image was refused (or the guest hit a refusal while running); the marker line was sent to the host.
    Refused(Refusal),
}

/// The guest stack: 64 pages, the lowest one a guard page.
const STACK: usize = 0x40000;

struct SessionGuard;

impl Drop for SessionGuard {
    fn drop(&mut self) {
        libsystem::end();
    }
}

/// Loads `file`, runs it natively on its own stack until it exits, and reports how it ended. One guest at a time per process.
pub fn run(file: &[u8], args: &[&str], host: &mut dyn Host, mem: &mut dyn CodeMem) -> Outcome {
    let mut save = SavedCtx::default();
    let mut session = libsystem::Session::new(host, &mut save, Vec::new());
    if !libsystem::begin(&mut session) {
        host.marker(&Refusal::Busy.marker());
        return Outcome::Refused(Refusal::Busy);
    }
    let _guard = SessionGuard;

    let l = match load::load(file, mem) {
        Ok(l) => l,
        Err(r) => {
            host.marker(&r.marker());
            return Outcome::Refused(r);
        }
    };
    host.marker(&format!("[OSXX86 LOAD OK] segments={} imports={}", l.segments, l.imports));

    let stack = match mem.reserve(STACK) {
        Some(stack) => stack,
        None => {
            mem.release(l.base, l.reserved);
            host.marker(&Refusal::OutOfMemory.marker());
            return Outcome::Refused(Refusal::OutOfMemory);
        }
    };
    if let Err(why) = mem.publish(stack, &[(0, 0x1000, Prot::None), (0x1000, STACK - 0x1000, Prot::RW)]) {
        mem.release(stack, STACK);
        mem.release(l.base, l.reserved);
        host.marker(&Refusal::Publish(why).marker());
        return Outcome::Refused(Refusal::Publish(why));
    }

    let top = stack as usize + STACK;
    let program = args.first().copied().unwrap_or("a.out");
    let mut argv_owned: Vec<Vec<u8>> = Vec::new();
    for arg in args {
        let mut bytes = arg.as_bytes().to_vec();
        bytes.push(0);
        argv_owned.push(bytes);
    }
    if argv_owned.is_empty() {
        let mut bytes = program.as_bytes().to_vec();
        bytes.push(0);
        argv_owned.push(bytes);
    }
    let mut argv_ptrs: Vec<*const u8> = argv_owned.iter().map(|arg| arg.as_ptr()).collect();
    argv_ptrs.push(null());
    let envp_ptrs = vec![null()];
    let apple_owned = format!("executable_path={}\0", program);
    let apple_ptrs = vec![apple_owned.as_ptr(), null()];
    let argc = (argv_ptrs.len() - 1) as i32;

    session.readable = vec![l.span(), (stack as usize + 0x1000, top)];
    // SAFETY: entry is the published executable address of the admitted image; top is the
    // 16-aligned top of its writable stack reservation. `save` outlives the call, and all
    // pointer arrays and NUL-terminated strings remain owned by this frame throughout it.
    let code = unsafe {
        osxx86_enter(
            l.entry,
            argc,
            argv_ptrs.as_ptr(),
            envp_ptrs.as_ptr(),
            apple_ptrs.as_ptr(),
            top,
            &mut save,
        )
    };

    mem.release(stack, STACK);
    mem.release(l.base, l.reserved);
    if session.stub_binder_hit {
        host.marker(&Refusal::StubBinder.marker());
        Outcome::Refused(Refusal::StubBinder)
    } else {
        host.marker(&format!("[OSXX86 EXIT code={}]", code));
        Outcome::Exited(code)
    }
}