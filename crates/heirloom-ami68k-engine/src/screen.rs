//! intuition.library public screens: LockPubScreen / UnlockPubScreen over ONE guest `struct Screen`, the "Workbench
//! screen" a CoralOS guest sees. Programs read its geometry, bar height, border sizes and font to lay out their own
//! windows, so these fields are real; the rest of the 346 bytes (ViewPort, RastPort, LayerInfo) is zero until a
//! program that reads them shows up in the census.
//!
//! Values: the AROS m68k oracle's Workbench screen (tests/oracle/screen.bin). They are the guest's view only; the
//! native window the guest's OpenWindow becomes is sized by CoralOS (guestui), not by these numbers.
use alloc::string::String;
use alloc::format;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use heirloom_mem_flat32::GuestMem;
use crate::cpu::Cpu;

pub const LVO_LOCK_PUB_SCREEN: i32 = -510;
pub const LVO_UNLOCK_PUB_SCREEN: i32 = -516;

pub const SCREEN: u32 = 0x6400;
const SCREEN_SIZE: u32 = 346;
const TEXT_ATTR: u32 = 0x6560;
const FONT_NAME: u32 = 0x6570;
const TITLE: u32 = 0x6590;

/// Lay out the Workbench screen (offsets: AROS intuition/screens.h, m68k ABI).
pub fn setup(mem: &mut GuestMem) -> Result<(), Refusal> {
    let m = Refusal::mem;
    for a in SCREEN..SCREEN + SCREEN_SIZE { mem.write_u8(a, 0).map_err(m("Screen"))?; }
    mem.write_u16(SCREEN + 12, 640).map_err(m("Screen.Width"))?;
    mem.write_u16(SCREEN + 14, 256).map_err(m("Screen.Height"))?;
    for (off, v) in [(30, 14u8), (31, 1), (32, 5), (33, 2), (34, 4), (35, 6), (36, 4), (37, 4), (38, 4)] {
        mem.write_u8(SCREEN + off, v).map_err(m("Screen bar/border sizes"))?;
    }
    for (i, b) in b"guteform.font\0".iter().enumerate() { mem.write_u8(FONT_NAME + i as u32, *b).map_err(m("font name"))?; }
    for (i, b) in b"Workbench Screen\0".iter().enumerate() { mem.write_u8(TITLE + i as u32, *b).map_err(m("screen title"))?; }
    mem.write_u32(TEXT_ATTR, FONT_NAME).map_err(m("ta_Name"))?;
    mem.write_u16(TEXT_ATTR + 4, heirloom_theme::uifont::Y_SIZE).map_err(m("ta_YSize"))?;
    mem.write_u32(SCREEN + 22, TITLE).map_err(m("Screen.Title"))?;
    mem.write_u32(SCREEN + 26, TITLE).map_err(m("Screen.DefaultTitle"))?;
    mem.write_u32(SCREEN + 40, TEXT_ATTR).map_err(m("Screen.Font"))?;
    // IntuitionBase.ActiveScreen / FirstScreen (readable between LockIBase and UnlockIBase): the one screen. Offsets: Library 34 + View 18 +
    // ActiveWindow = 52 (m68k, -mno-align-int, intuition/intuitionbase.h). Programs seed their preferences from ActiveScreen (Jano's Editor).
    mem.write_u32(crate::machine::INTUITION_BASE + 56, SCREEN).map_err(m("IntuitionBase.ActiveScreen"))?;
    mem.write_u32(crate::machine::INTUITION_BASE + 60, SCREEN).map_err(m("IntuitionBase.FirstScreen"))?;
    Ok(())
}

