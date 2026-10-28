//! `muimaster.library/MUI_RequestA` as a NATIVE alert on the requester seam (docs/AMIGA-TRANSLATION.md). Zune (mui_request.c, APL, read for
//! facts only) builds a Window of a Text and one SimpleButton per `|`-separated gadget label and runs the Application's own loop until a
//! button sends a ReturnID; here the same request is ONE `GuestUi::request` and the calling task parks until the host answers.
//! (`MUI_Request`, the varargs form, is amiga.lib/mui.lib glue INSIDE the guest that ends in this vector.)
//! * Arguments: d0 = app, d1 = win (a MUI Window object: `MUIA_Window_RefWindow`), d2 = flags (unused by Zune), a0 = title, a1 = gadgets,
//!   a2 = format (RawDoFmt, a3 = its argument stream).
//! * No Application: Zune hands the request to `EasyRequestArgs(NULL, {title, format, gadgets})`, so the return is Intuition's ([`requester::button_value`]);
//!   with one it is [`requester::mui_value`]: 1..n-1 left to right, the rightmost 0, and a lone button 1.
//! * A NULL title is the Application's `MUIA_Application_Title`.
//! * The body is a MUI Text: its `ESC` sequences (`ESC b/i/u/n/c/l/r`, pens `ESC 2..8`, `ESC P[..]`/`ESC I[..]` images) are STRIPPED, because the
//!   native alert has one plain-text body ([`super::roles::strip_format`], the same stripping every MUI Text gets); alignment is the host's.
//! * A gadget label is a MUI button: a leading `*` marks the DEFAULT button (Zune's `MUIA_Window_ActiveObject`) and the first `_` the hotkey
//!   character; the native alert shows neither (it has its own Return/Escape and mnemonics), so both are removed. Zune's no-Application path
//!   would show them raw through Intuition; the native alert never does.
//! * Zune has no Escape: the host's cancel is the LAST button, which returns 0 (what a headless run answers).
//! * `win` is the window the alert is modal to; an unopened or NULL one asks without a parent (Zune: centred on the screen).
//! * Zune keeps running the Application's loop while it waits; here only the calling task parks, the others keep running, and the
//!   Application's own events are queued as before. At most 32 buttons (Zune's `req_but[32]`): more is a named refusal, not Zune's overflow.
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use super::classes::Kind;
use super::ids_gen::MUIA_Application_Title;
use super::roles::strip_format;
use crate::requester;
use crate::{machine::Amiga, Refusal};
use heirloom_ui_abi::GuestUi;
use crate::cpu::Cpu;

const MAX_BUTTONS: usize = 32;
const MAX_STRING: usize = 8192;

fn latin1(b: &[u8]) -> String { b.iter().map(|&c| c as char).collect() }

/// The labels of a gadget string and the index of the default button: split at `|`, the leading `*` removed, the `_` hotkey marker removed.
fn labels(gadgets: &str) -> (Vec<String>, Option<u32>) {
    let mut default = None;
    let v = gadgets.split('|').enumerate().map(|(i, l)| {
        let l = match l.strip_prefix('*') { Some(r) => { default = Some(i as u32); r } None => l };
        strip_format(l, true) // the button's Text: ESC sequences go, the first '_' marks its hotkey
    }).collect();
    (v, default)
}

/// MUI_RequestA(d0 = app, d1 = win, d2 = flags, a0 = title, a1 = gadgets, a2 = format, a3 = params) -> the pressed button's number.
pub(super) fn mui_request_a<U: GuestUi>(a: &mut Amiga<U>, cpu: &mut Cpu) -> Result<String, Refusal> {
    let (app, win, title_p, gad_p, fmt_p, params) = (cpu.dreg(0), cpu.dreg(1), cpu.areg(0), cpu.areg(1), cpu.areg(2), cpu.areg(3));
    if app != 0 && a.objects.kind(app) != Some(Kind::Application) { return Err(Refusal(format!("MUI_RequestA: {app:#x} is not a MUI Application"))); }
    let ui_win = if win == 0 { None } else {
        if a.objects.kind(win) != Some(Kind::Window) { return Err(Refusal(format!("MUI_RequestA: the reference window {win:#x} is not a MUI Window object"))); }
        super::win::native_of(a, win)
    };
    let rd = |a: &Amiga<U>, p: u32, what: &'static str| crate::guest_str::read_bytes_z(&a.mem, p, MAX_STRING).map_err(Refusal::mem(what));
    let gadgets = latin1(&rd(a, gad_p, "MUI_RequestA gadgets")?);
    let (buttons, default) = labels(&gadgets);
    if buttons.len() > MAX_BUTTONS { return Err(Refusal(format!("MUI_RequestA: {} buttons (Zune's requester holds {MAX_BUTTONS})", buttons.len()))); }
    let body = crate::fmt::format(&a.mem, &rd(a, fmt_p, "MUI_RequestA format")?, params, false)?;
    let text = strip_format(&latin1(&body.bytes), false);
    let title = if title_p != 0 { latin1(&rd(a, title_p, "MUI_RequestA title")?) } else if app != 0 { a.objects.text(app, MUIA_Application_Title).into() } else { "System Request".into() }; // EasyRequestArgs' own fallback title
    let n = buttons.len() as u32;
    let m = heirloom_ui_abi::RequestModel { title, text, buttons, default, cancel: Some(n - 1), kind: heirloom_ui_abi::RequestKind::Unspecified };
    let what = format!("[muimaster] MUI_RequestA {:?} {:?} {:?}", m.title, m.text, m.buttons);
    // Without an Application the answer is Intuition's (EasyRequestArgs); with one, Zune's own ReturnID arithmetic.
    let kind = if app == 0 { requester::Kind::Easy } else { requester::Kind::Mui };
    requester::ask_sync(a, cpu, ui_win, m, kind, (0, 0), what)
}
