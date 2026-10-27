//! The screen device (`CWsScreenDevice`, w32std.h) `CCoeEnv` owns: `iScreen`. The guest reads it inline (`CCoeEnv::ScreenDevice()`
//! is `iScreen`) and then calls its virtuals, so it is a guest object with a vptr whose entries are engine stubs (`enginestub`).
use alloc::format;
use alloc::string::String;
use crate::enginestub::slots_with_gaps;
use crate::loader::Process;
use crate::natives::NativeOutcome;

/// The device's pixels: S60 5th edition nHD, portrait (360x640).
pub const SCREEN_WIDTH: i32 = 360;
pub const SCREEN_HEIGHT: i32 = 640;
/// `CCoeEnv::iScreen` (coemain.h:209). Not computed from the headers (RFs / RWsSession / RWindowGroup sizes are not in the SDK tree):
/// read off the app's own inline `ScreenDevice()`, `ldr r1,[env,#0x48]`, which is the ground truth for the guest-visible offset.
pub const COEENV_I_SCREEN: u32 = 0x48;
/// Bytes of the guest block of a screen device: the vptr and room; its state is host-side (`SCREEN_*`), the guest reads none.
const SCREEN_DEVICE_BYTES: u32 = 0x40;

/// `CWsScreenDevice`'s primary virtual table as far as it was verified. ~CWsScreenDevice 0,1 (D1, D0), `CBase::Extension_` 2, then
/// `CGraphicsDevice` (gdi.h): DisplayMode 3, SizeInPixels 4 (CONFIRMED: FileBrowser calls `[vtable+0x10]` for its size), SizeInTwips 5,
/// CreateContext 6, NumTypefaces 7, TypefaceSupport 8, FontHeightInTwips 9, PaletteAttributes 10, SetPalette 11, GetPalette 12.
/// `CBitmapDevice` and `CWsScreenDevice` add more after slot 12; they are unnamed gaps (a call to one is a named refusal).
const VTABLE_SLOTS_TOTAL: usize = 48;
const VTABLE_NAMED: &[(usize, &str)] = &[
    (0, "_ZN15CWsScreenDeviceD1Ev"),
    (1, "_ZN15CWsScreenDeviceD0Ev"),
    (2, "_ZN5CBase10Extension_EjRPvS0_"),
    (3, "_ZNK15CWsScreenDevice11DisplayModeEv"),
    (4, "_ZNK15CWsScreenDevice12SizeInPixelsEv"),
    (5, "_ZNK15CWsScreenDevice11SizeInTwipsEv"),
    (6, "_ZN15CWsScreenDevice13CreateContextERP16CGraphicsContext"),
    (7, "_ZNK15CWsScreenDevice12NumTypefacesEv"),
    (8, "_ZNK15CWsScreenDevice15TypefaceSupportER16TTypefaceSupporti"),
    (9, "_ZNK15CWsScreenDevice17FontHeightInTwipsEii"),
    (10, "_ZNK15CWsScreenDevice17PaletteAttributesERiS0_"),
    (11, "_ZN15CWsScreenDevice10SetPaletteEP8CPalette"),
    (12, "_ZNK15CWsScreenDevice10GetPaletteERP8CPalette"),
];

impl Process {
    /// Build the screen device and store it in the environment's `iScreen`.
    pub(crate) fn make_screen_device(&mut self, env: u32) -> Result<(), String> {
        let dev = self.alloc_zeroed(SCREEN_DEVICE_BYTES).ok_or_else(|| String::from("no guest heap for the screen device"))?;
        let vptr = self.build_vtable("CWsScreenDevice", &slots_with_gaps(VTABLE_SLOTS_TOTAL, VTABLE_NAMED))?;
        self.mem.vm.write_u32(dev, vptr).map_err(|_| String::from("the screen device is unwritable"))?;
        self.mem.vm.write_u32(env.wrapping_add(COEENV_I_SCREEN), dev).map_err(|_| String::from("the CEikonEnv is unwritable"))
    }
}

/// The natives of this module; `None` = not ours.
pub(crate) fn native(p: &mut Process, name: &str) -> Option<NativeOutcome> {
    let ret = p.cpu.regs[0];
    Some(match name {
        // TSize CGraphicsDevice::SizeInPixels() const: the TSize (iWidth, iHeight) is returned through the hidden pointer in r0.
        "_ZNK15CWsScreenDevice12SizeInPixelsEv" => {
            let ok = p.mem.vm.write_u32(ret, SCREEN_WIDTH as u32).is_ok() && p.mem.vm.write_u32(ret.wrapping_add(4), SCREEN_HEIGHT as u32).is_ok();
            if ok { NativeOutcome::Return } else { NativeOutcome::Refused(format!("NEXT-NEEDED ws32 {name}: the result at {ret:#x} is unwritable")) }
        }
        _ => return None,
    })
}
