//! The minimal host-side BOOPSI core (docs/MUI-NATIVE.md).
//!
//! An object is a 16-byte heap block; the object pointer is `block + 12`, so the guest's own amiga.lib `DoMethod`
//! (which reads `*(obj - 4)` = `_Object.o_Class`, loads the class's `Hook.h_Entry` at offset 8 and jumps there with
//! a0 = class, a1 = message, a2 = object) lands in ONE shared trap slot below. The class structs live in a fixed
//! region (no heap); only `h_Entry` and `h_Data` (the kind index) are filled in them.
//! All object STATE is host-side in [`Objects`]; the guest holds nothing it can corrupt.
use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::libtable::{Dispatch, LibraryBuilder};
use crate::{machine::Amiga, tags, Refusal};
use guestui_abi::{GuestUi, UiObjectRef};
use retro_guest_mem::GuestMem;
use retro_m68k::Cpu;

pub const LVO_NEW_OBJECT_A: i32 = -636;
pub const LVO_DISPOSE_OBJECT: i32 = -642;
pub const LVO_SET_ATTRS_A: i32 = -648;
pub const LVO_GET_ATTR: i32 = -654;

/// The pseudo-library whose one vector (-6) is every class's `h_Entry`. Free guest RAM below LOAD_BASE.
pub const BOOPSI_BASE: u32 = 0xE000;
const DISPATCH_LVO: i32 = -6;
/// `struct _Object` = MinNode (8) + o_Class (4); the heap block adds 4 spare bytes after it.
const HEADER: u32 = 12;
const BLOCK: u32 = 16;
const O_CLASS: u32 = 8;

pub use crate::mui::classes::Kind;

/// A MUIM_Notify record.
#[derive(Clone, Debug)]
pub struct Notify { pub attr: u32, pub val: u32, pub dest: u32, pub params: Vec<u32> }

/// Per-kind extra state.
pub enum Ext { None, App { bit: u32, broker: Option<crate::mui::broker::Broker>, pending: VecDeque<u32>, inbox: VecDeque<crate::mui::app::Inbound> }, Win { native: Option<UiObjectRef>, content: (i32, i32) }, Text { label: bool }, List(crate::mui::list::ListData), Sem(crate::mui::store::Sem), Space(Vec<crate::mui::store::Entry>) }

pub struct Obj {
    pub kind: Kind, pub block: u32,
    pub attrs: BTreeMap<u32, u32>, pub strs: BTreeMap<u32, String>, pub arrs: BTreeMap<u32, Vec<String>>,
    /// Guest copies of strings the guest reads back (GetAttr of a string attribute): attribute -> (address, size).
    pub bufs: BTreeMap<u32, (u32, u32)>,
    pub children: Vec<u32>, pub parent: Option<u32>, pub notifies: Vec<Notify>, pub ext: Ext,
}

