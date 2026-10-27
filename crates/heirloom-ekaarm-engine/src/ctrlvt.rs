//! Engine-built controls: a `CCoeControl`-sized guest block with CCoeControl's primary and `MObjectProvider` vtables, so a class the
//! engine implements (title pane, menu bar, list box) can be called through any CCoeControl virtual. The slot numbers and names are
//! read off a real guest subclass's vtable (`control.rs` lists them); a derived class overrides its own slots by name.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::control::CCOE_CONTROL_SIZE;
use crate::loader::Process;

/// CCoeControl's primary vtable, slots 2..=43 (0 and 1 are the class's destructors).
const CCOE_SLOTS: [&str; 42] = [
    "_ZN5CBase10Extension_EjRPvS0_",
    "_ZN11CCoeControl14OfferKeyEventLERK9TKeyEvent10TEventCode",
    "_ZN11CCoeControl11MakeVisibleEi",
    "_ZN11CCoeControl9SetDimmedEi",
    "_ZN11CCoeControl19SetContainerWindowLERKS_",
    "_ZN11CCoeControl22ConstructFromResourceLER15TResourceReader",
    "_ZN11CCoeControl9ActivateLEv",
    "_ZN11CCoeControl20PrepareForFocusLossLEv",
    "_ZN11CCoeControl20PrepareForFocusGainLEv",
    "_ZN11CCoeControl11SetAdjacentEi",
    "_ZN11CCoeControl11SetNeighborEPS_",
    "_ZNK11CCoeControl9HasBorderEv",
    "_ZN11CCoeControl11MinimumSizeEv",
    "_ZN11CCoeControl20HandleResourceChangeEi",
    "_ZNK11CCoeControl16GetColorUseListLER9CArrayFixI12TCoeColorUseE",
    "_ZNK11CCoeControl14GetHelpContextER15TCoeHelpContext",
    "_ZNK11CCoeControl17InputCapabilitiesEv",
    "_ZN11CCoeControl9SetParentEPS_",
    "_ZN11CCoeControl17SetLayoutManagerLEP17MCoeLayoutManager",
    "_ZN11CCoeControl15RequestRelayoutEPKS_",
    "_ZNK11CCoeControl18TextBaselineOffsetERK5TSize",
    "_ZN11CCoeControl22SetTextBaselineSpacingEi",
    "_ZN11CCoeControl19HandlePointerEventLERK13TPointerEvent",
    "_ZN11CCoeControl25HandlePointerBufferReadyLEv",
    "_ZN11CCoeControl12FocusChangedE8TDrawNow",
    "_ZN11CCoeControl11SizeChangedEv",
    "_ZN11CCoeControl15PositionChangedEv",
    "_ZNK11CCoeControl21CountComponentControlsEv",
    "_ZNK11CCoeControl16ComponentControlEi",
    "_ZN11CCoeControl24HandleControlArrayEventLEN16CCoeControlArray6TEventEPKS0_PS_i",
    "_ZN11CCoeControl23Reserved_CCoeControl_10Ev",
    "_ZN11CCoeControl23Reserved_CCoeControl_11Ev",
    "_ZN11CCoeControl23Reserved_CCoeControl_12Ev",
    "_ZN11CCoeControl23Reserved_CCoeControl_13Ev",
    "_ZNK11CCoeControl13GetTextDrawerERP18CCoeTextDrawerBasePKS_i",
    "_ZN11CCoeControl22Reserved_CCoeControl_8Ev",
    "_ZN11CCoeControl22Reserved_CCoeControl_9Ev",
    "_ZN11CCoeControl15MopSupplyObjectE8TTypeUid",
    "_ZN11CCoeControl7MopNextEv",
    "_ZNK11CCoeControl4DrawERK5TRect",
    "_ZNK11CCoeControl19WriteInternalStateLER12RWriteStream",
    "_ZN11CCoeControl10Reserved_2Ev",
];
/// The `MObjectProvider` secondary vtable at offset 4 (non-virtual thunks to CCoeControl's overrides).
const MOP_SLOTS: [&str; 2] = ["_ZThn4_N11CCoeControl15MopSupplyObjectE8TTypeUid", "_ZThn4_N11CCoeControl7MopNextEv"];

fn mangled_dtor(class: &str, kind: &str) -> String {
    format!("_ZN{}{}{}Ev", class.len(), class, kind)
}

impl Process {
    /// A new engine-built control of `class` (`bytes` >= the CCoeControl size): `CCoeControl::CCoeControl()` has run on it and its
    /// vtables are in place. `overrides` replaces CCoeControl slots (slot, name). The class's destructors are
    /// `_ZN<len><class>D1Ev` / `D0Ev`.
    pub(crate) fn make_control(&mut self, class: &str, bytes: u32, overrides: &[(usize, &str)]) -> Result<u32, String> {
        let obj = self.alloc_zeroed(bytes.max(CCOE_CONTROL_SIZE)).ok_or_else(|| format!("no guest heap for a {class}"))?;
        let (d1, d0) = (mangled_dtor(class, "D1"), mangled_dtor(class, "D0"));
        let mut names: Vec<Option<&str>> = Vec::new();
        names.push(Some(d1.as_str()));
        names.push(Some(d0.as_str()));
        names.extend(CCOE_SLOTS.iter().map(|n| Some(*n)));
        for &(slot, name) in overrides {
            if slot < names.len() { names[slot] = Some(name); }
        }
        let vptr = self.build_vtable(class, &names)?;
        let mop = self.build_vtable("MObjectProvider", &MOP_SLOTS.map(Some))?;
        self.mem.vm.write_u32(obj, vptr).map_err(|_| format!("{class} unwritable"))?;
        self.mem.vm.write_u32(obj + 4, mop).map_err(|_| format!("{class} unwritable"))?;
        self.control_init_fields(obj)?;
        Ok(obj)
    }
}
