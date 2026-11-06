//! The runtime's state: the selector table, the class registry (native classes and classes realized from an image's metadata),
//! method lookup, instance allocation with reference counts, weak slots and autorelease pools.
//!
//! One guest runs per engine process, so the state is a process-wide singleton behind a small spin lock held only for table
//! accesses; implementations (`IMP`s) always run with the lock released, so a method may message freely.
use crate::abi::{self, Category, ClassRo, ClassT, MethodList};
use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

/// An object pointer; the first word of an instance is its class.
pub type Id = *mut c_void;
/// A selector: the address of a unique NUL-terminated name.
pub type Sel = *const u8;
/// A method implementation address.
pub type Imp = usize;
/// A class (or metaclass) pointer.
pub type Class = *mut ClassT;

/// Called when the runtime cannot continue (an unrecognized selector, a bad receiver, a feature the runtime refuses by name).
/// The engine installs a function that ends the guest; it must not return.
pub type FatalHook = fn(&str) -> !;

/// What the runtime knows about one class or metaclass.
struct ClassInfo {
    name: String,
    superclass: usize,
    is_meta: bool,
    methods: BTreeMap<usize, Imp>,
    cache: BTreeMap<usize, Imp>,
    instance_size: u32,
    initialized: bool,
}

struct State {
    sels: BTreeMap<String, usize>,
    classes: BTreeMap<usize, ClassInfo>,
    by_name: BTreeMap<String, usize>,
    /// Live instances: address -> (reference count, allocation size).
    live: BTreeMap<usize, (isize, usize)>,
    /// Weak slots: referent -> addresses of the slots that point at it.
    weak: BTreeMap<usize, Vec<usize>>,
    pool: Vec<usize>,
    /// Classes whose realization is in progress (metaclass and root class refer to each other).
    realizing: Vec<usize>,
    fatal: Option<FatalHook>,
}

struct Global {
    lock: AtomicBool,
    state: UnsafeCell<Option<State>>,
}

// SAFETY: every access to `state` goes through `with`, which holds the spin lock.
unsafe impl Sync for Global {}

static GLOBAL: Global = Global { lock: AtomicBool::new(false), state: UnsafeCell::new(None) };

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    // The sections held under this lock are table accesses of microseconds and one guest thread runs at this rung.
    // delete-when: guest threads exist (osxx86 rung 5): take the OS lock primitive here instead of spinning.
    while GLOBAL.lock.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: the lock is held, so no other reference to the state exists.
    let slot = unsafe { &mut *GLOBAL.state.get() };
    let state = slot.get_or_insert_with(|| State {
        sels: BTreeMap::new(),
        classes: BTreeMap::new(),
        by_name: BTreeMap::new(),
        live: BTreeMap::new(),
        weak: BTreeMap::new(),
        pool: Vec::new(),
        realizing: Vec::new(),
        fatal: None,
    });
    let r = f(state);
    GLOBAL.lock.store(false, Ordering::Release);
    r
}

/// Forgets every class, selector and instance. Tests and the engine call it between guests.
pub fn reset() {
    // a previous guest may have been abandoned while the lock was held (a refusal raised inside a table access): the engine
    // calls reset only when no guest runs, so clearing the lock here is sound
    GLOBAL.lock.store(false, Ordering::Release);
    with(|s| {
        for (addr, (_, size)) in core::mem::take(&mut s.live) {
            // SAFETY: every live entry was allocated by `alloc_instance` with exactly this layout.
            unsafe { dealloc(addr as *mut u8, instance_layout(size)) };
        }
        s.classes.clear();
        s.by_name.clear();
        s.weak.clear();
        s.pool.clear();
        // selectors are kept: their strings are leaked and images that outlive a reset may still hold them
    });
}

/// Installs the function the runtime calls for conditions it refuses by name.
pub fn set_fatal_hook(hook: FatalHook) {
    with(|s| s.fatal = Some(hook));
}

/// Ends the guest through the installed hook; without one, faults the process.
pub fn fatal(message: &str) -> ! {
    let hook = with(|s| s.fatal);
    match hook {
        Some(h) => h(message),
        // SAFETY: `ud2` raises an invalid-opcode fault and never returns.
        None => unsafe { core::arch::asm!("ud2", options(noreturn)) },
    }
}

