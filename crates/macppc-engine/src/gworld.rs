//! Offscreen graphics worlds and the Carbon port accessors for PowerPC programs: the C names (`NewGWorld`, `LockPixels`, `GetPortBitMapForCopyBits` ...) bound onto the bodies
//! `mac68k-engine` keeps ONCE in `cqd/gworld.rs` (the 68k personality shares the same world records). The pixels are guest memory; `CopyBits` between a world and a window is the
//! shared trap, which presents through the native path. Direct (16/32 bit) worlds are refused by name by the shared body.
//! `GetPortBounds`/`GetWindowPortBounds` read the port's `portRect`; `GetPortBitMapForCopyBits` is the port's `portBits`/PixMap address (the thing `CopyBits` takes).
use crate::clib::arg;
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use guestui_abi::GuestUi;
use mac68k_engine::pascal::Rect;
use mac68k_engine::qd::port::P_RECT;
use retro_ppc::Cpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GwFn {
    NewGWorld, UpdateGWorld, DisposeGWorld, GetGWorld, SetGWorld, LockPixels, UnlockPixels, GetGWorldPixMap, GetGWorldDevice, GetPixBaseAddr, GetPixRowBytes,
    GetPixelsState, SetPixelsState, AllowPurgePixels, NoPurgePixels, PixMap32Bit, GetPortBounds, GetWindowPortBounds, GetPortBitMapForCopyBits, GetPixBounds,
}

impl GwFn {
    pub fn by_name(name: &str) -> Option<GwFn> {
        use GwFn::*;
        Some(match name {
            "NewGWorld" => NewGWorld, "UpdateGWorld" => UpdateGWorld, "DisposeGWorld" => DisposeGWorld, "GetGWorld" => GetGWorld, "SetGWorld" => SetGWorld,
            "LockPixels" => LockPixels, "UnlockPixels" => UnlockPixels, "GetGWorldPixMap" => GetGWorldPixMap, "GetGWorldDevice" => GetGWorldDevice,
            "GetPixBaseAddr" => GetPixBaseAddr, "GetPixRowBytes" => GetPixRowBytes, "GetPixelsState" => GetPixelsState, "SetPixelsState" => SetPixelsState,
            "AllowPurgePixels" => AllowPurgePixels, "NoPurgePixels" => NoPurgePixels, "PixMap32Bit" => PixMap32Bit,
            "GetPortBounds" => GetPortBounds, "GetWindowPortBounds" => GetWindowPortBounds, "GetPortBitMapForCopyBits" => GetPortBitMapForCopyBits, "GetPixBounds" => GetPixBounds,
            _ => return None,
        })
    }
}

impl<U: GuestUi> MacPpc<U> {
    fn rect_at(&self, at: u32, who: &str) -> Result<Rect, String> {
        let r = |o| self.mac.mem.read_u16(at.wrapping_add(o)).map(|v| v as i16).map_err(|_| format!("{who}: the Rect at {at:#x} leaves guest RAM"));
        Ok(Rect { top: r(0)?, left: r(2)?, bottom: r(4)?, right: r(6)? })
    }
    fn put_rect(&mut self, at: u32, r: Rect, who: &str) -> Result<(), String> {
        let m = &mut self.mac.mem;
        m.write_u16(at, r.top as u16).and_then(|_| m.write_u16(at + 2, r.left as u16)).and_then(|_| m.write_u16(at + 4, r.bottom as u16)).and_then(|_| m.write_u16(at + 6, r.right as u16))
            .map_err(|_| format!("{who}: the Rect at {at:#x} leaves guest RAM"))
    }
    fn put_u32(&mut self, at: u32, v: u32, who: &str) -> Result<(), String> { self.mac.mem.write_u32(at, v).map_err(|_| format!("{who}: the result pointer {at:#x} leaves guest RAM")) }

