//! Stateless Carbon Scrap Manager (`Scrap.h`, CarbonLib 1.0+) over the Mac host-backed scrap adapter
//! (`heirloom-mac68k-engine::mac_scrap` -> CoralOS `clipboardd`, REUSE U8).
//!
//! `GetCurrentScrap(&scrap)` syncs the host clipboard generation (C12 Watch / doorbell backed, zero polling)
//! and hands out a `ScrapRef` stamped with `scrapCount`. `GetScrapFlavorSize`, `GetScrapFlavorData`,
//! `ClearCurrentScrap`, and `PutScrapFlavor` delegate directly through `Mac::scrap_*`. A reference taken before
//! `ZeroScrap`, `ClearCurrentScrap`, or a native CoralOS copy is refused with `paramErr` (-50).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_mac68k_engine::scrap::{NO_SCRAP_ERR, NO_TYPE_ERR};
use heirloom_ui_abi::GuestUi;

/// A ScrapRef: this tag in the high half, the scrap's `scrapCount` in the low half.
const REF_TAG: u32 = 0x5343_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrapFn {
    GetCurrentScrap,
    GetScrapFlavorSize,
    GetScrapFlavorData,
    ClearCurrentScrap,
    PutScrapFlavor,
}

impl ScrapFn {
    pub fn by_name(name: &str) -> Option<ScrapFn> {
        Some(match name {
            "GetCurrentScrap" => ScrapFn::GetCurrentScrap,
            "GetScrapFlavorSize" => ScrapFn::GetScrapFlavorSize,
            "GetScrapFlavorData" => ScrapFn::GetScrapFlavorData,
            "ClearCurrentScrap" => ScrapFn::ClearCurrentScrap,
            "PutScrapFlavor" => ScrapFn::PutScrapFlavor,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn current_scrap_ref(&self) -> u32 {
        REF_TAG | u32::from(self.mac.scrap.count as u16)
    }

    pub(crate) fn scrap_call(&mut self, f: ScrapFn, regs: &mut [u32; 32]) -> Result<(), String> {
        let (a0, a1, a2, a3) = (regs[3], regs[4], regs[5], regs[6]);
        let e: i32 = match f {
            ScrapFn::GetCurrentScrap if a0 == 0 => -50,
            ScrapFn::GetCurrentScrap => {
                if let Err(err) = self.mac.scrap_sync_from_host() {
                    self.mac.note_refusal("GetCurrentScrap", format!("host clipboard {}", err.name()));
                    NO_SCRAP_ERR
                } else {
                    let r = self.current_scrap_ref();
                    self.mac.mem.write_u32(a0, r).map_err(|_| format!("GetCurrentScrap: {a0:#x} leaves guest RAM"))?;
                    0
                }
            }
            ScrapFn::ClearCurrentScrap => match self.mac.scrap_zero() {
                Ok(()) => 0,
                Err(err) => {
                    self.mac.note_refusal("ClearCurrentScrap", format!("host clipboard {}", err.name()));
                    NO_SCRAP_ERR
                }
            },
            ScrapFn::GetScrapFlavorSize => {
                let _ = self.mac.scrap_sync_from_host();
                if a0 != self.current_scrap_ref() || a2 == 0 {
                    self.mac.log.push(format!("[scrap] GetScrapFlavorSize: {a0:#x} is not the current scrap's reference (or no size pointer) -> paramErr"));
                    -50
                } else {
                    match self.mac.scrap_get(a1.to_be_bytes()) {
                        Ok(Some(d)) => {
                            let n = d.len() as u32;
                            self.mac.mem.write_u32(a2, n).map_err(|_| format!("GetScrapFlavorSize: {a2:#x} leaves guest RAM"))?;
                            0
                        }
                        Ok(None) => NO_TYPE_ERR,
                        Err(err) => {
                            self.mac.note_refusal("GetScrapFlavorSize", format!("host clipboard {}", err.name()));
                            NO_SCRAP_ERR
                        }
                    }
                }
            }
            ScrapFn::GetScrapFlavorData => {
                let _ = self.mac.scrap_sync_from_host();
                if a0 != self.current_scrap_ref() || a2 == 0 {
                    self.mac.log.push(format!("[scrap] GetScrapFlavorData: {a0:#x} is not the current scrap's reference (or no ioByteCount) -> paramErr"));
                    -50
                } else {
                    let cap = self.mac.mem.read_u32(a2).map_err(|_| format!("GetScrapFlavorData: {a2:#x} leaves guest RAM"))?;
                    match self.mac.scrap_get(a1.to_be_bytes()) {
                        Ok(Some(d)) => {
                            let take = d.len().min(cap as usize);
                            if a3 != 0 && take > 0 {
                                self.mac.mem.write_bytes(a3, &d[..take]).map_err(|_| format!("GetScrapFlavorData: {a3:#x} leaves guest RAM"))?;
                            }
                            self.mac.mem.write_u32(a2, take as u32).map_err(|_| format!("GetScrapFlavorData: {a2:#x} leaves guest RAM"))?;
                            0
                        }
                        Ok(None) => NO_TYPE_ERR,
                        Err(err) => {
                            self.mac.note_refusal("GetScrapFlavorData", format!("host clipboard {}", err.name()));
                            NO_SCRAP_ERR
                        }
                    }
                }
            }
            ScrapFn::PutScrapFlavor => {
                // PutScrapFlavor(scrap, flavorType, flavorFlags, flavorSize, flavorData)
                let (flavor_size, flavor_data) = (regs[6], regs[7]);
                if a0 != self.current_scrap_ref() || (flavor_size > 0 && flavor_data == 0) {
                    -50
                } else {
                    let bytes = if flavor_size == 0 {
                        alloc::vec::Vec::new()
                    } else {
                        self.mac.mem.span(flavor_data, flavor_size)
                            .and_then(|s| self.mac.mem.bytes(s).map(<[u8]>::to_vec))
                            .map_err(|_| format!("PutScrapFlavor: {flavor_data:#x}..+{flavor_size} leaves guest RAM"))?
                    };
                    match self.mac.scrap_put(a1.to_be_bytes(), bytes) {
                        Ok(()) => 0,
                        Err(err) => {
                            self.mac.note_refusal("PutScrapFlavor", format!("host clipboard {}", err.name()));
                            NO_SCRAP_ERR
                        }
                    }
                }
            }
        };
        self.mac.log.push(format!("[scrap] {f:?} -> {e}"));
        regs[3] = e as u32;
        Ok(())
    }
}
