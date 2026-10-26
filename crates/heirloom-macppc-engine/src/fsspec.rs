//! The FSSpec File Manager calls (`FSMakeFSSpec`, `FSpOpenDF`, `FSpCreate`, `FSpGetFInfo` ...) as a PowerPC program calls them. The C prototypes
//! (Files.h) take their arguments in declaration order, the same order the 68k `_HighLevelFSDispatch` bodies read, so this is only the argument
//! count and the selector: the work is `heirloom_mac68k_engine::fm::fsspec` (one volume, one set of refnums for both CPUs).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::fm::fsspec::sel;

/// (export name, selector, argument count). `FSpOpenResFile` answers a refnum (-1 on failure); `FSpCreateResFile` is a procedure.
pub const CALLS: &[(&str, u16, usize)] = &[
    ("FSMakeFSSpec", sel::MAKE_FSSPEC, 4), ("FSpOpenDF", sel::OPEN_DF, 3), ("FSpOpenRF", sel::OPEN_RF, 3), ("FSpCreate", sel::CREATE, 4),
    ("FSpDirCreate", sel::DIR_CREATE, 3), ("FSpDelete", sel::DELETE, 1), ("FSpGetFInfo", sel::GET_FINFO, 2), ("FSpSetFInfo", sel::SET_FINFO, 2),
    ("FSpSetFLock", sel::SET_FLOCK, 1), ("FSpRstFLock", sel::RST_FLOCK, 1), ("FSpRename", sel::RENAME, 2), ("FSpCatMove", sel::CAT_MOVE, 2),
    ("FSpOpenResFile", sel::OPEN_RES_FILE, 2), ("FSpCreateResFile", sel::CREATE_RES_FILE, 4), ("FSpExchangeFiles", sel::EXCHANGE_FILES, 2),
];

/// A served FSSpec call: its index in [`CALLS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpecCall(pub usize);

impl SpecCall {
    pub fn by_name(name: &str) -> Option<SpecCall> { CALLS.iter().position(|c| c.0 == name).map(SpecCall) }
    pub fn name(self) -> &'static str { CALLS[self.0].0 }
    pub fn arg_count(self) -> usize { CALLS[self.0].2 }
}

impl<U: GuestUi> MacPpc<U> {
    /// Run `c` with `args`; the value for r3 (an OSErr, or the refnum for `FSpOpenResFile`).
    pub(crate) fn spec_call(&mut self, c: SpecCall, args: &[u32]) -> Result<u32, String> {
        let (name, selector, n) = CALLS[c.0];
        if args.len() < n { return Err(format!("{name}: {n} arguments needed, {} given", args.len())); }
        let r = self.mac.fsp_call(selector, &args[..n]);
        Ok(match (selector, r) {
            (sel::OPEN_RES_FILE, Ok(refnum)) => refnum as u16 as i16 as i32 as u32,
            (sel::OPEN_RES_FILE, Err(_)) => (-1i32) as u32,
            (_, Ok(_)) => 0,
            (_, Err(e)) => e as i32 as u32,
        })
    }
}
