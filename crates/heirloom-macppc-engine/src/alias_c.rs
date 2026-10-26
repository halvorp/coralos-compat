//! The Alias Manager by its C names (`NewAlias`, `ResolveAliasFile`, `ResolveAliasFileWithMountFlags` ...): argument-order adapters over the 68k `_AliasDispatch` bodies
//! (`heirloom_mac68k_engine::fm::alias`), so a PowerPC program and a 68k program resolve the same record on the same volume. The record layout is the Alias Manager's documented fixed part
//! (Inside Macintosh: Files 4-43) followed by the absolute-path tag, written by `make_alias` there. The `...WithMountFlags` forms take one more argument (`kResolveAliasFileNoUI`
//! etc.): this machine has no network volumes to mount and no UI, so the flags change nothing and are dropped. `FSResolveAlias` / `FSNewAlias` take an `FSRef`, which this engine does not
//! model (no HFS+ catalogue IDs): they stay unbound and stop by name.
use crate::machine::MacPpc;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::fm::alias::sel;

/// (export name, selector, C argument count; the selector takes the first `n_sel` of them)
pub const CALLS: &[(&str, u16, usize, usize)] = &[
    ("NewAlias", sel::NEW_ALIAS, 3, 3), ("NewAliasMinimal", sel::NEW_ALIAS_MINIMAL, 2, 2), ("NewAliasMinimalFromFullPath", sel::NEW_ALIAS_MINIMAL_FROM_FULL_PATH, 5, 5),
    ("ResolveAlias", sel::RESOLVE_ALIAS, 4, 4), ("ResolveAliasWithMountFlags", sel::RESOLVE_ALIAS, 5, 4), ("UpdateAlias", sel::UPDATE_ALIAS, 4, 4),
    ("GetAliasInfo", sel::GET_ALIAS_INFO, 3, 3), ("ResolveAliasFile", sel::RESOLVE_ALIAS_FILE, 4, 4), ("ResolveAliasFileWithMountFlags", sel::RESOLVE_ALIAS_FILE, 5, 4),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AliasCall(pub usize);
impl AliasCall {
    pub fn by_name(name: &str) -> Option<AliasCall> { CALLS.iter().position(|c| c.0 == name).map(AliasCall) }
    pub fn name(self) -> &'static str { CALLS[self.0].0 }
    pub fn arg_count(self) -> usize { CALLS[self.0].2 }
}

impl<U: GuestUi> MacPpc<U> {
    /// The OSErr for r3. Boolean arguments arrive zero-extended in registers; the 68k body reads them as bytes, so they are passed through unchanged.
    pub(crate) fn alias_call(&mut self, c: AliasCall, args: &[u32]) -> Result<u32, String> {
        let (name, selector, n, n_sel) = CALLS[c.0];
        if args.len() < n { return Err(alloc::format!("{name}: {n} arguments needed, {} given", args.len())); }
        heirloom_mac68k_engine::fm::alias::alias_value_call(&mut self.mac, selector, &args[..n_sel]).map(|e| e as i32 as u32).map_err(|e| e.0)
    }
}
