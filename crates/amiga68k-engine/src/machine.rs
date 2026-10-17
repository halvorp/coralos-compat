//! The Amiga machine: guest RAM + fake ExecBase + dispatch table + a [`GuestUi`].
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::ToString;
use alloc::string::String;
use alloc::format;
use alloc::collections::BTreeMap;
use crate::libtable::{write_slot, Dispatch, LibraryBuilder};
use crate::{exec, intuition, Refusal};
use guestui_abi::{GuestUi, UiObjectRef};
use retro_guest_mem::{GuestMem, DEFAULT_SIZE};
use retro_hunk::Loaded;
use retro_m68k::{Cpu, Host, RunStats};

pub const EXEC_BASE: u32 = 0x1000;
pub const INTUITION_BASE: u32 = 0x2000;
pub const GRAPHICS_BASE: u32 = 0x3000;
pub const EXIT_SLOT: u32 = 0x500;
pub const LOAD_BASE: u32 = 0x10000;
pub const STACK_TOP: u32 = 0x20_0000;
const FIRST_HANDLE: u32 = 0x8000;

pub struct LibInfo { pub name: &'static str, pub base: u32, pub open_count: u32 }

pub struct Amiga<U: GuestUi> {
    pub mem: GuestMem,
    pub ui: U,
    pub libs: Vec<LibInfo>,
    pub(crate) windows: BTreeMap<u32, UiObjectRef>,
    dispatch: Dispatch<U>,
    next_handle: u32,
    /// One line per successful bridged call, in order.
    pub log: Vec<String>,
    /// Refusals (bad guest pointers, unimplemented LVOs ...), in order.
    pub refused: Vec<String>,
}

pub struct RunReport { pub stats: RunStats, pub loaded: Loaded }

impl<U: GuestUi> Amiga<U> {
    pub fn new(ui: U) -> Result<Self, Refusal> {
        let mut a = Amiga {
            mem: GuestMem::new(DEFAULT_SIZE), ui, libs: vec![], windows: BTreeMap::new(),
            dispatch: BTreeMap::new(), next_handle: FIRST_HANDLE, log: vec![], refused: vec![],
        };
        LibraryBuilder::new("exec.library", EXEC_BASE)
            .func(exec::LVO_OPEN_LIBRARY, "OpenLibrary", exec::open_library::<U>)
            .func(exec::LVO_CLOSE_LIBRARY, "CloseLibrary", exec::close_library::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("intuition.library", INTUITION_BASE)
            .func(intuition::LVO_OPEN_WINDOW, "OpenWindow", intuition::open_window::<U>)
            .func(intuition::LVO_CLOSE_WINDOW, "CloseWindow", intuition::close_window::<U>)
            .func(intuition::LVO_SET_MENU_STRIP, "SetMenuStrip", intuition::set_menu_strip::<U>)
            .func(intuition::LVO_CLEAR_MENU_STRIP, "ClearMenuStrip", intuition::clear_menu_strip::<U>)
            .install(&mut a.mem, &mut a.dispatch)?;
        LibraryBuilder::new("graphics.library", GRAPHICS_BASE)
            .stub(-240, "Move")
            .install(&mut a.mem, &mut a.dispatch)?;
        for (name, base) in [("exec.library", EXEC_BASE), ("intuition.library", INTUITION_BASE), ("graphics.library", GRAPHICS_BASE)] {
            a.libs.push(LibInfo { name, base, open_count: 0 });
        }
        write_slot(&mut a.mem, EXIT_SLOT).map_err(Refusal::mem("exit slot"))?;
        a.mem.write_u32(4, EXEC_BASE).map_err(Refusal::mem("AbsExecBase"))?; // AbsExecBase
        Ok(a)
    }

    pub(crate) fn alloc_handle(&mut self) -> u32 {
        let h = self.next_handle;
        self.next_handle += 0x40;
        h
    }

    /// Load a Hunk executable and run it to its final RTS (to the exit slot) or `max_insns`.
    pub fn run_hunk(&mut self, file: &[u8], max_insns: u64) -> Result<RunReport, String> {
        let loaded = retro_hunk::load(&mut self.mem, file, LOAD_BASE).map_err(|e| e.to_string())?;
        self.mem.write_u32(STACK_TOP - 4, EXIT_SLOT).map_err(|e| e.to_string())?; // return address for the final rts
        let stats = retro_m68k::run(self, loaded.entry, STACK_TOP - 4, max_insns);
        Ok(RunReport { stats, loaded })
    }
}

impl<U: GuestUi> Host for Amiga<U> {
    fn mem(&mut self) -> &mut GuestMem { &mut self.mem }

    fn illegal(&mut self, cpu: &mut Cpu, pc: u32) -> bool {
        if pc == EXIT_SLOT { cpu.halt(); return true; }
        let Some((lib, func, handler)) = self.dispatch.get(&pc).copied() else { return false };
        match handler(self, cpu) {
            Ok(line) => self.log.push(line),
            Err(Refusal(e)) => {
                cpu.set_d0(0); // NULL / FALSE
                self.log.push(format!("[{lib}] {func} REFUSED: {e}"));
                self.refused.push(e);
            }
        }
        true
    }
}
