//! `NSResponder` and `NSView`: a frame, a superview, subviews and the window a view belongs to.
use crate::string::imp;
use alloc::boxed::Box;
use alloc::vec::Vec;
use heirloom_objc::runtime::{self, Class, Id, NativeMethod, Sel};

/// `NSRect`: origin and size in doubles (32 bytes, passed and returned in memory on x86_64).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NsRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The instance layout of `NSView` (subclasses extend it).
#[repr(C)]
pub struct ViewObj {
    pub isa: Class,
    pub superview: usize,
    pub window: usize,
    pub subviews: *mut Vec<usize>,
    pub frame: NsRect,
}

pub(crate) fn view_of<'a>(v: Id) -> &'a mut ViewObj {
    // SAFETY: callers pass an NSView instance.
    unsafe { &mut *(v as *mut ViewObj) }
}

/// The subviews of a view, in order.
pub fn subviews(v: Id) -> Vec<usize> {
    let o = view_of(v);
    if o.subviews.is_null() {
        Vec::new()
    } else {
        // SAFETY: a non-null subviews pointer is a leaked Vec.
        unsafe { (*o.subviews).clone() }
    }
}

extern "C" fn init_with_frame(this: Id, _cmd: Sel, frame: NsRect) -> Id {
    view_of(this).frame = frame;
    this
}
extern "C" fn get_frame(this: Id, _cmd: Sel) -> NsRect {
    view_of(this).frame
}
extern "C" fn set_frame(this: Id, _cmd: Sel, frame: NsRect) {
    view_of(this).frame = frame;
}
extern "C" fn superview(this: Id, _cmd: Sel) -> Id {
    view_of(this).superview as Id
}
extern "C" fn window(this: Id, _cmd: Sel) -> Id {
    view_of(this).window as Id
}
extern "C" fn add_subview(this: Id, _cmd: Sel, child: Id) {
    if child.is_null() {
        return;
    }
    let parent = view_of(this);
    if parent.subviews.is_null() {
        parent.subviews = Box::into_raw(Box::new(Vec::new()));
    }
    runtime::retain(child);
    // SAFETY: subviews is a leaked Vec.
    unsafe { (*parent.subviews).push(child as usize) };
    let c = view_of(child);
    c.superview = this as usize;
    adopt(child, parent.window);
    if parent.window != 0 && crate::window::window_of(parent.window as Id).opened != 0 {
        crate::window::publish_view(parent.window as Id, child);
    }
}
extern "C" fn dealloc(this: Id, _cmd: Sel) {
    let o = view_of(this);
    if !o.subviews.is_null() {
        // SAFETY: a non-null subviews pointer is a leaked Vec owned by this view.
        let list = unsafe { Box::from_raw(o.subviews) };
        for c in list.iter() {
            runtime::release(*c as Id);
        }
        o.subviews = core::ptr::null_mut();
    }
    runtime::free_instance(this)
}

/// Sets the window of a view and all its descendants.
pub(crate) fn adopt(v: Id, window: usize) {
    view_of(v).window = window;
    for c in subviews(v) {
        adopt(c as Id, window);
    }
}

/// Registers `NSResponder` and `NSView`.
pub(crate) fn install() {
    if !runtime::class_named("NSView").is_null() {
        return;
    }
    let ns = runtime::class_named("NSObject");
    let responder = runtime::register_native_class("NSResponder", ns, 8, &[], &[]);
    let instance: [NativeMethod; 8] = [
        ("initWithFrame:", imp(init_with_frame as extern "C" fn(Id, Sel, NsRect) -> Id)),
        ("frame", imp(get_frame as extern "C" fn(Id, Sel) -> NsRect)),
        ("setFrame:", imp(set_frame as extern "C" fn(Id, Sel, NsRect))),
        ("superview", imp(superview as extern "C" fn(Id, Sel) -> Id)),
        ("window", imp(window as extern "C" fn(Id, Sel) -> Id)),
        ("addSubview:", imp(add_subview as extern "C" fn(Id, Sel, Id))),
        ("dealloc", imp(dealloc as extern "C" fn(Id, Sel))),
        ("isFlipped", imp(is_flipped as extern "C" fn(Id, Sel) -> bool)),
    ];
    runtime::register_native_class("NSView", responder, core::mem::size_of::<ViewObj>() as u32, &instance, &[]);
}

extern "C" fn is_flipped(_this: Id, _cmd: Sel) -> bool {
    false
}
