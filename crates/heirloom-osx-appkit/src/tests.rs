//! The adapter's rules proven directly, with no compiled app: classes are messaged through the runtime's own trampolines and the
//! host is a recording UI. Each rule here is also exercised by the engine's fixtures; these pin them where they live.
extern crate std;

use crate::state::{self, AppHost};
use crate::string;
use crate::view::NsRect;
use heirloom_objc::runtime::{self, Class, Id, Sel};
use heirloom_objc::{msgsend_address, nsobject};
use heirloom_ui_abi::{CallbackToken, ControlKind, ControlValue, GuestRect, GuestUi, GuestUiEvent, Intent, RecordingGuestUi, UiCall, UiObjectRef};
use std::string::{String, ToString};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::vec::Vec;

static LOCK: Mutex<()> = Mutex::new(());

struct Host {
    ui: RecordingGuestUi,
    markers: Vec<String>,
}

impl AppHost for Host {
    fn ui(&mut self) -> Option<&mut dyn GuestUi> {
        Some(&mut self.ui)
    }
    fn marker(&mut self, line: &str) {
        self.markers.push(line.to_string());
    }
}

/// One guest session: a fresh runtime, the classes installed, the host attached.
struct Session {
    _g: std::sync::MutexGuard<'static, ()>,
    host: std::boxed::Box<Host>,
}

fn session() -> Session {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::reset();
    nsobject::install();
    crate::install();
    let mut host = std::boxed::Box::new(Host { ui: RecordingGuestUi::new(), markers: Vec::new() });
    // SAFETY: the box outlives the session (dropped after `end` in Session::drop).
    unsafe { state::begin(&mut *host) };
    Session { _g: g, host }
}

impl Drop for Session {
    fn drop(&mut self) {
        state::end();
        runtime::reset();
    }
}

fn class(name: &str) -> Id {
    runtime::class_named(name) as Id
}
fn sel(n: &str) -> Sel {
    runtime::sel_register(n)
}
fn send0(o: Id, n: &str) -> Id {
    let f: extern "C" fn(Id, Sel) -> Id = unsafe { core::mem::transmute(msgsend_address()) };
    f(o, sel(n))
}
fn send1(o: Id, n: &str, a: Id) -> Id {
    let f: extern "C" fn(Id, Sel, Id) -> Id = unsafe { core::mem::transmute(msgsend_address()) };
    f(o, sel(n), a)
}
fn new_view(cls: &str, x: f64, y: f64, w: f64, h: f64) -> Id {
    let obj = send0(class(cls), "alloc");
    let f: extern "C" fn(Id, Sel, NsRect) -> Id = unsafe { core::mem::transmute(msgsend_address()) };
    f(obj, sel("initWithFrame:"), NsRect { x, y, w, h })
}
fn new_window(w: f64, h: f64) -> Id {
    let win = send0(class("NSWindow"), "alloc");
    let f: extern "C" fn(Id, Sel, NsRect, u64, u64, i8) -> Id = unsafe { core::mem::transmute(msgsend_address()) };
    f(win, sel("initWithContentRect:styleMask:backing:defer:"), NsRect { x: 100.0, y: 100.0, w, h }, 15, 2, 0)
}
fn text(s: &str) -> Id {
    string::make(s)
}

static ACTIONS: AtomicUsize = AtomicUsize::new(0);
static LAST_SENDER: AtomicUsize = AtomicUsize::new(0);
extern "C" fn on_click(_this: Id, _cmd: Sel, sender: Id) {
    ACTIONS.fetch_add(1, Ordering::SeqCst);
    LAST_SENDER.store(sender as usize, Ordering::SeqCst);
}

fn target() -> Id {
    let ns = runtime::class_named("NSObject");
    let imp: usize = unsafe { core::mem::transmute(on_click as extern "C" fn(Id, Sel, Id)) };
    let cls: Class = runtime::register_native_class("TestTarget", ns, 8, &[("clicked:", imp)], &[]);
    send0(send0(cls as Id, "alloc"), "init")
}

fn controls(s: &Session) -> Vec<(UiObjectRef, heirloom_ui_abi::ControlModel)> {
    s.host
        .ui
        .calls
        .iter()
        .filter_map(|c| if let UiCall::AddControl(_, m) = c { Some(m.clone()) } else { None })
        .enumerate()
        .map(|(i, m)| (UiObjectRef { id: 0, generation: i as u32 }, m))
        .collect()
}

