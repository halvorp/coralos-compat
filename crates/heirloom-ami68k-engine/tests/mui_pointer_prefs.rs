//! REAL AROS Prefs programs driven through the native seam: OM_SET / MUIM_Set reach a program's own class first (ReqTools prefs sets an
//! attribute only its class owns), and the Pointer preview, which listens with MUIM_Window_AddEventHandler, reacts to a native pointer event
//! in its plane rectangle. Each test SKIPS loudly when the corpus is absent.
mod aros;
use heirloom_ami68k_engine::Step;
use heirloom_ui_abi::{ControlKind, ControlValue, GuestUiEvent, Intent, PointerPhase, UiCall};

#[test]
fn a_program_class_sees_its_own_attributes_before_the_native_class_does() {
    // Prefs/ReqTools sets 0x90470501 (its class's attribute) with SetAttrs/MUIM_Set: it used to be a refusal from the native Group.
    let Some((a, s)) = aros::start("Prefs/ReqTools", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(!a.ui.controls.is_empty(), "its window opened");
}

#[test]
fn the_pointer_preview_reacts_to_a_native_pointer_event_inside_its_plane_and_not_outside() {
    let Some((mut a, s)) = aros::start("Prefs/Pointer", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let plane = a.ui.calls.iter().find_map(|c| match c { UiCall::PresentPlane(w, p) => Some((*w, p.rect)), _ => None }).expect("the preview plane");
    // The listener registered itself with its window when it was set up.
    assert!(!a.objects.handlers.evh.is_empty(), "MUIM_Window_AddEventHandler recorded");
    // "Set view" is a toggle: a native click switches it on, its notification is MUIM_Set(MUIA_PPreview_SetHSpot, TRUE) on the preview, which
    // goes through the preview's own dispatcher.
    let n = |a: &heirloom_ami68k_engine::Amiga<heirloom_ui_abi::RecordingGuestUi>, pre: &str| a.log.iter().filter(|l| l.starts_with(pre)).count();
    let (w, c, m) = a.ui.find_control(|m| matches!(m.kind, ControlKind::ToggleButton { on: false })).expect("the Set view toggle");
    let redraws = n(&a, "[muimaster] MUI_Redraw");
    a.deliver(GuestUiEvent::intent(w, c, m.token, Intent::SetValue(ControlValue::Checked(true)))).unwrap();
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(n(&a, "[muimaster] MUI_Redraw") > redraws, "MUIM_Set(MUIA_PPreview_SetHSpot) reached the preview's own OM_SET, which redraws it");
    // The event handler the preview registered is offered every mouse-button message of its window (its ehn_Events), wherever it lands: it is
    // the class that decides, from its own drawn offset, whether the release is on the image (this run has no picture datatype, so it is never).
    let handled = n(&a, "[muimaster] Handle");
    let (ix, iy) = (plane.1.left + plane.1.width / 2, plane.1.top + plane.1.height / 2);
    a.deliver(GuestUiEvent::primary(w, ix, iy, PointerPhase::Down)).unwrap();
    a.deliver(GuestUiEvent::primary(w, ix, iy, PointerPhase::Up)).unwrap();
    assert_eq!(a.step(20_000_000).0, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert_eq!(n(&a, "[muimaster] Handle") - handled, 2, "press and release reached MUIM_HandleEvent");
    // NEGATIVE CONTROL: a mouse MOVE is not in its ehn_Events (IDCMP_MOUSEBUTTONS only): no call.
    a.deliver(GuestUiEvent::primary(w, ix + 1, iy, PointerPhase::Move)).unwrap();
    assert_eq!(a.step(20_000_000).0, Step::Blocked);
    assert_eq!(n(&a, "[muimaster] Handle") - handled, 2);
}

#[test]
fn palette_prefs_opens_cleanly_with_or_without_disk_colorwheel_gadget() {
    // 1. Minimal SYS: without SYS:Classes/Gadgets (matches QEMU staged bundle): built-in V39
    // `gadgets/colorwheel.gadget` supplies ConvertRGBToHSB/ConvertHSBToRGB so Coloradjust.mui
    // finishes OM_NEW with 0 bus faults and opens "Palette Preferences".
    let Some((a, s)) = aros::start("Prefs/Palette", &["SYS:Classes/Gadgets"]) else { return };
    assert_eq!(s, Step::Blocked, "refused={:?} tail={:?}", a.refused, a.log.iter().rev().take(12).collect::<Vec<_>>());
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(a.ui.calls.iter().any(|c| matches!(c, UiCall::OpenWindow(m) if m.title == "Palette Preferences")), "Palette Preferences window must open");
    assert!(a.ui.controls.values().any(|(_, m)| matches!(&m.kind, ControlKind::List { rows, .. } if rows.len() >= 12)), "pen list populated");
    assert!(a.log.iter().any(|l| l.contains("[colorwheel] ConvertRGBToHSB")), "built-in colorwheel.gadget served ConvertRGBToHSB");

    // 2. Full mini SYS: with SYS:Classes/Gadgets/colorwheel.gadget present on disk: disk library wins.
    let Some((b, s2)) = aros::start("Prefs/Palette", &[]) else { return };
    assert_eq!(s2, Step::Blocked, "{:?}", b.refused);
    assert!(b.refused.is_empty(), "{:?}", b.refused);
    assert!(b.log.iter().any(|l| l.contains("loading LIBS:gadgets/colorwheel.gadget from disk")), "disk colorwheel.gadget wins when present");
}