/// LockPubScreen(a0 = name or NULL) -> the screen. NULL and "Workbench" are the one public screen; any other name is
/// a legitimate NULL (no such public screen), as on AmigaOS.
pub fn lock_pub_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let p = cpu.areg(0);
    let name = if p == 0 { None } else { Some(a.mem.read_cstr(p).map_err(Refusal::mem("LockPubScreen.name"))?) };
    let s = match name.as_deref() { None | Some("Workbench") => SCREEN, Some(_) => 0 };
    if s != 0 { a.screen_locks += 1; }
    cpu.set_d0(s);
    Ok(format!("[intuition] LockPubScreen {:?} -> {s:#x}", name.unwrap_or_else(|| "NULL".into())))
}

/// UnlockPubScreen(a0 = name, a1 = screen).
pub fn unlock_pub_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let s = cpu.areg(1);
    if s == 0 { return Ok("[intuition] UnlockPubScreen NULL".into()); }
    if s != SCREEN { return Err(Refusal(format!("UnlockPubScreen {s:#x}: not the public screen"))); }
    a.screen_locks = a.screen_locks.checked_sub(1).ok_or_else(|| Refusal("UnlockPubScreen without LockPubScreen".into()))?;
    Ok(format!("[intuition] UnlockPubScreen (locks {})", a.screen_locks))
}

pub fn register<U: GuestUi>(b: crate::libtable::LibraryBuilder<U>) -> crate::libtable::LibraryBuilder<U> {
    b.named("OpenScreen", open_screen::<U>)
        .named("OpenScreenTagList", open_screen_tag_list::<U>)
        .named("CloseScreen", close_screen::<U>)
        .named("AllocScreenBuffer", alloc_screen_buffer::<U>)
        .named("FreeScreenBuffer", free_screen_buffer::<U>)
        .named("ChangeScreenBuffer", change_screen_buffer::<U>)
        .named("LockPubScreenList", lock_pub_screen_list::<U>)
        .named("UnlockPubScreenList", unlock_pub_screen_list::<U>)
        .named("LockIBase", lock_ibase::<U>)
        .named("UnlockIBase", unlock_ibase::<U>)
        .named("ScreenToFront", |a, c| screen_depth(a, c, "ScreenToFront"))
        .named("ScreenToBack", |a, c| screen_depth(a, c, "ScreenToBack"))
        .named("WBenchToFront", |_, c| { c.set_d0(1); Ok("[intuition] WBenchToFront -> TRUE".into()) })
        .named("WBenchToBack", |_, c| { c.set_d0(1); Ok("[intuition] WBenchToBack -> TRUE".into()) })
        .named("CurrentTime", current_time::<U>)
        .named("QueryOverscan", query_overscan::<U>)
}

const SA_DUMMY: u32 = crate::tags::TAG_USER + 32;
const SA_WIDTH: u32 = SA_DUMMY + 3;
const SA_HEIGHT: u32 = SA_DUMMY + 4;
const SA_DEPTH: u32 = SA_DUMMY + 5;
const SA_BITMAP: u32 = SA_DUMMY + 15;
const SA_DISPLAY_ID: u32 = SA_DUMMY + 18;
const SB_SIZE: u32 = 16;

