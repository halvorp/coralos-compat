//! DrawSprocket (`DrawSprocketLib`) as a NATIVE library under the games-run-windowed rule: the one main display has a context per mode;
//! reserving it and making it active gives the game its own native window at the requested size -- never a display-mode change and never exclusive full screen. Apple's library is
//! not on the 9.2.1 CD, so the contract is the public API (Game Sprockets reference) and the structure offsets confirmed by systemless (GPL, facts only: the
//! `DSpContextAttributes` offsets frequency 0, width 4, height 8, contextOptions 28, backBufferDepthMask 32, displayDepthMask 36, backBufferBestDepth 40, displayBestDepth 44, pageCount 48).
//! * `DSpGetVersion` answers 1.7.5 as a `NumVersion` (0x01758000, a struct result: written through the hidden pointer in r3): the version Warcraft III requires. `DSpStartup` / `DSpShutdown` bracket everything (-30440 before startup).
//! * `DSpGetFirstContext(displayID, &ctx)`: display ID 1 (the main display; `DMGetDisplayIDByGDevice` answers the same) has one context PER MODE AND DEPTH ([`MODES`] x [`DEPTHS`]): the
//!   main display's own size first (640x480 until a real size is bridged: the game's default stays today's), then the standard 4:3, 5:4, 16:9 and 16:10 modes up to 1920x1200,
//!   each at 16 then 32 bits; `DSpGetNextContext` walks them and ends with kDSpContextNotFoundErr (-30446) and a NIL out. Warcraft III's mode list (FUN_10350930 in the 2026-11-01
//!   game call map) keeps every context whose displayWidth >= 640 and displayBestDepth >= 16, so its Video options list every mode here. A mode is a WINDOW size under the
//!   games-run-windowed rule (the display itself never changes mode): the host scales the window's GL picture to whatever size the user gives the window.
//!   `DSpFindBestContext[OnDisplayID]` answers the exact mode of the requested width/height, else the smallest that holds it, else the largest (a zero request: the first),
//!   and records the request.
//! * `Reserve(ctx, attrs)` records the attributes and makes the (hidden) native window of the requested size; `SetState(Active)` shows and fronts it, `Inactive` hides it (through the
//!   additive `GuestUi::hide_window`); `Release` closes it and the back buffer. Reserving twice is -30444, using an unreserved context -30445, an unknown reference -30446.
//! * `GetBackBuffer` is a `GWorld` of the window's size at the main display's depth ([`crate::gworld`]; a direct-colour depth is refused by name there); `SwapBuffers` copies it to the
//!   window through `CopyBits` (the busy proc is never called: the copy completes at once); `InvalBackBufferRect` is accepted (the whole buffer is copied).
//! * `FadeGamma`, `FadeGammaIn/Out`: the real display gamma is never touched; the fade applies to the game's window only, and the seam has no per-window fade yet, so the request is
//!   RECORDED and logged by name (`[dsp] fade ...`) and has no visible effect (a named limit, not a success of the hardware path).
//! * `DSpCanUserSelectContext` answers FALSE (there is no selection dialog: the game's own options choose a mode) and `DSpUserSelectContext` answers the best context for its request.
//! Error numbers (DrawSprocket.h, remembered; -30444..-30446 confirmed by systemless): kDSpNotInitializedErr -30440, kDSpInvalidContextErr -30442, kDSpContextAlreadyReservedErr -30444,
//! kDSpContextNotReservedErr -30445, kDSpContextNotFoundErr -30446. Not provided, so unbound and stopping by name: the CLUT calls, `SetVBLProc` (no vertical retrace), page flipping beyond the back buffer.
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_mac68k_engine::pascal::Rect;
use heirloom_mac68k_engine::toolbox::WindowSpec;
use crate::cpu::Cpu;

