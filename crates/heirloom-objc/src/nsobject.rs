//! The native root class `NSObject`: allocation, reference counting, introspection and `performSelector:`. Everything richer
//! (strings, collections, descriptions) belongs to the frameworks the guest loads on top of it.
use crate::runtime::{self, Class, Id, Imp, NativeMethod, Sel};

fn send0(obj: Id, name: &str) -> Id {
    let sel = runtime::sel_register(name);
    // SAFETY: obj is a non-nil object or class in every caller.
    let imp = unsafe { runtime::resolve(obj, sel) };
    // SAFETY: the implementation of a zero-argument message is an `extern "C" fn(Id, Sel) -> Id`.
    let f: extern "C" fn(Id, Sel) -> Id = unsafe { core::mem::transmute(imp) };
    f(obj, sel)
}

extern "C" fn alloc(cls: Id, _cmd: Sel) -> Id {
    runtime::alloc_instance(cls as Class, 0)
}
extern "C" fn init(this: Id, _cmd: Sel) -> Id {
    this
}
extern "C" fn new(cls: Id, _cmd: Sel) -> Id {
    let obj = send0(cls, "alloc");
    send0(obj, "init")
}
extern "C" fn dealloc(this: Id, _cmd: Sel) {
    runtime::free_instance(this)
}
extern "C" fn retain(this: Id, _cmd: Sel) -> Id {
    runtime::retain(this)
}
extern "C" fn release(this: Id, _cmd: Sel) {
    runtime::release(this)
}
extern "C" fn autorelease(this: Id, _cmd: Sel) -> Id {
    runtime::autorelease(this)
}
extern "C" fn retain_count(this: Id, _cmd: Sel) -> usize {
    runtime::retain_count(this).max(1) as usize
}
extern "C" fn this_class(this: Id, _cmd: Sel) -> Id {
    // SAFETY: an instance's first word is its class.
    unsafe { runtime::object_class(this) as Id }
}
extern "C" fn class_class(cls: Id, _cmd: Sel) -> Id {
    cls
}
extern "C" fn this_self(this: Id, _cmd: Sel) -> Id {
    this
}
extern "C" fn superclass(this: Id, _cmd: Sel) -> Id {
    // SAFETY: an instance's first word is its class.
    let c = unsafe { runtime::object_class(this) };
    runtime::class_superclass(c) as Id
}
extern "C" fn class_superclass(cls: Id, _cmd: Sel) -> Id {
    runtime::class_superclass(cls as Class) as Id
}
fn kind_of(mut c: Class, target: Class) -> bool {
    while !c.is_null() {
        if c == target {
            return true;
        }
        c = runtime::class_superclass(c);
    }
    false
}
extern "C" fn is_kind_of_class(this: Id, _cmd: Sel, cls: Id) -> bool {
    // SAFETY: an instance's first word is its class.
    kind_of(unsafe { runtime::object_class(this) }, cls as Class)
}
extern "C" fn is_member_of_class(this: Id, _cmd: Sel, cls: Id) -> bool {
    // SAFETY: an instance's first word is its class.
    unsafe { runtime::object_class(this) == cls as Class }
}
extern "C" fn responds_to_selector(this: Id, _cmd: Sel, sel: Sel) -> bool {
    // SAFETY: an instance's first word is its class.
    runtime::class_get_imp(unsafe { runtime::object_class(this) }, sel).is_some()
}
extern "C" fn instances_respond_to_selector(cls: Id, _cmd: Sel, sel: Sel) -> bool {
    runtime::class_get_imp(cls as Class, sel).is_some()
}
extern "C" fn is_equal(this: Id, _cmd: Sel, other: Id) -> bool {
    this == other
}
extern "C" fn hash(this: Id, _cmd: Sel) -> usize {
    this as usize
}
extern "C" fn perform0(this: Id, _cmd: Sel, sel: Sel) -> Id {
    // SAFETY: this is a non-nil object.
    let imp = unsafe { runtime::resolve(this, sel) };
    // SAFETY: a zero-argument implementation.
    let f: extern "C" fn(Id, Sel) -> Id = unsafe { core::mem::transmute(imp) };
    f(this, sel)
}
extern "C" fn perform1(this: Id, _cmd: Sel, sel: Sel, a: Id) -> Id {
    // SAFETY: this is a non-nil object.
    let imp = unsafe { runtime::resolve(this, sel) };
    // SAFETY: a one-argument implementation.
    let f: extern "C" fn(Id, Sel, Id) -> Id = unsafe { core::mem::transmute(imp) };
    f(this, sel, a)
}
extern "C" fn perform2(this: Id, _cmd: Sel, sel: Sel, a: Id, b: Id) -> Id {
    // SAFETY: this is a non-nil object.
    let imp = unsafe { runtime::resolve(this, sel) };
    // SAFETY: a two-argument implementation.
    let f: extern "C" fn(Id, Sel, Id, Id) -> Id = unsafe { core::mem::transmute(imp) };
    f(this, sel, a, b)
}
extern "C" fn noop(_this: Id, _cmd: Sel) {}
extern "C" fn is_proxy(_this: Id, _cmd: Sel) -> bool {
    false
}

