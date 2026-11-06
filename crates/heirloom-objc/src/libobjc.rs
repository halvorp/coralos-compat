//! The `libobjc.A.dylib` surface a Mach-O app binds: the message-send trampolines, reference counting and ARC entry points,
//! autorelease pools, weak references, synthesized-property accessors, class and selector lookups, and the data symbols
//! (`_OBJC_CLASS_$_NSObject`, `__objc_empty_cache`). The exception entry points refuse by name (`OBJC-EXCEPTIONS-UNSUPPORTED`).
use crate::msg;
use crate::runtime::{self, Class, Id, Sel};
use core::ffi::c_void;

extern "C" fn objc_retain(obj: Id) -> Id {
    runtime::retain(obj)
}
extern "C" fn objc_release(obj: Id) {
    runtime::release(obj)
}
extern "C" fn objc_autorelease(obj: Id) -> Id {
    runtime::autorelease(obj)
}
extern "C" fn objc_retain_autorelease(obj: Id) -> Id {
    runtime::autorelease(runtime::retain(obj))
}
extern "C" fn objc_pass_through(obj: Id) -> Id {
    obj
}
extern "C" fn objc_store_strong(loc: *mut Id, value: Id) {
    // SAFETY: the compiler passes the address of a strong variable.
    unsafe {
        let old = *loc;
        runtime::retain(value);
        *loc = value;
        runtime::release(old);
    }
}
extern "C" fn objc_store_weak(loc: *mut usize, obj: Id) -> Id {
    // SAFETY: the compiler passes the address of a weak variable.
    unsafe { runtime::weak_store(loc, obj) }
}
extern "C" fn objc_init_weak(loc: *mut usize, obj: Id) -> Id {
    // SAFETY: the compiler passes the address of an uninitialized weak variable.
    unsafe {
        *loc = 0;
        runtime::weak_store(loc, obj)
    }
}
extern "C" fn objc_load_weak_retained(loc: *const usize) -> Id {
    // SAFETY: the compiler passes the address of a weak variable.
    unsafe { runtime::weak_load_retained(loc) }
}
extern "C" fn objc_destroy_weak(loc: *mut usize) {
    // SAFETY: the compiler passes the address of a weak variable.
    unsafe { runtime::weak_destroy(loc) }
}
extern "C" fn objc_copy_weak(dst: *mut usize, src: *const usize) {
    // SAFETY: both are weak variables.
    unsafe {
        let obj = *src as Id;
        *dst = 0;
        runtime::weak_store(dst, obj);
    }
}
extern "C" fn objc_pool_push() -> *mut c_void {
    (runtime::pool_push() + 1) as *mut c_void
}
extern "C" fn objc_pool_pop(token: *mut c_void) {
    runtime::pool_pop((token as usize).saturating_sub(1));
}
extern "C" fn objc_alloc(cls: Id) -> Id {
    let sel = runtime::sel_register("alloc");
    // SAFETY: cls is a class object.
    let imp = unsafe { runtime::resolve(cls, sel) };
    // SAFETY: +alloc is an `extern "C" fn(Id, Sel) -> Id`.
    let f: extern "C" fn(Id, Sel) -> Id = unsafe { core::mem::transmute(imp) };
    f(cls, sel)
}
extern "C" fn objc_get_property(this: Id, _cmd: Sel, offset: isize, _atomic: bool) -> Id {
    // SAFETY: the compiler passes the byte offset of an object-pointer ivar.
    let value = unsafe { *((this as *mut u8).offset(offset) as *const Id) };
    runtime::autorelease(runtime::retain(value))
}
extern "C" fn objc_set_property_generic(this: Id, _cmd: Sel, offset: isize, value: Id, _atomic: bool, _copy: i8) {
    objc_set_property(this, _cmd, value, offset)
}
extern "C" fn objc_set_property(this: Id, _cmd: Sel, value: Id, offset: isize) {
    // SAFETY: the compiler passes the byte offset of an object-pointer ivar.
    unsafe {
        let slot = (this as *mut u8).offset(offset) as *mut Id;
        let old = *slot;
        runtime::retain(value);
        *slot = value;
        runtime::release(old);
    }
}
extern "C" fn objc_copy_struct(dst: *mut u8, src: *const u8, size: usize, _atomic: bool, _has_strong: bool) {
    // SAFETY: the compiler passes two non-overlapping buffers of `size` bytes.
    unsafe { core::ptr::copy_nonoverlapping(src, dst, size) }
}
extern "C" fn objc_sync(_obj: Id) -> i32 {
    0
}
extern "C" fn sel_get_name(sel: Sel) -> *const u8 {
    sel
}
extern "C" fn sel_register_name(name: *const u8) -> Sel {
    // SAFETY: the caller passes a NUL-terminated name.
    unsafe { runtime::sel_register_cstr(name) }
}
extern "C" fn objc_get_class(name: *const u8) -> Id {
    // SAFETY: the caller passes a NUL-terminated name.
    runtime::class_named(unsafe { crate::abi::cstr(name) }) as Id
}
extern "C" fn object_get_class(obj: Id) -> Id {
    if obj.is_null() {
        return obj;
    }
    // SAFETY: a non-nil object's first word is its class.
    unsafe { runtime::object_class(obj) as Id }
}
extern "C" fn class_get_name(cls: Id) -> *const u8 {
    let name = runtime::class_name(cls as Class);
    sel_register_name_owned(&name)
}
fn sel_register_name_owned(s: &str) -> *const u8 {
    runtime::sel_register(s)
}
extern "C" fn enumeration_mutation(_obj: Id) {
    runtime::fatal("OBJC-ENUMERATION-MUTATION a collection was mutated while enumerated")
}
extern "C" fn exceptions_unsupported() {
    runtime::fatal("OBJC-EXCEPTIONS-UNSUPPORTED Objective-C exceptions need the unwinder, which this runtime does not have yet")
}