#[test]
fn cocoa_origin_is_flipped_and_nested_views_add_their_offsets() {
    let mut s = session();
    let win = new_window(320.0, 120.0);
    let content = send0(win, "contentView");
    let field = new_view("NSTextField", 20.0, 70.0, 280.0, 24.0);
    let panel = new_view("NSView", 10.0, 10.0, 200.0, 100.0);
    let inner = new_view("NSButton", 5.0, 5.0, 50.0, 20.0);
    send1(panel, "addSubview:", inner);
    send1(content, "addSubview:", field);
    send1(content, "addSubview:", panel);
    send1(win, "makeKeyAndOrderFront:", core::ptr::null_mut());
    let rects: Vec<GuestRect> = controls(&s).iter().map(|(_, m)| m.rect).collect();
    assert_eq!(rects[0], GuestRect { left: 20, top: 120 - (70 + 24), width: 280, height: 24 }, "a top-level control");
    assert_eq!(rects[1], GuestRect { left: 15, top: 120 - (10 + 5 + 20), width: 50, height: 20 }, "a control inside a panel at (10,10)");
    assert!(s.host.ui.calls.iter().all(|c| !matches!(c, UiCall::PresentContent { .. } | UiCall::PresentPlane(..))));
    let _ = &mut s;
}

#[test]
fn controls_added_before_the_window_opens_are_published_when_it_opens_and_not_before() {
    let s = session();
    let win = new_window(200.0, 100.0);
    let content = send0(win, "contentView");
    send1(content, "addSubview:", new_view("NSButton", 0.0, 0.0, 10.0, 10.0));
    assert!(s.host.ui.calls.is_empty(), "nothing reaches the host before the window is ordered front");
    send1(win, "makeKeyAndOrderFront:", core::ptr::null_mut());
    assert!(matches!(s.host.ui.calls[0], UiCall::OpenWindow(_)));
    assert!(matches!(s.host.ui.calls[1], UiCall::AddControl(..)));
    // a control added to the open window is published at once
    send1(content, "addSubview:", new_view("NSTextField", 0.0, 20.0, 10.0, 10.0));
    assert!(matches!(s.host.ui.calls[2], UiCall::AddControl(..)));
}

fn run_app() {
    let app = send0(class("NSApplication"), "sharedApplication");
    send0(app, "run");
}