fn imp<T>(f: T) -> Imp
where
    T: Copy,
{
    debug_assert_eq!(core::mem::size_of::<T>(), core::mem::size_of::<usize>());
    // SAFETY: T is a function pointer, the same size as usize.
    unsafe { core::mem::transmute_copy::<T, usize>(&f) }
}

/// Registers `NSObject` if it is not registered yet and returns it.
pub fn install() -> Class {
    let existing = runtime::class_named("NSObject");
    if !existing.is_null() {
        return existing;
    }
    let instance: [NativeMethod; 18] = [
        ("init", imp(init as extern "C" fn(Id, Sel) -> Id)),
        ("dealloc", imp(dealloc as extern "C" fn(Id, Sel))),
        ("retain", imp(retain as extern "C" fn(Id, Sel) -> Id)),
        ("release", imp(release as extern "C" fn(Id, Sel))),
        ("autorelease", imp(autorelease as extern "C" fn(Id, Sel) -> Id)),
        ("retainCount", imp(retain_count as extern "C" fn(Id, Sel) -> usize)),
        ("class", imp(this_class as extern "C" fn(Id, Sel) -> Id)),
        ("self", imp(this_self as extern "C" fn(Id, Sel) -> Id)),
        ("superclass", imp(superclass as extern "C" fn(Id, Sel) -> Id)),
        ("isKindOfClass:", imp(is_kind_of_class as extern "C" fn(Id, Sel, Id) -> bool)),
        ("isMemberOfClass:", imp(is_member_of_class as extern "C" fn(Id, Sel, Id) -> bool)),
        ("respondsToSelector:", imp(responds_to_selector as extern "C" fn(Id, Sel, Sel) -> bool)),
        ("isEqual:", imp(is_equal as extern "C" fn(Id, Sel, Id) -> bool)),
        ("hash", imp(hash as extern "C" fn(Id, Sel) -> usize)),
        ("performSelector:", imp(perform0 as extern "C" fn(Id, Sel, Sel) -> Id)),
        ("performSelector:withObject:", imp(perform1 as extern "C" fn(Id, Sel, Sel, Id) -> Id)),
        ("performSelector:withObject:withObject:", imp(perform2 as extern "C" fn(Id, Sel, Sel, Id, Id) -> Id)),
        ("isProxy", imp(is_proxy as extern "C" fn(Id, Sel) -> bool)),
    ];
    let class: [NativeMethod; 8] = [
        ("alloc", imp(alloc as extern "C" fn(Id, Sel) -> Id)),
        ("allocWithZone:", imp(alloc as extern "C" fn(Id, Sel) -> Id)),
        ("new", imp(new as extern "C" fn(Id, Sel) -> Id)),
        ("class", imp(class_class as extern "C" fn(Id, Sel) -> Id)),
        ("superclass", imp(class_superclass as extern "C" fn(Id, Sel) -> Id)),
        ("instancesRespondToSelector:", imp(instances_respond_to_selector as extern "C" fn(Id, Sel, Sel) -> bool)),
        ("initialize", imp(noop as extern "C" fn(Id, Sel))),
        ("load", imp(noop as extern "C" fn(Id, Sel))),
    ];
    runtime::register_native_class("NSObject", core::ptr::null_mut(), 8, &instance, &class)
}
