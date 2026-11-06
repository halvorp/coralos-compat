//! Runtime proofs on hand-built Objective-C 2 metadata: classes laid out exactly as the compiler emits them (`class_t`,
//! `class_ro_t`, method and ivar lists), registered through the same path an image uses, then messaged through the real
//! assembly trampolines.
extern crate std;

use crate::abi::{ClassRo, ClassT, Ivar, IvarList, Method, MethodList, RO_META, RO_ROOT};
use crate::runtime::{self, Class, Id, ImageMeta, Imp, Sel};
use crate::{msgsend_address, nsobject};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::vec::Vec;

static LOCK: Mutex<()> = Mutex::new(());

fn fresh() -> std::sync::MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    runtime::reset();
    nsobject::install();
    g
}

fn leak_cstr(s: &str) -> *const u8 {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    std::boxed::Box::leak(v.into_boxed_slice()).as_ptr()
}

fn leak_methods(methods: &[(&str, Imp)]) -> *mut MethodList {
    if methods.is_empty() {
        return core::ptr::null_mut();
    }
    let n = methods.len();
    let mut words: Vec<u64> = std::vec![0u64; 1 + 3 * n];
    let p = std::boxed::Box::leak(words.split_off(0).into_boxed_slice()).as_mut_ptr();
    let list = p as *mut MethodList;
    // SAFETY: the leaked buffer holds an 8-byte header plus n 24-byte entries.
    unsafe {
        (*list).entsize_and_flags = 24;
        (*list).count = n as u32;
        for (i, (name, imp)) in methods.iter().enumerate() {
            let m = (p as *mut u8).add(8 + i * 24) as *mut Method;
            // a distinct copy of the name: the runtime must canonicalize it
            (*m).name = leak_cstr(name);
            (*m).types = leak_cstr("v16@0:8");
            (*m).imp = *imp;
        }
    }
    list
}

struct Built {
    class: Class,
    /// The ivar offset variables, in declaration order.
    offsets: Vec<*mut u32>,
}

/// Lays out a class the way the compiler does. `super_class` is null for a root class; `ivars` are `(offset the compiler assumed,
/// log2 alignment)`.
fn build_class(name: &str, super_class: Class, start: u32, size: u32, methods: &[(&str, Imp)], class_methods: &[(&str, Imp)], ivars: &[(u32, u32)]) -> Built {
    let nsobject = runtime::class_named("NSObject");
    // SAFETY: NSObject is registered by `fresh`.
    let root_meta = unsafe { (*nsobject).isa };
    let meta_ro = std::boxed::Box::leak(std::boxed::Box::new(ClassRo {
        flags: RO_META | if super_class.is_null() { RO_ROOT } else { 0 },
        instance_start: 40,
        instance_size: 40,
        reserved: 0,
        ivar_layout: core::ptr::null(),
        name: leak_cstr(name),
        base_methods: leak_methods(class_methods),
        base_protocols: core::ptr::null(),
        ivars: core::ptr::null_mut(),
        weak_ivar_layout: core::ptr::null(),
        base_properties: core::ptr::null(),
    }));
    let mut offsets = Vec::new();
    let ivar_list = if ivars.is_empty() {
        core::ptr::null_mut()
    } else {
        let n = ivars.len();
        let words = std::vec![0u64; 1 + 4 * n]; // header 8 + n * 32
        let p = std::boxed::Box::leak(words.into_boxed_slice()).as_mut_ptr();
        let list = p as *mut IvarList;
        // SAFETY: the buffer holds the header and n 32-byte entries.
        unsafe {
            (*list).entsize = 32;
            (*list).count = n as u32;
            for (i, (off, align)) in ivars.iter().enumerate() {
                let var = std::boxed::Box::leak(std::boxed::Box::new(*off)) as *mut u32;
                offsets.push(var);
                let iv = (p as *mut u8).add(8 + i * 32) as *mut Ivar;
                (*iv).offset = var;
                (*iv).name = leak_cstr("x");
                (*iv).type_ = leak_cstr("q");
                (*iv).alignment = *align;
                (*iv).size = 8;
            }
        }
        list
    };
    let ro = std::boxed::Box::leak(std::boxed::Box::new(ClassRo {
        flags: if super_class.is_null() { RO_ROOT } else { 0 },
        instance_start: start,
        instance_size: size,
        reserved: 0,
        ivar_layout: core::ptr::null(),
        name: leak_cstr(name),
        base_methods: leak_methods(methods),
        base_protocols: core::ptr::null(),
        ivars: ivar_list,
        weak_ivar_layout: core::ptr::null(),
        base_properties: core::ptr::null(),
    }));
    let meta = std::boxed::Box::leak(std::boxed::Box::new(ClassT {
        isa: root_meta,
        superclass: if super_class.is_null() { core::ptr::null_mut() } else { unsafe { (*super_class).isa } },
        cache: core::ptr::null_mut(),
        vtable: core::ptr::null_mut(),
        bits: meta_ro as *mut ClassRo as usize,
    })) as *mut ClassT;
    let class = std::boxed::Box::leak(std::boxed::Box::new(ClassT {
        isa: meta,
        superclass: super_class,
        cache: core::ptr::null_mut(),
        vtable: core::ptr::null_mut(),
        bits: ro as *mut ClassRo as usize,
    })) as *mut ClassT;
    Built { class, offsets }
}