fn open_screen_common<U: GuestUi>(a: &mut Amiga<U>, w: u16, h: u16, depth: u8, custom_bm: u32) -> Result<u32, Refusal> {
    use crate::graphics::layout::*;
    let m = Refusal::mem;
    let sc = a.heap.alloc(SCREEN_SIZE).ok_or_else(|| Refusal("OpenScreen: out of heap".into()))?;
    a.mem.zero(sc, SCREEN_SIZE).map_err(m("Screen"))?;
    a.mem.write_u16(sc + 12, w).map_err(m("Screen.Width"))?;
    a.mem.write_u16(sc + 14, h).map_err(m("Screen.Height"))?;
    a.mem.write_u32(sc + 40, TEXT_ATTR).map_err(m("Screen.Font"))?;
    // ViewPort at +SC_VIEWPORT: ColorMap +4, DWidth +24, DHeight +26
    a.mem.write_u32(sc + SC_VIEWPORT + VP_COLORMAP, SCREEN_COLORMAP).map_err(m("Screen.ViewPort.ColorMap"))?;
    a.mem.write_u16(sc + SC_VIEWPORT + VP_DWIDTH, w).map_err(m("Screen.ViewPort.DWidth"))?;
    a.mem.write_u16(sc + SC_VIEWPORT + VP_DHEIGHT, h).map_err(m("Screen.ViewPort.DHeight"))?;
    // BitMap at +SC_BITMAP (or custom_bm)
    let bm_addr = if custom_bm != 0 { custom_bm } else { sc + SC_BITMAP };
    if custom_bm == 0 {
        let bpr = ((u32::from(w) + 15) / 16) * 2;
        a.mem.write_u16(bm_addr + BM_BYTES_PER_ROW, bpr as u16).map_err(m("Screen.BitMap.BytesPerRow"))?;
        a.mem.write_u16(bm_addr + BM_ROWS, h).map_err(m("Screen.BitMap.Rows"))?;
        a.mem.write_u8(bm_addr + BM_DEPTH, depth.max(1)).map_err(m("Screen.BitMap.Depth"))?;
        let bm = heirloom_raster::Bitmap::new(i32::from(w), i32::from(h), depth.max(1))
            .ok_or_else(|| Refusal(format!("OpenScreen: invalid size {w}x{h}")))?;
        a.gfx.bitmaps.insert(bm_addr, crate::graphics::state::GBitmap { bm, window: None });
    }
    // RastPort at +SC_RASTPORT
    a.mem.write_u32(sc + SC_RASTPORT + RP_BITMAP, bm_addr).map_err(m("Screen.RastPort.BitMap"))?;
    a.mem.write_u8(sc + SC_RASTPORT + RP_MASK, 0xFF).map_err(m("Screen.RastPort.Mask"))?;
    a.mem.write_u8(sc + SC_RASTPORT + RP_FG_PEN, 1).map_err(m("Screen.RastPort.FgPen"))?;
    a.mem.write_u32(sc + SC_RASTPORT + RP_FONT, GF_FONT).map_err(m("Screen.RastPort.Font"))?;
    a.custom_screens.insert(sc, (w, h, bm_addr));
    Ok(sc)
}

fn open_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let ns = cpu.areg(0);
    let m = Refusal::mem("NewScreen");
    let w = a.mem.read_u16(ns + 4).map_err(&m)?.max(1);
    let h = a.mem.read_u16(ns + 6).map_err(&m)?.max(1);
    let d = a.mem.read_u16(ns + 8).map_err(&m)? as u8;
    let sc = open_screen_common(a, w, h, d, 0)?;
    cpu.set_d0(sc);
    Ok(format!("[intuition] OpenScreen {w}x{h}x{d} -> {sc:#x}"))
}

fn open_screen_tag_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (ns, tag_list) = (cpu.areg(0), cpu.areg(1));
    let (mut w, mut h, mut d) = (320u16, 200u16, 8u8);
    if ns != 0 {
        let m = Refusal::mem("NewScreen");
        w = a.mem.read_u16(ns + 4).map_err(&m)?.max(1);
        h = a.mem.read_u16(ns + 6).map_err(&m)?.max(1);
        d = a.mem.read_u16(ns + 8).map_err(&m)? as u8;
    }
    let t = crate::tags::walk(&a.mem, tag_list)?;
    if let Some(id) = crate::tags::get(&t, SA_DISPLAY_ID) {
        if crate::cybergraphics::is_cyber_mode_id(id) {
            let mode = a.cyber.mode_info(id);
            w = mode.width as u16;
            h = mode.height as u16;
            d = mode.depth as u8;
        }
    }
    if let Some(v) = crate::tags::get(&t, SA_WIDTH).filter(|&v| v > 0 && v <= 4096) { w = v as u16; }
    if let Some(v) = crate::tags::get(&t, SA_HEIGHT).filter(|&v| v > 0 && v <= 4096) { h = v as u16; }
    if let Some(v) = crate::tags::get(&t, SA_DEPTH).filter(|&v| v > 0 && v <= 32) { d = v as u8; }
    let custom_bm = crate::tags::get(&t, SA_BITMAP).unwrap_or(0);
    let sc = open_screen_common(a, w, h, d, custom_bm)?;
    cpu.set_d0(sc);
    Ok(format!("[intuition] OpenScreenTagList {w}x{h}x{d} -> {sc:#x}"))
}

