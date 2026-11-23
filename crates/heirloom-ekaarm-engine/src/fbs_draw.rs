//! `CFbsBitGc` / `CFbsDevice` rasterization and screen-bitmap presentation (`fbs.rs` split).
use alloc::string::String;
use alloc::vec;
use crate::fbs::{
    DEV_SLOTS_TOTAL, DEV_VTABLE, ECOLOR_16MU, FbsBitmap, FbsDeviceState, FbsGcState,
    GC_SLOTS_TOTAL, GC_VTABLE,
};
use crate::loader::Process;
use crate::mbm::glyph_5x7;
use crate::objects::HostObj;
use crate::enginestub::slots_with_gaps;
use heirloom_ui_abi::{DrawOp, GuestRect, Plane};

impl Process {
    pub(crate) fn rd_point(&self, addr: u32) -> Result<(i32, i32), String> {
        Ok((self.rd(addr)? as i32, self.rd(addr + 4)? as i32))
    }
    pub(crate) fn rd_rect(&self, addr: u32) -> Result<[i32; 4], String> {
        Ok([
            self.rd(addr)? as i32,
            self.rd(addr + 4)? as i32,
            self.rd(addr + 8)? as i32,
            self.rd(addr + 12)? as i32,
        ])
    }
    pub(crate) fn rd_rgb(&self, addr: u32) -> Result<u32, String> {
        // TRgb::iValue is 0xAARRGGBB (gdi.inl: Red=(v>>16)&0xff, Green=(v>>8)&0xff, Blue=v&0xff)
        Ok(self.rd(addr)? & 0x00FF_FFFF)
    }

    pub(crate) fn bitmap_handle_of(&mut self, addr: u32) -> Option<u32> {
        match self.obj_get(addr) {
            Some(HostObj::FbsBitmap(h)) => Some(*h),
            _ => None,
        }
    }

    pub(crate) fn make_fbs_device(&mut self, bitmap_handle: u32, is_screen: bool) -> Result<u32, String> {
        let dev = self.alloc_zeroed(0x40).ok_or_else(|| String::from("no heap for CFbsDevice"))?;
        let vptr = self.build_vtable("CFbsDevice", &slots_with_gaps(DEV_SLOTS_TOTAL, DEV_VTABLE))?;
        self.wr(dev, vptr)?;
        self.obj_set(dev, HostObj::FbsDevice(FbsDeviceState { bitmap_handle, is_screen }))?;
        Ok(dev)
    }

    pub(crate) fn make_fbs_gc(&mut self, device_addr: u32, bitmap_handle: u32) -> Result<u32, String> {
        let gc = self.alloc_zeroed(0x40).ok_or_else(|| String::from("no heap for CFbsBitGc"))?;
        let vptr = self.build_vtable("CFbsBitGc", &slots_with_gaps(GC_SLOTS_TOTAL, GC_VTABLE))?;
        self.wr(gc, vptr)?;
        self.obj_set(gc, HostObj::FbsGc(FbsGcState::new(device_addr, bitmap_handle)))?;
        Ok(gc)
    }

    /// Return `(screen_bmp_handle, gc_addr)` for `CCoeControl::SystemGc()` / `CCoeEnv::SystemGc()`, creating it on first use.
    pub(crate) fn ensure_system_gc(&mut self) -> Result<(u32, u32), String> {
        if let Some(pair) = self.nat.fbs.system_gc {
            return Ok(pair);
        }
        let (sw, sh) = self.screen_size();
        let bmp_h = self.nat.fbs.insert_bitmap(FbsBitmap::new(sw, sh, ECOLOR_16MU));
        let dev = self.make_fbs_device(bmp_h, true)?;
        let gc = self.make_fbs_gc(dev, bmp_h)?;
        self.nat.fbs.system_gc = Some((bmp_h, gc));
        Ok((bmp_h, gc))
    }

    pub(crate) fn present_screen_bitmap(&mut self, bmp_handle: u32) -> Result<(), String> {
        let Some(win) = self.host.window else { return Ok(()) };
        let Some(bmp) = self.nat.fbs.bitmaps.get(&bmp_handle) else { return Ok(()) };
        let rect = GuestRect { left: 0, top: 0, width: bmp.width, height: bmp.height };
        let plane = Plane {
            id: 0,
            rect,
            ops: vec![DrawOp::Pixels { rect, rgb: bmp.pixels.clone() }],
        };
        if let Some(ui) = self.host.ui.as_mut() {
            let _ = ui.present_plane(win, plane);
        }
        Ok(())
    }