fn register(classes: &[&Built]) {
    let list: Vec<usize> = classes.iter().map(|b| b.class as usize).collect();
    let leaked = std::boxed::Box::leak(list.into_boxed_slice());
    let meta = ImageMeta { classlist: (leaked.as_ptr() as usize, leaked.len()), ..Default::default() };
    // SAFETY: the metadata was built by build_class and is writable.
    unsafe { runtime::register_image(&meta) };
}

fn imp<T: Copy>(f: T) -> Imp {
    // SAFETY: T is a function pointer.
    unsafe { core::mem::transmute_copy::<T, usize>(&f) }
}

fn send0(obj: Id, name: &str) -> Id {
    let f: extern "C" fn(Id, Sel) -> Id = unsafe { core::mem::transmute(msgsend_address()) };
    f(obj, runtime::sel_register(name))
}

fn sel(name: &str) -> Sel {
    runtime::sel_register(name)
}

// ---- NSObject and the trampolines ------------------------------------------------------------------------------------------

#[test]
fn nsobject_new_retain_release_dealloc() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject") as Id;
    let obj = send0(ns, "new");
    assert!(runtime::is_live(obj));
    assert_eq!(runtime::retain_count(obj), 1);
    send0(obj, "retain");
    assert_eq!(runtime::retain_count(obj), 2);
    send0(obj, "release");
    assert_eq!(runtime::retain_count(obj), 1);
    send0(obj, "release");
    assert!(!runtime::is_live(obj), "the last release deallocates");
}

#[test]
fn nil_receiver_returns_zero_without_calling() {
    let _g = fresh();
    let f: extern "C" fn(Id, Sel) -> u64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(f(core::ptr::null_mut(), sel("anything")), 0);
    let g: extern "C" fn(Id, Sel) -> f64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(g(core::ptr::null_mut(), sel("anything")), 0.0);
}

static ANSWERS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn foo_answer(_this: Id, _cmd: Sel) -> i64 {
    ANSWERS.fetch_add(1, Ordering::SeqCst);
    42
}

extern "C" fn foo_make(cls: Id, _cmd: Sel) -> Id {
    send0(send0(cls, "alloc"), "init")
}

#[test]
fn image_class_instance_and_class_methods() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("answer", imp(foo_answer as extern "C" fn(Id, Sel) -> i64))], &[("make", imp(foo_make as extern "C" fn(Id, Sel) -> Id))], &[]);
    register(&[&foo]);
    assert_eq!(runtime::class_named("Foo"), foo.class);
    let obj = send0(foo.class as Id, "make");
    assert!(runtime::is_live(obj));
    // SAFETY: obj is a Foo instance.
    assert_eq!(unsafe { runtime::object_class(obj) }, foo.class);
    let answer: extern "C" fn(Id, Sel) -> i64 = unsafe { core::mem::transmute(msgsend_address()) };
    let before = ANSWERS.load(Ordering::SeqCst);
    assert_eq!(answer(obj, sel("answer")), 42);
    assert_eq!(ANSWERS.load(Ordering::SeqCst), before + 1);
    // inherited from NSObject through the chain
    let kind: extern "C" fn(Id, Sel, Id) -> bool = unsafe { core::mem::transmute(msgsend_address()) };
    assert!(kind(obj, sel("isKindOfClass:"), foo.class as Id));
    assert!(kind(obj, sel("isKindOfClass:"), ns as Id));
    let member: extern "C" fn(Id, Sel, Id) -> bool = unsafe { core::mem::transmute(msgsend_address()) };
    assert!(!member(obj, sel("isMemberOfClass:"), ns as Id));
    let responds: extern "C" fn(Id, Sel, Sel) -> bool = unsafe { core::mem::transmute(msgsend_address()) };
    assert!(responds(obj, sel("respondsToSelector:"), sel("answer")));
    assert!(!responds(obj, sel("respondsToSelector:"), sel("nope")));
}

