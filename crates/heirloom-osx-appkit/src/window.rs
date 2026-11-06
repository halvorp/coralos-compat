//! `NSWindow`: the object a guest window is; ordering it front opens the host window through the UI seam.
use crate::state::{self, reg};
use crate::string::{self, imp};
use crate::view::{self, NsRect};
use heirloom_objc::runtime::{self, Class, Id, NativeMethod, Sel};
use heirloom_ui_abi::{CallbackToken, GuestRect, UiObjectRef, WindowModel, WindowRole};

/// The instance layout of `NSWindow`.
#[repr(C)]
pub struct WindowObj {
    pub isa: Class,
    pub title: usize,
    pub content_view: usize,
    pub delegate: usize,
    pub ui_id: u32,
    pub ui_gen: u32,
    pub frame: NsRect,
    pub style: u64,
    pub opened: u64,
}

pub(crate) fn window_of<'a>(w: Id) -> &'a mut WindowObj {
    // SAFETY: callers pass an NSWindow instance.
    unsafe { &mut *(w as *mut WindowObj) }
}

/// The host reference of an open window.
pub(crate) fn ui_ref(w: Id) -> UiObjectRef {
    let o = window_of(w);
    UiObjectRef { id: o.ui_id, generation: o.ui_gen }
}

pub(crate) fn guest_rect(r: NsRect) -> GuestRect {
    GuestRect { left: r.x as i32, top: r.y as i32, width: r.w as i32, height: r.h as i32 }
}

extern "C" fn init_with_content_rect(this: Id, _cmd: Sel, rect: NsRect, style: u64, _backing: u64, _defer: i8) -> Id {
    let o = window_of(this);
    o.frame = rect;
    o.style = style;
    let view_class = runtime::class_named("NSView");
    let content = runtime::alloc_instance(view_class, 0);
    view::view_of(content).frame = NsRect { x: 0.0, y: 0.0, w: rect.w, h: rect.h };
    view::view_of(content).window = this as usize;
    o.content_view = content as usize;
    this
}
extern "C" fn set_title(this: Id, _cmd: Sel, title: Id) {
    let o = window_of(this);
    runtime::retain(title);
    runtime::release(o.title as Id);
    o.title = title as usize;
    if o.opened != 0 {
        let t = string::to_owned(title);
        let r = ui_ref(this);
        state::ui(|u| u.set_window_title(r, t)).unwrap_or_else(|e| state::refused("set_window_title", e));
    }
}
extern "C" fn title(this: Id, _cmd: Sel) -> Id {
    window_of(this).title as Id
}
extern "C" fn content_view(this: Id, _cmd: Sel) -> Id {
    window_of(this).content_view as Id
}
extern "C" fn set_delegate(this: Id, _cmd: Sel, delegate: Id) {
    window_of(this).delegate = delegate as usize;
}
extern "C" fn delegate(this: Id, _cmd: Sel) -> Id {
    window_of(this).delegate as Id
}
extern "C" fn get_frame(this: Id, _cmd: Sel) -> NsRect {
    window_of(this).frame
}
extern "C" fn is_visible(this: Id, _cmd: Sel) -> bool {
    window_of(this).opened != 0
}
extern "C" fn order_front(this: Id, _cmd: Sel, _sender: Id) {
    open(this);
}
extern "C" fn close(this: Id, _cmd: Sel) {
    close_window(this);
}
extern "C" fn dealloc(this: Id, _cmd: Sel) {
    let o = window_of(this);
    runtime::release(o.title as Id);
    runtime::release(o.content_view as Id);
    runtime::free_instance(this)
}

/// Opens the host window for `w` (once) and publishes the controls already in its content view.
pub(crate) fn open(w: Id) {
    let o = window_of(w);
    if o.opened != 0 {
        return;
    }
    let model = WindowModel {
        title: string::to_owned(o.title as Id),
        rect: guest_rect(o.frame),
        role: WindowRole::Document,
        closable: o.style & 2 != 0,
        resizable: o.style & 8 != 0,
        token: CallbackToken(w as usize as u64),
    };
    let r = state::ui(|u| u.open_window(model)).unwrap_or_else(|e| state::refused("open_window", e));
    o.ui_id = r.id;
    o.ui_gen = r.generation;
    o.opened = 1;
    reg().windows.push(w as usize);
    let content = o.content_view as Id;
    view::adopt(content, w as usize);
    for child in view::subviews(content) {
        publish_view(w, child as Id);
    }
}

/// Closes the host window of `w`.
pub(crate) fn close_window(w: Id) {
    let o = window_of(w);
    if o.opened == 0 {
        return;
    }
    let r = ui_ref(w);
    o.opened = 0;
    reg().windows.retain(|x| *x != w as usize);
    state::ui(|u| u.close_window(r)).unwrap_or_else(|e| state::refused("close_window", e));
}

/// Publishes a view that was added to an open window (a control becomes a host control; plain views have nothing to publish yet).
pub(crate) fn publish_view(w: Id, v: Id) {
    crate::controls::publish(w, v);
}

/// The open window the host addressed with `token` and `r`, when both still name it.
pub(crate) fn find(token: u64, r: UiObjectRef) -> Option<Id> {
    let w = token as usize;
    if !reg().windows.contains(&w) {
        return None;
    }
    let o = window_of(w as Id);
    (o.ui_id == r.id && o.ui_gen == r.generation).then_some(w as Id)
}

/// The `title`, width and height for the window marker.
pub(crate) fn describe(w: Id) -> alloc::string::String {
    let o = window_of(w);
    alloc::format!("title=\"{}\" w={} h={}", string::text(o.title as Id), o.frame.w as i32, o.frame.h as i32)
}

/// Registers `NSWindow`.
pub(crate) fn install() {
    if !runtime::class_named("NSWindow").is_null() {
        return;
    }
    let responder = runtime::class_named("NSResponder");
    let instance: [NativeMethod; 12] = [
        ("initWithContentRect:styleMask:backing:defer:", imp(init_with_content_rect as extern "C" fn(Id, Sel, NsRect, u64, u64, i8) -> Id)),
        ("setTitle:", imp(set_title as extern "C" fn(Id, Sel, Id))),
        ("title", imp(title as extern "C" fn(Id, Sel) -> Id)),
        ("contentView", imp(content_view as extern "C" fn(Id, Sel) -> Id)),
        ("setDelegate:", imp(set_delegate as extern "C" fn(Id, Sel, Id))),
        ("delegate", imp(delegate as extern "C" fn(Id, Sel) -> Id)),
        ("frame", imp(get_frame as extern "C" fn(Id, Sel) -> NsRect)),
        ("isVisible", imp(is_visible as extern "C" fn(Id, Sel) -> bool)),
        ("makeKeyAndOrderFront:", imp(order_front as extern "C" fn(Id, Sel, Id))),
        ("orderFront:", imp(order_front as extern "C" fn(Id, Sel, Id))),
        ("close", imp(close as extern "C" fn(Id, Sel))),
        ("dealloc", imp(dealloc as extern "C" fn(Id, Sel))),
    ];
    runtime::register_native_class("NSWindow", responder, core::mem::size_of::<WindowObj>() as u32, &instance, &[]);
}