    pub(crate) fn present_rgb_pixels(&mut self, width: i32, height: i32, rgb: alloc::vec::Vec<u32>) {
        let Some(win) = self.host.window else { return };
        let rect = GuestRect { left: 0, top: 0, width, height };
        let plane = Plane {
            id: 0,
            rect,
            ops: vec![DrawOp::Pixels { rect, rgb }],
        };
        if let Some(ui) = self.host.ui.as_mut() {
            let _ = ui.present_plane(win, plane);
        }
    }

    pub(crate) fn sync_bitmap_from_guest(&mut self, bmp_h: u32) {
        let Some((w, h, mode, stride, Some(addr))) = self.nat.fbs.bitmaps.get(&bmp_h).map(|b| {
            (b.width.max(0) as usize, b.height.max(0) as usize, b.display_mode, b.scanline_bytes(), b.data_addr)
        }) else {
            return;
        };
        let mut raw = vec![0u8; stride * h];
        if raw.is_empty() || self.mem.vm.read_bytes(addr, &mut raw).is_err() {
            return;
        }
        if let Some(bmp) = self.nat.fbs.bitmaps.get_mut(&bmp_h) {
            for y in 0..h {
                let row = y * stride;
                for x in 0..w {
                    let px = match mode {
                        1 => if (raw[row + x / 8] >> (x & 7)) & 1 != 0 { 0x00FF_FFFF } else { 0 },
                        2 => { let v = (((raw[row + x / 4] >> ((x & 3) * 2)) & 3) as u32) * 85; (v << 16) | (v << 8) | v }
                        3 | 5 => { let v = (((raw[row + x / 2] >> ((x & 1) * 4)) & 15) as u32) * 17; (v << 16) | (v << 8) | v }
                        4 | 6 => { let v = u32::from(raw[row + x]); (v << 16) | (v << 8) | v }
                        7 => {
                            let word = u16::from_le_bytes([raw[row + x * 2], raw[row + x * 2 + 1]]);
                            let r = (((word >> 11) & 0x1F) as u32 * 255) / 31;
                            let g = (((word >> 5) & 0x3F) as u32 * 255) / 63;
                            let b = ((word & 0x1F) as u32 * 255) / 31;
                            (r << 16) | (g << 8) | b
                        }
                        8 => (u32::from(raw[row + x * 3 + 2]) << 16) | (u32::from(raw[row + x * 3 + 1]) << 8) | u32::from(raw[row + x * 3]),
                        10 => {
                            let word = u16::from_le_bytes([raw[row + x * 2], raw[row + x * 2 + 1]]);
                            let r = (((word >> 8) & 0xF) as u32) * 17;
                            let g = (((word >> 4) & 0xF) as u32) * 17;
                            let b = ((word & 0xF) as u32) * 17;
                            (r << 16) | (g << 8) | b
                        }
                        _ => (u32::from(raw[row + x * 4 + 2]) << 16) | (u32::from(raw[row + x * 4 + 1]) << 8) | u32::from(raw[row + x * 4]),
                    };
                    bmp.pixels[y * w + x] = px;
                }
            }
        }
    }