#[test]
fn method_names_in_the_image_are_made_unique_selectors() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("answer", imp(foo_answer as extern "C" fn(Id, Sel) -> i64))], &[], &[]);
    // SAFETY: the method list was built by leak_methods.
    let name_before = unsafe { (*crate::abi::method_at((*((*foo.class).bits as *const ClassRo)).base_methods, 0).unwrap()).name };
    register(&[&foo]);
    // SAFETY: as above.
    let name_after = unsafe { (*crate::abi::method_at((*((*foo.class).bits as *const ClassRo)).base_methods, 0).unwrap()).name };
    assert_ne!(name_before, name_after, "the compiler's private copy was replaced");
    assert_eq!(name_after, sel("answer"), "by the unique selector");
}

#[test]
fn selector_references_are_rewritten_to_unique_selectors() {
    let _g = fresh();
    let private = leak_cstr("someSelector:");
    let slots = std::boxed::Box::leak(std::boxed::Box::new([private as usize]));
    let meta = ImageMeta { selrefs: (slots.as_ptr() as usize, 1), ..Default::default() };
    // SAFETY: the slot is writable and holds a string pointer.
    unsafe { runtime::register_image(&meta) };
    assert_eq!(slots[0], sel("someSelector:") as usize);
    assert_ne!(slots[0], private as usize);
}

// ---- super, categories, +initialize ---------------------------------------------------------------------------------------

#[repr(C)]
struct ObjcSuper {
    receiver: Id,
    class: Class,
}

static BAR: AtomicUsize = AtomicUsize::new(0);

extern "C" fn bar_answer(this: Id, cmd: Sel) -> i64 {
    let sup = ObjcSuper { receiver: this, class: BAR.load(Ordering::SeqCst) as Class };
    let f: extern "C" fn(*const ObjcSuper, Sel) -> i64 = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_super2 as *const () as usize) };
    f(&sup, cmd) + 100
}

#[test]
fn super_dispatch_starts_above_the_current_class() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("answer", imp(foo_answer as extern "C" fn(Id, Sel) -> i64))], &[], &[]);
    let bar = build_class("Bar", foo.class, 8, 8, &[("answer", imp(bar_answer as extern "C" fn(Id, Sel) -> i64))], &[], &[]);
    BAR.store(bar.class as usize, Ordering::SeqCst);
    register(&[&bar, &foo]); // subclass first: realization must order superclasses
    let obj = send0(send0(bar.class as Id, "alloc"), "init");
    let answer: extern "C" fn(Id, Sel) -> i64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(answer(obj, sel("answer")), 142, "Bar adds 100 to what Foo answers");
    // a Foo instance still answers 42
    let foo_obj = send0(send0(foo.class as Id, "alloc"), "init");
    assert_eq!(answer(foo_obj, sel("answer")), 42);
}

extern "C" fn foo_identity(this: Id, _cmd: Sel) -> Id {
    this
}

extern "C" fn bar_identity(this: Id, cmd: Sel) -> Id {
    let sup = ObjcSuper { receiver: this, class: BAR.load(Ordering::SeqCst) as Class };
    let f: extern "C" fn(*const ObjcSuper, Sel) -> Id = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_super2 as *const () as usize) };
    f(&sup, cmd)
}

