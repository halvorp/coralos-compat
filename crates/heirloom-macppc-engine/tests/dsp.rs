//! DrawSprocket by C name through the glue: the windowed-games contract (one context on the main display, Reserve/SetState give a native window of the requested size,
//! the back buffer is a GWorld swapped to that window), the 1.7.5 version through the hidden struct pointer, and the error numbers.
use heirloom_ui_abi::{RecordingGuestUi, UiCall};
use heirloom_macppc_engine::symbols::Binding;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};

type M = MacPpc<RecordingGuestUi>;
const OUT: u32 = 0x3300;
const ATTR: u32 = 0x3400;
const R: u32 = 0x3500;

fn call(m: &mut M, name: &str, args: &[u32]) -> i32 {
    assert!(!matches!(m.bind_from("DrawSprocketLib", name), Binding::Unbound(_)), "{name} must be bound");
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    let key = heirloom_macppc_engine::glue::binding_key("DrawSprocketLib", name);
    assert_eq!(m.glue_call(&key, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3] as i32
}
fn machine() -> M {
    let mut m = MacPpc::new(RecordingGuestUi::new()).unwrap();
    m.bind("InitGraf");
    let mut regs = [0u32; 32];
    regs[3] = 0x1000;
    assert_eq!(m.glue_call("InitGraf", &mut regs), GlueOutcome::Return);
    m
}

#[test]
fn version_enumeration_find_reserve_state_release_and_the_error_numbers() {
    let mut m = machine();
    // before DSpStartup everything else is kDSpNotInitializedErr (-30440)
    assert_eq!(call(&mut m, "DSpGetFirstContext", &[1, OUT]), -30440);
    assert_eq!(call(&mut m, "DSpStartup", &[]), 0);
    // DSpGetVersion: NumVersion 1.7.5 final through the hidden result pointer in r3
    assert_eq!(call(&mut m, "DSpGetVersion", &[R]) as u32, R);
    assert_eq!(m.mac.mem.read_u32(R).unwrap(), 0x0175_8000);
    // one display (id 1), a context per mode (the first is the display's own 640x480); Next ends with kDSpContextNotFoundErr and a NIL
    assert_eq!(call(&mut m, "DSpGetFirstContext", &[1, OUT]), 0);
    let first = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(first, 0);
    let modes = enumerate(&mut m, first);
    assert_eq!(modes.len(), 30, "15 modes, each at 16 and at 32 bits");
    assert_eq!(modes[0], (640, 480, 16), "the display's own size at 16 bits first: WC3's default colordepth 16 keeps today's mode");
    assert_eq!(modes[1], (640, 480, 32));
    assert!(modes.contains(&(1920, 1080, 32)) && modes.contains(&(1280, 800, 16)) && modes.contains(&(1024, 768, 32)));
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0, "the walk ends with a NIL");
    assert_eq!(call(&mut m, "DSpGetFirstContext", &[2, OUT]), -30446, "NEGATIVE control: no second display");
    // FindBestContext answers the requested mode's context; Reserve makes the native window of that size (hidden until Active)
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    m.mac.mem.write_u32(ATTR + 4, 800).unwrap();
    m.mac.mem.write_u32(ATTR + 8, 600).unwrap();
    assert_eq!(call(&mut m, "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]), 0);
    let ctx = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(ctx, first, "800x600 is its own context");
    m.mac.mem.write_u32(ATTR + 44, 32).unwrap();
    assert_eq!(call(&mut m, "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), ctx + 1, "a 32-bit request is the same mode's 32-bit context");
    m.mac.mem.write_u32(ATTR + 44, 0).unwrap();
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), 0);
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), -30444, "already reserved");
    let win = *m.mac.windows.keys().next().expect("the native window");
    let w = &m.mac.windows[&win];
    assert_eq!((w.rect.right - w.rect.left, w.rect.bottom - w.rect.top, w.visible), (800, 600, false));
    // Active: the guest's main GDevice reports the mode (the host display is never switched) and the window stays hidden - only a
    // game that presents through DrawSprocket shows it; Inactive gives the GDevice its own rectangle back
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 0]), 0);
    assert!(!m.mac.windows[&win].visible, "no empty host window for a game that makes its own");
    assert_eq!(gd_rect(&m), (800, 600));
    assert_eq!(call(&mut m, "DSpContext_GetState", &[ctx, OUT]), 0);
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 2]), 0);
    assert_eq!(gd_rect(&m), (640, 480), "Inactive: the display's own rectangle again");
    assert!(!m.mac.ui.calls.iter().any(|c| matches!(c, UiCall::HideWindow(_))), "a window never shown is not hidden");
    // attributes report the context's mode
    assert_eq!(call(&mut m, "DSpContext_GetAttributes", &[ctx, ATTR]), 0);
    assert_eq!((m.mac.mem.read_u32(ATTR).unwrap(), m.mac.mem.read_u32(ATTR + 48).unwrap()), (60 << 16, 1));
    assert_eq!((m.mac.mem.read_u32(ATTR + 4).unwrap(), m.mac.mem.read_u32(ATTR + 8).unwrap(), m.mac.mem.read_u32(ATTR + 44).unwrap()), (800, 600, 16), "the reserved context reports the mode and depth it runs");
    // a call that needs the reserved context, on another one, is kDSpContextNotReservedErr
    assert_eq!(call(&mut m, "DSpContext_SetState", &[first, 0]), -30445);
    // can the user pick? no: one context; UserSelectContext answers it without a dialog
    m.mac.mem.write_u8(OUT, 1).unwrap();
    assert_eq!(call(&mut m, "DSpCanUserSelectContext", &[ATTR, OUT]), 0);
    assert_eq!(m.mac.mem.read_u8(OUT).unwrap(), 0);
    assert_eq!(call(&mut m, "DSpUserSelectContext", &[ATTR, 1, 0, OUT]), 0);
    // fades are recorded (named limit), never an error and never gamma
    assert_eq!(call(&mut m, "DSpContext_FadeGammaOut", &[ctx, 0]), 0);
    assert!(m.mac.log.iter().any(|l| l.starts_with("[dsp] fade")));
    // Release closes the window; a second Release and use of an unreserved context are -30445; a wrong reference -30442
    assert_eq!(call(&mut m, "DSpContext_Release", &[ctx]), 0);
    assert!(m.mac.windows.is_empty());
    assert_eq!(call(&mut m, "DSpContext_Release", &[ctx]), -30445);
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 0]), -30445);
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[0x1234, ATTR]), -30442);
    assert_eq!(call(&mut m, "DSpContext_GetAttributes", &[first + 30, ATTR]), -30442, "NEGATIVE control: one past the last context names none");
}