#[derive(Default)]
pub struct Objects {
    pub map: BTreeMap<u32, Obj>, pub depth: u32,
    /// Which MUI object is which native control (token = the object's address): the one generic mapper of ruling 17.
    pub proj: retro_translate::Projections<u32>,
    /// Tokens whose native control the host REFUSED to draw: they are projected as the named label of [`crate::mui::win::degrade`]
    /// from then on (one control the host cannot draw never costs the window; the refusal is named in `Amiga::refused`).
    pub degraded: alloc::collections::BTreeSet<u64>,
    /// `MUIM_CallHook` follow-ups waiting to run as guest calls (mui/hooks.rs).
    pub hooks: VecDeque<crate::mui::hooks::HookCall>,
    /// The host's control metrics for MUI's layout (mui/layout.rs `Metrics`); default = Zune's own sizes.
    pub mui_metrics: crate::mui::layout::Metrics,
    /// MUI classes opened from disk (`.mcc`/`.mcp`), by the name they were asked for (mui/disk.rs).
    pub disk_classes: BTreeMap<String, crate::mui::disk::DiskClass>,
    /// Application -> its guest `struct MUI_GlobalInfo` (mui/mirror.rs).
    pub global_info: BTreeMap<u32, u32>,
    /// Self-drawing objects (mui/drawn.rs), the queue of methods still to send them, and each open window's drawing surface.
    pub drawn: BTreeMap<u32, crate::mui::drawn::Drawn>,
    pub draw_steps: VecDeque<crate::mui::drawn::Step>,
    /// A queued method is running as a guest call: a handler it reaches (its DoSuperMethod) must not start the next step inside it.
    pub draw_busy: bool,
    pub surfaces: BTreeMap<u32, crate::mui::drawn::Surface>,
    /// Input handlers (timers on the virtual clock, signals) and window event handlers a class registered (mui/handlers.rs).
    pub handlers: crate::mui::handlers::Handlers,
    /// `MUI_AddClipRegion` stacks (mri_rArray): RenderInfo -> the regions installed, innermost last.
    pub clip_stack: BTreeMap<u32, Vec<u32>>,
    /// What the host shows for each image object (mui/images.rs): its rectangle and display list, so an unchanged one is not sent again.
    pub images: BTreeMap<u32, (guestui_abi::GuestRect, Vec<guestui_abi::DrawOp>)>,
}

fn class_addr(k: Kind) -> u32 { crate::iclass::native_addr(crate::iclass::Native::Mui(k)) }

/// The trap slot every native class's `h_Entry` points at.
pub fn trap_entry() -> u32 { BOOPSI_BASE.wrapping_add(DISPATCH_LVO as u32) }

/// Register the one dispatcher trap slot (the native `struct IClass`es are built by iclass::init).
pub fn install<U: GuestUi>(mem: &mut GuestMem, dispatch: &mut Dispatch<U>) -> Result<(), Refusal> {
    LibraryBuilder::new("boopsi.dispatcher", BOOPSI_BASE).func(DISPATCH_LVO, "ClassDispatcher", dispatcher::<U>).install(mem, dispatch)
}

/// A new guest-visible object of `kind` (no attributes yet).
pub fn alloc_object<U: GuestUi>(a: &mut Amiga<U>, kind: Kind) -> Result<u32, Refusal> {
    let block = a.heap.alloc(BLOCK).ok_or_else(|| Refusal("MUI: no memory for an object (NOMEM)".into()))?;
    let e = Refusal::mem("object header");
    for i in (0..BLOCK).step_by(4) { a.mem.write_u32(block + i, 0).map_err(&e)?; }
    a.mem.write_u32(block + O_CLASS, class_addr(kind)).map_err(&e)?;
    track_object(a, kind, block + HEADER, block);
    Ok(block + HEADER)
}

/// The host record of an object whose memory the BOOPSI core already allocated (an instance of a program's own subclass of a MUI
/// class: rootclass made the block, `o_Class` is the subclass). `block` 0 = the BOOPSI core owns the memory.
pub fn adopt_object<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, obj: u32) { track_object(a, kind, obj, 0); }

fn track_object<U: GuestUi>(a: &mut Amiga<U>, kind: Kind, obj: u32, block: u32) {
    let ext = match kind { Kind::Window => Ext::Win { native: None, content: (0, 0) }, Kind::Text => Ext::Text { label: false }, Kind::List => Ext::List(Default::default()), Kind::Semaphore => Ext::Sem(Default::default()), Kind::Dataspace => Ext::Space(Vec::new()), _ => Ext::None };
    a.objects.map.insert(obj, Obj { kind, block, attrs: BTreeMap::new(), strs: BTreeMap::new(), arrs: BTreeMap::new(), bufs: BTreeMap::new(), children: Vec::new(), parent: None, notifies: Vec::new(), ext });
}

