//! AGL's drawable follows its port: Warcraft III changes mode with SizeWindow on the window it already set as the drawable and
//! never calls aglSetDrawable again (Z83 2026-11-04: 1280x800 rendered into the old 640x480 drawable, cropped). The next swap sends
//! the port's new size first, so the worker and the window owner see the guest's real resolution.
mod ppc_gl;
use heirloom_macppc_engine::{GlueOutcome, MacPpc, PpcHost};
use heirloom_ui_abi::RecordingGuestUi;
use ppc_gl::{saw, Mock};

type M = MacPpc<RecordingGuestUi>;

fn call(m: &mut M, lib: &str, name: &str, args: &[u32]) -> u32 {
    let mut regs = [0u32; 32];
    regs[1] = 0x6000;
    regs[3..3 + args.len()].copy_from_slice(args);
    let _ = m.bind_from(lib, name);
    let key = heirloom_macppc_engine::glue::binding_key(lib, name);
    assert_eq!(m.glue_call(&key, &mut regs), GlueOutcome::Return, "{name}: {:?}", m.mac.refused);
    regs[3]
}

#[test]
fn a_swap_after_the_port_was_resized_sends_the_new_drawable_size_first() {
    let mut m = M::new(RecordingGuestUi::new()).unwrap();
    m.bind("InitGraf");
    let mut regs = [0u32; 32];
    regs[3] = 0x1000;
    assert_eq!(m.glue_call("InitGraf", &mut regs), GlueOutcome::Return);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(ppc_gl::Seen::default()));
    m.gl_attach(Box::new(Mock(heirloom_gl_abi::TokenTable::default(), seen.clone())));
    // the game's window: DrawSprocket's context window at 640x480
    const ATTR: u32 = 0x3400;
    const OUT: u32 = 0x3300;
    call(&mut m, "DrawSprocketLib", "DSpStartup", &[]);
    m.mac.mem.write_bytes(ATTR, &[0u8; 72]).unwrap();
    m.mac.mem.write_u32(ATTR + 4, 640).unwrap();
    m.mac.mem.write_u32(ATTR + 8, 480).unwrap();
    call(&mut m, "DrawSprocketLib", "DSpFindBestContextOnDisplayID", &[ATTR, OUT, 1]);
    let dsp = m.mac.mem.read_u32(OUT).unwrap();
    call(&mut m, "DrawSprocketLib", "DSpContext_Reserve", &[dsp, ATTR]);
    let win = *m.mac.windows.keys().next().unwrap();
    let ctx = call(&mut m, "OpenGLLibrary", "aglCreateContext", &[1, 0]);
    assert_ne!(ctx, 0);
    call(&mut m, "OpenGLLibrary", "aglSetCurrentContext", &[ctx]);
    assert_eq!(call(&mut m, "OpenGLLibrary", "aglSetDrawable", &[ctx, win]), 1);
    call(&mut m, "OpenGLLibrary", "aglSwapBuffers", &[ctx]);
    let sizes = |s: &std::sync::Arc<std::sync::Mutex<ppc_gl::Seen>>| saw(s, "aglSetDrawable").iter().map(|(a, _)| (a[0], a[1])).collect::<Vec<_>>();
    assert_eq!(sizes(&seen), [(640, 480)], "NEGATIVE control: an unchanged port sends nothing more");
    // the mode change: the window is resized, no aglSetDrawable follows
    let op = heirloom_mac68k_engine::mixed::trap_opcode("SizeWindow").unwrap();
    m.mac.pascal_call(op, &[win, 1280, 800, 0]).unwrap();
    call(&mut m, "OpenGLLibrary", "aglSwapBuffers", &[ctx]);
    assert_eq!(sizes(&seen), [(640, 480), (1280, 800)], "the swap sent the port's new size first");
    let kinds: Vec<String> = seen.lock().unwrap().calls.iter().map(|c| c.0.clone()).collect();
    assert_eq!(kinds.last().map(String::as_str), Some("aglSetDrawable"), "the drawable precedes the swap that shows the new frame");
    call(&mut m, "OpenGLLibrary", "aglSwapBuffers", &[ctx]);
    assert_eq!(sizes(&seen).len(), 2, "said once per change");
}