fn instance_layout(size: usize) -> Layout {
    // SAFETY-free: size is non-zero and 16 is a power of two.
    Layout::from_size_align(size.max(16), 16).unwrap_or_else(|_| Layout::new::<u128>())
}

// ---- selectors ----------------------------------------------------------------------------------------------------------

/// The unique selector for `name`.
pub fn sel_register(name: &str) -> Sel {
    with(|s| intern(s, name))
}

fn intern(s: &mut State, name: &str) -> Sel {
    if let Some(&p) = s.sels.get(name) {
        return p as Sel;
    }
    let mut bytes = Vec::with_capacity(name.len() + 1);
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(0);
    let p = Box::leak(bytes.into_boxed_slice()).as_ptr();
    s.sels.insert(name.to_string(), p as usize);
    p
}

/// The unique selector for the NUL-terminated name at `p`.
///
/// # Safety
/// `p` must point at a NUL-terminated string.
pub unsafe fn sel_register_cstr(p: *const u8) -> Sel {
    // SAFETY: forwarded.
    let name = unsafe { abi::cstr(p) };
    sel_register(name)
}

/// The name of a selector.
///
/// # Safety
/// `sel` must be a selector (or any NUL-terminated string pointer).
pub unsafe fn sel_name<'a>(sel: Sel) -> &'a str {
    // SAFETY: forwarded.
    unsafe { abi::cstr(sel) }
}

// ---- class registration -------------------------------------------------------------------------------------------------

fn leak_class() -> Class {
    Box::leak(Box::new(ClassT {
        isa: core::ptr::null_mut(),
        superclass: core::ptr::null_mut(),
        cache: core::ptr::null_mut(),
        vtable: core::ptr::null_mut(),
        bits: 0,
    }))
}

/// A method for a native class: its selector name and the function implementing it.
pub type NativeMethod = (&'static str, Imp);

/// Defines a class implemented by the engine (its methods are `extern "C"` functions). `superclass` is null for a root class.
/// The class and its metaclass are ordinary `ClassT` objects, so an image's `_OBJC_CLASS_$_Name` binds can point at them.
pub fn register_native_class(name: &str, superclass: Class, instance_size: u32, instance: &[NativeMethod], class_methods: &[NativeMethod]) -> Class {
    let cls = leak_class();
    let meta = leak_class();
    with(|s| {
        let root_meta = if superclass.is_null() { meta } else { root_meta_of(s, superclass as usize).unwrap_or(meta as usize) as Class };
        // SAFETY: both ClassT objects were just allocated and are exclusively ours; a registered superclass is a valid ClassT.
        unsafe {
            (*cls).isa = meta;
            (*cls).superclass = superclass;
            (*meta).isa = root_meta;
            (*meta).superclass = if superclass.is_null() { cls } else { (*superclass).isa };
        }
        let mut methods = BTreeMap::new();
        for (n, imp) in instance {
            let sel = intern(s, n);
            methods.insert(sel as usize, *imp);
        }
        let mut cmethods = BTreeMap::new();
        for (n, imp) in class_methods {
            let sel = intern(s, n);
            cmethods.insert(sel as usize, *imp);
        }
        // SAFETY: the metaclass was initialized above.
        let super_meta = unsafe { (*meta).superclass as usize };
        s.classes.insert(
            cls as usize,
            ClassInfo { name: name.to_string(), superclass: superclass as usize, is_meta: false, methods, cache: BTreeMap::new(), instance_size, initialized: false },
        );
        s.classes.insert(
            meta as usize,
            ClassInfo { name: name.to_string(), superclass: super_meta, is_meta: true, methods: cmethods, cache: BTreeMap::new(), instance_size: 0, initialized: false },
        );
        s.by_name.insert(name.to_string(), cls as usize);
    });
    cls
}

fn root_class_of(s: &State, mut c: usize) -> Option<usize> {
    while let Some(info) = s.classes.get(&c) {
        if info.superclass == 0 {
            return Some(c);
        }
        c = info.superclass;
    }
    None
}

fn root_meta_of(s: &State, c: usize) -> Option<usize> {
    let root = root_class_of(s, c)?;
    // SAFETY: registered classes are valid ClassT objects.
    Some(unsafe { (*(root as Class)).isa as usize })
}

/// The class registered under `name`, or null.
pub fn class_named(name: &str) -> Class {
    with(|s| s.by_name.get(name).copied().unwrap_or(0)) as Class
}

/// The name of a class or metaclass, or "" when it is unknown.
pub fn class_name(c: Class) -> String {
    with(|s| s.classes.get(&(c as usize)).map(|i| i.name.clone()).unwrap_or_default())
}

/// The superclass pointer of a registered class (null for a root class).
pub fn class_superclass(c: Class) -> Class {
    with(|s| s.classes.get(&(c as usize)).map(|i| i.superclass).unwrap_or(0)) as Class
}

/// The instance size of a registered class.
pub fn class_instance_size(c: Class) -> u32 {
    with(|s| s.classes.get(&(c as usize)).map(|i| i.instance_size).unwrap_or(0))
}

// ---- realizing an image's metadata --------------------------------------------------------------------------------------

/// The locations of an image's Objective-C metadata sections, as slid addresses and element counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImageMeta {
    /// `__objc_selrefs`: address and number of selector reference slots.
    pub selrefs: (usize, usize),
    /// `__objc_classlist`: address and number of class pointers.
    pub classlist: (usize, usize),
    /// `__objc_catlist`: address and number of category pointers.
    pub catlist: (usize, usize),
}

