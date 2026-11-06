//! `NSControl`, `NSButton` and `NSTextField`: controls the host draws. A control added to an open window becomes a host
//! `ControlModel` with a token (the guest object) and a generation; the user acting on it comes back as an `Intent` event and is
//! delivered as the message the app expects (a button's action sent to its target, a field's text committed to its value).
//! Nothing here paints: a control is a model the host's kit renders.
use crate::state::{self, reg};
use crate::string::{self, imp};
use crate::view::{self, NsRect, ViewObj};
use crate::window;
use alloc::format;
use alloc::string::String;
use heirloom_objc::runtime::{self, Class, Id, NativeMethod, Sel};
use heirloom_ui_abi::{CallbackToken, ControlKind, ControlModel, ControlValue, GuestRect, GuestUiEvent, Intent, UiObjectRef};

/// The instance layout of `NSControl` and its subclasses.
#[repr(C)]
pub struct ControlObj {
    pub view: ViewObj,
    pub target: usize,
    pub action: usize,
    /// The control's string value (a text field's text; a button's title).
    pub value: usize,
    pub ui_id: u32,
    pub ui_gen: u32,
    /// Bit 0: editable (text fields).
    pub flags: u64,
}

const EDITABLE: u64 = 1;
/// A field's character limit as published: the host treats it as the cap on typed characters.
const FIELD_MAX: u32 = 4096;

fn control_of<'a>(c: Id) -> &'a mut ControlObj {
    // SAFETY: callers pass an NSControl instance.
    unsafe { &mut *(c as *mut ControlObj) }
}

fn is_kind(obj: Id, name: &str) -> bool {
    let target = runtime::class_named(name);
    // SAFETY: obj is an instance of a registered class.
    let mut c = unsafe { runtime::object_class(obj) };
    while !c.is_null() {
        if c == target {
            return true;
        }
        c = runtime::class_superclass(c);
    }
    false
}

fn set_value(c: Id, s: Id) {
    let o = control_of(c);
    runtime::retain(s);
    runtime::release(o.value as Id);
    o.value = s as usize;
}

fn published(c: Id) -> Option<(Id, UiObjectRef)> {
    let o = control_of(c);
    let win = o.view.window as Id;
    if o.ui_id == 0 || win.is_null() {
        return None;
    }
    Some((win, UiObjectRef { id: o.ui_id, generation: o.ui_gen }))
}

extern "C" fn set_target(this: Id, _cmd: Sel, target: Id) {
    control_of(this).target = target as usize;
}
extern "C" fn target(this: Id, _cmd: Sel) -> Id {
    control_of(this).target as Id
}
extern "C" fn set_action(this: Id, _cmd: Sel, action: Sel) {
    control_of(this).action = action as usize;
}
extern "C" fn action(this: Id, _cmd: Sel) -> Sel {
    control_of(this).action as Sel
}
extern "C" fn string_value(this: Id, _cmd: Sel) -> Id {
    control_of(this).value as Id
}
extern "C" fn set_string_value(this: Id, _cmd: Sel, s: Id) {
    set_value(this, s);
    if is_kind(this, "NSTextField") {
        if let Some((win, r)) = published(this) {
            let text = string::to_owned(s);
            let wr = window::ui_ref(win);
            state::ui(|u| u.set_control(wr, r, ControlValue::Text(text))).unwrap_or_else(|e| state::refused("set_control", e));
        }
    }
}
extern "C" fn set_title(this: Id, _cmd: Sel, title: Id) {
    set_value(this, title);
    // the host label changes with the control, so the control is replaced: the new incarnation has the next generation
    if let Some((win, r)) = published(this) {
        let wr = window::ui_ref(win);
        state::ui(|u| u.remove_control(wr, r)).unwrap_or_else(|e| state::refused("remove_control", e));
        control_of(this).ui_id = 0;
        reg().controls.retain(|x| *x != this as usize);
        publish(win, this);
    }
}
extern "C" fn set_editable(this: Id, _cmd: Sel, yes: bool) {
    let o = control_of(this);
    if yes {
        o.flags |= EDITABLE;
    } else {
        o.flags &= !EDITABLE;
    }
}
extern "C" fn is_editable(this: Id, _cmd: Sel) -> bool {
    control_of(this).flags & EDITABLE != 0
}
extern "C" fn init_text_field(this: Id, _cmd: Sel, frame: NsRect) -> Id {
    view::view_of(this).frame = frame;
    control_of(this).flags |= EDITABLE;
    this
}
extern "C" fn dealloc(this: Id, _cmd: Sel) {
    let o = control_of(this);
    runtime::release(o.value as Id);
    o.value = 0;
    reg().controls.retain(|x| *x != this as usize);
    // the view part (subviews) is released by NSView's dealloc through the chain
    let sel = runtime::sel_register("dealloc");
    let sup = runtime::class_superclass(runtime::class_named("NSControl"));
    if let Some(imp) = runtime::class_get_imp(sup, sel) {
        // SAFETY: NSView's dealloc is an `extern "C" fn(Id, Sel)`.
        let f: extern "C" fn(Id, Sel) = unsafe { core::mem::transmute(imp) };
        f(this, sel);
    }
}

fn origin_of(v: Id) -> (f64, f64) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut p = view::view_of(v).superview;
    while p != 0 {
        let f = view::view_of(p as Id).frame;
        x += f.x;
        y += f.y;
        p = view::view_of(p as Id).superview;
    }
    (x, y)
}