#[test]
fn an_intent_is_delivered_only_for_the_token_and_generation_the_host_issued() {
    let mut s = session();
    ACTIONS.store(0, Ordering::SeqCst);
    let win = new_window(320.0, 120.0);
    let button = new_view("NSButton", 20.0, 20.0, 100.0, 30.0);
    let t = target();
    send1(button, "setTarget:", t);
    let setaction: extern "C" fn(Id, Sel, Sel) = unsafe { core::mem::transmute(msgsend_address()) };
    setaction(button, sel("setAction:"), sel("clicked:"));
    send1(send0(win, "contentView"), "addSubview:", button);
    send1(win, "makeKeyAndOrderFront:", core::ptr::null_mut());
    let win_ref = UiObjectRef { id: 1, generation: 1 };
    let ctl = UiObjectRef { id: 2, generation: 1 };
    let token = CallbackToken(button as usize as u64);
    let ev = |control: UiObjectRef, token: CallbackToken, generation: u32| GuestUiEvent::Intent { win: win_ref, control, token, generation, intent: Intent::Activate };
    s.host.ui.events.push_back(ev(ctl, CallbackToken(token.0 + 8), 1)); // a token naming no control
    s.host.ui.events.push_back(ev(UiObjectRef { id: 2, generation: 7 }, token, 7)); // a generation never issued
    s.host.ui.events.push_back(ev(ctl, token, 9)); // the event's own generation disagrees with its reference
    s.host.ui.events.push_back(ev(UiObjectRef { id: 9, generation: 1 }, token, 1)); // another object's id
    s.host.ui.events.push_back(ev(ctl, CallbackToken(8), 1)); // a token that is not even an address of ours: it must never be dereferenced
    assert_eq!(ACTIONS.load(Ordering::SeqCst), 0);
    run_app();
    assert_eq!(ACTIONS.load(Ordering::SeqCst), 0, "none of the five reached the action");
    assert_eq!(s.host.markers.iter().filter(|m| m.contains("UI-STALE-EVENT intent")).count(), 5, "{:?}", s.host.markers);
    s.host.ui.events.push_back(ev(ctl, token, 1));
    run_app();
    assert_eq!(ACTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(LAST_SENDER.load(Ordering::SeqCst), button as usize, "the control is the sender");
}

#[test]
fn retitling_a_published_button_makes_a_new_control_and_retires_the_old_reference() {
    let mut s = session();
    ACTIONS.store(0, Ordering::SeqCst);
    let win = new_window(320.0, 120.0);
    let button = new_view("NSButton", 20.0, 20.0, 100.0, 30.0);
    send1(button, "setTitle:", text("One"));
    let t = target();
    send1(button, "setTarget:", t);
    let setaction: extern "C" fn(Id, Sel, Sel) = unsafe { core::mem::transmute(msgsend_address()) };
    setaction(button, sel("setAction:"), sel("clicked:"));
    send1(send0(win, "contentView"), "addSubview:", button);
    send1(win, "makeKeyAndOrderFront:", core::ptr::null_mut());
    send1(button, "setTitle:", text("Two"));
    let adds: Vec<_> = s.host.ui.calls.iter().filter_map(|c| if let UiCall::AddControl(_, m) = c { Some(m.label.clone()) } else { None }).collect();
    assert_eq!(adds, std::vec!["One".to_string(), "Two".to_string()]);
    assert_eq!(s.host.ui.calls.iter().filter(|c| matches!(c, UiCall::RemoveControl(..))).count(), 1);
    let old = UiObjectRef { id: 2, generation: 1 };
    let new_ref = UiObjectRef { id: 3, generation: 2 };
    let token = CallbackToken(button as usize as u64);
    let win_ref = UiObjectRef { id: 1, generation: 1 };
    s.host.ui.events.push_back(GuestUiEvent::Intent { win: win_ref, control: old, token, generation: 1, intent: Intent::Activate });
    run_app();
    assert_eq!(ACTIONS.load(Ordering::SeqCst), 0, "a click on the retired reference is refused");
    s.host.ui.events.push_back(GuestUiEvent::Intent { win: win_ref, control: new_ref, token, generation: 2, intent: Intent::Activate });
    run_app();
    assert_eq!(ACTIONS.load(Ordering::SeqCst), 1, "the new incarnation works");
}

#[test]
fn committed_text_is_stored_for_the_app_and_not_echoed_but_a_program_set_value_reaches_the_host() {
    let mut s = session();
    let win = new_window(320.0, 120.0);
    let field = new_view("NSTextField", 0.0, 0.0, 100.0, 20.0);
    send1(send0(win, "contentView"), "addSubview:", field);
    send1(win, "makeKeyAndOrderFront:", core::ptr::null_mut());
    let token = CallbackToken(field as usize as u64);
    s.host.ui.events.push_back(GuestUiEvent::Intent {
        win: UiObjectRef { id: 1, generation: 1 },
        control: UiObjectRef { id: 2, generation: 1 },
        token,
        generation: 1,
        intent: Intent::CommitText("typed".to_string()),
    });
    run_app();
    assert_eq!(string::to_owned(send0(field, "stringValue")), "typed");
    assert!(!s.host.ui.calls.iter().any(|c| matches!(c, UiCall::SetControl(..))), "no echo of what the host already shows");
    send1(field, "setStringValue:", text("from the app"));
    let last = s.host.ui.calls.last().unwrap();
    assert!(matches!(last, UiCall::SetControl(_, _, ControlValue::Text(t)) if t == "from the app"), "{last:?}");
    let field_kind = controls(&s)[0].1.kind.clone();
    assert!(matches!(field_kind, ControlKind::Field { editable: true, .. }));
}

#[test]
fn a_session_with_no_host_ui_refuses_by_name() {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::reset();
    nsobject::install();
    crate::install();
    let win = new_window(10.0, 10.0);
    runtime::set_fatal_hook(|m| panic!("{}", m));
    // called directly: a panic must not unwind through the assembly trampolines
    let r = std::panic::catch_unwind(|| crate::window::open(win));
    let msg = *r.expect_err("must refuse").downcast::<String>().unwrap();
    assert_eq!(msg, "OSXX86-UI-UNAVAILABLE the app used the UI with no host session");
    drop(g);
    runtime::reset();
}