/// Fixes up an image's selector references to the unique selectors, realizes its classes (superclasses first) and attaches its
/// categories. The image's pages must still be writable.
///
/// # Safety
/// Every address in `meta` must lie in a mapped, rebased and bound Objective-C image.
pub unsafe fn register_image(meta: &ImageMeta) {
    with(|s| {
        for i in 0..meta.selrefs.1 {
            let slot = (meta.selrefs.0 + i * 8) as *mut usize;
            // SAFETY: the slot lies in the image's writable selref section.
            unsafe {
                let name = abi::cstr(*slot as *const u8);
                *slot = intern(s, name) as usize;
            }
        }
        for i in 0..meta.classlist.1 {
            // SAFETY: the classlist slot is readable.
            let c = unsafe { *((meta.classlist.0 + i * 8) as *const usize) };
            // SAFETY: each classlist entry points at a class_t.
            unsafe { realize(s, c as Class) };
        }
        for i in 0..meta.catlist.1 {
            // SAFETY: the catlist slot is readable.
            let cat = unsafe { *((meta.catlist.0 + i * 8) as *const *mut Category) };
            // SAFETY: each catlist entry points at a category_t.
            unsafe { attach_category(s, cat) };
        }
    });
}

/// Realizes `c` and everything it needs; idempotent.
///
/// # Safety
/// `c` must point at a `class_t` whose superclass chain and `bits` are valid.
unsafe fn realize(s: &mut State, c: Class) {
    if c.is_null() || s.classes.contains_key(&(c as usize)) || s.realizing.contains(&(c as usize)) {
        return;
    }
    s.realizing.push(c as usize);
    // SAFETY: the caller guarantees a valid class_t.
    let (sup, isa, bits) = unsafe { ((*c).superclass, (*c).isa, (*c).bits) };
    // SAFETY: superclass and metaclass are class_t objects (or null).
    unsafe {
        realize(s, sup);
        realize(s, isa);
    }
    let ro = (bits & !7) as *const ClassRo;
    if ro.is_null() {
        fatal_in(s, "OBJC-REALIZE: class without class_ro_t");
    }
    // SAFETY: the ro pointer is valid for an image class.
    let r = unsafe { *ro };
    // SAFETY: the name is a NUL-terminated string in the image.
    let name = unsafe { abi::cstr(r.name) }.to_string();
    let is_meta = r.flags & abi::RO_META != 0;
    let mut methods = BTreeMap::new();
    // SAFETY: base_methods is null or a valid method list in the writable image.
    unsafe { collect_methods(s, r.base_methods, &mut methods) };
    // instance layout: the superclass may have a different size than the compiler assumed
    let super_size = s.classes.get(&(sup as usize)).map(|i| i.instance_size).unwrap_or(0);
    let mut instance_size = r.instance_size;
    if !is_meta && !sup.is_null() {
        // SAFETY: the ivar list is null or valid and writable.
        instance_size = unsafe { reconcile_ivars(&r, super_size) };
    }
    s.classes.insert(
        c as usize,
        ClassInfo { name: name.clone(), superclass: sup as usize, is_meta, methods, cache: BTreeMap::new(), instance_size, initialized: false },
    );
    if !is_meta {
        s.by_name.entry(name).or_insert(c as usize);
    }
    s.realizing.retain(|x| *x != c as usize);
}