#[test]
fn super_dispatch_passes_the_original_receiver_not_the_objc_super_struct() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("identity", imp(foo_identity as extern "C" fn(Id, Sel) -> Id))], &[], &[]);
    let bar = build_class("Bar", foo.class, 8, 8, &[("identity", imp(bar_identity as extern "C" fn(Id, Sel) -> Id))], &[], &[]);
    BAR.store(bar.class as usize, Ordering::SeqCst);
    register(&[&foo, &bar]);
    let obj = send0(send0(bar.class as Id, "alloc"), "init");
    assert_eq!(send0(obj, "identity"), obj, "Foo's identity saw the same self through [super identity]");
}

#[test]
fn super_stret_passes_the_original_receiver() {
    let _g = fresh();
    extern "C" fn foo_rect(out: *mut Rect, this: Id, _cmd: Sel) {
        // SAFETY: the caller passes a result buffer.
        unsafe { *out = Rect { a: this as usize as f64, b: 0.0, c: 0.0, d: 0.0 } };
    }
    extern "C" fn bar_rect(out: *mut Rect, this: Id, cmd: Sel) {
        let sup = ObjcSuper { receiver: this, class: BAR.load(Ordering::SeqCst) as Class };
        let f: extern "C" fn(*mut Rect, *const ObjcSuper, Sel) = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_super2_stret as *const () as usize) };
        f(out, &sup, cmd)
    }
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("rect", imp(foo_rect as extern "C" fn(*mut Rect, Id, Sel)))], &[], &[]);
    let bar = build_class("Bar", foo.class, 8, 8, &[("rect", imp(bar_rect as extern "C" fn(*mut Rect, Id, Sel)))], &[], &[]);
    BAR.store(bar.class as usize, Ordering::SeqCst);
    register(&[&foo, &bar]);
    let obj = send0(send0(bar.class as Id, "alloc"), "init");
    let rect: extern "C" fn(*mut Rect, Id, Sel) = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_stret as *const () as usize) };
    let mut out = Rect { a: 0.0, b: 0.0, c: 0.0, d: 0.0 };
    rect(&mut out, obj, sel("rect"));
    assert_eq!(out.a, obj as usize as f64);
}

extern "C" fn cat_answer(_this: Id, _cmd: Sel) -> i64 {
    7
}

#[test]
fn a_category_overrides_and_adds() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[("answer", imp(foo_answer as extern "C" fn(Id, Sel) -> i64))], &[], &[]);
    register(&[&foo]);
    let cat = std::boxed::Box::leak(std::boxed::Box::new(crate::abi::Category {
        name: leak_cstr("Extra"),
        cls: foo.class,
        instance_methods: leak_methods(&[("answer", imp(cat_answer as extern "C" fn(Id, Sel) -> i64)), ("added", imp(cat_answer as extern "C" fn(Id, Sel) -> i64))]),
        class_methods: core::ptr::null_mut(),
        protocols: core::ptr::null(),
        instance_properties: core::ptr::null(),
    })) as *mut crate::abi::Category;
    let cats = std::boxed::Box::leak(std::boxed::Box::new([cat as usize]));
    let meta = ImageMeta { catlist: (cats.as_ptr() as usize, 1), ..Default::default() };
    // SAFETY: the category was built above.
    unsafe { runtime::register_image(&meta) };
    let obj = send0(send0(foo.class as Id, "alloc"), "init");
    let answer: extern "C" fn(Id, Sel) -> i64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(answer(obj, sel("answer")), 7, "the category's method wins");
    assert_eq!(answer(obj, sel("added")), 7);
}

static INIT_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

extern "C" fn init_a(_cls: Id, _cmd: Sel) {
    INIT_ORDER.lock().unwrap().push("A");
}
extern "C" fn init_b(_cls: Id, _cmd: Sel) {
    INIT_ORDER.lock().unwrap().push("B");
}

#[test]
fn initialize_runs_once_superclass_first_and_only_on_first_use() {
    let _g = fresh();
    INIT_ORDER.lock().unwrap().clear();
    let ns = runtime::class_named("NSObject");
    let a = build_class("A", ns, 8, 8, &[], &[("initialize", imp(init_a as extern "C" fn(Id, Sel)))], &[]);
    let b = build_class("B", a.class, 8, 8, &[], &[("initialize", imp(init_b as extern "C" fn(Id, Sel)))], &[]);
    register(&[&a, &b]);
    assert!(INIT_ORDER.lock().unwrap().is_empty(), "nothing runs at registration");
    send0(send0(b.class as Id, "alloc"), "init");
    send0(send0(b.class as Id, "alloc"), "init");
    send0(send0(a.class as Id, "alloc"), "init");
    assert_eq!(*INIT_ORDER.lock().unwrap(), std::vec!["A", "B"]);
}

