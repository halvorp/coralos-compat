//! A [`GuestUi`] that records every call. Test double and headless runner backend.
use alloc::vec::Vec;
use alloc::collections::VecDeque;
use alloc::collections::BTreeMap;
use crate::*;


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCall {
    OpenWindow(WindowModel),
    CloseWindow(UiObjectRef),
    SetWindowTitle(UiObjectRef, alloc::string::String),
    SetMenuStrip(UiObjectRef, MenuModel),
    ClearMenuStrip(UiObjectRef),
    AddControl(UiObjectRef, ControlModel),
    RemoveControl(UiObjectRef, UiObjectRef),
    SetControl(UiObjectRef, UiObjectRef, ControlValue),
}

#[derive(Default)]
pub struct RecordingGuestUi {
    pub calls: Vec<UiCall>,
    /// Events queued by tests; handed out by `poll_event` in order.
    pub events: VecDeque<GuestUiEvent>,
    next_id: u32,
    /// live object id -> generation
    live: BTreeMap<u32, u32>,
}

impl RecordingGuestUi {
    pub fn new() -> Self { Self::default() }
    fn alloc(&mut self) -> UiObjectRef {
        self.next_id += 1;
        self.live.insert(self.next_id, 1);
        UiObjectRef { id: self.next_id, generation: 1 }
    }
    fn check(&self, r: UiObjectRef) -> Result<(), UiRefusal> {
        match self.live.get(&r.id) {
            None => Err(UiRefusal::UnknownObject(r)),
            Some(g) if *g != r.generation => Err(UiRefusal::StaleGeneration(r)),
            Some(_) => Ok(()),
        }
    }
}

impl GuestUi for RecordingGuestUi {
    fn open_window(&mut self, model: WindowModel) -> Result<UiObjectRef, UiRefusal> {
        if model.rect.width <= 0 || model.rect.height <= 0 {
            return Err(UiRefusal::InvalidGeometry("window width/height must be positive"));
        }
        self.calls.push(UiCall::OpenWindow(model));
        Ok(self.alloc())
    }
    fn close_window(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.live.remove(&win.id);
        self.calls.push(UiCall::CloseWindow(win));
        Ok(())
    }
    fn set_window_title(&mut self, win: UiObjectRef, title: alloc::string::String) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::SetWindowTitle(win, title));
        Ok(())
    }
    fn set_menu_strip(&mut self, win: UiObjectRef, menu: MenuModel) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::SetMenuStrip(win, menu));
        Ok(())
    }
    fn clear_menu_strip(&mut self, win: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.calls.push(UiCall::ClearMenuStrip(win));
        Ok(())
    }
    fn add_control(&mut self, win: UiObjectRef, control: ControlModel) -> Result<UiObjectRef, UiRefusal> {
        self.check(win)?;
        if let ControlKind::Slider { min, max, value } = &control.kind {
            if min > max || value < min || value > max {
                return Err(UiRefusal::InvalidGeometry("slider range"));
            }
        }
        self.calls.push(UiCall::AddControl(win, control));
        Ok(self.alloc())
    }
    fn set_control(&mut self, win: UiObjectRef, control: UiObjectRef, value: ControlValue) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        self.calls.push(UiCall::SetControl(win, control, value));
        Ok(())
    }
    fn remove_control(&mut self, win: UiObjectRef, control: UiObjectRef) -> Result<(), UiRefusal> {
        self.check(win)?;
        self.check(control)?;
        self.live.remove(&control.id);
        self.calls.push(UiCall::RemoveControl(win, control));
        Ok(())
    }
    fn poll_event(&mut self) -> Option<GuestUiEvent> { self.events.pop_front() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win() -> WindowModel {
        WindowModel { title: "T".into(), rect: GuestRect { left: 0, top: 0, width: 10, height: 10 },
            role: WindowRole::Document, closable: true, resizable: true, token: CallbackToken(1) }
    }
    fn ctl(kind: ControlKind) -> ControlModel {
        ControlModel { kind, label: "x".into(), rect: GuestRect { left: 1, top: 1, width: 5, height: 5 },
            enabled: true, token: CallbackToken(2) }
    }

    #[test]
    fn records_in_order_and_hands_back_events() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        let kinds = [ControlKind::Button, ControlKind::Checkbox { checked: true },
            ControlKind::String { text: "a".into(), max_len: 8 }, ControlKind::Slider { min: 0, max: 10, value: 3 }];
        for k in kinds { ui.add_control(w, ctl(k)).unwrap(); }
        ui.set_menu_strip(w, MenuModel::default()).unwrap();
        ui.set_window_title(w, "New".into()).unwrap();
        ui.close_window(w).unwrap();
        assert_eq!(ui.calls.len(), 8);
        assert!(matches!(ui.calls[0], UiCall::OpenWindow(_)));
        assert_eq!(ui.calls[6], UiCall::SetWindowTitle(w, "New".into()));
        assert!(matches!(ui.calls[7], UiCall::CloseWindow(_)));
        ui.events.push_back(GuestUiEvent::CloseRequested { win: w, token: CallbackToken(1) });
        assert!(ui.poll_event().is_some() && ui.poll_event().is_none());
    }

    #[test]
    fn stale_and_unknown_objects_refused_by_name() {
        let mut ui = RecordingGuestUi::new();
        let w = ui.open_window(win()).unwrap();
        ui.close_window(w).unwrap();
        assert_eq!(ui.close_window(w), Err(UiRefusal::UnknownObject(w)));
        assert_eq!(ui.set_window_title(w, "x".into()), Err(UiRefusal::UnknownObject(w)));
        assert_eq!(ui.clear_menu_strip(UiObjectRef { id: 99, generation: 1 }), Err(UiRefusal::UnknownObject(UiObjectRef { id: 99, generation: 1 })));
    }

    #[test]
    fn bad_geometry_and_slider_refused() {
        let mut ui = RecordingGuestUi::new();
        let mut m = win();
        m.rect.width = 0;
        assert!(matches!(ui.open_window(m), Err(UiRefusal::InvalidGeometry(_))));
        let w = ui.open_window(win()).unwrap();
        assert!(ui.add_control(w, ctl(ControlKind::Slider { min: 5, max: 1, value: 3 })).is_err());
        assert_eq!(ui.calls.len(), 1, "refused calls are not recorded");
    }
}
