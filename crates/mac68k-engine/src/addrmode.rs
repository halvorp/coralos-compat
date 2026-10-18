//! 24-bit versus 32-bit addressing (Inside Macintosh: Memory, "24-bit and 32-bit addressing").
//!
//! A 68000-era Mac has 24 address lines: the bus ignores the top byte of every address. Applications of that era (MacWrite
//! 4.5 hands `_OpenPort` the pointer `$8020A9FA`) keep flags in the high byte of pointers and handles, and the Memory
//! Manager itself keeps the lock/purge/resource bits there in a master pointer ([`crate::memmgr`]). So the engine's machine
//! starts in [`AddrMode::Bit24`]: [`retro_guest_mem::GuestMem`] drops the top byte of every access (the 68k bus and the host
//! handlers alike), `_StripAddress` removes it, and master pointers carry the flag bits. A 32-bit-clean application selects
//! [`AddrMode::Bit32`] (the machine option, or its own `_SwapMMUMode`): the full address is significant and master
//! pointers are bare addresses. The PowerPC personality is always 32-bit.
use crate::lowmem::MMU_32_BIT;
use crate::machine::Mac;
use crate::osconv::Regs;
use crate::Refusal;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use retro_guest_mem::{MASK_24, MASK_32};
use retro_m68k::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrMode { Bit24, Bit32 }

impl AddrMode {
    pub const fn mask(self) -> u32 { match self { AddrMode::Bit24 => MASK_24, AddrMode::Bit32 => MASK_32 } }
}

impl<U: GuestUi> Mac<U> {
    pub fn address_mode(&self) -> AddrMode { if self.mem.addr_mask() == MASK_32 { AddrMode::Bit32 } else { AddrMode::Bit24 } }

    /// Switch the machine's addressing mode: the bus mask, the `MMU32Bit` low-memory byte, and every master pointer.
    pub fn set_address_mode(&mut self, mode: AddrMode) -> Result<(), Refusal> {
        self.mem.set_addr_mask(mode.mask());
        self.mem.write_u8(MMU_32_BIT, (mode == AddrMode::Bit32) as u8).map_err(Refusal::mem("MMU32Bit"))?;
        self.mm.sync_all_masters(&mut self.mem);
        Ok(())
    }
}

pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    m.register_os(0xA05D, swap_mmu_mode::<U>)
}

/// `_SwapMMUMode`: D0.B = the mode to switch to (0 = 24-bit, 1 = 32-bit); on return D0.B = the previous mode.
fn swap_mmu_mode<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, r: &mut Regs) -> Result<String, Refusal> {
    let was = (m.address_mode() == AddrMode::Bit32) as u32;
    let want = r.d0 & 0xFF;
    let mode = match want { 0 => AddrMode::Bit24, 1 => AddrMode::Bit32, other => return Err(Refusal(format!("SwapMMUMode: mode {other} is neither 0 (24-bit) nor 1 (32-bit)"))) };
    m.set_address_mode(mode)?;
    r.d0 = (r.d0 & !0xFF) | was;
    Ok(format!("[mem] SwapMMUMode {was} -> {want}"))
}
