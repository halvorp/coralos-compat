//! `NSString` and the constant-string class compiled `@"..."` literals point at. A string is `{ isa, flags, chars, length }`, the
//! layout the compiler emits for a constant string, so literals and strings made at run time answer the same methods.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_objc::runtime::{self, Class, Id, Imp, NativeMethod, Sel};

/// The object layout shared by constant and run-time strings.
#[repr(C)]
pub struct StringObj {
    pub isa: Class,
    pub flags: u64,
    pub chars: *const u8,
    /// Length in bytes (UTF-8); the NUL after the last byte is not counted.
    pub len: u64,
}

/// `flags` of a string whose buffer this crate allocated (and frees at `dealloc`).
const OWNED: u64 = 1;

pub(crate) fn imp<T: Copy>(f: T) -> Imp {
    debug_assert_eq!(core::mem::size_of::<T>(), core::mem::size_of::<usize>());
    // SAFETY: T is a function pointer.
    unsafe { core::mem::transmute_copy::<T, usize>(&f) }
}

/// The text of a string object; an empty string for nil.
pub fn text<'a>(s: Id) -> &'a str {
    if s.is_null() {
        return "";
    }
    // SAFETY: a non-nil argument of a string parameter is a StringObj whose buffer holds `len` bytes.
    unsafe {
        let o = &*(s as *const StringObj);
        core::str::from_utf8(core::slice::from_raw_parts(o.chars, o.len as usize)).unwrap_or("")
    }
}

/// A new autoreleased string with a copy of `s`.
pub fn make(s: &str) -> Id {
    let obj = make_retained(s);
    runtime::autorelease(obj)
}

/// A new string with a retain count of one.
pub fn make_retained(s: &str) -> Id {
    let cls = runtime::class_named("NSString");
    let obj = runtime::alloc_instance(cls, 0);
    fill(obj, s);
    obj
}

fn fill(obj: Id, s: &str) {
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len() + 1);
    bytes.extend_from_slice(s.as_bytes());
    bytes.push(0);
    let leaked = Box::leak(bytes.into_boxed_slice());
    // SAFETY: obj is a fresh StringObj instance.
    unsafe {
        let o = &mut *(obj as *mut StringObj);
        o.flags = OWNED;
        o.chars = leaked.as_ptr();
        o.len = s.len() as u64;
    }
}

extern "C" fn string_with_utf8(cls: Id, _cmd: Sel, cstr: *const u8) -> Id {
    // SAFETY: the caller passes a NUL-terminated string.
    let s = unsafe { heirloom_objc::abi::cstr(cstr) };
    let obj = runtime::alloc_instance(cls as Class, 0);
    fill(obj, s);
    runtime::autorelease(obj)
}
extern "C" fn utf8_string(this: Id, _cmd: Sel) -> *const u8 {
    // SAFETY: this is a StringObj.
    unsafe { (*(this as *const StringObj)).chars }
}
extern "C" fn length(this: Id, _cmd: Sel) -> usize {
    text(this).encode_utf16().count()
}
extern "C" fn is_equal_to_string(this: Id, _cmd: Sel, other: Id) -> bool {
    !other.is_null() && text(this) == text(other)
}
extern "C" fn this_self(this: Id, _cmd: Sel) -> Id {
    this
}
extern "C" fn dealloc(this: Id, _cmd: Sel) {
    // SAFETY: this is a StringObj; an OWNED buffer was leaked from a boxed slice of len + 1 bytes.
    unsafe {
        let o = &mut *(this as *mut StringObj);
        if o.flags & OWNED != 0 && !o.chars.is_null() {
            drop(Box::from_raw(core::slice::from_raw_parts_mut(o.chars as *mut u8, o.len as usize + 1)));
        }
    }
    runtime::free_instance(this)
}

/// Registers `NSString` and `NSConstantString`.
pub(crate) fn install() {
    if !runtime::class_named("NSString").is_null() {
        return;
    }
    let ns = runtime::class_named("NSObject");
    let instance: [NativeMethod; 5] = [
        ("UTF8String", imp(utf8_string as extern "C" fn(Id, Sel) -> *const u8)),
        ("length", imp(length as extern "C" fn(Id, Sel) -> usize)),
        ("isEqualToString:", imp(is_equal_to_string as extern "C" fn(Id, Sel, Id) -> bool)),
        ("description", imp(this_self as extern "C" fn(Id, Sel) -> Id)),
        ("dealloc", imp(dealloc as extern "C" fn(Id, Sel))),
    ];
    let class: [NativeMethod; 1] = [("stringWithUTF8String:", imp(string_with_utf8 as extern "C" fn(Id, Sel, *const u8) -> Id))];
    let string = runtime::register_native_class("NSString", ns, core::mem::size_of::<StringObj>() as u32, &instance, &class);
    runtime::register_native_class("NSConstantString", string, core::mem::size_of::<StringObj>() as u32, &[], &[]);
}

/// Hands the text of a string back as an owned Rust string (for diagnostics).
pub fn to_owned(s: Id) -> String {
    String::from(text(s))
}