// ---- instance variables moved when the superclass differs -----------------------------------------------------------------

#[test]
fn ivars_move_when_the_superclass_is_larger_than_the_compiler_assumed() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    // the native base is 40 bytes; the compiler assumed 16 when it laid out the subclass
    let base = runtime::register_native_class("Base40", ns, 40, &[], &[]);
    let derived = build_class("Derived", base, 16, 24, &[], &[], &[(16, 3)]);
    register(&[&derived]);
    // SAFETY: offsets[0] is the leaked offset variable.
    assert_eq!(unsafe { *derived.offsets[0] }, 40, "the ivar moved to the end of the real superclass");
    assert_eq!(runtime::class_instance_size(derived.class), 48);
    let obj = send0(send0(derived.class as Id, "alloc"), "init");
    // SAFETY: the instance is 48 bytes; offset 40 is its ivar.
    unsafe { *((obj as *mut u8).add(40) as *mut u64) = 0xABCD };
    // SAFETY: as above.
    assert_eq!(unsafe { *((obj as *mut u8).add(40) as *const u64) }, 0xABCD);
}

#[test]
fn ivars_stay_when_the_superclass_matches() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let derived = build_class("Same", ns, 8, 16, &[], &[], &[(8, 3)]);
    register(&[&derived]);
    // SAFETY: offsets[0] is the leaked offset variable.
    assert_eq!(unsafe { *derived.offsets[0] }, 8);
    assert_eq!(runtime::class_instance_size(derived.class), 16);
}

#[test]
fn ivars_move_down_when_the_superclass_is_smaller() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let base = runtime::register_native_class("Base16", ns, 16, &[], &[]);
    let derived = build_class("Smaller", base, 64, 72, &[], &[], &[(64, 3)]);
    register(&[&derived]);
    // SAFETY: offsets[0] is the leaked offset variable.
    assert_eq!(unsafe { *derived.offsets[0] }, 16);
    assert_eq!(runtime::class_instance_size(derived.class), 24);
}

// ---- calling conventions through the trampolines ----------------------------------------------------------------------

extern "C" fn many_args(_this: Id, _cmd: Sel, a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> u64 {
    a + 10 * b + 100 * c + 1000 * d + 10_000 * e + 100_000 * f
}

extern "C" fn float_mix(_this: Id, _cmd: Sel, x: f64, n: i64, y: f64) -> f64 {
    x * n as f64 + y
}

#[repr(C)]
#[derive(Debug, PartialEq, Clone, Copy)]
struct Rect {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
}

extern "C" fn rect_stret(out: *mut Rect, _this: Id, _cmd: Sel, scale: f64) {
    // SAFETY: the caller passes a result buffer.
    unsafe { *out = Rect { a: 1.0 * scale, b: 2.0 * scale, c: 3.0 * scale, d: 4.0 * scale } };
}

extern "C" fn stack_args(_this: Id, _cmd: Sel, a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64) -> u64 {
    a + b + c + d + e + f + 1000 * g + 1_000_000 * h
}

#[test]
fn the_trampolines_preserve_every_argument_register_the_stack_and_the_struct_return_buffer() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class(
        "Conv",
        ns,
        8,
        8,
        &[
            ("many:", imp(many_args as extern "C" fn(Id, Sel, u64, u64, u64, u64, u64, u64) -> u64)),
            ("mix:", imp(float_mix as extern "C" fn(Id, Sel, f64, i64, f64) -> f64)),
            ("rect:", imp(rect_stret as extern "C" fn(*mut Rect, Id, Sel, f64))),
            ("stack:", imp(stack_args as extern "C" fn(Id, Sel, u64, u64, u64, u64, u64, u64, u64, u64) -> u64)),
        ],
        &[],
        &[],
    );
    register(&[&foo]);
    let obj = send0(send0(foo.class as Id, "alloc"), "init");
    let many: extern "C" fn(Id, Sel, u64, u64, u64, u64, u64, u64) -> u64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(many(obj, sel("many:"), 1, 2, 3, 4, 5, 6), 1 + 20 + 300 + 4000 + 50_000 + 600_000);
    let mix: extern "C" fn(Id, Sel, f64, i64, f64) -> f64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(mix(obj, sel("mix:"), 2.5, 4, 0.5), 10.5);
    let rect: extern "C" fn(*mut Rect, Id, Sel, f64) = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_stret as *const () as usize) };
    let mut out = Rect { a: 0.0, b: 0.0, c: 0.0, d: 0.0 };
    rect(&mut out, obj, sel("rect:"), 2.0);
    assert_eq!(out, Rect { a: 2.0, b: 4.0, c: 6.0, d: 8.0 });
    let stack: extern "C" fn(Id, Sel, u64, u64, u64, u64, u64, u64, u64, u64) -> u64 = unsafe { core::mem::transmute(msgsend_address()) };
    assert_eq!(stack(obj, sel("stack:"), 1, 2, 3, 4, 5, 6, 7, 8), 21 + 7000 + 8_000_000);
}