const NOT_INITIALIZED: i32 = -30440;
const INVALID_CONTEXT: i32 = -30442;
const ALREADY_RESERVED: i32 = -30444;
const NOT_RESERVED: i32 = -30445;
const NOT_FOUND: i32 = -30446;
/// The library name.
pub const NAME: &str = "DrawSprocketLib";
/// The first context reference (an opaque id; context `i` is `CONTEXT + i`, mode `i` of [`MODES`]) and the one display's id.
pub const CONTEXT: u32 = 0x4453_5001;
/// The modes after the display's own (index 0): 4:3, 5:4, 16:9 and 16:10, ascending by area. A mode the display's own size already
/// names is not listed twice.
pub const MODES: [(u32, u32); 14] = [
    (800, 600), (1024, 768), (1152, 864), (1280, 720), (1280, 800), (1280, 960), (1280, 1024), (1366, 768), (1440, 900), (1600, 900),
    (1680, 1050), (1600, 1200), (1920, 1080), (1920, 1200),
];
/// The display depths every mode is offered at, as its own context: 16 first (Warcraft III's `colordepth` preference defaults to
/// 16 and its in-game Video Options shows the running mode as `640x480x16`), then 32. Under the GPU path the depth is only what
/// the game is told: the GL picture is ARGB8888 either way.
pub const DEPTHS: [u32; 2] = [16, 32];
pub const MAIN_DISPLAY_ID: u32 = 1;
const VERSION_1_7_5: u32 = 0x0175_8000;
const ATTR_SIZE: u32 = 72;

#[derive(Default)]
pub struct DspState {
    started: bool,
    reserved: bool,
    /// The reserved context (valid while `reserved`).
    ctx: u32,
    play: u32, // 0 active, 1 paused, 2 inactive
    width: u32, height: u32,
    window: u32,
    back: u32,
    max_frame_rate: u32,
    fades: u32,
}