fn close_screen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let sc = cpu.areg(0);
    if let Some((_, _, bm)) = a.custom_screens.remove(&sc) {
        if bm == sc + crate::graphics::layout::SC_BITMAP {
            a.gfx.bitmaps.remove(&bm);
        }
        let _ = a.heap.free(sc, SCREEN_SIZE);
        cpu.set_d0(1);
        return Ok(format!("[intuition] CloseScreen {sc:#x} -> TRUE"));
    }
    cpu.set_d0(0);
    Ok(format!("[intuition] CloseScreen {sc:#x} -> FALSE"))
}

fn alloc_screen_buffer<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    use crate::graphics::layout::*;
    let (sc, bm_in, flags) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0));
    let (w, h, sc_bm) = a.custom_screens.get(&sc).copied().unwrap_or((640, 256, sc + SC_BITMAP));
    let sb = a.heap.alloc(SB_SIZE).ok_or_else(|| Refusal("AllocScreenBuffer: out of heap".into()))?;
    a.mem.zero(sb, SB_SIZE).map_err(Refusal::mem("ScreenBuffer"))?;
    let bm = if bm_in != 0 {
        bm_in
    } else if flags & 1 != 0 {
        // SB_SCREEN_BITMAP
        sc_bm
    } else {
        let b = a.heap.alloc(BM_SIZE).ok_or_else(|| Refusal("AllocScreenBuffer BitMap: out of heap".into()))?;
        a.mem.zero(b, BM_SIZE).map_err(Refusal::mem("ScreenBuffer.BitMap"))?;
        let bpr = ((u32::from(w) + 15) / 16) * 2;
        a.mem.write_u16(b + BM_BYTES_PER_ROW, bpr as u16).map_err(Refusal::mem("bm_BytesPerRow"))?;
        a.mem.write_u16(b + BM_ROWS, h).map_err(Refusal::mem("bm_Rows"))?;
        a.mem.write_u8(b + BM_DEPTH, 8).map_err(Refusal::mem("bm_Depth"))?;
        let bm_obj = heirloom_raster::Bitmap::new(i32::from(w), i32::from(h), 8)
            .ok_or_else(|| Refusal(format!("AllocScreenBuffer: invalid size {w}x{h}")))?;
        a.gfx.bitmaps.insert(b, crate::graphics::state::GBitmap { bm: bm_obj, window: None });
        b
    };
    let dbi = a.heap.alloc(64).ok_or_else(|| Refusal("AllocScreenBuffer DBufInfo: out of heap".into()))?;
    a.mem.zero(dbi, 64).map_err(Refusal::mem("DBufInfo"))?;
    a.mem.write_u32(sb, bm).map_err(Refusal::mem("sb_BitMap"))?;
    a.mem.write_u32(sb + 4, dbi).map_err(Refusal::mem("sb_DBufInfo"))?;
    cpu.set_d0(sb);
    Ok(format!("[intuition] AllocScreenBuffer sc={sc:#x} flags={flags:#x} -> {sb:#x} (bm={bm:#x})"))
}

fn free_screen_buffer<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let sb = cpu.areg(1);
    if sb != 0 {
        let dbi = a.mem.read_u32(sb + 4).unwrap_or(0);
        if dbi != 0 { let _ = a.heap.free(dbi, 64); }
        let _ = a.heap.free(sb, SB_SIZE);
    }
    Ok(format!("[intuition] FreeScreenBuffer {sb:#x}"))
}