fn fatal_in(s: &State, message: &str) -> ! {
    match s.fatal {
        Some(h) => h(message),
        // SAFETY: `ud2` raises an invalid-opcode fault and never returns.
        None => unsafe { core::arch::asm!("ud2", options(noreturn)) },
    }
}

/// # Safety
/// `list` must be null or a valid, writable method list.
unsafe fn collect_methods(s: &mut State, list: *mut MethodList, into: &mut BTreeMap<usize, Imp>) {
    // SAFETY: forwarded.
    let count = unsafe { abi::method_count(list) };
    for i in 0..count {
        // SAFETY: i < count.
        let Some(m) = (unsafe { abi::method_at(list, i) }) else {
            fatal_in(s, "OBJC-REALIZE: relative method lists are not supported");
        };
        // SAFETY: m points at a method_t in the writable list.
        unsafe {
            let sel = intern(s, abi::cstr((*m).name));
            (*m).name = sel;
            into.insert(sel as usize, (*m).imp);
        }
    }
}

/// Moves a subclass's instance variables when its superclass is larger or smaller than the compiler assumed, returning the
/// subclass's real instance size.
///
/// # Safety
/// `r.ivars` must be null or a valid list whose offset pointers are writable.
unsafe fn reconcile_ivars(r: &ClassRo, super_size: u32) -> u32 {
    // SAFETY: forwarded.
    let first = unsafe { abi::ivar_at(r.ivars, 0) };
    let Some(first) = first else {
        return super_size;
    };
    // SAFETY: first points at an ivar_t.
    let raw = unsafe { (*first).alignment };
    let align = if raw == u32::MAX { 8 } else { 1u32 << raw.min(5) };
    let new_start = super_size.div_ceil(align) * align;
    if new_start == r.instance_start {
        return r.instance_size;
    }
    let delta = new_start.wrapping_sub(r.instance_start);
    // SAFETY: forwarded.
    let count = unsafe { (*r.ivars).count } as usize;
    for i in 0..count {
        // SAFETY: i < count.
        if let Some(iv) = unsafe { abi::ivar_at(r.ivars, i) } {
            // SAFETY: the offset variable lives in the image's writable data.
            unsafe {
                let p = (*iv).offset;
                if !p.is_null() {
                    *p = (*p).wrapping_add(delta);
                }
            }
        }
    }
    r.instance_size.wrapping_add(delta)
}

/// # Safety
/// `cat` must point at a valid category_t whose class is realized or realizable.
unsafe fn attach_category(s: &mut State, cat: *mut Category) {
    // SAFETY: forwarded.
    let (cls, im, cm) = unsafe { ((*cat).cls, (*cat).instance_methods, (*cat).class_methods) };
    // SAFETY: the target class is a class_t.
    unsafe { realize(s, cls) };
    let meta = if cls.is_null() { core::ptr::null_mut() } else { unsafe { (*cls).isa } };
    for (target, list) in [(cls, im), (meta, cm)] {
        if target.is_null() || list.is_null() {
            continue;
        }
        let mut add = BTreeMap::new();
        // SAFETY: the list belongs to the category.
        unsafe { collect_methods(s, list, &mut add) };
        if let Some(info) = s.classes.get_mut(&(target as usize)) {
            info.methods.extend(add);
            info.cache.clear();
        }
    }
}

// ---- lookup -------------------------------------------------------------------------------------------------------------

fn lookup_in(s: &mut State, start: usize, sel: usize) -> Option<Imp> {
    if let Some(imp) = s.classes.get(&start).and_then(|i| i.cache.get(&sel)) {
        return Some(*imp);
    }
    let mut c = start;
    while c != 0 {
        let info = s.classes.get(&c)?;
        if let Some(&imp) = info.methods.get(&sel) {
            if let Some(i) = s.classes.get_mut(&start) {
                i.cache.insert(sel, imp);
            }
            return Some(imp);
        }
        c = info.superclass;
    }
    None
}