#[test]
fn stret_to_nil_returns_the_buffer_untouched() {
    let _g = fresh();
    let rect: extern "C" fn(*mut Rect, Id, Sel) -> *mut Rect = unsafe { core::mem::transmute(crate::msg::osxobjc_msgsend_stret as *const () as usize) };
    let mut out = Rect { a: 9.0, b: 9.0, c: 9.0, d: 9.0 };
    let ret = rect(&mut out, core::ptr::null_mut(), sel("x"));
    assert_eq!(ret, &mut out as *mut Rect);
    assert_eq!(out.a, 9.0);
}

// ---- ARC, weak references, pools --------------------------------------------------------------------------------------

static DEALLOCS: AtomicUsize = AtomicUsize::new(0);

extern "C" fn counting_dealloc(this: Id, cmd: Sel) {
    DEALLOCS.fetch_add(1, Ordering::SeqCst);
    // [super dealloc]
    runtime::free_instance(this);
    let _ = cmd;
}

#[test]
fn autorelease_pools_release_at_pop_and_weak_slots_zero_on_death() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Counted", ns, 8, 8, &[("dealloc", imp(counting_dealloc as extern "C" fn(Id, Sel)))], &[], &[]);
    register(&[&foo]);
    let before = DEALLOCS.load(Ordering::SeqCst);
    let token = runtime::pool_push();
    let obj = send0(send0(foo.class as Id, "alloc"), "init");
    runtime::autorelease(obj);
    assert!(runtime::is_live(obj), "still alive inside the pool");
    let mut weak: usize = 0;
    // SAFETY: weak is a local pointer-sized slot that outlives the referent in this test.
    unsafe { runtime::weak_store(&mut weak, obj) };
    assert_eq!(weak, obj as usize);
    runtime::pool_pop(token);
    assert!(!runtime::is_live(obj));
    assert_eq!(DEALLOCS.load(Ordering::SeqCst), before + 1, "the overriding dealloc ran exactly once");
    assert_eq!(weak, 0, "the weak slot was zeroed when the object died");
}

#[test]
fn nsobject_equality_hash_and_the_arc_entry_points() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject") as Id;
    let a = send0(ns, "new");
    let b = send0(ns, "new");
    let eq: extern "C" fn(Id, Sel, Id) -> bool = unsafe { core::mem::transmute(msgsend_address()) };
    assert!(eq(a, sel("isEqual:"), a));
    assert!(!eq(a, sel("isEqual:"), b));
    // objc_storeStrong(&slot, value): retains the new value and releases the old one
    use crate::libobjc::symbol;
    let store: extern "C" fn(*mut Id, Id) = unsafe { core::mem::transmute(symbol("_objc_storeStrong").unwrap()) };
    let mut slot: Id = core::ptr::null_mut();
    store(&mut slot, a);
    assert_eq!(slot, a);
    assert_eq!(runtime::retain_count(a), 2, "storeStrong retained the value");
    store(&mut slot, b);
    assert_eq!(runtime::retain_count(a), 1, "and released the old one");
    assert_eq!(runtime::retain_count(b), 2);
    let retain_auto: extern "C" fn(Id) -> Id = unsafe { core::mem::transmute(symbol("_objc_retainAutorelease").unwrap()) };
    let token = runtime::pool_push();
    retain_auto(a);
    assert_eq!(runtime::retain_count(a), 2);
    runtime::pool_pop(token);
    assert_eq!(runtime::retain_count(a), 1, "the pool released what retainAutorelease deferred");
}