    pub(crate) fn gc_blit(
        &mut self,
        gc_addr: u32,
        dst_pt: (i32, i32),
        src_bmp_addr: u32,
        src_rect: Option<[i32; 4]>,
        mask_bmp_addr: Option<u32>,
        invert_mask: bool,
    ) -> Result<(), String> {
        let (dst_h, origin, clip) = match self.obj_get(gc_addr) {
            Some(HostObj::FbsGc(g)) => (g.bitmap_handle, g.origin, g.clip_rect),
            _ => return Ok(()),
        };
        let Some(src_h) = self.bitmap_handle_of(src_bmp_addr) else { return Ok(()) };
        self.sync_bitmap_from_guest(src_h);
        let Some(src) = self.nat.fbs.bitmaps.get(&src_h).cloned() else { return Ok(()) };
        let mask = match mask_bmp_addr {
            Some(a) if a != 0 => self.bitmap_handle_of(a).and_then(|h| self.nat.fbs.bitmaps.get(&h).cloned()),
            _ => None,
        };
        let sr = src_rect.unwrap_or([0, 0, src.width, src.height]);
        let sw = (sr[2] - sr[0]).max(0);
        let sh = (sr[3] - sr[1]).max(0);
        let ox = dst_pt.0 + origin.0;
        let oy = dst_pt.1 + origin.1;
        let Some(dst) = self.nat.fbs.bitmaps.get_mut(&dst_h) else { return Ok(()) };
        // Full-screen rotated blit: when a landscape-pitched backbuffer (e.g. 640x360) is blitted at (0,0) onto a
        // portrait screen device (360x640), apply the 90 deg CW display scanout transform so UI and touch coords align.
        if dst_pt == (0, 0) && origin == (0, 0) && src_rect.is_none() && mask.is_none()
            && src.width == dst.height && src.height == dst.width && dst.width != dst.height
        {
            for sy in 0..src.height {
                let tx = dst.width - 1 - sy;
                for sx in 0..src.width {
                    if let Some(spx) = src.get(sx, sy) {
                        dst.set(tx, sx, spx);
                    }
                }
            }
            return Ok(());
        }
        for dy in 0..sh {
            let ty = oy + dy;
            if ty < 0 || ty >= dst.height { continue; }
            if let Some(c) = clip { if ty < c[1] || ty >= c[3] { continue; } }
            let sy = sr[1] + dy;
            for dx in 0..sw {
                let tx = ox + dx;
                if tx < 0 || tx >= dst.width { continue; }
                if let Some(c) = clip { if tx < c[0] || tx >= c[2] { continue; } }
                let sx = sr[0] + dx;
                let Some(spx) = src.get(sx, sy) else { continue };
                if let Some(ref m) = mask {
                    let mpx = m.get(sx, sy).or_else(|| m.get(dx, dy)).unwrap_or(0);
                    let mval = (mpx & 0xFF) as u32;
                    if m.display_mode == 1 {
                        // Symbian 1-bit EGray2 mask: black (0) = opaque, white (255) = transparent unless inverted.
                        let opaque = if invert_mask { mval > 127 } else { mval <= 127 };
                        if !opaque { continue; }
                        dst.set(tx, ty, spx);
                    } else {
                        // Symbian EGray256 alpha mask: 255 = opaque, 0 = transparent (invert_mask is ignored on alpha masks).
                        let a = mval;
                        if a == 0 { continue; }
                        if a == 255 {
                            dst.set(tx, ty, spx);
                        } else {
                            let dpx = dst.get(tx, ty).unwrap_or(0);
                            let inv = 255 - a;
                            let r = (((spx >> 16) & 0xFF) * a + ((dpx >> 16) & 0xFF) * inv) / 255;
                            let g = (((spx >> 8) & 0xFF) * a + ((dpx >> 8) & 0xFF) * inv) / 255;
                            let b = ((spx & 0xFF) * a + (dpx & 0xFF) * inv) / 255;
                            dst.set(tx, ty, (r << 16) | (g << 8) | b);
                        }
                    }
                } else {
                    dst.set(tx, ty, spx);
                }
            }
        }
        Ok(())
    }