    pub(crate) fn gw_call(&mut self, f: GwFn, cpu: &mut Cpu) -> Result<(), String> {
        use GwFn::*;
        let name = format!("{f:?}");
        let (a0, a1, a2, a3) = (arg(self, cpu, 0)?, arg(self, cpu, 1).unwrap_or(0), arg(self, cpu, 2).unwrap_or(0), arg(self, cpu, 3).unwrap_or(0));
        let e = |r: mac68k_engine::Refusal| r.0;
        let r: u32 = match f {
            // NewGWorld(GWorldPtr *offscreenGWorld, short pixelDepth, const Rect *boundsRect, CTabHandle cTable, GDHandle aGDevice, GWorldFlags flags) : QDErr
            NewGWorld => {
                let bounds = self.rect_at(a2, &name)?;
                let flags = arg(self, cpu, 5)?;
                let _ = a3; // aGDevice: the world makes its own device record
                match self.mac.gw_new(a1 as u16, bounds, arg(self, cpu, 3)?, flags).map_err(e)? {
                    Ok(port) => { self.put_u32(a0, port, &name)?; self.mac.cqd.qd_error = 0; 0 }
                    Err(code) => { self.put_u32(a0, 0, &name)?; self.mac.cqd.qd_error = code; code as i32 as u32 }
                }
            }
            // UpdateGWorld(GWorldPtr *offscreenGWorld, short pixelDepth, const Rect *boundsRect, CTabHandle cTable, GDHandle aGDevice, GWorldFlags flags) : GWorldFlags
            UpdateGWorld => {
                let old = self.mac.mem.read_u32(a0).map_err(|_| format!("{name}: the GWorldPtr at {a0:#x} leaves guest RAM"))?;
                let bounds = self.rect_at(a2, &name)?;
                let (flags, port) = self.mac.gw_update(old, a1 as u16, bounds, arg(self, cpu, 3)?, arg(self, cpu, 5)?).map_err(e)?;
                self.put_u32(a0, port, &name)?;
                flags
            }
            DisposeGWorld => { self.mac.gw_dispose(a0).map_err(e)?; 0 }
            // GetGWorld(CGrafPtr *port, GDHandle *gdh)
            GetGWorld => { let (p, d) = self.mac.gw_get().map_err(e)?; self.put_u32(a0, p, &name)?; self.put_u32(a1, d, &name)?; 0 }
            SetGWorld => { self.mac.gw_set(a0, a1).map_err(e)?; 0 }
            LockPixels => u32::from(self.mac.gw_lock(a0).map_err(e)?),
            UnlockPixels => { self.mac.gw_unlock(a0).map_err(e)?; 0 }
            GetGWorldPixMap => self.mac.gw_pixmap(a0).map_err(e)?,
            GetGWorldDevice => self.mac.gw_device(a0).map_err(e)?,
            GetPixBaseAddr => self.mac.pix_base_addr(a0).map_err(e)?,
            GetPixRowBytes => self.mac.pix_row_bytes(a0).map_err(e)?,
            GetPixelsState => self.mac.gw_state(a0).map_err(e)?,
            SetPixelsState => { self.mac.gw_set_state(a0, a1).map_err(e)?; 0 }
            AllowPurgePixels => { self.mac.gw_purge(a0, true).map_err(e)?; 0 }
            NoPurgePixels => { self.mac.gw_purge(a0, false).map_err(e)?; 0 }
            // the pixels are addressed with 32 bits in this machine
            PixMap32Bit => 1,
            // GetPortBounds(CGrafPtr port, Rect *rect) -> rect ; GetWindowPortBounds(WindowRef, Rect*): a window IS its port
            GetPortBounds | GetWindowPortBounds => { let r = self.rect_at(a0.wrapping_add(P_RECT), &name)?; self.put_rect(a1, r, &name)?; cpu.gpr[3] = a1; return Ok(()); }
            GetPixBounds => { let r = self.mac.pix_bounds(a0).map_err(e)?; self.put_rect(a1, r, &name)?; cpu.gpr[3] = a1; return Ok(()); }
            GetPortBitMapForCopyBits => self.mac.port_bits_for_copybits(a0).map_err(e)?,
        };
        cpu.gpr[3] = r;
        Ok(())
    }
}