/// The implementation `sel` resolves to for messages to instances of `cls` (or to class `cls`'s class methods when `cls` is a
/// metaclass), or None.
pub fn class_get_imp(cls: Class, sel: Sel) -> Option<Imp> {
    with(|s| lookup_in(s, cls as usize, sel as usize))
}

/// The class of an object (its first word).
///
/// # Safety
/// `obj` must point at an object.
pub unsafe fn object_class(obj: Id) -> Class {
    // SAFETY: the first word of every object is its isa.
    unsafe { *(obj as *const Class) }
}

/// Resolves a message for the asm trampolines: the implementation to jump to. Runs `+initialize` lazily; ends the guest by
/// name when the selector is unrecognized.
///
/// # Safety
/// `receiver` must be a non-null object or class.
pub unsafe fn resolve(receiver: Id, sel: Sel) -> Imp {
    // SAFETY: forwarded.
    let cls = unsafe { object_class(receiver) };
    resolve_in(cls, receiver, sel)
}

/// Resolves a `super` message: lookup starts at the superclass of `current`.
pub fn resolve_super(receiver: Id, sel: Sel, current: Class) -> Imp {
    let start = class_superclass(current);
    resolve_in(start, receiver, sel)
}

fn resolve_in(cls: Class, receiver: Id, sel: Sel) -> Imp {
    maybe_initialize(cls);
    let found = with(|s| lookup_in(s, cls as usize, sel as usize));
    match found {
        Some(imp) => imp,
        None => {
            let (cname, meta) = with(|s| s.classes.get(&(cls as usize)).map(|i| (i.name.clone(), i.is_meta)).unwrap_or_default());
            let _ = receiver;
            // SAFETY: sel is a NUL-terminated selector name.
            let sname = unsafe { sel_name(sel) };
            let msg = alloc::format!("OBJC-UNRECOGNIZED-SELECTOR {}[{} {}]", if meta { '+' } else { '-' }, cname, sname);
            fatal(&msg)
        }
    }
}

/// Sends `+initialize` once per class, superclass first, before the class's first message.
fn maybe_initialize(cls: Class) {
    let pending: Vec<usize> = with(|s| {
        let mut chain = Vec::new();
        let mut c = cls as usize;
        while c != 0 {
            match s.classes.get_mut(&c) {
                Some(i) if !i.initialized => {
                    i.initialized = true;
                    chain.push(c);
                    c = i.superclass;
                }
                _ => break,
            }
        }
        chain
    });
    for &meta_or_class in pending.iter().rev() {
        let (is_meta, name) = with(|s| s.classes.get(&meta_or_class).map(|i| (i.is_meta, i.name.clone())).unwrap_or_default());
        if !is_meta {
            continue;
        }
        let class_obj = class_named(&name);
        if class_obj.is_null() {
            continue;
        }
        let sel = sel_register("initialize");
        // only a class's OWN +initialize runs, so a subclass without one does not re-run its superclass's
        let own = with(|s| s.classes.get(&meta_or_class).and_then(|i| i.methods.get(&(sel as usize)).copied()));
        if let Some(imp) = own {
            // SAFETY: the implementation is an `extern "C" fn(Id, Sel)` registered under +initialize.
            let f: extern "C" fn(Id, Sel) = unsafe { core::mem::transmute(imp) };
            f(class_obj as Id, sel);
        }
    }
}

// ---- instances, reference counts, weak slots, pools ----------------------------------------------------------------------

/// Allocates a zeroed instance of `cls` with a reference count of one and `extra` bytes after its ivars.
pub fn alloc_instance(cls: Class, extra: usize) -> Id {
    let size = class_instance_size(cls) as usize + extra;
    let size = size.max(8);
    // SAFETY: the layout is non-zero sized with a power-of-two alignment.
    let p = unsafe { alloc_zeroed(instance_layout(size)) };
    if p.is_null() {
        fatal("OBJC-ALLOC out of memory");
    }
    // SAFETY: p points at at least 8 zeroed bytes.
    unsafe { *(p as *mut Class) = cls };
    with(|s| s.live.insert(p as usize, (1, size)));
    p as Id
}

/// Whether `obj` is a live instance this runtime allocated.
pub fn is_live(obj: Id) -> bool {
    with(|s| s.live.contains_key(&(obj as usize)))
}