/// Forget `obj` (already unlinked and torn down) and give its block back.
pub fn free_object<U: GuestUi>(a: &mut Amiga<U>, obj: u32) -> Result<(), Refusal> {
    crate::mui::handlers::forget(a, obj);
    let o = a.objects.map.remove(&obj).ok_or_else(|| Refusal(format!("free object {obj:#x}: not an object")))?;
    for (addr, size) in o.bufs.into_values() { let _ = a.heap.free(addr, size); }
    if let Ext::Space(entries) = o.ext { for e in entries { let _ = a.heap.free(e.addr, e.len); } }
    if o.block == 0 {
        // An instance of a program's subclass: the memory is rootclass's (OM_DISPOSE of the root class does exactly this).
        if let Some(rec) = a.classes.objs.remove(&obj) {
            crate::iclass::count_object(a, rec.class, false)?;
            a.heap.free(rec.block, rec.size).map_err(|e| Refusal(format!("free object: {e}")))?;
        }
        return Ok(());
    }
    a.heap.free(o.block, BLOCK).map_err(|e| Refusal(format!("free object: {e}")))
}

/// The shared `h_Entry`: a0 = class, a1 = message, a2 = object -> d0 = the method's result.
fn dispatcher<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (cl, msg, obj) = (cpu.areg(0), cpu.areg(1), cpu.areg(2));
    let native = a.classes.recs.get(&cl).and_then(|r| r.native)
        .ok_or_else(|| Refusal(format!("class dispatcher trap entered with A0={cl:#x}, which is not a native class")))?;
    if !matches!(native, crate::iclass::Native::Mui(_)) {
        let r = crate::boopsi_native::dispatch(a, native, cl, obj, msg)?;
        cpu.set_d0(r);
        let id = a.mem.read_u32(msg).unwrap_or(0);
        return Ok(format!("[boopsi] {} method {id:#x} on {obj:#x} -> {r:#x}", a.classes.name_of(cl)));
    }
    crate::mui::dispatch(a, cpu, cl, obj, msg)
}

/// intuition NewObjectA(a0 = class pointer (must be NULL), a1 = public class name, a2 = tags).
pub fn new_object_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if cpu.areg(0) != 0 { return Err(Refusal("NewObjectA: a private class pointer is not bridged (public class names only)".into())); }
    let name = a.mem.read_cstr(cpu.areg(1)).map_err(Refusal::mem("NewObjectA class name"))?;
    let obj = crate::mui::new_by_name(a, &name, cpu.areg(2))?;
    crate::mui::hooks::after_call(a, cpu, obj)?;
    Ok(format!("[intuition] NewObjectA \"{name}\" -> {obj:#x}"))
}

/// intuition DisposeObject(a0 = object).
pub fn dispose_object<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    crate::mui::dispose_start(a, cpu, obj)?;
    Ok(format!("[intuition] DisposeObject {obj:#x}"))
}

/// intuition SetAttrsA(a0 = object, a1 = tags) -> 0.
pub fn set_attrs_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let obj = cpu.areg(0);
    let t = tags::walk(&a.mem, cpu.areg(1))?;
    match crate::mui::custom::own_class(a, obj) {
        // An object of the program's own class: OM_SET goes to ITS dispatcher, which hands the MUI attributes down with DoSuperMethod.
        Some(cl) => crate::mui::custom::queue_om_set(a, cl, obj, crate::mui::hooks::Msg::Params(alloc::vec![crate::boopsi_ids::OM_SET, cpu.areg(1), 0])),
        None => crate::mui::set_list(a, obj, &t)?,
    }
    crate::mui::after_set(a, cpu)?;
    Ok(format!("[intuition] SetAttrsA {obj:#x} ({} tags)", t.len()))
}

/// intuition GetAttr(d0 = attribute, a0 = object, a1 = storage) -> TRUE when the attribute is known.
pub fn get_attr<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (attr, obj, store) = (cpu.dreg(0), cpu.areg(0), cpu.areg(1));
    let v = crate::mui::get_attr(a, obj, attr)?;
    a.mem.write_u32(store, v).map_err(Refusal::mem("GetAttr storage"))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] GetAttr {obj:#x} {} = {v:#x}", crate::mui::attrs::name(attr)))
}
