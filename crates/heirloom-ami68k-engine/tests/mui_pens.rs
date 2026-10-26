//! muimaster.library MUI_ObtainPen / MUI_ReleasePen over a hand-built struct MUI_RenderInfo (window.h: mri_DrawInfo 8, mri_Pens 12,
//! mri_Colormap 28): MUI/colormap/system/RGB pen specs, the allocated-bit protocol, and the -1 refusals (the negative controls).
mod gfxcommon;
use heirloom_ami68k_engine::machine::EXEC_BASE;
use gfxcommon::*;

const OPEN_LIBRARY: i32 = -552;
const OBTAIN_PEN: i32 = -156;
const RELEASE_PEN: i32 = -162;

struct T { g: G, mui: u32, mri: u32 }
fn setup() -> T {
    let mut g = G::new();
    let n = g.s("muimaster.library");
    g.run(&[(EXEC_BASE, OPEN_LIBRARY, vec![a(1, n), d(0, 0)])]);
    let mui = g.result(0);
    assert_ne!(mui, 0);
    let mri = g.zeroed(512);
    let pens = mri + 36;
    for i in 0..8u32 { g.a.mem.write_u16(pens + 2 * i, 10 + i as u16).unwrap(); }
    g.a.mem.write_u32(mri + 8, heirloom_ami68k_engine::graphics::layout::SCREEN_DRAWINFO).unwrap();
    g.a.mem.write_u32(mri + 12, pens).unwrap();
    g.a.mem.write_u32(mri + 28, heirloom_ami68k_engine::graphics::layout::SCREEN_COLORMAP).unwrap();
    T { g, mui, mri }
}
impl T {
    fn obtain(&mut self, spec: &str) -> i32 {
        let s = self.g.s(spec);
        self.g.run(&[(self.mui, OBTAIN_PEN, vec![a(0, self.mri), a(1, s), d(0, 0)])]);
        self.g.result(0) as i32
    }
    fn release(&mut self, pen: i32) { self.g.run(&[(self.mui, RELEASE_PEN, vec![a(0, self.mri), d(0, pen as u32)])]); }
    fn refs(&mut self, pen: usize) -> u32 { self.g.a.gfx.palette().refs[pen] as u32 }
}

#[test]
fn mui_colormap_and_system_pen_specs() {
    let mut t = setup();
    assert_eq!(t.obtain("m5"), 15, "MUI pen 5 is mri_Pens[5]");
    assert_eq!(t.obtain("m7"), 17);
    assert_eq!(t.obtain("p3"), 3, "a colormap entry");
    assert_eq!(t.obtain("p-1"), 255, "a negative colormap entry counts from the end of the map");
    let dri_pens = t.g.r32(heirloom_ami68k_engine::graphics::layout::SCREEN_DRAWINFO + 4);
    let sys2 = t.g.r16(dri_pens + 4) as i32;
    assert_eq!(t.obtain("s2"), sys2, "a DrawInfo pen");
    // NEGATIVE CONTROLS: out of range, unparsable, no colormap.
    assert_eq!(t.obtain("m8"), -1, "MPEN_COUNT is 8");
    assert_eq!(t.obtain("m-1"), -1);
    assert_eq!(t.obtain("p256"), -1);
    assert_eq!(t.obtain("p-257"), -1);
    assert_eq!(t.obtain("s99"), -1, "beyond dri_NumPens");
    assert_eq!(t.obtain("mx"), -1, "no digits");
    assert_eq!(t.obtain("zz"), -1, "an RGB spec needs three components");
    t.g.a.mem.write_u32(t.mri + 28, 0).unwrap();
    assert_eq!(t.obtain("m5"), -1, "a render info without a colormap");
    assert!(t.g.a.refused.is_empty(), "{:?}", t.g.a.refused);
}

#[test]
fn rgb_specs_allocate_shared_pens_that_release_gives_back() {
    let mut t = setup();
    let p1 = t.obtain("rff000000,00000000,00000000");
    assert!(p1 != -1 && p1 & 0x10000 != 0, "an allocated pen carries bit 16: {p1:#x}");
    let pen = (p1 & 0xFFFF) as usize;
    assert!(pen >= 8, "pens 0..8 are the system's");
    assert_eq!(t.g.a.gfx.palette().rgb[pen], 0xFF_0000, "the pen was set to the colour");
    let p2 = t.obtain("rff000000,00000000,00000000");
    assert_eq!(p2, p1, "the same colour shares the pen");
    assert_eq!(t.refs(pen), 2);
    let other = t.obtain("r00,00,ff000000");
    assert_ne!(other & 0xFFFF, p1 & 0xFFFF, "a different colour gets a different pen");
    t.release(p1);
    assert_eq!(t.refs(pen), 1);
    // NEGATIVE CONTROL: a pen without bit 16 (a colormap or MUI pen) was never allocated: releasing it changes nothing.
    let before = t.refs(5);
    t.release(5);
    t.release(-1);
    assert_eq!(t.refs(5), before);
    t.release(p2);
    assert_eq!(t.refs(pen), 0, "both references are back");
    assert!(t.g.a.refused.is_empty(), "{:?}", t.g.a.refused);
}

mod aros;
use heirloom_ami68k_engine::Step;

/// The Input prefs editor's BetterString objects read their pen specs in MUIM_Setup: mad_RenderInfo is set before the class sees the method
/// (window.c DoSetupMethod), MUIM_GetConfigItem finds nothing saved so the class's built-in defaults apply, and MUI_ObtainPen resolves them.
#[test]
fn a_real_class_obtains_its_default_pens_in_setup() {
    let Some((a, s)) = aros::start("Prefs/Input", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    let pens: Vec<&String> = a.log.iter().filter(|l| l.starts_with("[muimaster] MUI_ObtainPen")).collect();
    assert!(pens.len() >= 5 && pens.iter().all(|l| !l.ends_with("-> -1")), "{pens:?}");
    assert!(pens.iter().any(|l| l.contains("\"m4\"")), "BetterString's inactive text default");
    // NEGATIVE CONTROL: without the class on the volume nothing asks for those pens.
    let Some((b, _)) = aros::start("Prefs/Input", &["SYS:Classes/Zune/BetterString"]) else { return };
    assert!(b.log.iter().all(|l| !l.starts_with("[muimaster] MUI_ObtainPen")), "no BetterString, no pen request");
}

/// Find's search process is a disk class (Process.mui) over Semaphore.mui; the NList demo loads its saved settings at start. Both used to stop on
/// a class / method the engine lacked.
#[test]
fn a_disk_class_over_semaphore_mui_and_a_program_that_loads_its_settings_run() {
    let Some((a, s)) = aros::start("System/Find", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", a.refused);
    assert!(a.refused.is_empty(), "{:?}", a.refused);
    assert!(a.ui.controls.len() >= 10, "Find's window: {}", a.ui.controls.len());
    // NEGATIVE CONTROL: without Process.mui the class chain is incomplete and the refusal names it.
    let Some((b, _)) = aros::start("System/Find", &["SYS:Classes/Zune/Process.mui"]) else { return };
    assert!(b.ui.controls.is_empty() || !b.refused.is_empty(), "without its class Find cannot be whole");
    let Some((n, s)) = aros::start("Demos/Zune/MCC_NList/NList-Demo", &[]) else { return };
    assert_eq!(s, Step::Blocked, "{:?}", n.refused);
    assert!(n.refused.is_empty(), "{:?}", n.refused);
    assert!(n.log.iter().any(|l| l.starts_with("[muimaster] MUIM_Application_Load: ")), "the demo asked for its settings");
}
