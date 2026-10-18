//! The Amiga machine: guest RAM + fake ExecBase + dispatch table + a [`GuestUi`].
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use alloc::format;
use alloc::collections::BTreeMap;
use crate::libtable::{write_slot, Dispatch, LibraryBuilder};
use crate::{devices, dos, exec, exec_wait, lists, memheader, semaphores, gadtools, graphics, gt_kinds, gt_menus, idcmp, intuition, memory, menu_nav, ports, process, rdargs, screen, timer, window, Refusal};
use guestui_abi::{GuestUi, UiObjectRef};
use retro_guest_mem::{GuestMem, DEFAULT_SIZE};
use retro_hunk::Loaded;
use retro_m68k::{Cpu, Host, RunStats};

pub const EXEC_BASE: u32 = 0x1000;
pub const INTUITION_BASE: u32 = 0x2000;
pub const GRAPHICS_BASE: u32 = 0x3000;
/// dos.library's 160 vectors need ~1 KiB below the base; 0x5000 leaves graphics' Library struct room above 0x3000.
pub const DOS_BASE: u32 = 0x5000;
/// gadtools.library's 19 vectors fit below 0x7000, above the screen block (screen.rs, 0x6400-0x65FF).
pub const GADTOOLS_BASE: u32 = 0x7000;
pub const EXIT_SLOT: u32 = 0x500;
/// Bases for libraries opened on demand (OpenLibrary of a known name), each in its own 4 KiB stride: the largest
/// known table (graphics, 186 functions) needs ~1.2 KiB below its base, the Library struct 34 bytes above it.
pub const DYNAMIC_LIB_BASES: u32 = 0x4_0000;
const DYNAMIC_LIB_STRIDE: u32 = 0x1000;
const DYNAMIC_LIB_END: u32 = 0x8_0000;
pub const LOAD_BASE: u32 = 0x10000;
pub const STACK_TOP: u32 = 0x20_0000;
/// The program's stack: AmigaDOS's default CLI stack is 4 KiB; real programs ask for more, so 64 KiB.
pub const STACK_SIZE: u32 = 0x1_0000;
/// The guest heap: everything above the stack, to the end of the 16 MiB guest RAM.
pub const HEAP_START: u32 = STACK_TOP;
pub const HEAP_END: u32 = SYS_ARENA_START;
/// The engine's own arena, the top 256 KiB of guest RAM: structs the ENGINE keeps for the guest (the DOS list and its
/// locks) live here so that publishing them never moves the addresses the program's own allocations get.
pub const SYS_ARENA_START: u32 = 0x100_0000 - 0x4_0000;
pub const SYS_ARENA_END: u32 = 0x100_0000;
/// The CLI argument line a program gets when nothing is passed: AmigaDOS always ends it with a newline.
pub const EMPTY_ARGS: &str = "\n";