/// Publishes `v`, a view added to the open window `w`: a control becomes a host control, any other view publishes its subviews.
pub(crate) fn publish(w: Id, v: Id) {
    let (ox, oy) = origin_of(v);
    publish_at(w, v, ox, oy);
}

fn publish_at(w: Id, v: Id, ox: f64, oy: f64) {
    let frame = view::view_of(v).frame;
    let kind = if is_kind(v, "NSTextField") {
        let o = control_of(v);
        Some((ControlKind::Field { text: String::from(string::text(o.value as Id)), max_len: FIELD_MAX, editable: o.flags & EDITABLE != 0, integer: false }, String::new()))
    } else if is_kind(v, "NSButton") {
        let label = String::from(string::text(control_of(v).value as Id));
        Some((ControlKind::Button, label))
    } else {
        None
    };
    let Some((kind, label)) = kind else {
        for c in view::subviews(v) {
            publish_at(w, c as Id, ox + frame.x, oy + frame.y);
        }
        return;
    };
    if control_of(v).ui_id != 0 {
        return;
    }
    let content_h = window::window_of(w).frame.h;
    let model = ControlModel {
        kind,
        label,
        // Cocoa's origin is the bottom-left of the content area; the host's is the top-left
        rect: GuestRect { left: (ox + frame.x) as i32, top: (content_h - (oy + frame.y + frame.h)) as i32, width: frame.w as i32, height: frame.h as i32 },
        enabled: true,
        token: CallbackToken(v as usize as u64),
    };
    let wr = window::ui_ref(w);
    let r = state::ui(|u| u.add_control(wr, model)).unwrap_or_else(|e| state::refused("add_control", e));
    let o = control_of(v);
    o.ui_id = r.id;
    o.ui_gen = r.generation;
    reg().controls.push(v as usize);
}

/// The published control the host addressed with `token`, when its reference and generation still name it.
fn find(token: u64, control: UiObjectRef, generation: u32) -> Option<Id> {
    let c = token as usize;
    if !reg().controls.contains(&c) {
        return None;
    }
    let o = control_of(c as Id);
    (o.ui_id == control.id && o.ui_gen == control.generation && control.generation == generation).then_some(c as Id)
}

fn send_action(sender: Id) {
    let o = control_of(sender);
    let (target, action) = (o.target as Id, o.action as Sel);
    if target.is_null() || action.is_null() {
        return;
    }
    // SAFETY: target is a non-nil object.
    let imp = unsafe { runtime::resolve(target, action) };
    // SAFETY: an action method is `- (void)action:(id)sender`.
    let f: extern "C" fn(Id, Sel, Id) = unsafe { core::mem::transmute(imp) };
    f(target, action, sender);
    // SAFETY: action is a selector.
    let name = unsafe { runtime::sel_name(action) };
    state::marker(&format!("[OSXX86-ACTION OK] sel=\"{name}\""));
}

/// Handles the events the window and application loop leave to controls.
pub(crate) fn dispatch(_app: Id, event: GuestUiEvent) {
    let GuestUiEvent::Intent { control, token, generation, intent, .. } = event else {
        return;
    };
    let Some(c) = find(token.0, control, generation) else {
        state::marker(&format!("[OSXX86 UI-STALE-EVENT intent id={} generation={}]", control.id, generation));
        return;
    };
    // the host answered for this window: report it once, before the first action runs
    crate::app::first_report(control_of(c).view.window as Id);
    match intent {
        Intent::Activate => send_action(c),
        Intent::CommitText(text) | Intent::SetValue(ControlValue::Text(text)) => {
            // the host already shows the text: store it without echoing it back
            let s = string::make_retained(&text);
            let o = control_of(c);
            runtime::release(o.value as Id);
            o.value = s as usize;
            if control_of(c).action != 0 {
                send_action(c);
            }
        }
        _ => {}
    }
}

/// Registers the control classes.
pub(crate) fn install() {
    if !runtime::class_named("NSControl").is_null() {
        return;
    }
    let view_class = runtime::class_named("NSView");
    let size = core::mem::size_of::<ControlObj>() as u32;
    let control: [NativeMethod; 8] = [
        ("setTarget:", imp(set_target as extern "C" fn(Id, Sel, Id))),
        ("target", imp(target as extern "C" fn(Id, Sel) -> Id)),
        ("setAction:", imp(set_action as extern "C" fn(Id, Sel, Sel))),
        ("action", imp(action as extern "C" fn(Id, Sel) -> Sel)),
        ("stringValue", imp(string_value as extern "C" fn(Id, Sel) -> Id)),
        ("setStringValue:", imp(set_string_value as extern "C" fn(Id, Sel, Id))),
        ("dealloc", imp(dealloc as extern "C" fn(Id, Sel))),
        ("title", imp(string_value as extern "C" fn(Id, Sel) -> Id)),
    ];
    let control_class: Class = runtime::register_native_class("NSControl", view_class, size, &control, &[]);
    let button: [NativeMethod; 1] = [("setTitle:", imp(set_title as extern "C" fn(Id, Sel, Id)))];
    runtime::register_native_class("NSButton", control_class, size, &button, &[]);
    let field: [NativeMethod; 3] = [
        ("initWithFrame:", imp(init_text_field as extern "C" fn(Id, Sel, NsRect) -> Id)),
        ("setEditable:", imp(set_editable as extern "C" fn(Id, Sel, bool))),
        ("isEditable", imp(is_editable as extern "C" fn(Id, Sel) -> bool)),
    ];
    runtime::register_native_class("NSTextField", control_class, size, &field, &[]);
}
