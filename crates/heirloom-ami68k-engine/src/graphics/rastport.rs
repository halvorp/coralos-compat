//! RastPort state: the guest `struct RastPort` is the authority for pens, mode, cursor and font; the handlers read and
//! write it in place, so a program that pokes `rp->FgPen` directly (the macros in graphics/gfxmacros.h do) and one that
//! calls SetAPen agree.
use super::layout::*;
use crate::{machine::Amiga, tags, Refusal};
use alloc::format;
use alloc::string::String;
use heirloom_ui_abi::GuestUi;
use heirloom_cpu_68k::Cpu;
use heirloom_raster::paint::Brush;

/// The fields of a RastPort the drawing handlers use.
#[derive(Debug, Clone, Copy)]
pub struct Rp {
    pub fg: u8, pub bg: u8, pub aol: u8, pub mode: u8, pub mask: u8,
    pub cp: (i32, i32),
    pub font: u32,
    pub algo: u8,
    pub area_ptrn: u32, pub area_pt_sz: i8,
    pub line_ptrn: u16, pub linpatcnt: u8, pub flags: u16,
}

impl Rp {
    pub fn brush(&self) -> Brush { Brush { fg: self.fg, bg: self.bg, mode: self.mode & 7, mask: self.mask } }
}

pub fn read<U: GuestUi>(a: &Amiga<U>, rp: u32) -> Result<Rp, Refusal> {
    let m = Refusal::mem("RastPort");
    let u8_ = |o| a.mem.read_u8(rp + o).map_err(&m);
    Ok(Rp {
        fg: u8_(RP_FG_PEN)?, bg: u8_(RP_BG_PEN)?, aol: u8_(RP_AOL_PEN)?, mode: u8_(RP_DRAW_MODE)?, mask: u8_(RP_MASK)?,
        cp: (a.mem.read_i16(rp + RP_CP_X).map_err(&m)? as i32, a.mem.read_i16(rp + RP_CP_Y).map_err(&m)? as i32),
        font: a.mem.read_u32(rp + RP_FONT).map_err(&m)?, algo: u8_(RP_ALGO_STYLE)?,
        area_ptrn: a.mem.read_u32(rp + RP_AREA_PTRN).map_err(&m)?, area_pt_sz: u8_(RP_AREA_PT_SZ)? as i8,
        line_ptrn: a.mem.read_u16(rp + RP_LINE_PTRN).map_err(&m)?, linpatcnt: u8_(RP_LINPATCNT)?, flags: a.mem.read_u16(rp + RP_FLAGS).map_err(&m)?,
    })
}

pub fn set_cp<U: GuestUi>(a: &mut Amiga<U>, rp: u32, x: i32, y: i32) -> Result<(), Refusal> {
    let m = Refusal::mem("RastPort.cp");
    a.mem.write_u16(rp + RP_CP_X, x as u16).map_err(&m)?;
    a.mem.write_u16(rp + RP_CP_Y, y as u16).map_err(&m)
}

fn put8<U: GuestUi>(a: &mut Amiga<U>, rp: u32, off: u32, v: u8) -> Result<(), Refusal> { a.mem.write_u8(rp + off, v).map_err(Refusal::mem("RastPort")) }

/// Make `rp` a freshly initialised RastPort (InitRastPort's defaults, as AROS sets them): full write mask, FgPen/AOlPen
/// all-ones, JAM2, a solid line pattern, the default font.
pub fn init<U: GuestUi>(a: &mut Amiga<U>, rp: u32) -> Result<(), Refusal> {
    a.mem.zero(rp, RP_SIZE).map_err(Refusal::mem("InitRastPort"))?;
    put8(a, rp, RP_MASK, 0xFF)?; put8(a, rp, RP_FG_PEN, 0xFF)?; put8(a, rp, RP_AOL_PEN, 0xFF)?; put8(a, rp, RP_DRAW_MODE, JAM2)?;
    a.mem.write_u16(rp + RP_LINE_PTRN, 0xFFFF).map_err(Refusal::mem("RastPort"))?;
    for i in 0..8 { put8(a, rp, RP_MINTERMS + i, 0)?; }
    set_font(a, rp, 0)
}

/// SetFont: Font, TxWidth, TxHeight, TxBaseline from the font (NULL = the default font).
pub fn set_font<U: GuestUi>(a: &mut Amiga<U>, rp: u32, font: u32) -> Result<(), Refusal> {
    let tf = if font == 0 { super::fonts::default_font(a)? } else { font };
    let m = Refusal::mem("SetFont");
    let (x, y, b) = (a.mem.read_u16(tf + TF_X_SIZE).map_err(&m)?, a.mem.read_u16(tf + TF_Y_SIZE).map_err(&m)?, a.mem.read_u16(tf + TF_BASELINE).map_err(&m)?);
    a.mem.write_u32(rp + RP_FONT, tf).map_err(&m)?;
    a.mem.write_u16(rp + RP_TX_WIDTH, x).map_err(&m)?;
    a.mem.write_u16(rp + RP_TX_HEIGHT, y).map_err(&m)?;
    a.mem.write_u16(rp + RP_TX_BASELINE, b).map_err(&m)
}

pub fn init_rast_port<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let rp = cpu.areg(1);
    init(a, rp)?;
    Ok(format!("[graphics] InitRastPort {rp:#x}"))
}

