//! The host-only state of guest objects: a per-process side table keyed by the object's guest address AND its allocation
//! generation (`Heap::generation_of`), so a guest that frees an object and allocates another at the same address never reaches
//! the old object's host state. Everything the guest can see of an object lives in guest memory at the SDK's offsets; only what
//! has no guest representation (a UI handle, a resolved resource) lives here.
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use heirloom_ui_abi::UiObjectRef;
use crate::loader::Process;

/// The host state of one `CAknView`: the AVKON_VIEW resource ids it was constructed from (the guest-visible copy is `iViewInfo`).
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    pub resource_id: u32,
}

/// The host state of a list box: the model the guest filled (`CTextListBoxModel::iItemTextArray`) is in guest memory; this is the
/// control the host was told about and the rows last projected, so an unchanged model costs no UI call.
#[derive(Debug, Clone, Default)]
pub struct ListState {
    pub control: Option<UiObjectRef>,
    pub shown: Vec<Vec<String>>,
}

/// A `CRemConInterfaceSelector`: the interface objects registered with it (owned by it, remconinterfaceselector.h) and whether a
/// target session was opened.
#[derive(Debug, Clone, Default)]
pub struct RemConSelector {
    /// (target object, its observer) in registration order.
    pub targets: Vec<(u32, u32)>,
    pub target_open: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DictState {
    pub path: String,
    pub entries: BTreeMap<u32, Vec<u8>>,
}

/// One file server session, keyed by the handle the guest holds in its `RFs` (`RHandleBase::iHandle`): handles are never reused, so
/// the key carries its own generation, and an `RFs` on the guest stack (no heap allocation to tie to) works the same as a member.
#[derive(Debug, Clone, Default)]
pub struct FsSessionState {
    /// Empty until the guest sets one (the default session path of a new session is `\` on the system drive; no path is relative yet).
    pub session_path: String,
}

/// A `CAknViewAppUi`'s views and the activations queued for the event loop. The view server of a device runs an activation as an
/// active object once the thread is back in its scheduler; `pending` is that queue.
#[derive(Debug, Clone, Default)]
pub struct AppUiState {
    /// (view, its `Id()` UID) in `AddViewL` order; the UID is 0 until `Id()` returned.
    pub views: Vec<(u32, u32)>,
    /// (view, custom message id, custom message bytes) waiting to be activated.
    pub pending: alloc::collections::VecDeque<(u32, u32, Vec<u8>)>,
    pub active: Option<u32>,
    /// The view `SetDefaultViewL` named: activated when the application comes to the foreground with no view active.
    pub default_view: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum HostObj {
    /// A `CDictionaryFileStore`: the file it is the store of and the streams (by UID) it holds.
    Dict(DictState),
    AppUi(AppUiState),
    RemCon(RemConSelector),
    View(ViewState),
    List(ListState),
}

#[derive(Default)]
pub struct Objects {
    map: BTreeMap<u32, (u32, HostObj)>,
    /// File server sessions by handle, and the next handle to hand out.
    pub fs_sessions: BTreeMap<u32, FsSessionState>,
    pub next_fs_handle: u32,
}

impl Process {
    /// The generation of the live guest allocation containing `addr` (None = nothing allocated there, or no heap yet).
    pub(crate) fn alloc_generation(&self, addr: u32) -> Option<u32> {
        self.heap.as_ref()?.generation_containing(addr)
    }

    /// Record `obj` as the host state of the guest object at `addr` (replacing any state of an earlier allocation there).
    pub(crate) fn obj_set(&mut self, addr: u32, obj: HostObj) -> Result<(), String> {
        let generation = self.alloc_generation(addr).ok_or_else(|| alloc::format!("{addr:#x} is not a live guest allocation: no host state can be bound to it"))?;
        self.nat.objs.map.insert(addr, (generation, obj));
        Ok(())
    }

    /// The `CAknViewAppUi` state of the AppUi at `addr`, created on first use.
    pub(crate) fn appui_state(&mut self, addr: u32) -> Result<&mut AppUiState, String> {
        if !matches!(self.obj_get(addr), Some(HostObj::AppUi(_))) {
            self.obj_set(addr, HostObj::AppUi(AppUiState::default()))?;
        }
        match self.obj_get(addr) {
            Some(HostObj::AppUi(s)) => Ok(s),
            _ => Err(alloc::format!("{addr:#x}: host state is not an AppUi's")),
        }
    }

    /// Forget the host state of the guest object at `addr` (its destructor ran).
    pub(crate) fn obj_remove(&mut self, addr: u32) {
        self.nat.objs.map.remove(&addr);
    }

    /// The host state of the guest object at `addr`, if it was recorded for the allocation that is live there now.
    pub(crate) fn obj_get(&mut self, addr: u32) -> Option<&mut HostObj> {
        let live = self.alloc_generation(addr)?;
        match self.nat.objs.map.get_mut(&addr) {
            Some((generation, obj)) if *generation == live => Some(obj),
            _ => None,
        }
    }
}
