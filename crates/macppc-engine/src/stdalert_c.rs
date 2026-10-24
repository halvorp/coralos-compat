//! `StandardAlert(alertType, error, explanation, param, &itemHit)`: the Appearance Manager's alert, ASKED of the host as the native kit alert (`GuestUi::request`, kind from `alertType`),
//! never drawn. `AlertStdAlertParamRec` (Dialogs.h): movable 0, helpButton 1, filterProc 4, defaultText 8, cancelText 12, otherText 16, defaultButton 20, cancelButton 22, position 24.
//! A NULL `param` is one OK button. In `param` a NULL text pointer means "no such button" and `(StringPtr)-1` the standard title (OK, Cancel, Don't Save). The item numbers returned are the
//! Dialogs.h ones: 1 default (OK), 2 cancel, 3 other. Buttons go left to right as the Platinum alert lays them out: other, cancel, default. The message is the `error` line, then the
//! `explanation` line. Ignored, said in the log: `movable`, `position`, a non-NIL `filterProc` (a guest callback the glue cannot run mid-call), `helpButton` (no help item is added).
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
        let kind = match kind { 0 => RequestKind::Stop, 1 => RequestKind::Note, 2 => RequestKind::Caution, 3 => RequestKind::Unspecified, k => return Err(format!("StandardAlert: alert type {k} is not one of the four (stop, note, caution, plain)")) };
        let mut text = if err_p != 0 { self.sa_pstr(err_p, "error")? } else { String::new() };
        if expl_p != 0 { let e = self.sa_pstr(expl_p, "explanation")?; if !text.is_empty() && !e.is_empty() { text.push('\n'); } text.push_str(&e); }
        // (item number, title, is default, is cancel)
        let mut items: Vec<(i16, String)> = Vec::new();
        let (mut default_item, mut cancel_item) = (1i16, 0i16);
        if param == 0 { items.push((1, "OK".into())); } else {
            let rd32 = |o: u32| self.mac.mem.read_u32(param + o).map_err(|_| format!("StandardAlert: the parameter record at {param:#x} leaves guest RAM"));
            let rd16 = |o: u32| self.mac.mem.read_u16(param + o).map_err(|_| format!("StandardAlert: the parameter record at {param:#x} leaves guest RAM"));
            if rd32(4)? != 0 { self.mac.log.push("[alert] StandardAlert: filterProc is a guest callback and is not called".into()); }
            if self.mac.mem.read_u8(param + 1).unwrap_or(0) != 0 { self.mac.log.push("[alert] StandardAlert: helpButton is not modelled (no help item added)".into()); }
            for (item, off, std) in [(1i16, 8u32, "OK"), (2, 12, "Cancel"), (3, 16, "Don't Save")] {
                if let Some(t) = self.sa_button(rd32(off)?, std, "button")? { items.push((item, t)); }
            }
            default_item = rd16(20)? as i16;
            cancel_item = rd16(22)? as i16;
        }
        if items.is_empty() { return Err("StandardAlert: the parameter record names no button".into()); }
        items.sort_by_key(|(n, _)| match n { 3 => 0, 2 => 1, _ => 2 }); // left to right: other, cancel, default
        let find = |want: i16| items.iter().position(|(n, _)| *n == want && want != 0).map(|i| i as u32);
        let model = RequestModel { title: String::new(), text, buttons: items.iter().map(|(_, t)| t.clone()).collect(), default: find(default_item), cancel: find(cancel_item), kind };
        match self.mac.ui.request(None, model) {
            Ok(req) => {
                self.mac.tr.dlg.asked = Some(Asked { req, items: items.iter().map(|(n, _)| *n).collect(), slot: 0, answer: None });
                self.rt.std_alert = true;
                self.rt.wait_dialog = true;
                self.mac.log.push("[alert] StandardAlert asked of the host; the caller waits for its answer".into());
                Ok(None)
            }
            Err(UiRefusal::Unsupported(why)) => Err(format!("StandardAlert: the host cannot ask ({why}); a standard alert has no DITL to show as a dialog")),
            Err(e) => Err(format!("StandardAlert: {e:?}")),
        }
    }
}