pub fn set_a_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, pen) = (cpu.areg(1), cpu.dreg(0) as u8);
    put8(a, rp, RP_FG_PEN, pen)?;
    Ok(format!("[graphics] SetAPen {rp:#x} {pen}"))
}
pub fn set_b_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, pen) = (cpu.areg(1), cpu.dreg(0) as u8);
    put8(a, rp, RP_BG_PEN, pen)?;
    Ok(format!("[graphics] SetBPen {rp:#x} {pen}"))
}
pub fn set_dr_md<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, md) = (cpu.areg(1), cpu.dreg(0) as u8);
    put8(a, rp, RP_DRAW_MODE, md)?;
    Ok(format!("[graphics] SetDrMd {rp:#x} {md}"))
}
pub fn set_abpen_drmd<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, ap, bp, md) = (cpu.areg(1), cpu.dreg(0) as u8, cpu.dreg(1) as u8, cpu.dreg(2) as u8);
    put8(a, rp, RP_FG_PEN, ap)?; put8(a, rp, RP_BG_PEN, bp)?; put8(a, rp, RP_DRAW_MODE, md)?;
    Ok(format!("[graphics] SetABPenDrMd {rp:#x} a={ap} b={bp} mode={md}"))
}
pub fn get_a_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let v = read(a, cpu.areg(0))?.fg; cpu.set_d0(v as u32); Ok(format!("[graphics] GetAPen -> {v}"))
}
pub fn get_b_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let v = read(a, cpu.areg(0))?.bg; cpu.set_d0(v as u32); Ok(format!("[graphics] GetBPen -> {v}"))
}
pub fn get_dr_md<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let v = read(a, cpu.areg(0))?.mode; cpu.set_d0(v as u32); Ok(format!("[graphics] GetDrMd -> {v}"))
}
pub fn get_outline_pen<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let v = read(a, cpu.areg(0))?.aol; cpu.set_d0(v as u32); Ok(format!("[graphics] GetOutlinePen -> {v}"))
}
pub fn set_write_mask<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, m) = (cpu.areg(0), cpu.dreg(0) as u8);
    put8(a, rp, RP_MASK, m)?;
    cpu.set_d0(1);
    Ok(format!("[graphics] SetWriteMask {rp:#x} {m:#x}"))
}
pub fn set_max_pen<U: GuestUi>(_a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    Ok(format!("[graphics] SetMaxPen {:#x} {} (accepted: pens are host-side, no remap pass)", cpu.areg(0), cpu.dreg(0)))
}

pub fn move_<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, x, y) = (cpu.areg(1), sx(cpu.dreg(0)), sx(cpu.dreg(1)));
    move_to(a, rp, x, y)?;
    Ok(format!("[graphics] Move {rp:#x} {x},{y}"))
}

/// `Move`: the cursor, the line-pattern counter back to its first bit and FRST_DOT.
pub fn move_to<U: GuestUi>(a: &mut Amiga<U>, rp: u32, x: i32, y: i32) -> Result<(), Refusal> {
    set_cp(a, rp, x, y)?;
    put8(a, rp, RP_LINPATCNT, 15)?;
    let f = a.mem.read_u16(rp + RP_FLAGS).map_err(Refusal::mem("RastPort.Flags"))?;
    a.mem.write_u16(rp + RP_FLAGS, f | FRST_DOT).map_err(Refusal::mem("RastPort.Flags"))
}

// RPTAG_ (graphics/rpattr.h)
const RPTAG_FONT: u32 = 0x8000_0000;
const RPTAG_APEN: u32 = 0x8000_0002;
const RPTAG_BPEN: u32 = 0x8000_0003;
const RPTAG_DRMD: u32 = 0x8000_0004;
const RPTAG_OUTLINE_PEN: u32 = 0x8000_0005;
const RPTAG_WRITE_MASK: u32 = 0x8000_0006;
const RPTAG_MAX_PEN: u32 = 0x8000_0007;

pub fn set_rp_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, t) = (cpu.areg(0), tags::walk(&a.mem, cpu.areg(1))?);
    for &(tag, d) in &t {
        match tag {
            RPTAG_FONT => set_font(a, rp, d)?,
            RPTAG_APEN => put8(a, rp, RP_FG_PEN, d as u8)?,
            RPTAG_BPEN => put8(a, rp, RP_BG_PEN, d as u8)?,
            RPTAG_DRMD => put8(a, rp, RP_DRAW_MODE, d as u8)?,
            RPTAG_OUTLINE_PEN => put8(a, rp, RP_AOL_PEN, d as u8)?,
            RPTAG_WRITE_MASK => put8(a, rp, RP_MASK, d as u8)?,
            RPTAG_MAX_PEN => {}
            other => return Err(Refusal(format!("SetRPAttrsA: tag {other:#x} is not implemented"))),
        }
    }
    Ok(format!("[graphics] SetRPAttrsA {rp:#x} ({} tags)", t.len()))
}

pub fn get_rp_attrs<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (rp, t) = (cpu.areg(0), tags::walk(&a.mem, cpu.areg(1))?);
    let r = read(a, rp)?;
    for &(tag, ptr) in &t {
        let v = match tag {
            RPTAG_FONT => r.font, RPTAG_APEN => r.fg as u32, RPTAG_BPEN => r.bg as u32, RPTAG_DRMD => r.mode as u32,
            RPTAG_OUTLINE_PEN => r.aol as u32, RPTAG_WRITE_MASK => r.mask as u32, RPTAG_MAX_PEN => 255,
            other => return Err(Refusal(format!("GetRPAttrsA: tag {other:#x} is not implemented"))),
        };
        a.mem.write_u32(ptr, v).map_err(Refusal::mem("GetRPAttrsA result"))?;
    }
    Ok(format!("[graphics] GetRPAttrsA {rp:#x} ({} tags)", t.len()))
}