/// The main GDevice's gdRect as (width, height).
fn gd_rect(m: &M) -> (i32, i32) {
    let p = m.mac.mm.block(m.mac.cqd.main_device).unwrap().ptr + 0x22;
    let r = |o: u32| m.mac.mem.read_u16(p + o).unwrap() as i16 as i32;
    (r(6) - r(2), r(4) - r(0))
}

/// Warcraft III's mode change (FUN_10351aa0, Z83 2026-11-04): FindBest + Reserve + SetState(Active), then CreateNewWindow sized from the
/// main GDevice's gdRect. With the mode reported there the game's own GL window is 1280x800 (it was the 640x480 display's, and the
/// 1280x800 frame showed cropped); the context's own window stays hidden (no second, empty host window); Release restores the GDevice.
#[test]
fn an_active_context_is_the_gdevices_rectangle_and_its_window_shows_only_for_a_drawsprocket_presenter() {
    let mut m = machine();
    call(&mut m, "DSpStartup", &[]);
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    m.mac.mem.write_u32(ATTR + 4, 1280).unwrap();
    m.mac.mem.write_u32(ATTR + 8, 800).unwrap();
    assert_eq!(call(&mut m, "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]), 0);
    let ctx = m.mac.mem.read_u32(OUT).unwrap();
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), 0);
    assert_eq!(gd_rect(&m), (640, 480), "reserving changes nothing yet");
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 0]), 0);
    assert_eq!(gd_rect(&m), (1280, 800), "the game sizes its window from this");
    let win = *m.mac.windows.keys().next().unwrap();
    assert!(!m.mac.windows[&win].visible);
    // the mode list still starts at the DISPLAY's size while a context reports its mode there
    call(&mut m, "DSpGetFirstContext", &[1, OUT]);
    let first = m.mac.mem.read_u32(OUT).unwrap();
    call(&mut m, "DSpContext_GetAttributes", &[first, ATTR + 0x100]);
    assert_eq!((m.mac.mem.read_u32(ATTR + 0x104).unwrap(), m.mac.mem.read_u32(ATTR + 0x108).unwrap()), (640, 480));
    // a game that presents through DrawSprocket gets the window shown
    assert_eq!(call(&mut m, "DSpContext_GetBackBuffer", &[ctx, 0, OUT]), 0);
    assert!(m.mac.windows[&win].visible, "GetBackBuffer shows the window SwapBuffers draws into");
    assert_eq!(call(&mut m, "DSpContext_Release", &[ctx]), 0);
    assert_eq!(gd_rect(&m), (640, 480), "Release: the display's own rectangle again");
}