impl DspState {
    /// The reserved context's native window (its guest WindowPtr), 0 when no context is reserved: where the games-windowed rule
    /// shows an `aglSetFullScreen` drawable.
    pub(crate) fn window(&self) -> u32 {
        self.window
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspFn {
    Startup, Shutdown, GetVersion, GetFirstContext, GetNextContext, FindBestContext, FindBestContextOnDisplayID, UserSelectContext, CanUserSelectContext,
    GetAttributes, Reserve, Release, SetState, GetState, GetDisplayID, GetMonitorFrequency, GetBackBuffer, SwapBuffers, InvalBackBufferRect, IsBusy,
    FadeGamma, FadeGammaIn, FadeGammaOut, GetMaxFrameRate, SetMaxFrameRate,
}

impl DspFn {
    pub fn by_name(name: &str) -> Option<DspFn> {
        use DspFn::*;
        Some(match name {
            "DSpStartup" => Startup, "DSpShutdown" => Shutdown, "DSpGetVersion" => GetVersion, "DSpGetFirstContext" => GetFirstContext, "DSpGetNextContext" => GetNextContext,
            "DSpFindBestContext" => FindBestContext, "DSpFindBestContextOnDisplayID" => FindBestContextOnDisplayID, "DSpUserSelectContext" => UserSelectContext,
            "DSpCanUserSelectContext" => CanUserSelectContext, "DSpContext_GetAttributes" => GetAttributes, "DSpContext_Reserve" => Reserve, "DSpContext_Release" => Release,
            "DSpContext_SetState" => SetState, "DSpContext_GetState" => GetState, "DSpContext_GetDisplayID" => GetDisplayID, "DSpContext_GetMonitorFrequency" => GetMonitorFrequency,
            "DSpContext_GetBackBuffer" => GetBackBuffer, "DSpContext_SwapBuffers" => SwapBuffers, "DSpContext_InvalBackBufferRect" => InvalBackBufferRect, "DSpContext_IsBusy" => IsBusy,
            "DSpContext_FadeGamma" => FadeGamma, "DSpContext_FadeGammaIn" => FadeGammaIn, "DSpContext_FadeGammaOut" => FadeGammaOut,
            "DSpContext_GetMaxFrameRate" => GetMaxFrameRate, "DSpContext_SetMaxFrameRate" => SetMaxFrameRate,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn dsp_put32(&mut self, at: u32, v: u32, who: &str) -> Result<(), String> {
        if at == 0 { return Ok(()); }
        self.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: the result pointer {at:#x} leaves guest RAM"))
    }
    /// The main display's size (its GDevice rectangle), 640x480 before QuickDraw exists.
    fn dsp_display_size(&self) -> (u32, u32) {
        if self.mac.cqd.main_device == 0 { return (640, 480); }
        match self.mac.mm.block(self.mac.cqd.main_device).map(|b| b.ptr).and_then(|p| heirloom_mac68k_engine::pascal::read_rect(&self.mac.mem, p + 0x22).ok()) {
            Some(r) if r.right > r.left && r.bottom > r.top => ((r.right - r.left) as u32, (r.bottom - r.top) as u32),
            _ => (640, 480),
        }
    }
    fn dsp_trap(&mut self, name: &str, v: &[u32]) -> Result<u32, String> {
        let op = heirloom_mac68k_engine::mixed::trap_opcode(name).ok_or_else(|| format!("{name} is not a trap"))?;
        self.mac.pascal_call(op, v).map(|r| r.value).map_err(|e| format!("{name}: {e:?}"))
    }
    /// Every context's mode, in context order: the display's own size, then each of [`MODES`] it does not already name.
    fn dsp_modes(&self) -> alloc::vec::Vec<(u32, u32)> {
        let own = self.dsp_display_size();
        core::iter::once(own).chain(MODES.iter().copied().filter(|m| *m != own)).collect()
    }
    /// The mode of context reference `ctx`, None for a reference that names no context. Each mode is two contexts, its
    /// [`DEPTHS`] in order: context `CONTEXT + 2m + d` is mode `m` at depth `DEPTHS[d]`.
    fn dsp_mode_of(&self, ctx: u32) -> Option<(u32, u32)> {
        let i = ctx.checked_sub(CONTEXT)? as usize;
        self.dsp_modes().get(i / DEPTHS.len()).copied()
    }
    /// The display depth context `ctx` reports (its displayBestDepth).
    fn dsp_depth_of(ctx: u32) -> u32 {
        DEPTHS[ctx.wrapping_sub(CONTEXT) as usize % DEPTHS.len()]
    }
    /// The width and height an attributes record asks for ((0, 0) for a NIL record).
    fn dsp_request(&self, attrs: u32) -> Result<(u32, u32), String> {
        Ok((self.dsp_attr(attrs, 4)?, self.dsp_attr(attrs, 8)?))
    }
    /// One u32 field of an attributes record (0 for a NIL record).
    fn dsp_attr(&self, attrs: u32, off: u32) -> Result<u32, String> {
        if attrs == 0 { return Ok(0); }
        self.mac.mem.read_u32(attrs + off).map_err(|_| String::from("DrawSprocket: the attributes leave guest RAM"))
    }
    /// The best context for a requested size (see the module doc), and the request recorded for the next Reserve (a zero keeps the
    /// context's own mode).
    fn dsp_find_best(&mut self, attrs: u32) -> Result<u32, String> {
        let (w, h) = self.dsp_request(attrs)?;
        let i = best_mode(&self.dsp_modes(), (w, h));
        let (mw, mh) = self.dsp_modes()[i];
        self.dsp.width = if w == 0 { mw } else { w };
        self.dsp.height = if h == 0 { mh } else { h };
        // the depth the record asks for (displayBestDepth): the context of that depth, else the first (16)
        let d = DEPTHS.iter().position(|d| *d == self.dsp_attr(attrs, 44).unwrap_or(0)).unwrap_or(0);
        Ok(CONTEXT + (i * DEPTHS.len() + d) as u32)
    }

    pub(crate) fn dsp_call(&mut self, f: DspFn, cpu: &mut Cpu) -> Result<(), String> {
        use DspFn::*;
        let n = format!("DrawSprocket {f:?}");
        let n = n.as_str();
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        if !self.dsp.started && !matches!(f, Startup | Shutdown | GetVersion) { cpu.gpr[3] = NOT_INITIALIZED as u32; return Ok(()); }
        // calls on a context reference
        let on_ctx = matches!(f, GetAttributes | Reserve | Release | SetState | GetState | GetDisplayID | GetMonitorFrequency | GetBackBuffer | SwapBuffers | InvalBackBufferRect | IsBusy | FadeGamma | FadeGammaIn | FadeGammaOut | GetMaxFrameRate | SetMaxFrameRate | GetNextContext);
        if on_ctx && self.dsp_mode_of(a0).is_none() { if f == GetNextContext { self.dsp_put32(a1, 0, n)?; } cpu.gpr[3] = (if f == GetNextContext { NOT_FOUND } else { INVALID_CONTEXT }) as u32; return Ok(()); }
        // a call that needs the reserved context, on another (valid) one: that context is not reserved
        let needs_reserved = matches!(f, Release | SetState | GetBackBuffer | SwapBuffers);
        if needs_reserved && self.dsp.reserved && a0 != self.dsp.ctx { cpu.gpr[3] = NOT_RESERVED as u32; return Ok(()); }
        let r: i32 = match f {
            Startup => { self.dsp.started = true; 0 }
            Shutdown => { self.dsp_release()?; self.dsp.started = false; 0 }
            // NumVersion is a struct returned by value: the Mac OS PowerPC ABI passes the address of the result in r3 (measured: the game's r3 is a stack address) and expects it back
            GetVersion => { self.dsp_put32(a0, VERSION_1_7_5, n)?; a0 as i32 }
            GetFirstContext => if a0 == MAIN_DISPLAY_ID { self.dsp_put32(a1, CONTEXT, n)?; 0 } else { self.dsp_put32(a1, 0, n)?; NOT_FOUND },
            GetNextContext => match self.dsp_mode_of(a0 + 1) { Some(_) => { self.dsp_put32(a1, a0 + 1, n)?; 0 } None => { self.dsp_put32(a1, 0, n)?; NOT_FOUND } },
            FindBestContext => { let c = self.dsp_find_best(a0)?; self.dsp_put32(a1, c, n)?; 0 }
            FindBestContextOnDisplayID => if a2 == MAIN_DISPLAY_ID { let c = self.dsp_find_best(a0)?; self.dsp_put32(a1, c, n)?; 0 } else { self.dsp_put32(a1, 0, n)?; NOT_FOUND },
            // DSpUserSelectContext(attributes, displayID, eventProc, &context): no dialog, the best context for the request
            UserSelectContext => { let c = self.dsp_find_best(a0)?; self.dsp_put32(a3, c, n)?; 0 }
            CanUserSelectContext => { if a1 != 0 { self.mac.mem.write_u8(a1, 0).map_err(|_| format!("{n}: the result pointer leaves guest RAM"))?; } 0 }
            GetAttributes => {
                // a context's attributes are its mode (the reserved one: the size it was reserved at)
                let (w, h) = if self.dsp.reserved && a0 == self.dsp.ctx { (self.dsp.width, self.dsp.height) } else { self.dsp_mode_of(a0).unwrap_or((640, 480)) };
                let depth = if self.mac.cqd.main_device != 0 { self.mac.device_pixmap(self.mac.cqd.main_device).map(|d| d.0).unwrap_or(8) } else { 8 };
                let mask = 1u32 << depth.trailing_zeros() as u32 + 0; // depth 1,2,4,8,16,32 -> bit 0..5 (kDSpDepthMask_n = 1 << log2(n))
                self.put_bytes(a1, &alloc::vec![0u8; ATTR_SIZE as usize], n)?;
                // the DISPLAY's depths are the host's: a true-colour display that presents 8-, 16- and 32-bit pictures (the GPU path composes every guest
                // depth), so displayDepthMask = 8|16|32 and displayBestDepth = 32 (Warcraft III keeps only a context whose displayBestDepth >= 16 and
                // displayWidth >= 640: its mode list, FUN_10350930 in the 2026-11-01 game call map). The BACK BUFFER stays the main device's depth: a
                // DSp back buffer is a GWorld at that depth (GetBackBuffer), so its mask and best depth say what GetBackBuffer gives.
                let display_mask = (1u32 << 3) | (1 << 4) | (1 << 5);
                for (o, v) in [(0u32, 60u32 << 16), (4, w), (8, h), (28, 0), (32, mask), (36, display_mask), (40, depth), (44, Self::dsp_depth_of(a0)), (48, 1)] { self.dsp_put32(a1 + o, v, n)?; }
                0
            }
            Reserve => {
                if self.dsp.reserved { ALREADY_RESERVED } else {
                    // the request (a zero keeps the context's own mode)
                    let (rw, rh) = self.dsp_request(a1)?;
                    let (mw, mh) = self.dsp_mode_of(a0).unwrap_or((640, 480));
                    self.dsp.width = if rw == 0 { mw } else { rw };
                    self.dsp.height = if rh == 0 { mh } else { rh };
                    self.dsp.ctx = a0;
                    let (w, h) = (self.dsp.width as i16, self.dsp.height as i16);
                    // the game's own native window, hidden until the context goes active (documentProc without a grow box: a titled, movable window)
                    let spec = WindowSpec { storage: 0, rect: Rect { top: 60, left: 20, bottom: 60 + h, right: 20 + w }, title: String::new(), visible: false, proc_id: 4, behind: 0xFFFF_FFFF, go_away: false, ref_con: 0 };
                    let win = self.mac.make_window(spec).map_err(|e| format!("{n}: {}", e.0))?;
                    self.dsp.window = win; self.dsp.reserved = true; self.dsp.play = 2;
                    self.mac.log.push(format!("[dsp] context reserved: a native {w}x{h} window {win:#x} (windowed, no display mode change)"));
                    0
                }
            }
            Release => if !self.dsp.reserved { NOT_RESERVED } else { self.dsp_release()?; 0 },
            // kDSpContextState_Active 0, Paused 1, Inactive 2
            SetState => if !self.dsp.reserved { NOT_RESERVED } else {
                match a1 {
                    0 => { let w = self.dsp.window; self.dsp_trap("ShowWindow", &[w])?; self.dsp_trap("SelectWindow", &[w])?; self.dsp.play = 0; 0 }
                    1 => { self.dsp.play = 1; 0 }
                    2 => { let w = self.dsp.window; self.dsp_trap("HideWindow", &[w])?; self.dsp.play = 2; 0 }
                    s => return Err(format!("{n}: context state {s} is not Active (0), Paused (1) or Inactive (2)")),
                }
            },
            GetState => { self.dsp_put32(a1, if self.dsp.reserved { self.dsp.play } else { 2 }, n)?; 0 }
            GetDisplayID => { self.dsp_put32(a1, MAIN_DISPLAY_ID, n)?; 0 }
            GetMonitorFrequency => { self.dsp_put32(a1, 60 << 16, n)?; 0 }
            IsBusy => { if a1 != 0 { self.mac.mem.write_u8(a1, 0).ok(); } 0 }
            GetMaxFrameRate => { let v = self.dsp.max_frame_rate; self.dsp_put32(a1, v, n)?; 0 }
            SetMaxFrameRate => { self.dsp.max_frame_rate = a1; 0 }
            // GetBackBuffer(ctx, bufferKind, CGrafPtr *): a GWorld of the window's size at the main display's depth
            GetBackBuffer => if !self.dsp.reserved { NOT_RESERVED } else {
                if self.dsp.back == 0 {
                    let b = Rect { top: 0, left: 0, bottom: self.dsp.height as i16, right: self.dsp.width as i16 };
                    match self.mac.gw_new(0, b, 0, 0).map_err(|e| format!("{n}: {}", e.0))? { Ok(p) => self.dsp.back = p, Err(code) => return Ok(cpu.gpr[3] = i32::from(code) as u32) }
                }
                let p = self.dsp.back;
                self.dsp_put32(a2, p, n)?;
                0
            },
            // SwapBuffers(ctx, busyProc, refCon): back buffer -> the window, completed at once
            SwapBuffers => if !self.dsp.reserved { NOT_RESERVED } else {
                if self.dsp.back != 0 {
                    let tmp = self.mac.mm.new_ptr(&mut self.mac.mem, &mut self.mac.heap, 8, true).map_err(|_| format!("{n}: out of memory"))?;
                    for (i, v) in [0i16, 0, self.dsp.height as i16, self.dsp.width as i16].into_iter().enumerate() { let _ = self.mac.mem.write_u16(tmp + 2 * i as u32, v as u16); }
                    let (src, dst) = (self.mac.port_bits_for_copybits(self.dsp.back), self.mac.port_bits_for_copybits(self.dsp.window));
                    let r = match (src, dst) { (Ok(s), Ok(d)) => self.dsp_trap("CopyBits", &[s, d, tmp, tmp, 0, 0]).map(|_| ()), (Err(e), _) | (_, Err(e)) => Err(format!("{n}: {}", e.0)) };
                    let _ = self.mac.mm.dispose_ptr(&mut self.mac.heap, tmp);
                    r?;
                }
                0
            },
            InvalBackBufferRect => 0,
            FadeGamma | FadeGammaIn | FadeGammaOut => {
                self.dsp.fades += 1;
                self.mac.log.push(format!("[dsp] fade ({f:?}{}) recorded: it applies to the game's window only and the seam has no per-window fade yet; the display gamma is not touched", if f == FadeGamma { format!(", {}%", a1 as i32) } else { String::new() }));
                0
            }
        };
        self.mac.log.push(format!("[dsp] {f:?}({a0:#x}, {a1:#x}, {a2:#x}) -> {r:#x}"));
        cpu.gpr[3] = r as u32;
        Ok(())
    }

    /// Release the context: close the native window and the back buffer.
    fn dsp_release(&mut self) -> Result<(), String> {
        if self.dsp.back != 0 { let b = self.dsp.back; self.mac.gw_dispose(b).map_err(|e| e.0)?; self.dsp.back = 0; }
        if self.dsp.window != 0 { let w = self.dsp.window; self.dsp_trap("DisposeWindow", &[w])?; self.dsp.window = 0; }
        self.dsp.reserved = false; self.dsp.play = 2;
        Ok(())
    }
}

/// The index of the best mode for a requested size: the exact one, else the smallest (by area) that holds it, else the largest; a zero
/// request is the first (the display's own).
pub fn best_mode(modes: &[(u32, u32)], want: (u32, u32)) -> usize {
    if want == (0, 0) || modes.is_empty() { return 0; }
    if let Some(i) = modes.iter().position(|m| *m == want) { return i; }
    let area = |m: &(u32, u32)| u64::from(m.0) * u64::from(m.1);
    let holds = modes.iter().enumerate().filter(|(_, m)| m.0 >= want.0 && m.1 >= want.1).min_by_key(|(_, m)| area(m)).map(|(i, _)| i);
    holds.unwrap_or_else(|| modes.iter().enumerate().max_by_key(|(_, m)| area(m)).map_or(0, |(i, _)| i))
}
