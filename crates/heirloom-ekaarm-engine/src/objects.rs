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
/// control the host was told about and what was last projected to it, so an unchanged list costs no UI call (`listproject.rs`).
#[derive(Debug, Clone, Default)]
pub struct ListState {
    /// The list box is a `CCoeControl` too: its control state (observer, focus, ...) lives here, not in a second map entry that would
    /// replace this one (`control_state` finds it).
    pub ctrl: ControlState,
    pub control: Option<UiObjectRef>,
    pub shown: Vec<Vec<String>>,
    /// `ActivateL` ran: the list is on screen, so the host is told about it.
    pub activated: bool,
    /// What was last sent for the current / top item (the guest's own are the view's `iCurrentItemIndex` / `iTopItemIndex`, in guest
    /// memory) and the rectangle, to send only changes.
    pub shown_current: u32,
    pub shown_top: u32,
    /// The rectangle last sent (left, top, width, height).
    pub shown_rect: (i32, i32, i32, i32),
}

/// A `CRemConInterfaceSelector`: the interface objects registered with it (owned by it, remconinterfaceselector.h) and whether a
/// target session was opened.
#[derive(Debug, Clone, Default)]
pub struct RemConSelector {
    /// (target object, its observer) in registration order.
    pub targets: Vec<(u32, u32)>,
    pub target_open: bool,
}

/// The fixed toolbar a touch device's AppUi owns (akntoolbar.h): its items are the buttons of the application's own toolbar resource
/// (EIK_APP_INFO's extension, `toolbar.rs`); what the guest sets on it is recorded and nothing is drawn.
#[derive(Debug, Clone, Default)]
pub struct ToolbarState {
    /// (item id, its `CAknButton` guest object) in resource order: the items of the toolbar resource (`toolbar.rs`).
    pub items: Vec<(u32, u32)>,
    pub observer: u32,
    pub visible: bool,
    pub disabled: bool,
    pub sliding: bool,
    pub close_on_all: bool,
}

/// The host state of a `CCoeControl`: whether `ActivateL` ran (the guest's `iFlags` bits for it are private).
#[derive(Debug, Clone, Default)]
pub struct ControlState {
    pub activated: bool,
    pub observer: u32,
    pub non_focusing: bool,
    pub focused: bool,
    pub blank: bool,
    pub drag_events: bool,
    /// `MakeVisible` has not been called: a control starts visible.
    pub hidden: bool,
    /// Set when the control is a `CAknToolbar` (`toolbar.rs`).
    pub toolbar: Option<ToolbarState>,
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
    /// The fixed toolbar (`CurrentFixedToolbar`), made on first ask.
    pub toolbar: Option<u32>,
    /// The control stack (`CCoeAppUi::iStack`, coeaui.h:229): the controls offered key events, topmost first (`ctrlstack.rs`).
    pub stack: Vec<StackItem>,
}

/// One entry of the control stack (`CCoeAppUi::AddToStackL`, coeaui.h:112-113).
#[derive(Debug, Clone)]
pub struct StackItem {
    pub control: u32,
    pub priority: i32,
    /// `ECoeStackFlag*` (coeaui.h:75-86).
    pub flags: u32,
    /// The `MCoeView` the control was stacked for (coeaui.h:112), 0 for the application's own.
    pub view: u32,
}

#[derive(Debug, Clone)]
pub enum HostObj {
    Control(ControlState),
    /// A `CRepository`: its parsed keys.
    Repo(BTreeMap<u32, crate::cenrep::CenValue>),
    /// A `CDictionaryFileStore`: the file it is the store of and the streams (by UID) it holds.
    Dict(DictState),
    AppUi(AppUiState),
    RemCon(RemConSelector),
    View(ViewState),
    List(ListState),
    /// A `CFormattedCellListBoxData` (listdata.rs).
    CellData(crate::listdata::CellData),
    /// A `CAknsBasicBackgroundControlContext`: the rectangle the guest set (listdata.rs).
    SkinContext([i32; 4]),
    /// A `CFont` made by `GetNearestFontInTwips`: the height (twips) the guest asked for (screen.rs).
    Font(u32),
    /// A `CListBoxView`: the text shown when the list has no items (`SetListEmptyTextL`), recorded.
    /// DELETE WHEN the GuestUi List control carries an empty-list text (then project it).
    ListView(String),
    /// A `CEikScrollBarFrame` (listdata.rs).
    ScrollFrame(crate::listdata::ScrollFrame),
    NaviDecorator(crate::navi::NaviDecorator),
    /// A `CColorList` / `CColorArray` (colorlist.rs).
    ColorList(crate::colorlist::ColorListState),
    /// A bitmap made for an icon (icons.rs).
    IconBitmap(crate::icons::IconBitmap),
    ColorArray(crate::colorlist::ColorArrayState),
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
