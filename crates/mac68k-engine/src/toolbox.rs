//! Toolbox handlers bridged onto GuestUi: the Init* family (accepted, logged), NewWindow, DisposeWindow, ExitToShell.
//! Signatures are Inside Macintosh Pascal; the convention itself lives in [`crate::pascal`].
use alloc::format;
use alloc::string::String;
use crate::machine::{Call, Done, Mac, MacWindow};
use crate::pascal::{read_pstring, read_rect, Arg};
use crate::Refusal;
use guestui_abi::{CallbackToken, GuestRect, GuestUi, WindowModel, WindowRole};
use retro_m68k::Cpu;

/// sizeof(WindowRecord) (a GrafPort's 108 bytes + the window fields), Inside Macintosh. Offsets used below.
pub const WINDOW_RECORD_SIZE: u32 = 156;
const WR_VISIBLE: u32 = 110;
const WR_GO_AWAY: u32 = 112;
const WR_REF_CON: u32 = 152;

fn done(log: String) -> Result<Done, Refusal> { Ok(Done { result: 0, log }) }

/// Install every implemented trap.
pub fn register<U: GuestUi>(m: &mut Mac<U>) -> Result<(), Refusal> {
    // PROCEDURE InitGraf(globalPtr: Ptr); InitDialogs(resumeProc: ProcPtr): one pointer each, accepted.
    for t in [0xA86E, 0xA97B] { m.register(t, &[Arg::L], None, accept::<U>)?; }
    // PROCEDURE InitFonts; InitWindows; InitMenus; TEInit; InitCursor.
    for t in [0xA8FE, 0xA912, 0xA930, 0xA9CC, 0xA850] { m.register(t, &[], None, accept::<U>)?; }
    // FUNCTION NewWindow(wStorage: Ptr; boundsRect: Rect; title: Str255; visible: BOOLEAN; theProc: INTEGER;
    //                    behind: WindowPtr; goAwayFlag: BOOLEAN; refCon: LONGINT): WindowPtr
    m.register(0xA913, &[Arg::L, Arg::L, Arg::L, Arg::B, Arg::W, Arg::L, Arg::B, Arg::L], Some(Arg::L), new_window::<U>)?;
    // PROCEDURE DisposeWindow(theWindow: WindowPtr)
    m.register(0xA914, &[Arg::L], None, dispose_window::<U>)?;
    // PROCEDURE ExitToShell
    m.register(0xA9F4, &[], None, exit_to_shell::<U>)
}

fn accept<U: GuestUi>(_: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    done(format!("[toolbox] {} (accepted)", c.name))
}

fn exit_to_shell<U: GuestUi>(_: &mut Mac<U>, cpu: &mut Cpu, _: &Call) -> Result<Done, Refusal> {
    cpu.halt();
    done("[toolbox] ExitToShell".into())
}

/// documentProc (0) and zoomDocProc (8) have a grow box; noGrowDocProc (4) / zoomNoGrow (12) are documents without
/// one; every other proc id (dBoxProc 1, plainDBox 2, altDBoxProc 3, movableDBoxProc 5, ...) is a dialog frame.
fn role_of(proc_id: u16) -> (WindowRole, bool) {
    match proc_id & 7 { 0 => (WindowRole::Document, true), 4 => (WindowRole::Document, false), _ => (WindowRole::Dialog, false) }
}

fn new_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let (storage, rect_p, title_p) = (c.args.get(0), c.args.get(1), c.args.get(2));
    let (visible, proc_id, go_away, ref_con) = (c.args.get(3) as u8 != 0, c.args.get(4) as u16, c.args.get(6) as u8 != 0, c.args.get(7));
    let r = read_rect(&m.mem, rect_p)?;
    let title = read_pstring(&m.mem, title_p)?;
    let (w, h) = (i32::from(r.right) - i32::from(r.left), i32::from(r.bottom) - i32::from(r.top));
    if w <= 0 || h <= 0 { return Err(Refusal(format!("NewWindow: empty boundsRect ({},{},{},{})", r.top, r.left, r.bottom, r.right))); }
    let (win, owned) = if storage != 0 { (storage, false) } else {
        (m.heap.alloc(WINDOW_RECORD_SIZE).ok_or_else(|| Refusal("NewWindow: no memory for a WindowRecord (NOMEM)".into()))?, true)
    };
    let free_on_err = |m: &mut Mac<U>| if owned { let _ = m.heap.free(win, WINDOW_RECORD_SIZE); };
    let fill = (|| {
        let e = Refusal::mem("WindowRecord");
        m.mem.zero(win, WINDOW_RECORD_SIZE).map_err(&e)?;
        m.mem.write_u8(win + WR_VISIBLE, visible as u8).map_err(&e)?;
        m.mem.write_u8(win + WR_GO_AWAY, go_away as u8).map_err(&e)?;
        m.mem.write_u32(win + WR_REF_CON, ref_con).map_err(&e)
    })();
    if let Err(e) = fill { free_on_err(m); return Err(e); }
    let (role, resizable) = role_of(proc_id);
    let model = WindowModel {
        title: title.clone(), rect: GuestRect { left: i32::from(r.left), top: i32::from(r.top), width: w, height: h },
        role, closable: go_away, resizable, token: CallbackToken(u64::from(win)),
    };
    let native = match m.ui.open_window(model) { Ok(n) => n, Err(e) => { free_on_err(m); return Err(Refusal::ui(e)); } };
    m.windows.insert(win, MacWindow { native, owned });
    Ok(Done { result: win, log: format!("[toolbox] NewWindow \"{title}\" top={} left={} w={w} h={h} proc={proc_id} -> {win:#x}", r.top, r.left) })
}

fn dispose_window<U: GuestUi>(m: &mut Mac<U>, _: &mut Cpu, c: &Call) -> Result<Done, Refusal> {
    let win = c.args.get(0);
    let w = m.windows.remove(&win).ok_or_else(|| Refusal(format!("DisposeWindow: {win:#x} is not an open window")))?;
    m.ui.close_window(w.native).map_err(Refusal::ui)?;
    if w.owned { m.heap.free(win, WINDOW_RECORD_SIZE).map_err(|e| Refusal(format!("DisposeWindow: {e}")))?; }
    m.pending.retain(|&(_, msg)| msg != win);
    done(format!("[toolbox] DisposeWindow {win:#x}"))
}