/// The reference count of a live instance (0 for anything else).
pub fn retain_count(obj: Id) -> isize {
    with(|s| s.live.get(&(obj as usize)).map(|e| e.0).unwrap_or(0))
}

/// Retains `obj` if it is a live instance; other pointers (classes, constant strings, nil) are immortal.
pub fn retain(obj: Id) -> Id {
    with(|s| {
        if let Some(e) = s.live.get_mut(&(obj as usize)) {
            e.0 += 1;
        }
    });
    obj
}

/// Releases `obj`; at zero sends `dealloc`.
pub fn release(obj: Id) {
    let last = with(|s| match s.live.get_mut(&(obj as usize)) {
        Some(e) if e.0 > 1 => {
            e.0 -= 1;
            false
        }
        Some(_) => true,
        None => false,
    });
    if last {
        let sel = sel_register("dealloc");
        // SAFETY: obj is a live instance, so its isa is a registered class.
        let imp = unsafe { resolve(obj, sel) };
        // SAFETY: the implementation is an `extern "C" fn(Id, Sel)` registered under dealloc.
        let f: extern "C" fn(Id, Sel) = unsafe { core::mem::transmute(imp) };
        f(obj, sel);
    }
}

/// Frees an instance's memory and clears the weak slots that pointed at it: the end of `-[NSObject dealloc]`.
pub fn free_instance(obj: Id) {
    let freed = with(|s| {
        let slots = s.weak.remove(&(obj as usize)).unwrap_or_default();
        for slot in slots {
            // SAFETY: a registered weak slot is a live pointer-sized location the guest gave us.
            unsafe { *(slot as *mut usize) = 0 };
        }
        s.live.remove(&(obj as usize))
    });
    if let Some((_, size)) = freed {
        // SAFETY: the allocation was made by `alloc_instance` with this layout and is no longer referenced.
        unsafe { dealloc(obj as *mut u8, instance_layout(size)) };
    }
}

/// Defers a release to the innermost autorelease pool.
pub fn autorelease(obj: Id) -> Id {
    with(|s| {
        if s.live.contains_key(&(obj as usize)) {
            s.pool.push(obj as usize);
        }
    });
    obj
}

/// Opens an autorelease pool; the token goes back to [`pool_pop`].
pub fn pool_push() -> usize {
    with(|s| s.pool.len())
}

/// Drains the pool opened with `token`, releasing everything autoreleased since.
pub fn pool_pop(token: usize) {
    loop {
        let next = with(|s| if s.pool.len() > token { s.pool.pop() } else { None });
        match next {
            Some(o) => release(o as Id),
            None => break,
        }
    }
}

/// Stores `obj` into the weak slot `loc`, registering it so it is zeroed when `obj` is freed.
///
/// # Safety
/// `loc` must be a writable pointer-sized location that stays valid until [`weak_destroy`] or the referent's death.
pub unsafe fn weak_store(loc: *mut usize, obj: Id) -> Id {
    with(|s| {
        // SAFETY: loc is writable per the contract.
        let old = unsafe { *loc };
        if old != 0 {
            if let Some(v) = s.weak.get_mut(&old) {
                v.retain(|a| *a != loc as usize);
            }
        }
        if obj.is_null() || !s.live.contains_key(&(obj as usize)) {
            // SAFETY: as above.
            unsafe { *loc = if obj.is_null() { 0 } else { obj as usize } };
            return obj;
        }
        s.weak.entry(obj as usize).or_default().push(loc as usize);
        // SAFETY: as above.
        unsafe { *loc = obj as usize };
        obj
    })
}

/// Reads a weak slot, retaining the referent when it is still alive.
///
/// # Safety
/// `loc` must be readable.
pub unsafe fn weak_load_retained(loc: *const usize) -> Id {
    // SAFETY: readable per the contract.
    let obj = unsafe { *loc } as Id;
    if obj.is_null() {
        return obj;
    }
    if is_live(obj) {
        retain(obj)
    } else {
        obj
    }
}

/// Unregisters a weak slot.
///
/// # Safety
/// `loc` must be readable.
pub unsafe fn weak_destroy(loc: *mut usize) {
    with(|s| {
        // SAFETY: readable per the contract.
        let old = unsafe { *loc };
        if let Some(v) = s.weak.get_mut(&old) {
            v.retain(|a| *a != loc as usize);
        }
    });
}
