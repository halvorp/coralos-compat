//! The in-memory layouts of the Apple Objective-C 2 ("modern") runtime metadata on x86_64, as the compiler emits them into
//! `__objc_classlist`, `__objc_const`, `__objc_data` and friends. Layouts are the published ABI; every reader checks nothing
//! itself: the caller guarantees the pointers lie inside a mapped, rebased image.
use core::ffi::c_void;

/// `class_t`: the object a class pointer points at. A metaclass has the same shape.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassT {
    /// The metaclass (for a class) or the root metaclass (for a metaclass).
    pub isa: *mut ClassT,
    pub superclass: *mut ClassT,
    pub cache: *mut c_void,
    pub vtable: *mut c_void,
    /// `class_ro_t *` in the low bits-masked form (`& !7`; bit 0 and 1 mark Swift classes).
    pub bits: usize,
}

/// `class_ro_t`: the compile-time half of a class.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClassRo {
    pub flags: u32,
    pub instance_start: u32,
    pub instance_size: u32,
    pub reserved: u32,
    pub ivar_layout: *const u8,
    pub name: *const u8,
    pub base_methods: *mut MethodList,
    pub base_protocols: *const c_void,
    pub ivars: *mut IvarList,
    pub weak_ivar_layout: *const u8,
    pub base_properties: *const c_void,
}

/// `RO_META`: the class is a metaclass.
pub const RO_META: u32 = 1;
/// `RO_ROOT`: the class has no superclass.
pub const RO_ROOT: u32 = 2;

/// `method_t` in its absolute (not relative) form.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Method {
    pub name: *const u8,
    pub types: *const u8,
    pub imp: usize,
}

/// `method_list_t`: a header followed by `count` entries of `entsize` bytes.
#[repr(C)]
pub struct MethodList {
    /// Entry size in the low 16 bits; bit 31 marks relative ("small") methods, bit 30 a sorted list.
    pub entsize_and_flags: u32,
    pub count: u32,
}

/// The flag that marks method lists in the relative ("small") form.
pub const METHOD_LIST_SMALL: u32 = 0x8000_0000;

/// `ivar_t`.
#[repr(C)]
pub struct Ivar {
    /// A pointer to the `u32` offset variable (`_OBJC_IVAR_$_Class.name`), which the runtime rewrites when the superclass differs.
    pub offset: *mut u32,
    pub name: *const u8,
    pub type_: *const u8,
    pub alignment: u32,
    pub size: u32,
}

/// `ivar_list_t`: a header followed by `count` entries of `entsize` bytes.
#[repr(C)]
pub struct IvarList {
    pub entsize: u32,
    pub count: u32,
}

/// `category_t`.
#[repr(C)]
pub struct Category {
    pub name: *const u8,
    pub cls: *mut ClassT,
    pub instance_methods: *mut MethodList,
    pub class_methods: *mut MethodList,
    pub protocols: *const c_void,
    pub instance_properties: *const c_void,
}

/// The nth entry of a method list, or None past the end and for the relative ("small") form that only newer system images carry;
/// realization refuses such a list by name.
///
/// # Safety
/// `list` must be null or point at a readable list with `count` entries of `entsize` bytes.
pub unsafe fn method_at(list: *mut MethodList, i: usize) -> Option<*mut Method> {
    if list.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the list is readable.
    let (entsize, count) = unsafe { ((*list).entsize_and_flags, (*list).count) };
    if entsize & METHOD_LIST_SMALL != 0 || i >= count as usize {
        return None;
    }
    let stride = (entsize & 0xffff) as usize;
    // SAFETY: entries follow the 8-byte header.
    Some(unsafe { (list as *mut u8).add(8 + i * stride) } as *mut Method)
}

/// The nth ivar of a list.
///
/// # Safety
/// `list` must be null or point at a readable list.
pub unsafe fn ivar_at(list: *mut IvarList, i: usize) -> Option<*mut Ivar> {
    if list.is_null() {
        return None;
    }
    // SAFETY: the caller guarantees the list is readable.
    let (entsize, count) = unsafe { ((*list).entsize, (*list).count) };
    if i >= count as usize {
        return None;
    }
    // SAFETY: entries follow the 8-byte header.
    Some(unsafe { (list as *mut u8).add(8 + i * entsize as usize) } as *mut Ivar)
}

/// The number of methods in a list (0 for null).
///
/// # Safety
/// `list` must be null or readable.
pub unsafe fn method_count(list: *mut MethodList) -> usize {
    if list.is_null() {
        0
    } else {
        // SAFETY: the caller guarantees readability.
        unsafe { (*list).count as usize }
    }
}

/// The bytes of a NUL-terminated C string as a `&str` (lossy replaced by an empty string on invalid UTF-8).
///
/// # Safety
/// `p` must be null or point at a NUL-terminated string.
pub unsafe fn cstr<'a>(p: *const u8) -> &'a str {
    if p.is_null() {
        return "";
    }
    let mut n = 0;
    // SAFETY: the caller guarantees a terminating NUL.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: n bytes were just read.
    core::str::from_utf8(unsafe { core::slice::from_raw_parts(p, n) }).unwrap_or("")
}