fn change_screen_buffer<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (sc, sb) = (cpu.areg(0), cpu.areg(1));
    let bm = a.mem.read_u32(sb).map_err(Refusal::mem("sb_BitMap"))?;
    let dbi = a.mem.read_u32(sb + 4).map_err(Refusal::mem("sb_DBufInfo"))?;
    // Copy the buffer's pixels into any window belonging to this screen and mark it damaged.
    let src_pixels = a.gfx.bitmaps.get(&bm).map(|g| (g.bm.width(), g.bm.height(), g.bm.pixels().to_vec()));
    if let Some((sw, sh, pix)) = src_pixels {
        let targets: alloc::vec::Vec<(u32, u32)> = a.gfx.layers.values()
            .filter(|l| a.mem.read_u32(l.window + crate::window::WD_WSCREEN).ok() == Some(sc))
            .map(|l| (l.window, l.canvas))
            .collect();
        for (win, canvas) in targets {
            if let Some(dst) = a.gfx.bitmaps.get_mut(&canvas) {
                let w = dst.bm.width().min(sw);
                let h = dst.bm.height().min(sh);
                for y in 0..h {
                    for x in 0..w {
                        dst.bm.set(x, y, u32::from(pix[(y * sw + x) as usize]));
                    }
                }
                a.gfx.add_damage(win, heirloom_raster::Rect::new(0, 0, w, h));
            }
        }
    }
    // Reply SafeMessage (offset 20 in DBufInfo -> dbi_SafeMessage) and DispMessage (+40) if reply ports set.
    if dbi != 0 {
        for msg_off in [20u32, 40u32] {
            let msg = dbi + msg_off;
            if let Ok(rp) = a.mem.read_u32(msg + 14) {
                if rp != 0 {
                    let _ = crate::exec_wait::put_msg(a, rp, msg, 7);
                }
            }
        }
    }
    cpu.set_d0(1);
    Ok(format!("[intuition] ChangeScreenBuffer sc={sc:#x} sb={sb:#x} bm={bm:#x} -> TRUE"))
}

/// CurrentTime(a0 = ULONG* seconds, a1 = ULONG* micros): the engine's virtual clock, matching timer.device/GetSysTime.
fn current_time<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (secs_ptr, micros_ptr) = (cpu.areg(0), cpu.areg(1));
    let (s, us) = ((a.clock_us / 1_000_000) as u32, (a.clock_us % 1_000_000) as u32);
    if secs_ptr != 0 { a.mem.write_u32(secs_ptr, s).map_err(Refusal::mem("CurrentTime.seconds"))?; }
    if micros_ptr != 0 { a.mem.write_u32(micros_ptr, us).map_err(Refusal::mem("CurrentTime.micros"))?; }
    Ok(format!("[intuition] CurrentTime -> {s}.{us:06}"))
}

/// QueryOverscan(a0 = displayid, a1 = Rectangle*, d0.w = oscantype) -> TRUE when the mode exists and the type is known
/// (rom/intuition/queryoverscan.c:73-105): the rectangle is the matching one of the mode's DimensionInfo (DTAG_DIMS), OSCAN_TEXT 1 ->
/// TxtOScan, STANDARD 2 -> StdOScan, MAX 3 -> MaxOScan, VIDEO 4 -> VideoOScan. An unknown mode or type answers FALSE and leaves the
/// rectangle alone. DimensionInfo offsets: MaxOScan 34, VideoOScan 42, TxtOScan 50, StdOScan 58 (graphics/modes.rs).
fn query_overscan<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (id, rect, ty) = (cpu.areg(0), cpu.areg(1), cpu.dreg(0) as u16);
    let Some(dims) = crate::graphics::modes::record(crate::graphics::modes::DTAG_DIMS, id).filter(|_| id == crate::graphics::modes::WORKBENCH_MODE) else {
        cpu.set_d0(0);
        return Ok(format!("[intuition] QueryOverscan mode {id:#x}: no such mode -> FALSE"));
    };
    let off = match ty { 1 => 50, 2 => 58, 3 => 34, 4 => 42, _ => { cpu.set_d0(0); return Ok(format!("[intuition] QueryOverscan type {ty}: not OSCAN_TEXT/STANDARD/MAX/VIDEO -> FALSE")); } };
    a.mem.write_bytes(rect, &dims[off..off + 8]).map_err(Refusal::mem("QueryOverscan rectangle"))?;
    cpu.set_d0(1);
    Ok(format!("[intuition] QueryOverscan mode {id:#x} type {ty} -> TRUE"))
}

