//! `NSApplication`: the singleton, the event loop that turns host events into the messages a Cocoa app expects, and `_NSApp`.
use crate::state::{self, reg};
use crate::string::imp;
use crate::window;
use alloc::boxed::Box;
use alloc::format;
use heirloom_objc::runtime::{self, Class, Id, NativeMethod, Sel};
use heirloom_ui_abi::{GuestUiEvent, UiObjectRef};

/// The instance layout of `NSApplication`.
#[repr(C)]
pub struct AppObj {
    pub isa: Class,
    pub delegate: usize,
}

/// The address `_NSApp` binds to: a pointer-sized cell the singleton is stored into.
pub(crate) fn nsapp_cell() -> usize {
    let r = reg();
    if r.nsapp == 0 {
        r.nsapp = Box::into_raw(Box::new(0usize)) as usize;
    }
    r.nsapp
}

extern "C" fn shared_application(cls: Id, _cmd: Sel) -> Id {
    let r = reg();
    if r.app == 0 {
        let app = runtime::alloc_instance(cls as Class, 0);
        r.app = app as usize;
        // SAFETY: nsapp_cell is a leaked usize.
        unsafe { *(nsapp_cell() as *mut usize) = app as usize };
    }
    r.app as Id
}
extern "C" fn set_delegate(this: Id, _cmd: Sel, delegate: Id) {
    // SAFETY: this is an NSApplication instance.
    unsafe { (*(this as *mut AppObj)).delegate = delegate as usize };
}
extern "C" fn delegate(this: Id, _cmd: Sel) -> Id {
    // SAFETY: this is an NSApplication instance.
    unsafe { (*(this as *mut AppObj)).delegate as Id }
}
extern "C" fn noop_bool(_this: Id, _cmd: Sel, _arg: u64) -> bool {
    true
}
extern "C" fn noop(_this: Id, _cmd: Sel) {}
extern "C" fn stop(_this: Id, _cmd: Sel, _sender: Id) {
    reg().stop = true;
}
extern "C" fn run(this: Id, _cmd: Sel) {
    reg().stop = false;
    loop {
        if reg().stop {
            break;
        }
        let event = state::ui(|u| u.poll_event());
        match event {
            None => break,
            Some(e) => dispatch(this, e),
        }
    }
}

fn respond_bool(obj: Id, sel_name: &str, arg: Id) -> Option<bool> {
    if obj.is_null() {
        return None;
    }
    let sel = runtime::sel_register(sel_name);
    // SAFETY: obj is a non-nil object.
    let cls = unsafe { runtime::object_class(obj) };
    let imp = runtime::class_get_imp(cls, sel)?;
    // SAFETY: a delegate predicate is `BOOL (id, SEL, id)`.
    let f: extern "C" fn(Id, Sel, Id) -> bool = unsafe { core::mem::transmute(imp) };
    Some(f(obj, sel, arg))
}

fn first_report(w: Id) {
    let r = reg();
    if !r.reported.contains(&(w as usize)) {
        r.reported.push(w as usize);
        state::marker(&format!("[OSXX86-WINDOW OK] {}", window::describe(w)));
    }
}

fn stale(what: &str, r: UiObjectRef) {
    state::marker(&format!("[OSXX86 UI-STALE-EVENT {what} id={} generation={}]", r.id, r.generation));
}

fn dispatch(app: Id, event: GuestUiEvent) {
    match event {
        GuestUiEvent::CloseRequested { win, token } => match window::find(token.0, win) {
            None => stale("close", win),
            Some(w) => {
                first_report(w);
                // SAFETY: w is a live NSWindow.
                let delegate = unsafe { (*(w as *mut window::WindowObj)).delegate } as Id;
                if respond_bool(delegate, "windowShouldClose:", w).unwrap_or(true) {
                    window::close_window(w);
                }
            }
        },
        GuestUiEvent::Resized { win, rect } => {
            // Resized carries no token: find the window by its host reference.
            let found = reg().windows.iter().copied().find(|w| {
                let r = window::ui_ref(*w as Id);
                r.id == win.id && r.generation == win.generation
            });
            match found {
                None => stale("resize", win),
                Some(w) => {
                    let w = w as Id;
                    let o = window::window_of(w);
                    o.frame.w = rect.width as f64;
                    o.frame.h = rect.height as f64;
                    first_report(w);
                    crate::view::view_of(o.content_view as Id).frame = crate::view::NsRect { x: 0.0, y: 0.0, w: rect.width as f64, h: rect.height as f64 };
                }
            }
        }
        other => crate::controls::dispatch(app, other),
    }
}

/// Registers `NSApplication`.
pub(crate) fn install() {
    if !runtime::class_named("NSApplication").is_null() {
        return;
    }
    let responder = runtime::class_named("NSResponder");
    let instance: [NativeMethod; 9] = [
        ("run", imp(run as extern "C" fn(Id, Sel))),
        ("stop:", imp(stop as extern "C" fn(Id, Sel, Id))),
        ("terminate:", imp(stop as extern "C" fn(Id, Sel, Id))),
        ("setDelegate:", imp(set_delegate as extern "C" fn(Id, Sel, Id))),
        ("delegate", imp(delegate as extern "C" fn(Id, Sel) -> Id)),
        ("finishLaunching", imp(noop as extern "C" fn(Id, Sel))),
        ("activateIgnoringOtherApps:", imp(noop_bool as extern "C" fn(Id, Sel, u64) -> bool)),
        ("setActivationPolicy:", imp(noop_bool as extern "C" fn(Id, Sel, u64) -> bool)),
        ("isRunning", imp(noop_bool as extern "C" fn(Id, Sel, u64) -> bool)),
    ];
    let class: [NativeMethod; 1] = [("sharedApplication", imp(shared_application as extern "C" fn(Id, Sel) -> Id))];
    runtime::register_native_class("NSApplication", responder, core::mem::size_of::<AppObj>() as u32, &instance, &class);
}