#[test]
fn retain_and_release_of_non_instances_are_inert() {
    let _g = fresh();
    let ns = runtime::class_named("NSObject") as Id;
    assert_eq!(runtime::retain(ns), ns);
    runtime::release(ns); // a class object is immortal
    assert_eq!(runtime::retain(core::ptr::null_mut()), core::ptr::null_mut());
    runtime::release(core::ptr::null_mut());
}

// ---- refusals ---------------------------------------------------------------------------------------------------------

fn panicking(msg: &str) -> ! {
    panic!("{}", msg)
}

#[test]
fn an_unrecognized_selector_ends_the_guest_by_name() {
    let _g = fresh();
    runtime::set_fatal_hook(panicking);
    let ns = runtime::class_named("NSObject");
    let foo = build_class("Foo", ns, 8, 8, &[], &[], &[]);
    register(&[&foo]);
    let obj = send0(send0(foo.class as Id, "alloc"), "init");
    let r = std::panic::catch_unwind(|| {
        // SAFETY: obj is a live instance.
        unsafe { runtime::resolve(obj, sel("noSuchMethod:")) }
    });
    let msg = *r.expect_err("must not resolve").downcast::<std::string::String>().unwrap();
    assert_eq!(msg, "OBJC-UNRECOGNIZED-SELECTOR -[Foo noSuchMethod:]");
    let r = std::panic::catch_unwind(|| {
        // SAFETY: foo.class is a class object.
        unsafe { runtime::resolve(foo.class as Id, sel("noClassMethod")) }
    });
    let msg = *r.expect_err("must not resolve").downcast::<std::string::String>().unwrap();
    assert_eq!(msg, "OBJC-UNRECOGNIZED-SELECTOR +[Foo noClassMethod]");
}

#[test]
fn libobjc_symbols_resolve_and_unknown_ones_do_not() {
    let _g = fresh();
    use crate::libobjc::symbol;
    assert_eq!(symbol("_OBJC_CLASS_$_NSObject"), Some(runtime::class_named("NSObject") as usize));
    // SAFETY: NSObject is a registered class.
    assert_eq!(symbol("_OBJC_METACLASS_$_NSObject"), Some(unsafe { (*runtime::class_named("NSObject")).isa } as usize));
    assert_eq!(symbol("_objc_msgSend"), Some(msgsend_address()));
    assert!(symbol("_objc_retain").is_some());
    assert!(symbol("_OBJC_CLASS_$_NSWindow").is_none(), "no native NSWindow is registered at this rung");
    assert!(symbol("_objc_noSuchThing").is_none());
}

#[test]
fn a_refusal_raised_inside_a_table_access_does_not_wedge_the_next_guest() {
    let _g = fresh();
    runtime::set_fatal_hook(panicking);
    // a class without class_ro_t makes realization refuse while the table lock is held
    let bad = std::boxed::Box::leak(std::boxed::Box::new(ClassT { isa: core::ptr::null_mut(), superclass: core::ptr::null_mut(), cache: core::ptr::null_mut(), vtable: core::ptr::null_mut(), bits: 0 })) as *mut ClassT;
    let list = std::boxed::Box::leak(std::boxed::Box::new([bad as usize]));
    let meta = ImageMeta { classlist: (list.as_ptr() as usize, 1), ..Default::default() };
    let r = std::panic::catch_unwind(|| {
        // SAFETY: the metadata is deliberately malformed to provoke the refusal.
        unsafe { runtime::register_image(&meta) }
    });
    assert!(r.is_err());
    // the engine resets between guests; the lock the abandoned guest held must not survive that
    runtime::reset();
    nsobject::install();
    assert!(!runtime::class_named("NSObject").is_null());
}