/// Walk the contexts from `first` as Warcraft III's mode list does (GetAttributes, then Next until it fails): every (width, height).
fn enumerate(m: &mut M, first: u32) -> Vec<(u32, u32, u32)> {
    let mut ctx = first;
    let mut out = Vec::new();
    loop {
        assert_eq!(call(m, "DSpContext_GetAttributes", &[ctx, ATTR]), 0);
        // the game keeps a context whose displayWidth >= 640 and displayBestDepth >= 16 (FUN_10350930)
        assert!(m.mac.mem.read_u32(ATTR + 4).unwrap() >= 640 && m.mac.mem.read_u32(ATTR + 44).unwrap() >= 16);
        out.push((m.mac.mem.read_u32(ATTR + 4).unwrap(), m.mac.mem.read_u32(ATTR + 8).unwrap(), m.mac.mem.read_u32(ATTR + 44).unwrap()));
        if call(m, "DSpGetNextContext", &[ctx, OUT]) != 0 {
            return out;
        }
        ctx = m.mac.mem.read_u32(OUT).unwrap();
    }
}

#[test]
fn a_mode_change_releases_and_reserves_a_window_of_the_new_size() {
    let mut m = machine();
    call(&mut m, "DSpStartup", &[]);
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    for (w, h) in [(1920u32, 1080u32), (1280, 800)] {
        m.mac.mem.write_u32(ATTR + 4, w).unwrap();
        m.mac.mem.write_u32(ATTR + 8, h).unwrap();
        assert_eq!(call(&mut m, "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]), 0);
        let ctx = m.mac.mem.read_u32(OUT).unwrap();
        assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), 0);
        let win = *m.mac.windows.keys().next().unwrap();
        let r = m.mac.windows[&win].rect;
        assert_eq!(((r.right - r.left) as u32, (r.bottom - r.top) as u32), (w, h));
        assert_eq!(call(&mut m, "DSpContext_Release", &[ctx]), 0);
    }
}

#[test]
fn the_best_mode_is_exact_else_the_smallest_that_holds_it_else_the_largest() {
    use heirloom_macppc_engine::dsp_c::best_mode;
    let modes = [(640, 480), (800, 600), (1280, 720), (1920, 1080)];
    assert_eq!(best_mode(&modes, (0, 0)), 0);
    assert_eq!(best_mode(&modes, (1280, 720)), 2);
    assert_eq!(best_mode(&modes, (1000, 700)), 2, "1000x700 fits in 1280x720, not 800x600");
    assert_eq!(best_mode(&modes, (4000, 3000)), 3, "nothing holds it: the largest");
}

#[test]
fn the_back_buffer_is_a_gworld_swapped_to_the_native_window() {
    let mut m = machine();
    call(&mut m, "DSpStartup", &[]);
    call(&mut m, "DSpGetFirstContext", &[1, OUT]);
    let ctx = m.mac.mem.read_u32(OUT).unwrap();
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    m.mac.mem.write_u32(ATTR + 4, 64).unwrap();
    m.mac.mem.write_u32(ATTR + 8, 48).unwrap();
    assert_eq!(call(&mut m, "DSpContext_GetBackBuffer", &[ctx, 0, OUT]), -30445, "NEGATIVE control: not reserved yet");
    call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]);
    call(&mut m, "DSpContext_SetState", &[ctx, 0]);
    assert_eq!(call(&mut m, "DSpContext_GetBackBuffer", &[ctx, 0, OUT]), 0, "{:?}", m.mac.log.last());
    let back = m.mac.mem.read_u32(OUT).unwrap();
    assert!(m.mac.cqd.gworlds.contains_key(&back), "a GWorld");
    // draw black into the back buffer, swap, the window canvas shows it
    m.mac.gw_set(back, 0).unwrap();
    for (i, v) in [4i16, 4, 20, 20].into_iter().enumerate() { m.mac.mem.write_u16(R + 2 * i as u32, v as u16).unwrap(); }
    let op = heirloom_mac68k_engine::mixed::trap_opcode("PaintRect").unwrap();
    m.mac.pascal_call(op, &[R]).unwrap();
    assert_eq!(call(&mut m, "DSpContext_SwapBuffers", &[ctx, 0, 0]), 0);
    let win = *m.mac.windows.keys().next().unwrap();
    let canvas = &m.mac.qd.canvases[&win];
    assert_eq!(canvas.get(10, 10), Some(1), "the swapped pixels are on the window");
    assert_eq!(canvas.get(40, 40), Some(0));
    // Release disposes the back buffer too
    call(&mut m, "DSpContext_Release", &[ctx]);
    assert!(!m.mac.cqd.gworlds.contains_key(&back));
}
