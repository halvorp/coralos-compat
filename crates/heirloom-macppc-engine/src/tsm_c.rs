//! Text Services Manager documents (`NewTSMDocument`, `ActivateTSMDocument`, `DeactivateTSMDocument`, `DeleteTSMDocument`, `FixTSMDocument`, `UseInputWindow`) for a machine with NO input
//! method installed: the Roman keyboard types straight into the program's own key events, so there is never an inline input session to fix or an input window to show. A document is an ID
//! the program can create, activate, deactivate and delete (the bookkeeping a caller checks); `FixTSMDocument` has nothing pending and succeeds; `UseInputWindow(doc, TRUE)` succeeds
//! and logs, by name, that no input method will use it. A stale or unknown ID is `tsmInvalidDocIDErr`. The error number -2505 (tsmInvalidDocIDErr) is CONFIRMED by the Stage-A spec (spec: macos9-specs/TextServices/DeleteTSMDocument): it answers a NULL ID, -1, a disposed handle, an unregistered one and a record that is not 'TDOC'; deleting twice is -2505 the second time; activating twice is 0 both times. Not provided, so left unbound: `TSMGetInlineObject`, `SetDefaultInputMethod`, text-service components.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_ppc::Cpu;

const TSM_INVALID_DOC_ID: i32 = -2505;
const ID_BASE: u32 = 0x5453_4D00; // 'TSM\0' + n: easy to recognise in a dump

#[derive(Default)]
pub struct TsmState { docs: BTreeSet<u32>, active: Option<u32>, next: u32 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsmFn { NewTSMDocument, DeleteTSMDocument, ActivateTSMDocument, DeactivateTSMDocument, FixTSMDocument, UseInputWindow }

impl TsmFn {
    pub fn by_name(name: &str) -> Option<TsmFn> {
        use TsmFn::*;
        Some(match name {
            "NewTSMDocument" => NewTSMDocument, "DeleteTSMDocument" => DeleteTSMDocument, "ActivateTSMDocument" => ActivateTSMDocument,
            "DeactivateTSMDocument" => DeactivateTSMDocument, "FixTSMDocument" => FixTSMDocument, "UseInputWindow" => UseInputWindow,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn tsm_call(&mut self, f: TsmFn, cpu: &mut Cpu) -> Result<(), String> {
        use TsmFn::*;
        let (a0, a1, a2) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0));
        let known = |s: &Self, id: u32| s.tsm.docs.contains(&id);
        let r: i32 = match f {
            // NewTSMDocument(short numOfInterface, InterfaceTypeList types, TSMDocumentID *idocID, long refcon)
            NewTSMDocument => {
                if a2 == 0 { -50 } else {
                    self.tsm.next += 1;
                    let id = ID_BASE + self.tsm.next;
                    self.tsm.docs.insert(id);
                    self.mac.mem.write_u32(a2, id).map_err(|_| format!("NewTSMDocument: the result pointer {a2:#x} leaves guest RAM"))?;
                    let _ = (a0, a1);
                    0
                }
            }
            DeleteTSMDocument => if self.tsm.docs.remove(&a0) { if self.tsm.active == Some(a0) { self.tsm.active = None; } 0 } else { TSM_INVALID_DOC_ID },
            ActivateTSMDocument => if known(self, a0) { self.tsm.active = Some(a0); 0 } else { TSM_INVALID_DOC_ID },
            DeactivateTSMDocument => if known(self, a0) { if self.tsm.active == Some(a0) { self.tsm.active = None; } 0 } else { TSM_INVALID_DOC_ID },
            FixTSMDocument => if known(self, a0) { 0 } else { TSM_INVALID_DOC_ID },
            UseInputWindow => if known(self, a0) {
                if a1 & 0xFF != 0 { self.mac.log.push(format!("[tsm] UseInputWindow({a0:#x}, TRUE): no input method is installed, so no input window will ever be shown; keys arrive as ordinary key events")); }
                0
            } else { TSM_INVALID_DOC_ID },
        };
        cpu.gpr[3] = r as u32;
        Ok(())
    }
}