pub struct LibInfo { pub name: &'static str, pub base: u32, pub open_count: u32 }

pub struct Amiga<U: GuestUi> {
    pub mem: GuestMem,
    pub ui: U,
    pub libs: Vec<LibInfo>,
    /// Open guest windows: guest struct Window address -> the native window.
    pub windows: BTreeMap<u32, UiObjectRef>,
    dispatch: Dispatch<U>,
    /// One line per successful bridged call, in order.
    pub log: Vec<String>,
    /// Refusals (bad guest pointers, unimplemented LVOs ...), in order.
    pub refused: Vec<String>,
    /// Census mode: the first call to an unimplemented function halts the guest (the run's last refusal names it).
    pub stop_on_unimplemented: bool,
    /// The guest heap AllocMem/AllocVec serve from (bookkeeping host-side).
    pub heap: memory::Heap,
    /// The engine's own arena (see SYS_ARENA_START).
    pub sys_heap: memory::Heap,
    /// Forbid nesting depth (counted, not enforced: one task).
    pub forbid: u32,
    /// What the program wrote to its console.
    pub console: dos::Console,
    /// Outstanding LockPubScreen locks on the one public screen.
    pub screen_locks: u32,
    /// Live ReadArgs results: (RDArgs, value buffer, its size, RDArgs allocated by ReadArgs).
    pub rdargs: Vec<(u32, u32, u32, bool)>,
    /// GadTools gadgets by guest address (gadtools.rs).
    pub gadgets: BTreeMap<u32, gadtools::GadgetInfo>,
    /// Each open window's current menu strip (first Menu), and the blocks of every CreateMenusA result by its first menu.
    pub menu_strips: BTreeMap<u32, u32>,
    pub menu_allocs: BTreeMap<u32, Vec<(u32, u32)>>,
    /// What a blocked guest waits for (exec_wait.rs); None = runnable.
    pub waiting: Option<exec_wait::Waiting>,
    /// The virtual clock, microseconds (timer.rs), and the pending TR_ADDREQUEST timers: (deadline, IORequest).
    pub clock_us: u64,
    pub timers: Vec<(u64, u32)>,
    /// Messages the ENGINE sent (IntuiMessages): address -> delivered to the guest (GetMsg) yet?
    pub engine_msgs: BTreeMap<u32, bool>,
    /// The running guest's CPU context (run.rs).
    pub(crate) ctx: Option<retro_m68k::CpuContext>,
    /// exec memory pools: handle -> (requirements, live blocks address -> size).
    pub pools: BTreeMap<u32, (u32, BTreeMap<u32, u32>)>,
    /// The guest's filesystem and what hangs off it: locks, files, seglists, disk libraries (fs.rs).
    pub fs: crate::fs::FsState,
    /// Guest calls in flight, innermost last (guest_call.rs).
    pub(crate) calls: Vec<crate::guest_call::Frame<U>>,
    /// SignalSemaphores by guest address -> the one task's nest count; the public list by name (semaphores.rs).
    pub semaphores: BTreeMap<u32, u16>,
    pub public_semaphores: BTreeMap<String, u32>,
    /// BOOPSI/MUI objects (boopsi.rs, mui/).
    pub objects: crate::boopsi::Objects,
    /// Functions the census runner was told to pretend exist (`--stub lib/Func`, census.rs).
    pub census_stubs: Vec<(String, String)>,
    next_lib_base: u32,
}

pub struct RunReport { pub stats: RunStats, pub loaded: Loaded }

impl<U: GuestUi> Amiga<U> {
    pub fn new(ui: U) -> Result<Self, Refusal> {
        let mut a = Amiga {
            mem: GuestMem::new(DEFAULT_SIZE), ui, libs: vec![], windows: BTreeMap::new(),
            dispatch: BTreeMap::new(), log: vec![], refused: vec![],
            stop_on_unimplemented: false, next_lib_base: DYNAMIC_LIB_BASES, heap: memory::Heap::new(HEAP_START, HEAP_END), sys_heap: memory::Heap::new(SYS_ARENA_START, SYS_ARENA_END), forbid: 0, console: Default::default(), screen_locks: 0, rdargs: vec![], gadgets: BTreeMap::new(), menu_strips: BTreeMap::new(), menu_allocs: BTreeMap::new(),
            waiting: None, clock_us: 0, timers: vec![], engine_msgs: BTreeMap::new(), ctx: None, pools: BTreeMap::new(), semaphores: BTreeMap::new(), public_semaphores: BTreeMap::new(), objects: Default::default(),
            fs: Default::default(), calls: vec![], census_stubs: vec![],
        };
        LibraryBuilder::new("exec.library", EXEC_BASE)
            .func(exec::LVO_OPEN_LIBRARY, "OpenLibrary", exec::open_library::<U>)
            .func(exec::LVO_CLOSE_LIBRARY, "CloseLibrary", exec::close_library::<U>)
            .func(process::LVO_FIND_TASK, "FindTask", process::find_task::<U>)
            .func(exec::LVO_OLD_OPEN_LIBRARY, "OldOpenLibrary", exec::old_open_library::<U>)
            .func(exec::LVO_ALERT, "Alert", exec::alert::<U>)
            .func(memory::LVO_ALLOC_MEM, "AllocMem", memory::alloc_mem::<U>)
            .func(memory::LVO_FREE_MEM, "FreeMem", memory::free_mem::<U>)
            .func(memory::LVO_ALLOC_VEC, "AllocVec", memory::alloc_vec::<U>)
            .func(memory::LVO_FREE_VEC, "FreeVec", memory::free_vec::<U>)
            .func(memory::LVO_AVAIL_MEM, "AvailMem", memory::avail_mem::<U>)
            .func(memory::LVO_COPY_MEM, "CopyMem", memory::copy_mem::<U>)
            .func(memory::LVO_FORBID, "Forbid", memory::forbid::<U>)
            .func(memory::LVO_PERMIT, "Permit", memory::permit::<U>)
            .func(memory::LVO_SET_SIGNAL, "SetSignal", memory::set_signal::<U>)
            .func(memory::LVO_STACK_SWAP, "StackSwap", memory::stack_swap::<U>)
            .func(memory::LVO_CREATE_POOL, "CreatePool", memory::create_pool::<U>)
            .func(memory::LVO_DELETE_POOL, "DeletePool", memory::delete_pool::<U>)
            .func(memory::LVO_ALLOC_POOLED, "AllocPooled", memory::alloc_pooled::<U>)
            .func(memory::LVO_FREE_POOLED, "FreePooled", memory::free_pooled::<U>)
            .func(ports::LVO_ALLOC_SIGNAL, "AllocSignal", ports::alloc_signal::<U>)
            .func(ports::LVO_FREE_SIGNAL, "FreeSignal", ports::free_signal::<U>)
            .func(ports::LVO_CREATE_MSG_PORT, "CreateMsgPort", ports::create_msg_port::<U>)
            .func(ports::LVO_DELETE_MSG_PORT, "DeleteMsgPort", ports::delete_msg_port::<U>)
            .func(exec_wait::LVO_SIGNAL, "Signal", exec_wait::signal::<U>)
            .func(exec_wait::LVO_WAIT, "Wait", exec_wait::wait::<U>)
            .func(exec_wait::LVO_PUT_MSG, "PutMsg", exec_wait::put_msg_lvo::<U>)
            .func(exec_wait::LVO_GET_MSG, "GetMsg", exec_wait::get_msg::<U>)
            .func(exec_wait::LVO_REPLY_MSG, "ReplyMsg", exec_wait::reply_msg::<U>)
            .func(exec_wait::LVO_WAIT_PORT, "WaitPort", exec_wait::wait_port::<U>)
            .func(timer::LVO_DO_IO, "DoIO", timer::do_io::<U>)
            .func(timer::LVO_SEND_IO, "SendIO", timer::send_io::<U>)
            .func(timer::LVO_CHECK_IO, "CheckIO", timer::check_io::<U>)
            .func(timer::LVO_WAIT_IO, "WaitIO", timer::wait_io::<U>)
            .func(timer::LVO_ABORT_IO, "AbortIO", timer::abort_io::<U>)
            .func(memheader::LVO_ALLOCATE, "Allocate", memheader::allocate_lvo::<U>)
            .func(memheader::LVO_DEALLOCATE, "Deallocate", memheader::deallocate_lvo::<U>)
            .func(lists::LVO_INSERT, "Insert", lists::insert::<U>)
            .func(lists::LVO_ADD_HEAD, "AddHead", lists::add_head::<U>)
            .func(lists::LVO_ADD_TAIL, "AddTail", lists::add_tail::<U>)
            .func(lists::LVO_REMOVE, "Remove", lists::remove::<U>)
            .func(lists::LVO_REM_HEAD, "RemHead", lists::rem_head::<U>)
            .func(lists::LVO_REM_TAIL, "RemTail", lists::rem_tail::<U>)
            .func(lists::LVO_ENQUEUE, "Enqueue", lists::enqueue::<U>)
            .func(lists::LVO_FIND_NAME, "FindName", lists::find_name::<U>)
            .func(semaphores::LVO_FIND_SEMAPHORE, "FindSemaphore", semaphores::find::<U>)
            .func(semaphores::LVO_INIT_SEMAPHORE, "InitSemaphore", semaphores::init::<U>)
            .func(semaphores::LVO_OBTAIN_SEMAPHORE, "ObtainSemaphore", semaphores::obtain::<U>)
            .func(semaphores::LVO_RELEASE_SEMAPHORE, "ReleaseSemaphore", semaphores::release::<U>)
            .func(semaphores::LVO_ATTEMPT_SEMAPHORE, "AttemptSemaphore", semaphores::attempt::<U>)
            .func(semaphores::LVO_OBTAIN_SEMAPHORE_SHARED, "ObtainSemaphoreShared", semaphores::obtain_shared::<U>)
            .func(semaphores::LVO_ATTEMPT_SEMAPHORE_SHARED, "AttemptSemaphoreShared", semaphores::attempt_shared::<U>)
            .func(semaphores::LVO_ADD_SEMAPHORE, "AddSemaphore", semaphores::add::<U>)
            .func(semaphores::LVO_REM_SEMAPHORE, "RemSemaphore", semaphores::rem::<U>)
            .func(ports::LVO_CREATE_IO_REQUEST, "CreateIORequest", ports::create_io_request::<U>)
            .func(ports::LVO_DELETE_IO_REQUEST, "DeleteIORequest", ports::delete_io_request::<U>)
            .func(devices::LVO_OPEN_DEVICE, "OpenDevice", devices::open_device::<U>)
            .func(devices::LVO_CLOSE_DEVICE, "CloseDevice", devices::close_device::<U>)
            .with(crate::makelib::register::<U>)
            .with(crate::execlists::register::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("dos.library", DOS_BASE)
            .func(dos::LVO_READ, "Read", dos::read::<U>)
            .func(dos::LVO_WRITE, "Write", dos::write::<U>)
            .func(dos::LVO_INPUT, "Input", dos::input::<U>)
            .func(dos::LVO_OUTPUT, "Output", dos::output::<U>)
            .func(dos::LVO_IO_ERR, "IoErr", dos::io_err::<U>)
            .func(dos::LVO_DELAY, "Delay", dos::delay::<U>)
            .func(dos::LVO_FPUTS, "FPuts", dos::fputs::<U>)
            .func(dos::LVO_FLUSH, "Flush", dos::flush::<U>)
            .func(dos::LVO_SET_IO_ERR, "SetIoErr", dos::set_io_err::<U>)
            .func(dos::LVO_CLI, "Cli", dos::cli::<U>)
            .func(dos::LVO_GET_PROGRAM_NAME, "GetProgramName", dos::get_program_name::<U>)
            .func(dos::LVO_PUT_STR, "PutStr", dos::put_str::<U>)
            .func(dos::LVO_IS_INTERACTIVE, "IsInteractive", dos::is_interactive::<U>)
            .func(rdargs::LVO_READ_ARGS, "ReadArgs", rdargs::read_args::<U>)
            .func(rdargs::LVO_FREE_ARGS, "FreeArgs", rdargs::free_args::<U>)
            .with(crate::dos_fs::register::<U>)
            .with(crate::dos2::register::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("intuition.library", INTUITION_BASE)
            .func(window::LVO_OPEN_WINDOW, "OpenWindow", window::open_window::<U>)
            .func(window::LVO_OPEN_WINDOW_TAG_LIST, "OpenWindowTagList", window::open_window_tag_list::<U>)
            .func(window::LVO_CLOSE_WINDOW, "CloseWindow", window::close_window::<U>)
            .func(intuition::LVO_SET_MENU_STRIP, "SetMenuStrip", intuition::set_menu_strip::<U>)
            .func(intuition::LVO_CLEAR_MENU_STRIP, "ClearMenuStrip", intuition::clear_menu_strip::<U>)
            .func(idcmp::LVO_MODIFY_IDCMP, "ModifyIDCMP", idcmp::modify_idcmp::<U>)
            .func(menu_nav::LVO_ITEM_ADDRESS, "ItemAddress", menu_nav::item_address::<U>)
            .func(menu_nav::LVO_ON_MENU, "OnMenu", menu_nav::on_menu::<U>)
            .func(menu_nav::LVO_OFF_MENU, "OffMenu", menu_nav::off_menu::<U>)
            .func(menu_nav::LVO_RESET_MENU_STRIP, "ResetMenuStrip", intuition::set_menu_strip::<U>)
            .func(intuition::LVO_ACTIVATE_WINDOW, "ActivateWindow", intuition::activate_window::<U>)
            .func(intuition::LVO_WINDOW_TO_FRONT, "WindowToFront", intuition::window_to_front::<U>)
            .func(intuition::LVO_WINDOW_TO_BACK, "WindowToBack", intuition::window_to_back::<U>)
            .func(intuition::LVO_SET_WINDOW_TITLES, "SetWindowTitles", intuition::set_window_titles::<U>)
            .func(intuition::LVO_REFRESH_GADGETS, "RefreshGadgets", intuition::refresh_gadgets::<U>)
            .func(intuition::LVO_REFRESH_GLIST, "RefreshGList", intuition::refresh_gadgets::<U>)
            .func(screen::LVO_LOCK_PUB_SCREEN, "LockPubScreen", screen::lock_pub_screen::<U>)
            .func(screen::LVO_UNLOCK_PUB_SCREEN, "UnlockPubScreen", screen::unlock_pub_screen::<U>)
            .func(crate::boopsi::LVO_NEW_OBJECT_A, "NewObjectA", crate::boopsi::new_object_a::<U>)
            .func(crate::boopsi::LVO_DISPOSE_OBJECT, "DisposeObject", crate::boopsi::dispose_object::<U>)
            .func(crate::boopsi::LVO_SET_ATTRS_A, "SetAttrsA", crate::boopsi::set_attrs_a::<U>)
            .func(crate::boopsi::LVO_GET_ATTR, "GetAttr", crate::boopsi::get_attr::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("gadtools.library", GADTOOLS_BASE)
            .func(gadtools::LVO_CREATE_GADGET_A, "CreateGadgetA", gadtools::create_gadget::<U>)
            .func(gadtools::LVO_FREE_GADGETS, "FreeGadgets", gadtools::free_gadgets::<U>)
            .func(gadtools::LVO_GT_SET_GADGET_ATTRS_A, "GT_SetGadgetAttrsA", gt_kinds::set_gadget_attrs::<U>)
            .func(gt_kinds::LVO_GT_GET_GADGET_ATTRS_A, "GT_GetGadgetAttrsA", gt_kinds::get_gadget_attrs::<U>)
            .func(gt_menus::LVO_CREATE_MENUS_A, "CreateMenusA", gt_menus::create_menus::<U>)
            .func(gt_menus::LVO_FREE_MENUS, "FreeMenus", gt_menus::free_menus::<U>)
            .func(gt_menus::LVO_LAYOUT_MENU_ITEMS_A, "LayoutMenuItemsA", gt_menus::layout_menu_items::<U>)
            .func(gt_menus::LVO_LAYOUT_MENUS_A, "LayoutMenusA", gt_menus::layout_menus::<U>)
            .func(gadtools::LVO_GT_GET_IMSG, "GT_GetIMsg", gadtools::get_imsg::<U>)
            .func(gadtools::LVO_GT_REPLY_IMSG, "GT_ReplyIMsg", gadtools::reply_imsg::<U>)
            .func(gadtools::LVO_GT_REFRESH_WINDOW, "GT_RefreshWindow", gadtools::refresh::<U>)
            .func(gadtools::LVO_GT_BEGIN_REFRESH, "GT_BeginRefresh", gadtools::refresh::<U>)
            .func(gadtools::LVO_GT_END_REFRESH, "GT_EndRefresh", gadtools::refresh::<U>)
            .func(gadtools::LVO_CREATE_CONTEXT, "CreateContext", gadtools::create_context::<U>)
            .func(gadtools::LVO_GET_VISUAL_INFO_A, "GetVisualInfoA", gadtools::get_visual_info::<U>)
            .func(gadtools::LVO_FREE_VISUAL_INFO, "FreeVisualInfo", gadtools::free_visual_info::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        crate::boopsi::install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("graphics.library", GRAPHICS_BASE)
            .func(graphics::LVO_TEXT_LENGTH, "TextLength", graphics::text_length::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        for (name, base) in [("exec.library", EXEC_BASE), ("dos.library", DOS_BASE), ("intuition.library", INTUITION_BASE), ("graphics.library", GRAPHICS_BASE), ("gadtools.library", GADTOOLS_BASE)] {
            a.libs.push(LibInfo { name, base, open_count: 0 });
        }
        crate::execlists::init(&mut a.mem)?;
        crate::doslist::init(&mut a)?;
        crate::guest_call::install(&mut a.mem)?;
        a.dispatch.insert(crate::guest_call::RET_SLOT, ("exec.library", "GuestCallReturn", crate::guest_call::on_return::<U>));
        write_slot(&mut a.mem, EXIT_SLOT).map_err(Refusal::mem("exit slot"))?;
        a.mem.write_u32(4, EXEC_BASE).map_err(Refusal::mem("AbsExecBase"))?; // AbsExecBase
        Ok(a)
    }

    /// Guest calls in flight (guest_call.rs): 0 when no host-started guest call is outstanding.
    pub fn calls_in_flight(&self) -> usize { self.calls.len() }

    /// Open a library the name table knows but nothing bridges yet: a fresh base, every vector a named refusal.
    pub(crate) fn open_known_library(&mut self, name: &'static str) -> Result<u32, Refusal> {
        let base = self.next_lib_base;
        if base >= DYNAMIC_LIB_END { return Err(Refusal(format!("OpenLibrary {name}: no library base left (NOMEM)"))); }
        self.next_lib_base += DYNAMIC_LIB_STRIDE;
        let lib = if name == "muimaster.library" { crate::mui::library(base) } else { crate::libfactory::library(name, base) };
        lib.install(&mut self.mem, &mut self.dispatch)?;
        self.libs.push(LibInfo { name, base, open_count: 1 });
        Ok(base)
    }
}

impl<U: GuestUi> Host for Amiga<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }

    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool {
        if pc == EXIT_SLOT { cpu.halt(); return true; }
        let Some((lib, func, handler)) = self.dispatch.get(&pc).copied() else { return false };
        let handler = if self.census_stubs.iter().any(|(l, f)| l == lib && f == func) { crate::census::stub::<U> } else { handler };
        match handler(self, cpu) {
            Ok(line) => self.log.push(line),
            Err(Refusal(e)) => {
                cpu.set_d0(0); // NULL / FALSE
                self.log.push(format!("[{lib}] {func} REFUSED: {e}"));
                self.refused.push(format!("{lib}/{func}: {e}"));
            }
        }
        true
    }
}