static EMPTY_CACHE: [usize; 4] = [0; 4];

/// The address a `libobjc.A.dylib` symbol binds to, or None when this runtime does not serve it. Names carry the Mach-O
/// leading underscore.
pub fn symbol(name: &str) -> Option<usize> {
    fn f<T: Copy>(x: T) -> usize {
        debug_assert_eq!(core::mem::size_of::<T>(), core::mem::size_of::<usize>());
        // SAFETY: T is a function pointer.
        unsafe { core::mem::transmute_copy::<T, usize>(&x) }
    }
    if let Some(rest) = name.strip_prefix("_OBJC_CLASS_$_") {
        let c = runtime::class_named(rest);
        return (!c.is_null()).then_some(c as usize);
    }
    if let Some(rest) = name.strip_prefix("_OBJC_METACLASS_$_") {
        let c = runtime::class_named(rest);
        // SAFETY: a registered class is a valid ClassT whose isa is its metaclass.
        return (!c.is_null()).then(|| unsafe { (*c).isa as usize });
    }
    Some(match name {
        "_objc_msgSend" | "_objc_msgSend_fpret" => msg::osxobjc_msgsend as *const () as usize,
        "_objc_msgSend_stret" => msg::osxobjc_msgsend_stret as *const () as usize,
        "_objc_msgSendSuper2" => msg::osxobjc_msgsend_super2 as *const () as usize,
        "_objc_msgSendSuper2_stret" => msg::osxobjc_msgsend_super2_stret as *const () as usize,
        "__objc_empty_cache" => EMPTY_CACHE.as_ptr() as usize,
        "_objc_retain" | "_objc_retainBlock" => f(objc_retain as extern "C" fn(Id) -> Id),
        "_objc_release" => f(objc_release as extern "C" fn(Id)),
        "_objc_autorelease" | "_objc_autoreleaseReturnValue" => f(objc_autorelease as extern "C" fn(Id) -> Id),
        "_objc_retainAutorelease" | "_objc_retainAutoreleaseReturnValue" => f(objc_retain_autorelease as extern "C" fn(Id) -> Id),
        "_objc_retainAutoreleasedReturnValue" => f(objc_retain as extern "C" fn(Id) -> Id),
        "_objc_unsafeClaimAutoreleasedReturnValue" => f(objc_pass_through as extern "C" fn(Id) -> Id),
        "_objc_storeStrong" => f(objc_store_strong as extern "C" fn(*mut Id, Id)),
        "_objc_storeWeak" => f(objc_store_weak as extern "C" fn(*mut usize, Id) -> Id),
        "_objc_initWeak" => f(objc_init_weak as extern "C" fn(*mut usize, Id) -> Id),
        "_objc_loadWeakRetained" => f(objc_load_weak_retained as extern "C" fn(*const usize) -> Id),
        "_objc_destroyWeak" => f(objc_destroy_weak as extern "C" fn(*mut usize)),
        "_objc_copyWeak" => f(objc_copy_weak as extern "C" fn(*mut usize, *const usize)),
        "_objc_autoreleasePoolPush" => f(objc_pool_push as extern "C" fn() -> *mut c_void),
        "_objc_autoreleasePoolPop" => f(objc_pool_pop as extern "C" fn(*mut c_void)),
        "_objc_alloc" | "_objc_allocWithZone" => f(objc_alloc as extern "C" fn(Id) -> Id),
        "_objc_getProperty" => f(objc_get_property as extern "C" fn(Id, Sel, isize, bool) -> Id),
        "_objc_setProperty" => f(objc_set_property_generic as extern "C" fn(Id, Sel, isize, Id, bool, i8)),
        "_objc_setProperty_atomic" | "_objc_setProperty_nonatomic" | "_objc_setProperty_atomic_copy" | "_objc_setProperty_nonatomic_copy" => {
            f(objc_set_property as extern "C" fn(Id, Sel, Id, isize))
        }
        "_objc_copyStruct" => f(objc_copy_struct as extern "C" fn(*mut u8, *const u8, usize, bool, bool)),
        "_objc_enumerationMutation" => f(enumeration_mutation as extern "C" fn(Id)),
        "_objc_sync_enter" | "_objc_sync_exit" => f(objc_sync as extern "C" fn(Id) -> i32),
        "_sel_getName" => f(sel_get_name as extern "C" fn(Sel) -> *const u8),
        "_sel_registerName" | "_sel_getUid" => f(sel_register_name as extern "C" fn(*const u8) -> Sel),
        "_objc_getClass" | "_objc_lookUpClass" => f(objc_get_class as extern "C" fn(*const u8) -> Id),
        "_object_getClass" => f(object_get_class as extern "C" fn(Id) -> Id),
        "_class_getName" => f(class_get_name as extern "C" fn(Id) -> *const u8),
        "_objc_begin_catch" | "_objc_end_catch" | "_objc_exception_throw" | "_objc_exception_rethrow" | "_objc_terminate" | "___objc_personality_v0" => {
            f(exceptions_unsupported as extern "C" fn())
        }
        _ => return None,
    })
}
