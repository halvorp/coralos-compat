//! Direct writes to the screen's frame buffer, shown in the windows over them.
//!
//! QuickDraw here draws into each window's canvas; the frame buffer at `ScrnBase` (the main device's PixMap `baseAddr`) is real guest
//! RAM that nothing writes -- until a program does it itself: a game that has opened a full-screen window and then blits its pictures
//! straight into video memory with its own code (Glider PRO does, after `SwapMMUMode`). On a Mac a pixel written there simply appears,
//! in whatever window is over it. This module does that: at a step after something wrote the frame buffer it compares it with what it showed last time, and
//! every pixel that CHANGED is copied into the canvas of the topmost visible window whose content covers it (damage and the display list
//! follow, through the ordinary `target::finish`). Pixels no window covers have no desktop to appear on and are dropped; a window whose
//! canvas is not the screen's depth (a 1-bit window under an 8-bit screen) is skipped and logged once.
//!
//! Only CHANGED pixels are copied, so what QuickDraw drew into a canvas is never overwritten by the (blank) buffer beneath.
//!
//! The buffer is compared only when something WROTE to it (h86 aot-step, 2026-11-04): after each comparison its pages are put
//! under guest RAM's write watch (heirloom-mem-flat32 watch.rs, the code-page generations - every write to guest RAM, by the CPU,
//! a native or the loader, passes it), and the next step compares again only if one of their generations moved. On the Z83 the
//! unconditional compare was the frame ledger's `step_other`: ~3 ms a Warcraft III frame (14 steps x the whole 8-bit screen
//! through memcmp, the L2 flushed of the JIT's code with it) for a program that never writes the buffer.
use super::target;
use crate::lowmem::{screen_size, SCREEN_BASE};
use crate::machine::Mac;
use alloc::vec::Vec;
use heirloom_ui_abi::GuestUi;
use heirloom_raster::{Rect, Region};
use heirloom_mem_flat32::CODE_PAGE_SHIFT;

/// What the frame buffer held when it was last shown.
#[derive(Default)]
pub struct ScreenSync {
    shadow: Vec<u8>,
    warned_depth: bool,
    /// The buffer's pages' write-watch generations when it was last compared (empty: not watched yet).
    gens: Vec<u64>,
    /// Comparisons made (the watch's tests count them).
    pub compares: u64,
}

/// The screen's depth: the main device's, else a 1-bit Plus.
fn depth<U: GuestUi>(m: &Mac<U>) -> u8 { if m.cqd.main_device != 0 { crate::cqd::gdevice::SCREEN_DEPTH } else { 1 } }

pub fn sync<U: GuestUi>(m: &mut Mac<U>) {
    let d = depth(m);
    if d != 8 { return; } // 1-bit direct screen writes are not shown (named in the module note); no corpus program does it
    let (sw, sh) = screen_size(true); // an 8-bit screen is a colour machine's
    let bytes = sw as u32 * sh as u32;
    let pages = (SCREEN_BASE >> CODE_PAGE_SHIFT)..=((SCREEN_BASE + bytes - 1) >> CODE_PAGE_SHIFT);
    // nothing written since the last comparison: every page still at the generation the watch armed it at
    let gens = &m.qd.screen.gens;
    if !gens.is_empty() && pages.clone().zip(gens).all(|(p, &g)| m.mem.code_gen(p << CODE_PAGE_SHIFT) == g) { return; }
    compare(m, sw, sh, bytes);
    // arm the watch for the next step: any write to the buffer from now on moves one of these generations
    m.qd.screen.gens = pages.map(|p| m.mem.watch_code_page(p << CODE_PAGE_SHIFT)).collect();
}

/// Compare the buffer with the shadow and copy the changed pixels into the windows over them (the body of [`sync`]).
fn compare<U: GuestUi>(m: &mut Mac<U>, sw: i16, sh: i16, bytes: u32) {
    m.qd.screen.compares += 1;
    let Ok(span) = m.mem.span(SCREEN_BASE, bytes) else { return };
    let Ok(now) = m.mem.bytes(span) else { return };
    if m.qd.screen.shadow.is_empty() { m.qd.screen.shadow = alloc::vec![0; bytes as usize]; }
    if m.qd.screen.shadow.as_slice() == now { return; }
    let now = now.to_vec();
    let w = sw as usize;
    // the changed pixels' bounding box
    let (mut x0, mut y0, mut x1, mut y1) = (w, sh as usize, 0usize, 0usize);
    for y in 0..sh as usize {
        let (a, b) = (&m.qd.screen.shadow[y * w..(y + 1) * w], &now[y * w..(y + 1) * w]);
        if a == b { continue; }
        let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap_or(0);
        let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap_or(0);
        x0 = x0.min(first); x1 = x1.max(last + 1); y0 = y0.min(y); y1 = y1.max(y + 1);
    }
    let dirty = Rect::new(x0 as i32, y0 as i32, x1 as i32, y1 as i32);
    let mut claimed = Region::new();
    let order: Vec<u32> = m.z.clone();
    for win in order {
        let Some(w_) = m.windows.get(&win) else { continue };
        if !w_.visible { continue; }
        let wr = Rect::new(i32::from(w_.rect.left), i32::from(w_.rect.top), i32::from(w_.rect.right), i32::from(w_.rect.bottom));
        let inter = wr.intersect(&dirty);
        if inter.is_empty() { continue; }
        let mut todo = Region::from_rect(inter);
        todo.clear_region(&claimed);
        claimed.or_rect(inter);
        if todo.is_empty() { continue; }
        if m.qd.canvases.get(&win).map(|c| c.depth()) != Some(8) {
            if !m.qd.screen.warned_depth {
                m.qd.screen.warned_depth = true;
                m.log.push(alloc::format!("[quickdraw] direct screen write under window {win:#x}: its canvas is not 8 bits deep, so the write is not shown (a named limit)"));
            }
            continue;
        }
        let Ok(mut t) = target::open(m, win) else { continue };
        let mut touched = Rect::default();
        for r in todo.rects() {
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let i = y as usize * w + x as usize;
                    if m.qd.screen.shadow[i] != now[i] { t.bm.set(x - wr.x0, y - wr.y0, u32::from(now[i])); touched = touched.union(&Rect::new(x - wr.x0, y - wr.y0, x - wr.x0 + 1, y - wr.y0 + 1)); }
                }
            }
        }
        let _ = target::finish(m, t, touched);
    }
    m.qd.screen.shadow = now;
}
