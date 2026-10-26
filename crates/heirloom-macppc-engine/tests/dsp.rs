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
    // one display (id 1), one context; Next ends with kDSpContextNotFoundErr and a NIL
    assert_eq!(call(&mut m, "DSpGetFirstContext", &[1, OUT]), 0);
    let ctx = m.mac.mem.read_u32(OUT).unwrap();
    assert_ne!(ctx, 0);
    assert_eq!(call(&mut m, "DSpGetNextContext", &[ctx, OUT]), -30446);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), 0);
    assert_eq!(call(&mut m, "DSpGetFirstContext", &[2, OUT]), -30446, "NEGATIVE control: no second display");
    // FindBestContext records the requested size; Reserve makes the native window of that size (hidden until Active)
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    m.mac.mem.write_u32(ATTR + 4, 800).unwrap();
    m.mac.mem.write_u32(ATTR + 8, 600).unwrap();
    assert_eq!(call(&mut m, "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]), 0);
    assert_eq!(m.mac.mem.read_u32(OUT).unwrap(), ctx);
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), 0);
    assert_eq!(call(&mut m, "DSpContext_Reserve", &[ctx, ATTR]), -30444, "already reserved");
    let win = *m.mac.windows.keys().next().expect("the native window");
    let w = &m.mac.windows[&win];
    assert_eq!((w.rect.right - w.rect.left, w.rect.bottom - w.rect.top, w.visible), (800, 600, false));
    // Active shows it (never a display mode change: the main device's rectangle is untouched); Inactive hides it again
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 0]), 0);
    assert!(m.mac.windows[&win].visible);
    assert_eq!(call(&mut m, "DSpContext_GetState", &[ctx, OUT]), 0);
    assert_eq!(call(&mut m, "DSpContext_SetState", &[ctx, 2]), 0);
    assert!(!m.mac.windows[&win].visible);
    assert!(m.mac.ui.calls.iter().any(|c| matches!(c, UiCall::HideWindow(_))));
    // attributes report the one display
    assert_eq!(call(&mut m, "DSpContext_GetAttributes", &[ctx, ATTR]), 0);
    assert_eq!((m.mac.mem.read_u32(ATTR).unwrap(), m.mac.mem.read_u32(ATTR + 48).unwrap()), (60 << 16, 1));
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
