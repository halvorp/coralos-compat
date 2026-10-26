//! The drawing surface of a MUI window that holds self-drawing objects ([`super::drawn`]): a `struct MUI_RenderInfo` shared by its
//! objects (made at `MUIM_Setup`), and, once the native window is open, a real `struct Window` with a canvas, layer and RastPort
//! (`window::make_surface_window`), so `_rp(obj)` is a RastPort the graphics layer records into planes. Closing the window gives all
//! of it back. Facts: classes/window.h `struct MUI_RenderInfo` (offsets in `drawn.rs`), Zune's window.c (mri_Window / mri_RastPort are
//! valid between Show and Hide, the screen and pens between Setup and Cleanup).
use super::drawn::{self, Surface};
use crate::{machine::Amiga, memory, Refusal};
use heirloom_ui_abi::{GuestUi, UiObjectRef};

const O: (u32, u32, u32, u32, u32, u32, u32) = drawn::RI;
const WD_RPORT: u32 = crate::graphics::layout::WD_RPORT;

/// The `struct MUI_RenderInfo` of window `w`, made on first need (Setup).
pub fn render_info<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<u32, Refusal> {
    if let Some(s) = a.objects.surfaces.get(&w) { return Ok(s.ri); }
    let ri = memory::alloc_zeroed(a, O.5, "MUI_RenderInfo")?;
    let e = Refusal::mem("MUI_RenderInfo");
    a.mem.write_u32(ri + O.0, w).map_err(&e)?;
    a.mem.write_u32(ri + O.1, crate::screen::SCREEN).map_err(&e)?;
    a.mem.write_u32(ri + O.2, ri + O.6).map_err(&e)?;
    a.mem.write_u32(ri + 8, crate::graphics::layout::SCREEN_DRAWINFO).map_err(&e)?; // mri_DrawInfo
    a.mem.write_u32(ri + 28, crate::graphics::layout::SCREEN_COLORMAP).map_err(&e)?; // mri_Colormap
    for (i, p) in drawn::pens().into_iter().enumerate() { a.mem.write_u16(ri + O.6 + 2 * i as u32, p).map_err(&e)?; }
    a.objects.surfaces.insert(w, Surface { win: 0, ri });
    Ok(ri)
}

/// The native window of `w` is open with an inner area of `inner`: build the struct Window and point the RenderInfo at it.
pub fn attach<U: GuestUi>(a: &mut Amiga<U>, w: u32, native: UiObjectRef, inner: (i32, i32)) -> Result<(), Refusal> {
    let ri = render_info(a, w)?;
    if let Some(s) = a.objects.surfaces.get(&w).copied().filter(|s| s.win != 0) { return resize(a, w, s, inner); }
    let win = crate::window::make_surface_window(a, native, inner)?;
    let rp = a.mem.read_u32(win + WD_RPORT).map_err(Refusal::mem("Window.RPort"))?;
    let e = Refusal::mem("MUI_RenderInfo");
    a.mem.write_u32(ri + O.3, win).map_err(&e)?;
    a.mem.write_u32(ri + O.4, rp).map_err(&e)?;
    a.objects.surfaces.insert(w, Surface { win, ri });
    Ok(())
}

fn resize<U: GuestUi>(a: &mut Amiga<U>, _w: u32, s: Surface, inner: (i32, i32)) -> Result<(), Refusal> {
    let (bl, bt) = (4, 15); // window.rs borders(0, true)
    crate::graphics::resize_window(a, s.win, (inner.0 + bl + 4, inner.1 + bt + 4)).map(|_| ())
}

/// The window closed: its struct Window, surface and RenderInfo go back.
pub fn detach<U: GuestUi>(a: &mut Amiga<U>, w: u32) -> Result<(), Refusal> {
    let Some(s) = a.objects.surfaces.remove(&w) else { return Ok(()) };
    a.objects.clip_stack.remove(&s.ri);
    if s.win != 0 { crate::window::drop_surface_window(a, s.win)?; }
    a.heap.free(s.ri, O.5).map_err(|e| Refusal(alloc::format!("MUI_RenderInfo: {e}")))
}

const MRI_WINDOW: u32 = O.3;
const WD_WLAYER: u32 = crate::graphics::layout::WD_WLAYER;
const NO_HANDLE: u32 = u32::MAX;

/// muimaster `MUI_AddClipRegion(a0 = RenderInfo, a1 = region)` -> a handle for `MUI_RemoveClipRegion`. The region is installed as the clip of
/// the window's layer (intersected with the one installed before it: Zune does `AndRegionRegion(previous, new)`), and owned by the stack until
/// removed. A window-less RenderInfo or a NULL region disposes the region and answers -1, as mui_addclipregion.c does.
pub fn add_clip_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut heirloom_cpu_68k::Cpu) -> Result<alloc::string::String, Refusal> {
    let (mri, region) = (cpu.areg(0), cpu.areg(1));
    let win = a.mem.read_u32(mri + MRI_WINDOW).map_err(Refusal::mem("mri_Window"))?;
    let layer = if win != 0 { a.mem.read_u32(win + WD_WLAYER).map_err(Refusal::mem("Window.WLayer"))? } else { 0 };
    if layer == 0 || region == 0 {
        if region != 0 { cpu.set_areg(0, region); crate::graphics::regions::dispose_region(a, cpu)?; }
        cpu.set_d0(NO_HANDLE);
        return Ok("[muimaster] MUI_AddClipRegion -> -1 (no window layer or no region)".into());
    }
    if let Some(&top) = a.objects.clip_stack.get(&mri).and_then(|v| v.last()) {
        (cpu.set_areg(0, top), cpu.set_areg(1, region));
        crate::graphics::regions::and_region_region(a, cpu)?;
    }
    (cpu.set_areg(0, layer), cpu.set_areg(1, region));
    crate::graphics::regions::install_clip_region(a, cpu)?; // D0 = the region that was installed before
    a.objects.clip_stack.entry(mri).or_default().push(region);
    Ok(alloc::format!("[muimaster] MUI_AddClipRegion {region:#x} on layer {layer:#x}"))
}

/// muimaster `MUI_RemoveClipRegion(a0 = RenderInfo, a1 = handle)`: the region below the handle is installed again and the handle's is disposed.
pub fn remove_clip_region<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut heirloom_cpu_68k::Cpu) -> Result<alloc::string::String, Refusal> {
    let (mri, handle) = (cpu.areg(0), cpu.areg(1));
    if handle == NO_HANDLE { return Ok("[muimaster] MUI_RemoveClipRegion -1".into()); }
    let win = a.mem.read_u32(mri + MRI_WINDOW).map_err(Refusal::mem("mri_Window"))?;
    let layer = if win != 0 { a.mem.read_u32(win + WD_WLAYER).map_err(Refusal::mem("Window.WLayer"))? } else { 0 };
    if layer == 0 { return Ok("[muimaster] MUI_RemoveClipRegion: no layer".into()); }
    let stack = a.objects.clip_stack.entry(mri).or_default();
    let top = stack.pop().ok_or_else(|| Refusal(alloc::format!("MUI_RemoveClipRegion {handle:#x}: no region was added")))?;
    let below = stack.last().copied().unwrap_or(0);
    (cpu.set_areg(0, layer), cpu.set_areg(1, below));
    crate::graphics::regions::install_clip_region(a, cpu)?;
    cpu.set_areg(0, top);
    crate::graphics::regions::dispose_region(a, cpu)?;
    Ok(alloc::format!("[muimaster] MUI_RemoveClipRegion {top:#x}"))
}
