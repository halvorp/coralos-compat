//! AppleTalk on a machine that has none: the Heirloom Mac is one machine with no AppleTalk network (no .MPP driver, no LocalTalk/EtherTalk port).
//! `MPPOpen` answers what Mac OS answers with AppleTalk turned off in the Chooser: portNotCf (-98, "driver Open error, port not configured for this
//! connection"; Inside Macintosh: Networking, AppleTalk Utilities result codes), and logs it by name. A program checking for a network (Photoshop 7's
//! serial-number check) learns the truth and goes on without one. The rest of AppleTalk (NBP, ATP, PPC Toolbox) stays unbound: a program that calls
//! it after MPPOpen failed stops by name.
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

/// portNotCf: the AppleTalk port is not configured (AppleTalk inactive).
pub const PORT_NOT_CF: i32 = -98;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtFn { MPPOpen }

impl AtFn {
    pub fn by_name(name: &str) -> Option<AtFn> { match name { "MPPOpen" => Some(AtFn::MPPOpen), _ => None } }
}

impl<U: GuestUi> MacPpc<U> {
    pub(crate) fn at_call(&mut self, f: AtFn, cpu: &mut Cpu) -> Result<(), String> {
        match f {
            AtFn::MPPOpen => {
                self.mac.log.push(format!("[appletalk] MPPOpen -> portNotCf ({PORT_NOT_CF}): this machine has no AppleTalk (inactive, as with AppleTalk off in the Chooser)"));
                cpu.gpr[3] = PORT_NOT_CF as u32;
            }
        }
        Ok(())
    }
}