    /// `CFbsBitGc::DrawBitmap` / `DrawBitmapMasked` with a destination `TRect`:
    /// stretches or shrinks `src_rect` (or the whole source bitmap) to fill `dst_rect`.
    pub(crate) fn gc_draw_bitmap_scaled(
        &mut self,
        gc_addr: u32,
        dst_rect: [i32; 4],
        src_bmp_addr: u32,
        src_rect: Option<[i32; 4]>,
        mask_bmp_addr: Option<u32>,
        invert_mask: bool,
    ) -> Result<(), String> {
        let dw = dst_rect[2] - dst_rect[0];
        let dh = dst_rect[3] - dst_rect[1];
        if dw <= 0 || dh <= 0 { return Ok(()); }
        let (dst_h, origin, clip) = match self.obj_get(gc_addr) {
            Some(HostObj::FbsGc(g)) => (g.bitmap_handle, g.origin, g.clip_rect),
            _ => return Ok(()),
        };
        let Some(src_h) = self.bitmap_handle_of(src_bmp_addr) else { return Ok(()) };
        let Some(src) = self.nat.fbs.bitmaps.get(&src_h).cloned() else { return Ok(()) };
        let sr = src_rect.unwrap_or([0, 0, src.width, src.height]);
        let sw = sr[2] - sr[0];
        let sh = sr[3] - sr[1];
        if sw <= 0 || sh <= 0 { return Ok(()); }
        if dw == sw && dh == sh {
            return self.gc_blit(gc_addr, (dst_rect[0], dst_rect[1]), src_bmp_addr, src_rect, mask_bmp_addr, invert_mask);
        }
        let mask = match mask_bmp_addr {
            Some(a) if a != 0 => self.bitmap_handle_of(a).and_then(|h| self.nat.fbs.bitmaps.get(&h).cloned()),
            _ => None,
        };
        let ox = dst_rect[0] + origin.0;
        let oy = dst_rect[1] + origin.1;
        let Some(dst) = self.nat.fbs.bitmaps.get_mut(&dst_h) else { return Ok(()) };
        for dy in 0..dh {
            let ty = oy + dy;
            if ty < 0 || ty >= dst.height { continue; }
            if let Some(c) = clip { if ty < c[1] || ty >= c[3] { continue; } }
            let sy = sr[1] + ((dy as i64 * sh as i64) / dh as i64) as i32;
            let mdy = ((dy as i64 * sh as i64) / dh as i64) as i32;
            for dx in 0..dw {
                let tx = ox + dx;
                if tx < 0 || tx >= dst.width { continue; }
                if let Some(c) = clip { if tx < c[0] || tx >= c[2] { continue; } }
                let sx = sr[0] + ((dx as i64 * sw as i64) / dw as i64) as i32;
                let mdx = ((dx as i64 * sw as i64) / dw as i64) as i32;
                let Some(spx) = src.get(sx, sy) else { continue };
                if let Some(ref m) = mask {
                    let mpx = m.get(sx, sy).or_else(|| m.get(mdx, mdy)).unwrap_or(0);
                    let mval = (mpx & 0xFF) as u32;
                    if m.display_mode == 1 {
                        let opaque = if invert_mask { mval > 127 } else { mval <= 127 };
                        if !opaque { continue; }
                        dst.set(tx, ty, spx);
                    } else {
                        let a = mval;
                        if a == 0 { continue; }
                        if a == 255 {
                            dst.set(tx, ty, spx);
                        } else {
                            let dpx = dst.get(tx, ty).unwrap_or(0);
                            let inv = 255 - a;
                            let r = (((spx >> 16) & 0xFF) * a + ((dpx >> 16) & 0xFF) * inv) / 255;
                            let g = (((spx >> 8) & 0xFF) * a + ((dpx >> 8) & 0xFF) * inv) / 255;
                            let b = ((spx & 0xFF) * a + (dpx & 0xFF) * inv) / 255;
                            dst.set(tx, ty, (r << 16) | (g << 8) | b);
                        }
                    }
                } else {
                    dst.set(tx, ty, spx);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn gc_fill_rect(&mut self, gc_addr: u32, rect: Option<[i32; 4]>, color: u32) {
        let (dst_h, origin, clip) = match self.obj_get(gc_addr) {
            Some(HostObj::FbsGc(g)) => (g.bitmap_handle, g.origin, g.clip_rect),
            _ => return,
        };
        let Some(dst) = self.nat.fbs.bitmaps.get_mut(&dst_h) else { return };
        let r = rect.map(|r| [r[0] + origin.0, r[1] + origin.1, r[2] + origin.0, r[3] + origin.1])
            .unwrap_or([0, 0, dst.width, dst.height]);
        let (mut x0, mut y0, mut x1, mut y1) = (r[0].max(0), r[1].max(0), r[2].min(dst.width), r[3].min(dst.height));
        if let Some(c) = clip {
            x0 = x0.max(c[0]); y0 = y0.max(c[1]); x1 = x1.min(c[2]); y1 = y1.min(c[3]);
        }
        for y in y0..y1 {
            for x in x0..x1 {
                dst.set(x, y, color);
            }
        }
    }

    pub(crate) fn gc_draw_text(&mut self, gc_addr: u32, text: &str, x: i32, baseline_y: i32) {
        let (dst_h, origin, clip, color) = match self.obj_get(gc_addr) {
            Some(HostObj::FbsGc(g)) => (g.bitmap_handle, g.origin, g.clip_rect, g.pen_color),
            _ => return,
        };
        let Some(dst) = self.nat.fbs.bitmaps.get_mut(&dst_h) else { return };
        let mut cx = x + origin.0;
        let top_y = baseline_y - 7 + origin.1;
        for ch in text.chars() {
            let rows = glyph_5x7(ch);
            for (ry, bits) in rows.iter().enumerate() {
                let ty = top_y + ry as i32;
                if ty < 0 || ty >= dst.height { continue; }
                if let Some(c) = clip { if ty < c[1] || ty >= c[3] { continue; } }
                for rx in 0..5 {
                    if (bits >> (4 - rx)) & 1 != 0 {
                        let tx = cx + rx;
                        if tx < 0 || tx >= dst.width { continue; }
                        if let Some(c) = clip { if tx < c[0] || tx >= c[2] { continue; } }
                        dst.set(tx, ty, color);
                    }
                }
            }
            cx += 6;
        }
    }
}
