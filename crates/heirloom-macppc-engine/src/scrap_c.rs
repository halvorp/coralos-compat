//! The Carbon Scrap Manager (Scrap.h, CarbonLib 1.0+) over the ONE desk scrap the classic calls keep (heirloom-mac68k-engine `scrap`):
//! `GetCurrentScrap(&scrap)` hands out a reference to the current scrap; `GetScrapFlavorSize(scrap, flavor, &size)` reads an entry's length,
//! noTypeErr (-102) when the scrap holds no such flavor. A reference names the scrap as it was when it was handed out: once the scrap is
//! cleared (ZeroScrap) an old reference is refused with paramErr (-50) and named in the log, never answered from the new scrap.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_mac68k_engine::scrap::NO_TYPE_ERR;
use heirloom_ui_abi::GuestUi;

/// A ScrapRef: this tag in the high half, the scrap's `scrapCount` in the low half.
const REF_TAG: u32 = 0x5343_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrapFn { GetCurrentScrap, GetScrapFlavorSize }

impl ScrapFn {
    pub fn by_name(name: &str) -> Option<ScrapFn> {
        Some(match name { "GetCurrentScrap" => ScrapFn::GetCurrentScrap, "GetScrapFlavorSize" => ScrapFn::GetScrapFlavorSize, _ => return None })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn current_scrap_ref(&self) -> u32 { REF_TAG | u32::from(self.mac.scrap.count as u16) }

    pub(crate) fn scrap_call(&mut self, f: ScrapFn, regs: &mut [u32; 32]) -> Result<(), String> {
        let (a0, a1, a2) = (regs[3], regs[4], regs[5]);
        let e: i32 = match f {
            ScrapFn::GetCurrentScrap if a0 == 0 => -50,
            ScrapFn::GetCurrentScrap => { let r = self.current_scrap_ref(); self.mac.mem.write_u32(a0, r).map_err(|_| format!("GetCurrentScrap: {a0:#x} leaves guest RAM"))?; 0 }
            ScrapFn::GetScrapFlavorSize if a0 != self.current_scrap_ref() || a2 == 0 => {
                self.mac.log.push(format!("[scrap] GetScrapFlavorSize: {a0:#x} is not the current scrap's reference (or no size pointer) -> paramErr"));
                -50
            }
            ScrapFn::GetScrapFlavorSize => match self.mac.scrap.get(a1.to_be_bytes()) {
                Some(d) => { let n = d.len() as u32; self.mac.mem.write_u32(a2, n).map_err(|_| format!("GetScrapFlavorSize: {a2:#x} leaves guest RAM"))?; 0 }
                None => NO_TYPE_ERR,
            },
        };
        self.mac.log.push(format!("[scrap] {f:?} -> {e}"));
        regs[3] = e as u32;
        Ok(())
    }
}
