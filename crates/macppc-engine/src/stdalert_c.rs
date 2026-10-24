//! `StandardAlert(alertType, error, explanation, param, &itemHit)`: the Appearance Manager's alert, ASKED of the host as the native kit alert (`GuestUi::request`, kind from `alertType`),
//! never drawn. `AlertStdAlertParamRec` (Dialogs.h) is 68k-ALIGNED (2-byte packed), as MEASURED from Warcraft III World Editor's own record: movable 0, helpButton 1, filterProc 2, defaultText 6, cancelText 10, otherText 14, defaultButton 18, cancelButton 20, position 22 (a PowerPC natural-alignment guess read -1 as 0xffff0000).
//! A NULL `param` is one OK button. In `param` a NULL cancel/other text means "no such button" and `(StringPtr)-1` the standard title (OK, Cancel, Don't Save -- the template's own wording for Other is not in the spec); item 1 is always shown and item 4 is Help. The item numbers returned are the
//! Dialogs.h ones: 1 default (OK), 2 cancel, 3 other. Buttons go left to right as the Platinum alert lays them out: other, cancel, default. The message is the `error` line, then the
//! `explanation` line. Ignored, said in the log: `movable`, `position`, a non-NIL `filterProc` (a guest callback the glue cannot run mid-call).
//! Spec: macos9-specs/AppearanceLib/StandardAlert (validation, -50, the NULL/-1 title rules, *itemHit = -1 up front).
//! The call PARKS the program (`GlueOutcome::Idle`, like `ModalDialog`) until the host answers; a host with no alert part refuses by name (there is no `DITL` to fall back to).
use crate::machine::MacPpc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use guestui_abi::{GuestUi, RequestKind, RequestModel, UiRefusal};
use mac68k_engine::translate::alerts::Asked;

impl<U: GuestUi> MacPpc<U> {
    fn sa_pstr(&self, p: u32, what: &str) -> Result<String, String> {
        let n = self.mac.mem.read_u8(p).map_err(|_| format!("StandardAlert: the {what} string at {p:#x} leaves guest RAM"))?;
        Ok(mac68k_engine::menu::mac_text(&self.get_bytes(p + 1, u32::from(n), "StandardAlert")?))
    }
    fn sa_button(&self, ptr: u32, standard: &str, what: &str) -> Result<Option<String>, String> {
        Ok(match ptr { 0 => None, 0xFFFF_FFFF => Some(standard.into()), p => Some(self.sa_pstr(p, what)?) })
    }

    /// First call: ask the host. Second (retried) call: the answer. `Ok(None)` = still parked; `Ok(Some(OSErr))` = done.
    pub(crate) fn standard_alert(&mut self, a: &[u32]) -> Result<Option<u32>, String> {
        let (kind, err_p, expl_p, param, hit_p) = (a[0] as u16, a[1], a[2], a[3], a[4]);
        if self.rt.std_alert {
            let Some(asked) = self.mac.tr.dlg.asked.as_ref() else { return Err("StandardAlert: the parked alert's request is gone".into()) };
            let Some(item) = asked.answer else { return Ok(None) };
            self.mac.tr.dlg.asked = None;
            self.rt.std_alert = false;
            if hit_p != 0 { self.mac.mem.write_u16(hit_p, item as u16).map_err(|_| format!("StandardAlert: itemHit {hit_p:#x} leaves guest RAM"))?; }
            return Ok(Some(0));
        }
        // spec (macos9-specs/AppearanceLib/StandardAlert): *itemHit is -1 at once, before validation; an alert type outside 0..3 is a plain alert with no icon, not an error
        if hit_p != 0 { self.mac.mem.write_u16(hit_p, 0xFFFF).map_err(|_| format!("StandardAlert: itemHit {hit_p:#x} leaves guest RAM"))?; }
        let kind = match kind { 0 => RequestKind::Stop, 1 => RequestKind::Note, 2 => RequestKind::Caution, _ => RequestKind::Unspecified };
        let mut text = if err_p != 0 { self.sa_pstr(err_p, "error")? } else { String::new() };
        if expl_p != 0 { let e = self.sa_pstr(expl_p, "explanation")?; if !text.is_empty() && !e.is_empty() { text.push('\n'); } text.push_str(&e); }
        // the parameter record: NULL = the built-in default (OK only)
        let (mut help, mut dt, mut ct, mut ot, mut db, mut cb) = (false, 0xFFFF_FFFFu32, 0u32, 0u32, 1i16, 0i16);
        if param != 0 {
            let rd32 = |o: u32| self.mac.mem.read_u32(param + o).map_err(|_| format!("StandardAlert: the parameter record at {param:#x} leaves guest RAM"));
            let rd16 = |o: u32| self.mac.mem.read_u16(param + o).map_err(|_| format!("StandardAlert: the parameter record at {param:#x} leaves guest RAM"));
            if rd32(2)? != 0 { self.mac.log.push("[alert] StandardAlert: filterProc is a guest callback and is not called".into()); }
            help = self.mac.mem.read_u8(param + 1).unwrap_or(0) != 0;
            (dt, ct, ot) = (rd32(6)?, rd32(10)?, rd32(14)?);
            (db, cb) = (rd16(18)? as i16, rd16(20)? as i16);
        }
        // validation, all paramErr (-50) before anything is created
        if !(1..=3).contains(&db) || !(0..=3).contains(&cb) { return Ok(Some((-50i32) as u32)); }
        if (db == 2 && ct == 0) || (db == 3 && ot == 0) { db = 0; }
        if (cb == 3 && ot == 0) || (cb == 2 && ct == 0) { cb = 0; }
        if cb == db { return Ok(Some((-50i32) as u32)); }
        // buttons: item 1 always (NULL or -1 = the standard title), 2 and 3 only when their text is non-NULL (-1 = standard), 4 = Help
        let mut items: Vec<(i16, String)> = Vec::new();
        items.push((1, self.sa_button(if dt == 0 { 0xFFFF_FFFF } else { dt }, "OK", "default button")?.unwrap_or_default()));
        if let Some(t) = self.sa_button(ct, "Cancel", "cancel button")? { items.push((2, t)); }
        if let Some(t) = self.sa_button(ot, "Don't Save", "other button")? { items.push((3, t)); }
        if help { items.push((4, "Help".into())); }
        items.sort_by_key(|(n, _)| match n { 4 => 0, 3 => 1, 2 => 2, _ => 3 }); // left to right: help, other, cancel, default
        let find = |want: i16| items.iter().position(|(n, _)| *n == want && want != 0).map(|i| i as u32);
        let model = RequestModel { title: String::new(), text, buttons: items.iter().map(|(_, t)| t.clone()).collect(), default: find(db), cancel: find(cb), kind };
        let shown = model.text.replace('\n', " / ");
        match self.mac.ui.request(None, model) {
            Ok(req) => {
                self.mac.tr.dlg.asked = Some(Asked { req, items: items.iter().map(|(n, _)| *n).collect(), slot: 0, answer: None });
                self.rt.std_alert = true;
                self.rt.wait_dialog = true;
                self.mac.log.push(format!("[alert] StandardAlert asked of the host ({kind:?}, \"{shown}\"); the caller waits for its answer"));
                Ok(None)
            }
            Err(UiRefusal::Unsupported(why)) => Err(format!("StandardAlert: the host cannot ask ({why}); a standard alert has no DITL to show as a dialog")),
            Err(e) => Err(format!("StandardAlert: {e:?}")),
        }
    }
}