/// ScreenToFront / ScreenToBack(a0 = screen): there is one screen, so its depth order cannot change; the call is accepted for
/// that screen and a named refusal for any other pointer.
fn screen_depth<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu, name: &str) -> Result<String, Refusal> {
    let s = cpu.areg(0);
    if s != SCREEN && !a.custom_screens.contains_key(&s) { return Err(Refusal(format!("{name} {s:#x}: not a live screen"))); }
    Ok(format!("[intuition] {name} {s:#x}"))
}

/// LockIBase(d0 = dontknow) -> a lock token for UnlockIBase. Tasks switch only after a library call, so the lock never has to
/// exclude anybody; it is counted so an UnlockIBase without a LockIBase is a named refusal.
fn lock_ibase<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    a.ibase_locks += 1;
    cpu.set_d0(a.ibase_locks);
    Ok(format!("[intuition] LockIBase -> token {}", a.ibase_locks))
}

fn unlock_ibase<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let token = cpu.areg(0);
    a.ibase_locks = a.ibase_locks.checked_sub(1).ok_or_else(|| Refusal("UnlockIBase without LockIBase".into()))?;
    Ok(format!("[intuition] UnlockIBase {token:#x}"))
}

/// LockPubScreenList() -> the list of public screens: a real List with the one PubScreenNode, "Workbench" (struct PubScreenNode,
/// NDK intuition/screens.h: Node 14 bytes, psn_Screen 14, psn_Flags 18, psn_Size 20, psn_VisitorCount 22, psn_SigTask 24,
/// psn_SigBit 28; 30 bytes) with psn_Flags 0, psn_VisitorCount = the current LockPubScreen count at
/// each call. Built on first use in the engine's arena.
fn lock_pub_screen_list<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    if a.pub_list == 0 {
        let m = Refusal::mem("PubScreenList");
        let blk = crate::fs::alloc_zeroed_sys(a, 14 + 30 + 16)?.ok_or_else(|| Refusal("LockPubScreenList: no memory (NOMEM)".into()))?;
        let (list, node, name) = (blk, blk + 14, blk + 44);
        a.mem.write_bytes(name, b"Workbench\0").map_err(&m)?;
        crate::execlists::new_list(&mut a.mem, list, 0)?;
        a.mem.write_u8(node + 8, 2).map_err(&m)?; // ln_Type NT_USER-ish: a plain node
        a.mem.write_u32(node + 10, name).map_err(&m)?;
        a.mem.write_u32(node + 14, SCREEN).map_err(&m)?;
        crate::execlists::add_tail(&mut a.mem, list, node)?;
        a.pub_list = list;
    }
    a.mem.write_u16(a.pub_list + 14 + 22, a.screen_locks as u16).map_err(Refusal::mem("psn_VisitorCount"))?;
    cpu.set_d0(a.pub_list);
    Ok(format!("[intuition] LockPubScreenList -> {:#x}", a.pub_list))
}

fn unlock_pub_screen_list<U: GuestUi>(_: &mut Amiga<U>, _: &mut Cpu) -> Result<String, Refusal> { Ok("[intuition] UnlockPubScreenList".into()) }
